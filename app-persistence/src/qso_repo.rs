use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use logbook_domain::{
    AwardCredit, AwardQso, CreditStatus, DedupKey, FieldProvenance, ImportBatch, ImportedCredit,
    LocationDetail, LocationFilter, PendingUploadCount, QsoRepository, QsoSearch,
    QsoServiceState, QsoSummary, RepoResult, RepositoryError, SyncBatch, SyncCandidate,
};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// Type-preserving bind for the filter builder. We can't just collect
/// `Vec<String>` because dxcc_id is bound as i64 and binding it as text
/// would silently change query semantics on numeric comparisons.
enum DynBind {
    Text(String),
    Int(i64),
}

/// Build the WHERE clause + binds for a QsoSearch. Used by search_qsos,
/// search_full_qsos, and count_matching so they share filter semantics.
/// Caller adds SELECT prefix, ORDER BY, and LIMIT.
fn build_filter(query: &QsoSearch) -> (String, Vec<DynBind>) {
    let mut sql = String::new();
    let mut binds: Vec<DynBind> = Vec::new();

    if let Some(prefix) = &query.call_prefix {
        sql.push_str(" AND q.call LIKE ?");
        binds.push(DynBind::Text(format!("{}%", prefix.to_ascii_uppercase())));
    }
    if let Some(call) = &query.exact_call {
        sql.push_str(" AND q.call = ?");
        binds.push(DynBind::Text(call.as_str().to_string()));
    }
    if let Some(band) = query.band {
        sql.push_str(" AND q.band = ?");
        binds.push(DynBind::Text(band.as_adif().to_string()));
    }
    if let Some(mode) = &query.mode {
        sql.push_str(" AND q.mode = ?");
        binds.push(DynBind::Text(mode.as_adif().to_string()));
    }
    if let Some(dxcc_id) = query.dxcc_id {
        sql.push_str(" AND q.dxcc_id = ?");
        binds.push(DynBind::Int(dxcc_id as i64));
    }
    match query.station_location {
        LocationFilter::Any => {}
        LocationFilter::Unassigned => sql.push_str(" AND q.station_location_id IS NULL"),
        LocationFilter::Is(loc_id) => {
            sql.push_str(" AND q.station_location_id = ?");
            binds.push(DynBind::Text(fmt_id(loc_id.as_uuid())));
        }
    }
    if let Some(from) = &query.from {
        sql.push_str(" AND q.qso_begin >= ?");
        binds.push(DynBind::Text(fmt_dt(from)));
    }
    if let Some(to) = &query.to {
        sql.push_str(" AND q.qso_begin <= ?");
        binds.push(DynBind::Text(fmt_dt(to)));
    }
    if let Some(state) = &query.state {
        sql.push_str(" AND q.state = ?");
        binds.push(DynBind::Text(state.clone()));
    }
    if let Some(iota) = &query.iota {
        sql.push_str(" AND q.iota = ?");
        binds.push(DynBind::Text(iota.clone()));
    }
    if let Some(continent) = &query.continent {
        sql.push_str(" AND q.continent = ?");
        binds.push(DynBind::Text(continent.clone()));
    }
    match query.lotw_confirmed {
        Some(true) => sql.push_str(" AND s.confirmation_state = 'confirmed'"),
        Some(false) => sql.push_str(" AND (s.id IS NULL OR s.confirmation_state != 'confirmed')"),
        None => {}
    }
    if query.differs_from_lotw {
        sql.push_str(
            " AND EXISTS (SELECT 1 FROM qso_field_provenance p \
             WHERE p.qso_id = q.id AND p.source = ?)",
        );
        binds.push(DynBind::Text(
            FieldSource::ServiceDiffers("lotw".into()).as_storage_str(),
        ));
    }

    (sql, binds)
}
use radio_core::{
    Band, Callsign, FieldSource, Mode, OperatorId, PropagationMode, Qso, QsoExchangeField, QsoId,
    StationLocation, StationLocationId,
};

use crate::db::Database;
use crate::station_repo::{insert_location_sql, row_to_location};

/// The `qsos` columns a record writes, other than `id` and the
/// timestamps, in the order `bind_qso_columns` binds them. Shared by the
/// single-row insert, the update and the import batch so the three can't
/// drift apart.
const QSO_COLUMNS: &[&str] = &[
    "call",
    "qso_begin",
    "qso_end",
    "band",
    "freq_hz",
    "mode",
    "submode",
    "rst_sent",
    "rst_rcvd",
    "operator_id",
    "station_location_id",
    "station_callsign",
    "owner_callsign",
    "operator_callsign",
    "dxcc_id",
    "dxcc_prefix",
    "wpx_prefix",
    "continent",
    "cq_zone",
    "itu_zone",
    "grid",
    "state",
    "county",
    "province",
    "iota",
    "latitude",
    "longitude",
    "name",
    "qth",
    "address",
    "email",
    "age",
    "qsl_via",
    "comment",
    "contest_id",
    "serial_sent",
    "serial_rcvd",
    "exchange_sent",
    "exchange_rcvd",
    "arrl_section",
    "band_rx",
    "freq_rx_hz",
    "tx_power_w",
    "rx_power_w",
    "propagation_mode",
    "sat_name",
    "sat_mode",
    "distance_km",
    "bearing_deg",
    "sfi",
    "a_index",
    "k_index",
];

static INSERT_QSO_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "INSERT INTO qsos (id, {}, created_at, updated_at) VALUES (?, {}, ?, ?)",
        QSO_COLUMNS.join(", "),
        vec!["?"; QSO_COLUMNS.len()].join(", "),
    )
});

static UPDATE_QSO_SQL: LazyLock<String> = LazyLock::new(|| {
    let sets: Vec<String> = QSO_COLUMNS.iter().map(|c| format!("{c} = ?")).collect();
    format!("UPDATE qsos SET {}, updated_at = ? WHERE id = ?", sets.join(", "))
});

type SqliteQuery<'q> = sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>;

/// Bind `q`'s columns in `QSO_COLUMNS` order.
fn bind_qso_columns<'q>(query: SqliteQuery<'q>, q: &'q Qso) -> SqliteQuery<'q> {
    let call = |c: &Option<Callsign>| c.as_ref().map(|c| c.as_str().to_string());
    query
        .bind(q.call.as_str())
        .bind(fmt_dt(&q.qso_begin))
        .bind(q.qso_end.as_ref().map(fmt_dt))
        .bind(q.band.map(|b| b.as_adif().to_string()))
        .bind(q.freq_hz)
        .bind(q.mode.as_ref().map(|m| m.as_adif().to_string()))
        .bind(&q.submode)
        .bind(&q.rst_sent)
        .bind(&q.rst_rcvd)
        .bind(q.operator_id.map(|id| fmt_id(id.as_uuid())))
        .bind(q.station_location_id.map(|id| fmt_id(id.as_uuid())))
        .bind(call(&q.station_callsign))
        .bind(call(&q.owner_callsign))
        .bind(call(&q.operator_callsign))
        .bind(q.dxcc_id.map(|v| v as i64))
        .bind(&q.dxcc_prefix)
        .bind(&q.wpx_prefix)
        .bind(&q.continent)
        .bind(q.cq_zone.map(|v| v as i64))
        .bind(q.itu_zone.map(|v| v as i64))
        .bind(&q.grid)
        .bind(&q.state)
        .bind(&q.county)
        .bind(&q.province)
        .bind(&q.iota)
        .bind(q.latitude)
        .bind(q.longitude)
        .bind(&q.name)
        .bind(&q.qth)
        .bind(&q.address)
        .bind(&q.email)
        .bind(q.age.map(|v| v as i64))
        .bind(&q.qsl_via)
        .bind(&q.comment)
        .bind(&q.contest_id)
        .bind(q.serial_sent)
        .bind(q.serial_rcvd)
        .bind(&q.exchange_sent)
        .bind(&q.exchange_rcvd)
        .bind(&q.arrl_section)
        .bind(q.band_rx.map(|b| b.as_adif().to_string()))
        .bind(q.freq_rx_hz)
        .bind(q.tx_power_w.map(|v| v as f64))
        .bind(q.rx_power_w.map(|v| v as f64))
        .bind(q.propagation_mode.as_ref().map(propagation_to_str))
        .bind(&q.sat_name)
        .bind(&q.sat_mode)
        .bind(q.distance_km)
        .bind(q.bearing_deg)
        .bind(q.sfi.map(|v| v as i64))
        .bind(q.a_index.map(|v| v as i64))
        .bind(q.k_index.map(|v| v as i64))
}

/// `INSERT INTO qsos` for `q`, ready to execute.
fn insert_qso_query(q: &Qso) -> SqliteQuery<'_> {
    let query = sqlx::query(INSERT_QSO_SQL.as_str()).bind(fmt_id(q.id.as_uuid()));
    bind_qso_columns(query, q)
        .bind(fmt_dt(&q.created_at))
        .bind(fmt_dt(&q.updated_at))
}

/// JSON string literal for `s`. `award_credit_imports.raw_json` holds a
/// two-key object; building it by hand saves a serde_json dependency.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn credit_raw_json(credit: &AwardCredit) -> String {
    format!(
        "{{\"medium\":{},\"status\":{}}}",
        credit.medium.as_deref().map(json_str).unwrap_or_else(|| "null".into()),
        json_str(credit.status.as_storage_str()),
    )
}

/// Award credit rows for `credits`: an `award_credit_imports` row per
/// credit, linked to its QSO by an `award_credit_links` row, creating
/// each award's `award_definitions` row on first sight. Shared by ADIF
/// import and service sync.
async fn insert_credits(
    conn: &mut sqlx::SqliteConnection,
    credits: &[ImportedCredit],
    now: &str,
) -> RepoResult<()> {
    if credits.is_empty() {
        return Ok(());
    }
    // One award_definitions row per award name, created on first sight;
    // the definition itself (rules, units) isn't known from an ADIF
    // credit list, hence the empty JSON.
    let mut names: Vec<&str> = credits.iter().map(|c| c.credit.award.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        sqlx::query(
            r#"
            INSERT INTO award_definitions
                (id, name, version, definition_json, created_at, updated_at)
            VALUES (?, ?, '1', '{}', ?, ?)
            ON CONFLICT(name) DO NOTHING
            "#,
        )
        .bind(fmt_id(Uuid::new_v4()))
        .bind(name)
        .bind(now)
        .bind(now)
        .execute(&mut *conn)
        .await
        .map_err(map_err)?;
    }
    let award_ids: HashMap<String, String> = sqlx::query("SELECT id, name FROM award_definitions")
        .fetch_all(&mut *conn)
        .await
        .map_err(map_err)?
        .iter()
        .map(|row| Ok((row.try_get("name")?, row.try_get("id")?)))
        .collect::<Result<_, sqlx::Error>>()
        .map_err(map_err)?;

    for c in credits {
        let import_id = fmt_id(Uuid::new_v4());
        sqlx::query(
            r#"
            INSERT INTO award_credit_imports
                (id, award_id, call, qso_date, band, mode, unit,
                 raw_json, imported_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&import_id)
        .bind(&award_ids[&c.credit.award])
        .bind(&c.call)
        .bind(&c.qso_date)
        .bind(&c.band)
        .bind(&c.mode)
        .bind(&c.unit)
        .bind(credit_raw_json(&c.credit))
        .bind(now)
        .execute(&mut *conn)
        .await
        .map_err(map_err)?;
        sqlx::query(
            r#"
            INSERT INTO award_credit_links (id, credit_import_id, qso_id, link_status)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(fmt_id(Uuid::new_v4()))
        .bind(&import_id)
        .bind(fmt_id(c.qso_id.as_uuid()))
        .bind(c.credit.status.as_storage_str())
        .execute(&mut *conn)
        .await
        .map_err(map_err)?;
    }
    Ok(())
}

fn row_to_provenance(row: &sqlx::sqlite::SqliteRow) -> RepoResult<FieldProvenance> {
    let source: String = row.try_get("source").map_err(map_err)?;
    let created_at: String = row.try_get("created_at").map_err(map_err)?;
    Ok(FieldProvenance {
        field_name: row.try_get("field_name").map_err(map_err)?,
        source: FieldSource::parse_storage(&source),
        source_detail: row.try_get("source_detail").map_err(map_err)?,
        created_at: parse_dt(&created_at)?,
    })
}

/// Rows per `IN (...)` list: well under SQLite's bound-parameter limit.
const IN_CHUNK: usize = 500;

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

#[derive(Debug)]
pub struct SqliteQsoRepository {
    pool: SqlitePool,
}

impl SqliteQsoRepository {
    pub fn new(db: &Database) -> Self {
        Self { pool: db.pool().clone() }
    }
}

fn map_err(e: sqlx::Error) -> RepositoryError {
    match e {
        sqlx::Error::RowNotFound => RepositoryError::NotFound,
        sqlx::Error::Database(err) if err.is_unique_violation() || err.is_foreign_key_violation() => {
            RepositoryError::Constraint(err.to_string())
        }
        other => RepositoryError::Storage(other.to_string()),
    }
}

fn parse_callsign(s: &str) -> RepoResult<Callsign> {
    Callsign::parse(s).map_err(|e| RepositoryError::Storage(format!("bad callsign in db: {e}")))
}

fn parse_dt(s: &str) -> RepoResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| RepositoryError::Storage(format!("bad timestamp in db: {e}")))
}

fn fmt_dt(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

fn fmt_id(uuid: Uuid) -> String {
    uuid.to_string()
}

fn parse_uuid(s: &str) -> RepoResult<Uuid> {
    Uuid::parse_str(s).map_err(|e| RepositoryError::Storage(format!("bad uuid: {e}")))
}

fn propagation_to_str(p: &PropagationMode) -> String {
    match p {
        PropagationMode::Terrestrial => "terrestrial".into(),
        PropagationMode::Satellite => "satellite".into(),
        PropagationMode::Eme => "eme".into(),
        PropagationMode::MeteorScatter => "meteor_scatter".into(),
        PropagationMode::Aurora => "aurora".into(),
        PropagationMode::AircraftScatter => "aircraft_scatter".into(),
        PropagationMode::Other(s) => format!("other:{s}"),
    }
}

fn propagation_from_str(s: &str) -> PropagationMode {
    if let Some(rest) = s.strip_prefix("other:") {
        return PropagationMode::Other(rest.to_string());
    }
    match s {
        "terrestrial" => PropagationMode::Terrestrial,
        "satellite" => PropagationMode::Satellite,
        "eme" => PropagationMode::Eme,
        "meteor_scatter" => PropagationMode::MeteorScatter,
        "aurora" => PropagationMode::Aurora,
        "aircraft_scatter" => PropagationMode::AircraftScatter,
        other => PropagationMode::Other(other.to_string()),
    }
}

fn row_to_qso(row: &sqlx::sqlite::SqliteRow) -> RepoResult<Qso> {
    let id: String = row.try_get("id").map_err(map_err)?;
    let call: String = row.try_get("call").map_err(map_err)?;
    let qso_begin: String = row.try_get("qso_begin").map_err(map_err)?;
    let qso_end: Option<String> = row.try_get("qso_end").map_err(map_err)?;
    let band: Option<String> = row.try_get("band").map_err(map_err)?;
    let freq_hz: Option<i64> = row.try_get("freq_hz").map_err(map_err)?;
    let mode: Option<String> = row.try_get("mode").map_err(map_err)?;
    let submode: Option<String> = row.try_get("submode").map_err(map_err)?;
    let rst_sent: Option<String> = row.try_get("rst_sent").map_err(map_err)?;
    let rst_rcvd: Option<String> = row.try_get("rst_rcvd").map_err(map_err)?;
    let operator_id: Option<String> = row.try_get("operator_id").map_err(map_err)?;
    let station_location_id: Option<String> =
        row.try_get("station_location_id").map_err(map_err)?;
    let station_callsign: Option<String> = row.try_get("station_callsign").map_err(map_err)?;
    let owner_callsign: Option<String> = row.try_get("owner_callsign").map_err(map_err)?;
    let operator_callsign: Option<String> = row.try_get("operator_callsign").map_err(map_err)?;
    let dxcc_id: Option<i64> = row.try_get("dxcc_id").map_err(map_err)?;
    let dxcc_prefix: Option<String> = row.try_get("dxcc_prefix").map_err(map_err)?;
    let continent: Option<String> = row.try_get("continent").map_err(map_err)?;
    let cq_zone: Option<i64> = row.try_get("cq_zone").map_err(map_err)?;
    let itu_zone: Option<i64> = row.try_get("itu_zone").map_err(map_err)?;
    let grid: Option<String> = row.try_get("grid").map_err(map_err)?;
    let state: Option<String> = row.try_get("state").map_err(map_err)?;
    let county: Option<String> = row.try_get("county").map_err(map_err)?;
    let province: Option<String> = row.try_get("province").map_err(map_err)?;
    let iota: Option<String> = row.try_get("iota").map_err(map_err)?;
    let tx_power_w: Option<f64> = row.try_get("tx_power_w").map_err(map_err)?;
    let rx_power_w: Option<f64> = row.try_get("rx_power_w").map_err(map_err)?;
    let propagation_mode: Option<String> = row.try_get("propagation_mode").map_err(map_err)?;
    let sat_name: Option<String> = row.try_get("sat_name").map_err(map_err)?;
    let sat_mode: Option<String> = row.try_get("sat_mode").map_err(map_err)?;
    let distance_km: Option<f64> = row.try_get("distance_km").map_err(map_err)?;
    let bearing_deg: Option<f64> = row.try_get("bearing_deg").map_err(map_err)?;
    let text = |col: &str| -> RepoResult<Option<String>> { row.try_get(col).map_err(map_err) };
    let int = |col: &str| -> RepoResult<Option<i64>> { row.try_get(col).map_err(map_err) };
    let real = |col: &str| -> RepoResult<Option<f64>> { row.try_get(col).map_err(map_err) };
    let created_at: String = row.try_get("created_at").map_err(map_err)?;
    let updated_at: String = row.try_get("updated_at").map_err(map_err)?;

    Ok(Qso {
        id: QsoId::from_uuid(parse_uuid(&id)?),
        call: parse_callsign(&call)?,
        qso_begin: parse_dt(&qso_begin)?,
        qso_end: qso_end.as_deref().map(parse_dt).transpose()?,
        band: band.as_deref().and_then(Band::from_adif),
        freq_hz,
        mode: mode.as_deref().map(Mode::from_adif),
        submode,
        rst_sent,
        rst_rcvd,
        operator_id: operator_id
            .as_deref()
            .map(parse_uuid)
            .transpose()?
            .map(OperatorId::from_uuid),
        station_location_id: station_location_id
            .as_deref()
            .map(parse_uuid)
            .transpose()?
            .map(StationLocationId::from_uuid),
        station_callsign: station_callsign.as_deref().map(parse_callsign).transpose()?,
        owner_callsign: owner_callsign.as_deref().map(parse_callsign).transpose()?,
        operator_callsign: operator_callsign.as_deref().map(parse_callsign).transpose()?,
        dxcc_id: dxcc_id.map(|v| v as u16),
        dxcc_prefix,
        wpx_prefix: text("wpx_prefix")?,
        continent,
        cq_zone: cq_zone.map(|v| v as u8),
        itu_zone: itu_zone.map(|v| v as u8),
        grid,
        state,
        county,
        province,
        iota,
        latitude: real("latitude")?,
        longitude: real("longitude")?,
        name: text("name")?,
        qth: text("qth")?,
        address: text("address")?,
        email: text("email")?,
        age: int("age")?.map(|v| v as u16),
        qsl_via: text("qsl_via")?,
        comment: text("comment")?,
        contest_id: text("contest_id")?,
        serial_sent: int("serial_sent")?,
        serial_rcvd: int("serial_rcvd")?,
        exchange_sent: text("exchange_sent")?,
        exchange_rcvd: text("exchange_rcvd")?,
        arrl_section: text("arrl_section")?,
        band_rx: text("band_rx")?.as_deref().and_then(Band::from_adif),
        freq_rx_hz: int("freq_rx_hz")?,
        tx_power_w: tx_power_w.map(|v| v as f32),
        rx_power_w: rx_power_w.map(|v| v as f32),
        propagation_mode: propagation_mode.as_deref().map(propagation_from_str),
        sat_name,
        sat_mode,
        distance_km,
        bearing_deg,
        sfi: int("sfi")?.map(|v| v as u16),
        a_index: int("a_index")?.map(|v| v as u16),
        k_index: int("k_index")?.map(|v| v as u8),
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

#[async_trait]
impl QsoRepository for SqliteQsoRepository {
    async fn insert_qso(&self, qso: &Qso) -> RepoResult<()> {
        insert_qso_query(qso)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn update_qso(&self, qso: &Qso) -> RepoResult<()> {
        bind_qso_columns(sqlx::query(UPDATE_QSO_SQL.as_str()), qso)
            .bind(fmt_dt(&qso.updated_at))
            .bind(fmt_id(qso.id.as_uuid()))
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn get_qso(&self, id: &QsoId) -> RepoResult<Option<Qso>> {
        let row = sqlx::query("SELECT * FROM qsos WHERE id = ? AND deleted_at IS NULL")
            .bind(fmt_id(id.as_uuid()))
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        row.as_ref().map(row_to_qso).transpose()
    }

    async fn soft_delete_qso(&self, id: &QsoId) -> RepoResult<()> {
        let now = fmt_dt(&Utc::now());
        sqlx::query("UPDATE qsos SET deleted_at = ?, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(&now)
            .bind(fmt_id(id.as_uuid()))
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn add_exchange_field(
        &self,
        qso_id: &QsoId,
        field: &QsoExchangeField,
    ) -> RepoResult<()> {
        let id = Uuid::new_v4();
        let now = fmt_dt(&Utc::now());
        sqlx::query(
            r#"
            INSERT INTO qso_exchange_fields
                (id, qso_id, field_name, raw_value, normalized_value, source, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(fmt_id(id))
        .bind(fmt_id(qso_id.as_uuid()))
        .bind(&field.name)
        .bind(&field.raw_value)
        .bind(&field.normalized_value)
        .bind(field.source.as_storage_str())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn list_service_states(&self, qso_id: &QsoId) -> RepoResult<Vec<QsoServiceState>> {
        let rows = sqlx::query(
            r#"
            SELECT service, upload_state, confirmation_state,
                   uploaded_at, confirmed_at, last_error
            FROM qso_service_state
            WHERE qso_id = ?
            ORDER BY service
            "#,
        )
        .bind(fmt_id(qso_id.as_uuid()))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let uploaded_at: Option<String> = row.try_get("uploaded_at").map_err(map_err)?;
            let confirmed_at: Option<String> = row.try_get("confirmed_at").map_err(map_err)?;
            out.push(QsoServiceState {
                service: row.try_get("service").map_err(map_err)?,
                upload_state: row.try_get("upload_state").map_err(map_err)?,
                confirmation_state: row.try_get("confirmation_state").map_err(map_err)?,
                uploaded_at: uploaded_at.as_deref().map(parse_dt).transpose()?,
                confirmed_at: confirmed_at.as_deref().map(parse_dt).transpose()?,
                last_error: row.try_get("last_error").map_err(map_err)?,
            });
        }
        Ok(out)
    }

    async fn list_credits(&self, qso_id: &QsoId) -> RepoResult<Vec<AwardCredit>> {
        // rowid order is insertion order: the order the record listed them.
        let rows = sqlx::query(
            r#"
            SELECT d.name AS award,
                   json_extract(i.raw_json, '$.medium') AS medium,
                   l.link_status
            FROM award_credit_links l
            JOIN award_credit_imports i ON i.id = l.credit_import_id
            JOIN award_definitions d ON d.id = i.award_id
            WHERE l.qso_id = ?
            ORDER BY l.rowid
            "#,
        )
        .bind(fmt_id(qso_id.as_uuid()))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let status: String = row.try_get("link_status").map_err(map_err)?;
            // Links in another state (a future "rejected" review outcome)
            // aren't credit the QSO carries.
            let Some(status) = CreditStatus::parse_storage(&status) else {
                continue;
            };
            out.push(AwardCredit {
                award: row.try_get("award").map_err(map_err)?,
                medium: row.try_get("medium").map_err(map_err)?,
                status,
            });
        }
        Ok(out)
    }

    async fn list_field_provenance(&self, qso_id: &QsoId) -> RepoResult<Vec<FieldProvenance>> {
        let rows = sqlx::query(
            "SELECT field_name, source, source_detail, created_at
             FROM qso_field_provenance WHERE qso_id = ? ORDER BY field_name, source",
        )
        .bind(fmt_id(qso_id.as_uuid()))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        rows.iter().map(row_to_provenance).collect()
    }

    async fn list_exchange_fields(
        &self,
        qso_id: &QsoId,
    ) -> RepoResult<Vec<QsoExchangeField>> {
        let rows = sqlx::query(
            r#"
            SELECT field_name, raw_value, normalized_value, source
            FROM qso_exchange_fields
            WHERE qso_id = ?
            ORDER BY field_name
            "#,
        )
        .bind(fmt_id(qso_id.as_uuid()))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let name: String = row.try_get("field_name").map_err(map_err)?;
            let raw_value: String = row.try_get("raw_value").map_err(map_err)?;
            let normalized_value: Option<String> =
                row.try_get("normalized_value").map_err(map_err)?;
            let source: String = row.try_get("source").map_err(map_err)?;
            out.push(QsoExchangeField {
                name,
                raw_value,
                normalized_value,
                source: FieldSource::parse_storage(&source),
            });
        }
        Ok(out)
    }

    async fn search_qsos(&self, query: QsoSearch) -> RepoResult<Vec<QsoSummary>> {
        let (filter_sql, binds) = build_filter(&query);
        let mut sql = String::from(
            "SELECT q.id, q.call, q.qso_begin, q.band, q.mode, q.freq_hz, q.dxcc_id, \
             q.dxcc_prefix, q.continent, q.cq_zone \
             FROM qsos q \
             LEFT JOIN qso_service_state s ON s.qso_id = q.id AND s.service = 'lotw' \
             WHERE q.deleted_at IS NULL",
        );
        sql.push_str(&filter_sql);
        sql.push_str(" ORDER BY ");
        sql.push_str(query.sort.sql_clause());
        sql.push_str(&format!(" LIMIT {}", query.limit.unwrap_or(500)));

        let mut q = sqlx::query(&sql);
        for b in binds {
            q = match b {
                DynBind::Text(s) => q.bind(s),
                DynBind::Int(i) => q.bind(i),
            };
        }
        let rows = q.fetch_all(&self.pool).await.map_err(map_err)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let id: String = row.try_get("id").map_err(map_err)?;
            let call: String = row.try_get("call").map_err(map_err)?;
            let qso_begin: String = row.try_get("qso_begin").map_err(map_err)?;
            let band: Option<String> = row.try_get("band").map_err(map_err)?;
            let mode: Option<String> = row.try_get("mode").map_err(map_err)?;
            let freq_hz: Option<i64> = row.try_get("freq_hz").map_err(map_err)?;
            let dxcc_id: Option<i64> = row.try_get("dxcc_id").map_err(map_err)?;
            let dxcc_prefix: Option<String> = row.try_get("dxcc_prefix").map_err(map_err)?;
            let continent: Option<String> = row.try_get("continent").map_err(map_err)?;
            let cq_zone: Option<i64> = row.try_get("cq_zone").map_err(map_err)?;

            out.push(QsoSummary {
                id: QsoId::from_uuid(parse_uuid(&id)?),
                call: parse_callsign(&call)?,
                qso_begin: parse_dt(&qso_begin)?,
                band: band.as_deref().and_then(Band::from_adif),
                mode: mode.as_deref().map(Mode::from_adif),
                freq_hz,
                dxcc_id: dxcc_id.map(|v| v as u16),
                dxcc_prefix,
                continent,
                cq_zone: cq_zone.map(|v| v as u8),
            });
        }
        Ok(out)
    }

    async fn search_full_qsos(&self, query: QsoSearch) -> RepoResult<Vec<Qso>> {
        let (filter_sql, binds) = build_filter(&query);
        let mut sql = String::from(
            "SELECT q.* FROM qsos q \
             LEFT JOIN qso_service_state s ON s.qso_id = q.id AND s.service = 'lotw' \
             WHERE q.deleted_at IS NULL",
        );
        sql.push_str(&filter_sql);
        sql.push_str(" ORDER BY ");
        sql.push_str(query.sort.sql_clause());
        sql.push_str(&format!(" LIMIT {}", query.limit.unwrap_or(500)));

        let mut q = sqlx::query(&sql);
        for b in binds {
            q = match b {
                DynBind::Text(s) => q.bind(s),
                DynBind::Int(i) => q.bind(i),
            };
        }
        let rows = q.fetch_all(&self.pool).await.map_err(map_err)?;
        rows.iter().map(row_to_qso).collect()
    }

    async fn count_matching(&self, query: QsoSearch) -> RepoResult<usize> {
        let (filter_sql, binds) = build_filter(&query);
        // Count doesn't need ORDER BY or LIMIT — wants just the WHERE.
        let mut sql = String::from(
            "SELECT COUNT(*) AS n FROM qsos q \
             LEFT JOIN qso_service_state s ON s.qso_id = q.id AND s.service = 'lotw' \
             WHERE q.deleted_at IS NULL",
        );
        sql.push_str(&filter_sql);

        let mut q = sqlx::query(&sql);
        for b in binds {
            q = match b {
                DynBind::Text(s) => q.bind(s),
                DynBind::Int(i) => q.bind(i),
            };
        }
        let row = q.fetch_one(&self.pool).await.map_err(map_err)?;
        let n: i64 = row.try_get("n").map_err(map_err)?;
        Ok(n.max(0) as usize)
    }

    async fn assign_station_location(
        &self,
        query: QsoSearch,
        location: StationLocationId,
    ) -> RepoResult<usize> {
        let (filter_sql, binds) = build_filter(&query);
        // The filter may reference the LotW join, so select the ids
        // through the same FROM as the searches.
        let mut sql = String::from(
            "UPDATE qsos SET station_location_id = ?, updated_at = ? \
             WHERE id IN (SELECT q.id FROM qsos q \
             LEFT JOIN qso_service_state s ON s.qso_id = q.id AND s.service = 'lotw' \
             WHERE q.deleted_at IS NULL",
        );
        sql.push_str(&filter_sql);
        sql.push(')');

        let mut q = sqlx::query(&sql)
            .bind(fmt_id(location.as_uuid()))
            .bind(fmt_dt(&Utc::now()));
        for b in binds {
            q = match b {
                DynBind::Text(s) => q.bind(s),
                DynBind::Int(i) => q.bind(i),
            };
        }
        let done = q.execute(&self.pool).await.map_err(map_err)?;
        Ok(done.rows_affected() as usize)
    }

    async fn list_pending_uploads(
        &self,
        service: &str,
        limit: Option<u32>,
    ) -> RepoResult<Vec<Qso>> {
        let limit = limit.unwrap_or(500);
        // 'verified' and 'uploaded' both count as not-pending. 'uploaded'
        // remains in flight but the next sync will verify or re-fail it,
        // so we don't re-upload while waiting.
        let sql = "SELECT q.* FROM qsos q
            LEFT JOIN qso_service_state s ON s.qso_id = q.id AND s.service = ?
            WHERE q.deleted_at IS NULL
              AND (s.id IS NULL OR s.upload_state IN ('pending', 'failed'))
            ORDER BY q.qso_begin DESC
            LIMIT ?";
        let rows = sqlx::query(sql)
            .bind(service)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        rows.iter().map(row_to_qso).collect()
    }

    async fn count_pending_uploads_by_location(
        &self,
        service: &str,
    ) -> RepoResult<Vec<PendingUploadCount>> {
        let sql = "SELECT q.station_location_id AS location,
                (q.station_callsign IS NOT NULL AND q.station_callsign <> '') AS has_call,
                COUNT(*) AS n
            FROM qsos q
            LEFT JOIN qso_service_state s ON s.qso_id = q.id AND s.service = ?
            WHERE q.deleted_at IS NULL
              AND (s.id IS NULL OR s.upload_state IN ('pending', 'failed'))
            GROUP BY location, has_call";
        let rows = sqlx::query(sql)
            .bind(service)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        rows.iter()
            .map(|row| {
                let location: Option<String> = row.try_get("location").map_err(map_err)?;
                let has_call: bool = row.try_get("has_call").map_err(map_err)?;
                let n: i64 = row.try_get("n").map_err(map_err)?;
                Ok(PendingUploadCount {
                    station_location_id: location
                        .as_deref()
                        .map(|s| parse_uuid(s).map(StationLocationId::from_uuid))
                        .transpose()?,
                    has_station_callsign: has_call,
                    count: n as usize,
                })
            })
            .collect()
    }

    async fn list_pending_uploads_at(
        &self,
        service: &str,
        locations: &[StationLocationId],
        limit: Option<u32>,
    ) -> RepoResult<Vec<Qso>> {
        if locations.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; locations.len()].join(", ");
        let sql = format!(
            "SELECT q.* FROM qsos q
            LEFT JOIN qso_service_state s ON s.qso_id = q.id AND s.service = ?
            WHERE q.deleted_at IS NULL
              AND (s.id IS NULL OR s.upload_state IN ('pending', 'failed'))
              AND q.station_location_id IN ({placeholders})
              AND q.station_callsign IS NOT NULL AND q.station_callsign <> ''
            ORDER BY q.qso_begin DESC
            LIMIT ?"
        );
        let mut query = sqlx::query(&sql).bind(service);
        for id in locations {
            query = query.bind(fmt_id(id.as_uuid()));
        }
        let rows = query
            .bind(limit.unwrap_or(500) as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        rows.iter().map(row_to_qso).collect()
    }

    async fn mark_uploaded(
        &self,
        qso_id: &QsoId,
        service: &str,
        uploaded_at: DateTime<Utc>,
        remote_id: Option<&str>,
    ) -> RepoResult<()> {
        let id = Uuid::new_v4();
        let now = fmt_dt(&Utc::now());
        let uploaded_str = fmt_dt(&uploaded_at);
        sqlx::query(
            r#"
            INSERT INTO qso_service_state
                (id, qso_id, service, upload_state, confirmation_state,
                 uploaded_at, remote_id, created_at, updated_at)
            VALUES (?, ?, ?, 'uploaded', 'pending', ?, ?, ?, ?)
            ON CONFLICT(qso_id, service) DO UPDATE SET
                upload_state = 'uploaded',
                uploaded_at = excluded.uploaded_at,
                remote_id = COALESCE(excluded.remote_id, qso_service_state.remote_id),
                last_error = NULL,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(fmt_id(id))
        .bind(fmt_id(qso_id.as_uuid()))
        .bind(service)
        .bind(&uploaded_str)
        .bind(remote_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn mark_upload_failed(
        &self,
        qso_id: &QsoId,
        service: &str,
        error: &str,
    ) -> RepoResult<()> {
        let id = Uuid::new_v4();
        let now = fmt_dt(&Utc::now());
        sqlx::query(
            r#"
            INSERT INTO qso_service_state
                (id, qso_id, service, upload_state, confirmation_state,
                 last_error, created_at, updated_at)
            VALUES (?, ?, ?, 'failed', 'pending', ?, ?, ?)
            ON CONFLICT(qso_id, service) DO UPDATE SET
                upload_state = 'failed',
                last_error = excluded.last_error,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(fmt_id(id))
        .bind(fmt_id(qso_id.as_uuid()))
        .bind(service)
        .bind(error)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn mark_upload_verified(
        &self,
        qso_id: &QsoId,
        service: &str,
        verified_at: DateTime<Utc>,
        remote_id: Option<&str>,
    ) -> RepoResult<()> {
        let id = Uuid::new_v4();
        let now = fmt_dt(&Utc::now());
        let verified_str = fmt_dt(&verified_at);
        sqlx::query(
            r#"
            INSERT INTO qso_service_state
                (id, qso_id, service, upload_state, confirmation_state,
                 uploaded_at, last_sync_at, remote_id, created_at, updated_at)
            VALUES (?, ?, ?, 'verified', 'pending', ?, ?, ?, ?, ?)
            ON CONFLICT(qso_id, service) DO UPDATE SET
                upload_state = 'verified',
                uploaded_at = COALESCE(qso_service_state.uploaded_at, excluded.uploaded_at),
                last_sync_at = excluded.last_sync_at,
                remote_id = COALESCE(excluded.remote_id, qso_service_state.remote_id),
                last_error = NULL,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(fmt_id(id))
        .bind(fmt_id(qso_id.as_uuid()))
        .bind(service)
        .bind(&verified_str)
        .bind(&verified_str)
        .bind(remote_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn mark_confirmed(
        &self,
        qso_id: &QsoId,
        service: &str,
        confirmed_at: DateTime<Utc>,
        remote_id: Option<&str>,
    ) -> RepoResult<()> {
        let id = Uuid::new_v4();
        let now = fmt_dt(&Utc::now());
        let confirmed_str = fmt_dt(&confirmed_at);
        sqlx::query(
            r#"
            INSERT INTO qso_service_state
                (id, qso_id, service, upload_state, confirmation_state,
                 confirmed_at, remote_id, created_at, updated_at)
            VALUES (?, ?, ?, 'pending', 'confirmed', ?, ?, ?, ?)
            ON CONFLICT(qso_id, service) DO UPDATE SET
                confirmation_state = 'confirmed',
                confirmed_at = excluded.confirmed_at,
                remote_id = COALESCE(excluded.remote_id, qso_service_state.remote_id),
                updated_at = excluded.updated_at
            "#,
        )
        .bind(fmt_id(id))
        .bind(fmt_id(qso_id.as_uuid()))
        .bind(service)
        .bind(&confirmed_str)
        .bind(remote_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn list_award_qsos(&self) -> RepoResult<Vec<AwardQso>> {
        let sql = "SELECT q.id, q.call, q.qso_begin, q.band, q.mode,
                          q.dxcc_id, q.dxcc_prefix, q.continent, q.state, q.iota,
                          COALESCE(s.confirmation_state = 'confirmed', 0) AS lotw_confirmed,
                          (SELECT group_concat(d.name, ',')
                           FROM award_credit_links l
                           JOIN award_credit_imports i ON i.id = l.credit_import_id
                           JOIN award_definitions d ON d.id = i.award_id
                           WHERE l.qso_id = q.id AND l.link_status = 'granted'
                          ) AS granted_credits
                   FROM qsos q
                   LEFT JOIN qso_service_state s ON s.qso_id = q.id AND s.service = 'lotw'
                   WHERE q.deleted_at IS NULL";
        let rows = sqlx::query(sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let id: String = row.try_get("id").map_err(map_err)?;
            let call: String = row.try_get("call").map_err(map_err)?;
            let qso_begin: String = row.try_get("qso_begin").map_err(map_err)?;
            let band: Option<String> = row.try_get("band").map_err(map_err)?;
            let mode: Option<String> = row.try_get("mode").map_err(map_err)?;
            let dxcc_id: Option<i64> = row.try_get("dxcc_id").map_err(map_err)?;
            let dxcc_prefix: Option<String> = row.try_get("dxcc_prefix").map_err(map_err)?;
            let continent: Option<String> = row.try_get("continent").map_err(map_err)?;
            let state: Option<String> = row.try_get("state").map_err(map_err)?;
            let iota: Option<String> = row.try_get("iota").map_err(map_err)?;
            let confirmed: i64 = row.try_get("lotw_confirmed").map_err(map_err)?;
            let granted: Option<String> = row.try_get("granted_credits").map_err(map_err)?;
            out.push(AwardQso {
                id: QsoId::from_uuid(parse_uuid(&id)?),
                call: parse_callsign(&call)?,
                qso_begin: parse_dt(&qso_begin)?,
                band: band.as_deref().and_then(Band::from_adif),
                mode: mode.as_deref().map(Mode::from_adif),
                dxcc_id: dxcc_id.map(|v| v as u16),
                dxcc_prefix,
                continent,
                state,
                iota,
                lotw_confirmed: confirmed != 0,
                granted_credits: granted
                    .map(|g| g.split(',').map(String::from).collect())
                    .unwrap_or_default(),
            });
        }
        Ok(out)
    }

    async fn find_match_for_confirmation(
        &self,
        station_call: &str,
        worked_call: &str,
        qso_date: &str,
        band: Option<&str>,
        mode: Option<&str>,
    ) -> RepoResult<Option<QsoId>> {
        let sql = "SELECT id FROM qsos
            WHERE deleted_at IS NULL
              AND station_callsign = ?
              AND call = ?
              AND substr(qso_begin, 1, 10) = ?
              AND (? IS NULL OR band = ?)
              AND (? IS NULL OR mode = ?)
            ORDER BY qso_begin DESC
            LIMIT 1";
        let row = sqlx::query(sql)
            .bind(station_call)
            .bind(worked_call)
            .bind(qso_date)
            .bind(band)
            .bind(band)
            .bind(mode)
            .bind(mode)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        match row {
            None => Ok(None),
            Some(r) => {
                let id: String = r.try_get("id").map_err(map_err)?;
                Ok(Some(QsoId::from_uuid(parse_uuid(&id)?)))
            }
        }
    }

    async fn load_dedup_keys(&self) -> RepoResult<HashSet<DedupKey>> {
        // Station callsign + worked call + UTC start time to the second
        // + band + mode; must match `dedup_key` in service.rs. Only QSOs
        // with station_callsign set qualify.
        let rows = sqlx::query(
            r#"
            SELECT station_callsign, call,
                   substr(qso_begin, 1, 19) AS begin_second,
                   band, mode
            FROM qsos
            WHERE deleted_at IS NULL
              AND station_callsign IS NOT NULL
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        let mut out = HashSet::with_capacity(rows.len());
        for row in rows {
            let station: String = row.try_get("station_callsign").map_err(map_err)?;
            let call: String = row.try_get("call").map_err(map_err)?;
            let date: String = row.try_get("begin_second").map_err(map_err)?;
            let band: Option<String> = row.try_get("band").map_err(map_err)?;
            let mode: Option<String> = row.try_get("mode").map_err(map_err)?;
            out.insert((station, call, date, band, mode));
        }
        Ok(out)
    }

    async fn load_station_locations(&self) -> RepoResult<Vec<StationLocation>> {
        let rows = sqlx::query("SELECT * FROM station_locations ORDER BY created_at, rowid")
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        rows.iter().map(row_to_location).collect()
    }

    async fn insert_qsos_batch(&self, batch: &ImportBatch) -> RepoResult<()> {
        // One transaction, one INSERT loop per table. Per-row binds are
        // the same as the corresponding single-row methods (insert_qso,
        // add_exchange_field, mark_uploaded/mark_upload_verified/
        // mark_confirmed, StationRepository::insert_location) — we just
        // swap the executor from `&self.pool` for `&mut tx`. ~50k QSOs →
        // one commit instead of ~1.5M.
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        let now = fmt_dt(&Utc::now());

        // Locations before the QSOs that reference them (foreign key).
        for loc in &batch.locations {
            insert_location_sql(loc)
                .execute(&mut *tx)
                .await
                .map_err(map_err)?;
        }

        for q in &batch.qsos {
            insert_qso_query(q)
                .execute(&mut *tx)
                .await
                .map_err(map_err)?;
        }

        for (qso_id, field) in &batch.exchange_fields {
            let id = Uuid::new_v4();
            sqlx::query(
                r#"
                INSERT INTO qso_exchange_fields
                    (id, qso_id, field_name, raw_value, normalized_value, source, created_at)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(fmt_id(id))
            .bind(fmt_id(qso_id.as_uuid()))
            .bind(&field.name)
            .bind(&field.raw_value)
            .bind(&field.normalized_value)
            .bind(field.source.as_storage_str())
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }

        for s in &batch.service_states {
            // Defaults match the schema's "fresh row, no signal yet"
            // semantics: pending upload, pending confirmation.
            let upload_state = s.upload_state.unwrap_or("pending");
            let confirmation_state = s.confirmation_state.unwrap_or("pending");
            let row_id = Uuid::new_v4();
            sqlx::query(
                r#"
                INSERT INTO qso_service_state
                    (id, qso_id, service,
                     upload_state, confirmation_state,
                     uploaded_at, confirmed_at, remote_id,
                     created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(fmt_id(row_id))
            .bind(fmt_id(s.qso_id.as_uuid()))
            .bind(s.service)
            .bind(upload_state)
            .bind(confirmation_state)
            .bind(s.uploaded_at.as_ref().map(fmt_dt))
            .bind(s.confirmed_at.as_ref().map(fmt_dt))
            .bind(&s.remote_id)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }

        insert_credits(&mut tx, &batch.credits, &now).await?;

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn load_sync_candidates(
        &self,
        service: &str,
        calls: &[String],
    ) -> RepoResult<Vec<SyncCandidate>> {
        let mut out = Vec::new();
        for chunk in calls.chunks(IN_CHUNK) {
            let sql = format!(
                "SELECT q.id, q.call, q.station_callsign, q.qso_begin, q.band, q.mode,
                        q.dxcc_id, q.continent, q.cq_zone, q.itu_zone, q.iota, q.grid,
                        q.state, q.county, q.wpx_prefix,
                        COALESCE(s.upload_state = 'verified', 0) AS verified,
                        COALESCE(s.confirmation_state = 'confirmed', 0) AS confirmed
                 FROM qsos q
                 LEFT JOIN qso_service_state s ON s.qso_id = q.id AND s.service = ?
                 WHERE q.deleted_at IS NULL AND q.call IN ({})",
                placeholders(chunk.len())
            );
            let mut query = sqlx::query(&sql).bind(service);
            for call in chunk {
                query = query.bind(call);
            }
            let rows = query.fetch_all(&self.pool).await.map_err(map_err)?;
            for row in &rows {
                let id: String = row.try_get("id").map_err(map_err)?;
                let begin: String = row.try_get("qso_begin").map_err(map_err)?;
                let int = |col| -> RepoResult<Option<i64>> { row.try_get(col).map_err(map_err) };
                let text = |col| -> RepoResult<Option<String>> { row.try_get(col).map_err(map_err) };
                let verified: i64 = row.try_get("verified").map_err(map_err)?;
                let confirmed: i64 = row.try_get("confirmed").map_err(map_err)?;
                out.push(SyncCandidate {
                    qso_id: QsoId::from_uuid(parse_uuid(&id)?),
                    call: row.try_get("call").map_err(map_err)?,
                    station_callsign: row.try_get("station_callsign").map_err(map_err)?,
                    qso_begin: parse_dt(&begin)?,
                    band: row.try_get("band").map_err(map_err)?,
                    mode: row.try_get("mode").map_err(map_err)?,
                    detail: LocationDetail {
                        dxcc_id: int("dxcc_id")?.map(|v| v as u16),
                        continent: text("continent")?,
                        cq_zone: int("cq_zone")?.map(|v| v as u8),
                        itu_zone: int("itu_zone")?.map(|v| v as u8),
                        iota: text("iota")?,
                        grid: text("grid")?,
                        state: text("state")?,
                        county: text("county")?,
                        wpx_prefix: text("wpx_prefix")?,
                    },
                    upload_verified: verified != 0,
                    confirmed: confirmed != 0,
                });
            }
        }
        Ok(out)
    }

    async fn list_credits_for(
        &self,
        qso_ids: &[QsoId],
    ) -> RepoResult<HashMap<QsoId, Vec<AwardCredit>>> {
        let mut out: HashMap<QsoId, Vec<AwardCredit>> = HashMap::new();
        for chunk in qso_ids.chunks(IN_CHUNK) {
            // Same shape and order as list_credits.
            let sql = format!(
                "SELECT l.qso_id, d.name AS award,
                        json_extract(i.raw_json, '$.medium') AS medium,
                        l.link_status
                 FROM award_credit_links l
                 JOIN award_credit_imports i ON i.id = l.credit_import_id
                 JOIN award_definitions d ON d.id = i.award_id
                 WHERE l.qso_id IN ({})
                 ORDER BY l.rowid",
                placeholders(chunk.len())
            );
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(fmt_id(id.as_uuid()));
            }
            let rows = query.fetch_all(&self.pool).await.map_err(map_err)?;
            for row in &rows {
                let status: String = row.try_get("link_status").map_err(map_err)?;
                let Some(status) = CreditStatus::parse_storage(&status) else {
                    continue;
                };
                let qso_id: String = row.try_get("qso_id").map_err(map_err)?;
                out.entry(QsoId::from_uuid(parse_uuid(&qso_id)?))
                    .or_default()
                    .push(AwardCredit {
                        award: row.try_get("award").map_err(map_err)?,
                        medium: row.try_get("medium").map_err(map_err)?,
                        status,
                    });
            }
        }
        Ok(out)
    }

    async fn list_service_provenance_for(
        &self,
        service: &str,
        qso_ids: &[QsoId],
    ) -> RepoResult<HashMap<QsoId, Vec<FieldProvenance>>> {
        let synced = FieldSource::ServiceSync(service.to_string()).as_storage_str();
        let differs = FieldSource::ServiceDiffers(service.to_string()).as_storage_str();
        let mut out: HashMap<QsoId, Vec<FieldProvenance>> = HashMap::new();
        for chunk in qso_ids.chunks(IN_CHUNK) {
            let sql = format!(
                "SELECT qso_id, field_name, source, source_detail, created_at
                 FROM qso_field_provenance
                 WHERE source IN (?, ?) AND qso_id IN ({})",
                placeholders(chunk.len())
            );
            let mut query = sqlx::query(&sql).bind(&synced).bind(&differs);
            for id in chunk {
                query = query.bind(fmt_id(id.as_uuid()));
            }
            let rows = query.fetch_all(&self.pool).await.map_err(map_err)?;
            for row in &rows {
                let qso_id: String = row.try_get("qso_id").map_err(map_err)?;
                out.entry(QsoId::from_uuid(parse_uuid(&qso_id)?))
                    .or_default()
                    .push(row_to_provenance(row)?);
            }
        }
        Ok(out)
    }

    async fn apply_sync_batch(&self, batch: &SyncBatch) -> RepoResult<()> {
        // One transaction for the whole report, like insert_qsos_batch.
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        let now = fmt_dt(&Utc::now());

        for s in &batch.states {
            // mark_upload_verified + mark_confirmed in one statement, with
            // nothing downgraded: the upload state only ever rises to
            // verified, confirmation is only ever set, and an existing
            // confirmation keeps its date.
            let confirmation = if s.confirmed_at.is_some() { "confirmed" } else { "pending" };
            sqlx::query(
                r#"
                INSERT INTO qso_service_state
                    (id, qso_id, service, upload_state, confirmation_state,
                     uploaded_at, confirmed_at, last_sync_at, created_at, updated_at)
                VALUES (?, ?, ?, 'verified', ?, ?, ?, ?, ?, ?)
                ON CONFLICT(qso_id, service) DO UPDATE SET
                    upload_state = 'verified',
                    uploaded_at = COALESCE(qso_service_state.uploaded_at, excluded.uploaded_at),
                    confirmed_at = CASE
                        WHEN excluded.confirmation_state = 'confirmed'
                             AND qso_service_state.confirmation_state <> 'confirmed'
                        THEN excluded.confirmed_at
                        ELSE qso_service_state.confirmed_at END,
                    confirmation_state = CASE
                        WHEN excluded.confirmation_state = 'confirmed' THEN 'confirmed'
                        ELSE qso_service_state.confirmation_state END,
                    last_sync_at = excluded.last_sync_at,
                    last_error = NULL,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(fmt_id(Uuid::new_v4()))
            .bind(fmt_id(s.qso_id.as_uuid()))
            .bind(&batch.service)
            .bind(confirmation)
            .bind(fmt_dt(&s.uploaded_at))
            .bind(s.confirmed_at.as_ref().map(fmt_dt))
            .bind(&now)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }

        insert_credits(&mut tx, &batch.new_credits, &now).await?;

        for u in &batch.credit_upgrades {
            sqlx::query(
                r#"
                UPDATE award_credit_links SET link_status = 'granted'
                WHERE qso_id = ? AND link_status = 'submitted'
                  AND credit_import_id IN (
                      SELECT i.id FROM award_credit_imports i
                      JOIN award_definitions d ON d.id = i.award_id
                      WHERE d.name = ? AND json_extract(i.raw_json, '$.medium') IS ?)
                "#,
            )
            .bind(fmt_id(u.qso_id.as_uuid()))
            .bind(&u.award)
            .bind(&u.medium)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }

        for f in &batch.detail_fills {
            // Only the fields the plan fills are bound; NULL keeps the
            // column. One statement shape for every QSO, whatever it fills.
            let v = &f.values;
            sqlx::query(
                r#"
                UPDATE qsos SET
                    dxcc_id = COALESCE(?, dxcc_id),
                    continent = COALESCE(?, continent),
                    cq_zone = COALESCE(?, cq_zone),
                    itu_zone = COALESCE(?, itu_zone),
                    iota = COALESCE(?, iota),
                    grid = COALESCE(?, grid),
                    state = COALESCE(?, state),
                    county = COALESCE(?, county),
                    wpx_prefix = COALESCE(?, wpx_prefix),
                    updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(v.dxcc_id.map(i64::from))
            .bind(&v.continent)
            .bind(v.cq_zone.map(i64::from))
            .bind(v.itu_zone.map(i64::from))
            .bind(&v.iota)
            .bind(&v.grid)
            .bind(&v.state)
            .bind(&v.county)
            .bind(&v.wpx_prefix)
            .bind(&now)
            .bind(fmt_id(f.qso_id.as_uuid()))
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }

        // In index order: a first sync adds several rows to each of tens
        // of thousands of QSOs, and inserting them sorted by the unique
        // index's key, rather than in report order, saves about a tenth.
        let mut writes: Vec<_> = batch.provenance.iter().collect();
        writes.sort_unstable_by(|a, b| {
            (a.qso_id.as_uuid(), &a.field_name).cmp(&(b.qso_id.as_uuid(), &b.field_name))
        });
        for p in writes {
            let qso_id = fmt_id(p.qso_id.as_uuid());
            let query = match (&p.replaces, &p.row) {
                (None, None) => continue,
                // Upsert rather than insert: a row the plan didn't see
                // (written since it loaded) is replaced, not an error
                // that loses the whole report.
                (None, Some((source, detail))) => sqlx::query(
                    r#"
                    INSERT INTO qso_field_provenance
                        (id, qso_id, field_name, source, source_detail, created_at)
                    VALUES (?, ?, ?, ?, ?, ?)
                    ON CONFLICT(qso_id, field_name, source) DO UPDATE SET
                        source_detail = excluded.source_detail,
                        created_at = excluded.created_at
                    "#,
                )
                .bind(fmt_id(Uuid::new_v4()))
                .bind(qso_id)
                .bind(&p.field_name)
                .bind(source.as_storage_str())
                .bind(detail)
                .bind(&now),
                (Some(old), Some((source, detail))) => sqlx::query(
                    r#"
                    UPDATE qso_field_provenance
                    SET source = ?, source_detail = ?, created_at = ?
                    WHERE qso_id = ? AND field_name = ? AND source = ?
                    "#,
                )
                .bind(source.as_storage_str())
                .bind(detail)
                .bind(&now)
                .bind(qso_id)
                .bind(&p.field_name)
                .bind(old.as_storage_str()),
                (Some(old), None) => sqlx::query(
                    "DELETE FROM qso_field_provenance
                     WHERE qso_id = ? AND field_name = ? AND source = ?",
                )
                .bind(qso_id)
                .bind(&p.field_name)
                .bind(old.as_storage_str()),
            };
            query.execute(&mut *tx).await.map_err(map_err)?;
        }

        if let Some(w) = &batch.watermark {
            sqlx::query(
                r#"
                INSERT INTO service_sync_watermarks (service, report, value, updated_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(service, report) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&w.service)
            .bind(&w.key)
            .bind(&w.value)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }

        tx.commit().await.map_err(map_err)?;
        Ok(())
    }

    async fn get_sync_watermark(&self, service: &str, key: &str) -> RepoResult<Option<String>> {
        sqlx::query_scalar(
            "SELECT value FROM service_sync_watermarks WHERE service = ? AND report = ?",
        )
        .bind(service)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use logbook_domain::{CreateQsoCommand, LogbookService, QsoSearch};
    use radio_core::{Band, Callsign, Mode};
    use std::sync::Arc;

    use super::*;

    async fn fresh_service() -> LogbookService {
        let db = Database::open_in_memory().await.unwrap();
        let repo = Arc::new(SqliteQsoRepository::new(&db));
        LogbookService::new(repo)
    }

    /// Returns (service, pool) so tests can poke the DB directly to
    /// verify side-effects (e.g. qso_service_state rows).
    async fn fresh_service_with_db() -> (LogbookService, Database) {
        let db = Database::open_in_memory().await.unwrap();
        let repo = Arc::new(SqliteQsoRepository::new(&db));
        (LogbookService::new(repo), db)
    }

    #[tokio::test]
    async fn qso_detail_includes_extra_fields_and_service_state() {
        let db = Database::open_in_memory().await.unwrap();
        let repo = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo.clone());
        let mut cmd = CreateQsoCommand::minimal(
            Callsign::parse("DL1XYZ").unwrap(),
            Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
        );
        cmd.band = Some(Band::M20);
        cmd.mode = Some(Mode::CW);
        let id = svc.create_qso(cmd).await.unwrap();

        let confirmed = Utc.with_ymd_and_hms(2026, 5, 9, 12, 0, 0).unwrap();
        repo.mark_confirmed(&id, "lotw", confirmed, None)
            .await
            .unwrap();
        repo.add_exchange_field(
                &id,
                &QsoExchangeField {
                    name: "NAME".into(),
                    raw_value: "Hans".into(),
                    normalized_value: None,
                    source: FieldSource::ImportedAdif,
                },
            )
            .await
            .unwrap();

        let detail = svc.get_qso_detail(&id).await.unwrap().unwrap();
        assert_eq!(detail.qso.call.as_str(), "DL1XYZ");
        assert_eq!(detail.exchange_fields.len(), 1);
        assert_eq!(detail.exchange_fields[0].raw_value, "Hans");
        assert_eq!(detail.service_states.len(), 1);
        assert_eq!(detail.service_states[0].service, "lotw");
        assert_eq!(detail.service_states[0].confirmation_state, "confirmed");
        assert_eq!(detail.service_states[0].confirmed_at, Some(confirmed));

        assert!(svc.get_qso_detail(&QsoId::new()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn batch_import_inserts_all_qsos_in_one_transaction() {
        let svc = fresh_service().await;
        let mut commands = Vec::with_capacity(1000);
        for i in 0..1000 {
            let mut cmd = CreateQsoCommand::minimal(
                Callsign::parse(&format!("TEST{i:04}")).unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap()
                    + chrono::Duration::minutes(i as i64),
            );
            cmd.band = Some(Band::M20);
            cmd.mode = Some(Mode::FT8);
            cmd.station_callsign = Some(Callsign::parse("W1ABC").unwrap());
            commands.push(cmd);
        }
        let start = std::time::Instant::now();
        let report = svc.import_qsos(commands).await;
        let elapsed = start.elapsed();
        assert_eq!(report.created, 1000);
        assert_eq!(report.duplicates, 0);
        assert!(
            elapsed.as_secs() < 5,
            "batch import of 1000 records took {elapsed:?} (expected < 5s)"
        );
    }

    #[tokio::test]
    async fn batch_import_dedups_intra_batch_and_against_existing() {
        let svc = fresh_service().await;
        // Pre-existing QSO with station W1ABC + call W1AW.
        let existing = CreateQsoCommand {
            band: Some(Band::M20),
            mode: Some(Mode::FT8),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            ..CreateQsoCommand::minimal(
                Callsign::parse("W1AW").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
            )
        };
        svc.create_qso(existing).await.unwrap();

        // Import three commands: one collides with the pre-existing one,
        // two duplicates of each other (intra-batch), one fresh.
        let dup = CreateQsoCommand {
            band: Some(Band::M20),
            mode: Some(Mode::FT8),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            ..CreateQsoCommand::minimal(
                Callsign::parse("W1AW").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
            )
        };
        let intra_a = CreateQsoCommand {
            band: Some(Band::M20),
            mode: Some(Mode::FT8),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            ..CreateQsoCommand::minimal(
                Callsign::parse("JA1NUT").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 19, 30, 0).unwrap(),
            )
        };
        let intra_b = CreateQsoCommand {
            band: Some(Band::M20),
            mode: Some(Mode::FT8),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            ..CreateQsoCommand::minimal(
                Callsign::parse("JA1NUT").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 19, 30, 0).unwrap(),
            )
        };
        let fresh = CreateQsoCommand {
            band: Some(Band::M40),
            mode: Some(Mode::CW),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            ..CreateQsoCommand::minimal(
                Callsign::parse("VE3XYZ").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 21, 0, 0).unwrap(),
            )
        };

        let report = svc.import_qsos(vec![dup, intra_a, intra_b, fresh]).await;
        assert_eq!(report.duplicates, 2); // dup-vs-existing + intra-batch
        assert_eq!(report.created, 2); // intra_a + fresh
    }

    #[tokio::test]
    async fn batch_import_writes_service_state_from_adif_fields() {
        use radio_core::{FieldSource, QsoExchangeField};
        let (svc, db) = fresh_service_with_db().await;
        let mut cmd = CreateQsoCommand::minimal(
            Callsign::parse("W1AW").unwrap(),
            Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
        );
        cmd.band = Some(Band::M20);
        cmd.mode = Some(Mode::FT8);
        cmd.station_callsign = Some(Callsign::parse("W1ABC").unwrap());
        let mk_field = |n: &str, v: &str| QsoExchangeField {
            name: n.to_string(),
            raw_value: v.to_string(),
            normalized_value: None,
            source: FieldSource::ImportedAdif,
        };
        cmd.exchange_fields = vec![
            mk_field("LOTW_QSL_SENT", "V"),
            mk_field("LOTW_QSLSDATE", "20240101"),
            mk_field("LOTW_QSL_RCVD", "V"),
            mk_field("LOTW_QSLRDATE", "20240105"),
            mk_field("QRZCOM_QSO_UPLOAD_STATUS", "Y"),
            mk_field("QRZCOM_QSO_UPLOAD_DATE", "20240506"),
        ];
        let report = svc.import_qsos(vec![cmd]).await;
        assert_eq!(report.created, 1);

        // Verify rows landed in qso_service_state.
        let lotw_row = sqlx::query(
            "SELECT upload_state, confirmation_state FROM qso_service_state \
             WHERE service = 'lotw'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        let lotw_upload: String = lotw_row.try_get("upload_state").unwrap();
        let lotw_confirm: String = lotw_row.try_get("confirmation_state").unwrap();
        assert_eq!(lotw_upload, "verified");
        assert_eq!(lotw_confirm, "confirmed");

        let qrz_row = sqlx::query(
            "SELECT upload_state FROM qso_service_state WHERE service = 'qrz'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        let qrz_upload: String = qrz_row.try_get("upload_state").unwrap();
        assert_eq!(qrz_upload, "uploaded");
    }

    #[tokio::test]
    async fn batch_import_with_lotw_sent_n_emits_no_service_row() {
        use radio_core::{FieldSource, QsoExchangeField};
        let (svc, db) = fresh_service_with_db().await;
        let mut cmd = CreateQsoCommand::minimal(
            Callsign::parse("W1AW").unwrap(),
            Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
        );
        cmd.band = Some(Band::M20);
        cmd.mode = Some(Mode::FT8);
        cmd.station_callsign = Some(Callsign::parse("W1ABC").unwrap());
        cmd.exchange_fields = vec![QsoExchangeField {
            name: "LOTW_QSL_SENT".into(),
            raw_value: "N".into(),
            normalized_value: None,
            source: FieldSource::ImportedAdif,
        }];
        let report = svc.import_qsos(vec![cmd]).await;
        assert_eq!(report.created, 1);
        // No qso_service_state rows for LotW = pending (default).
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM qso_service_state WHERE service = 'lotw'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn create_and_search_qso() {
        let svc = fresh_service().await;
        let cmd = CreateQsoCommand {
            band: Some(Band::M20),
            freq_hz: Some(14_074_000),
            mode: Some(Mode::FT8),
            rst_sent: Some("-12".into()),
            rst_rcvd: Some("-09".into()),
            ..CreateQsoCommand::minimal(
                Callsign::parse("W1AW").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
            )
        };
        let id = svc.create_qso(cmd).await.unwrap();

        let results = svc.search_qsos(QsoSearch::default()).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
        assert_eq!(results[0].call.as_str(), "W1AW");
        assert_eq!(results[0].band, Some(Band::M20));
    }

    #[tokio::test]
    async fn search_filters_by_band() {
        let svc = fresh_service().await;
        for (call, band, freq) in [
            ("W1AW", Band::M20, 14_074_000_i64),
            ("VE3XYZ", Band::M40, 7_074_000),
        ] {
            svc.create_qso(CreateQsoCommand {
                band: Some(band),
                freq_hz: Some(freq),
                mode: Some(Mode::FT8),
                ..CreateQsoCommand::minimal(
                    Callsign::parse(call).unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                )
            })
            .await
            .unwrap();
        }
        let results = svc
            .search_qsos(QsoSearch {
                band: Some(Band::M20),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].call.as_str(), "W1AW");
    }

    const MINI_CTY: &str = "United States of America:           05:  08:  NA:   37.53:   97.00:     5.0:  K:
    K,W,N,AA,AB,AC,AD,AE,AF,AG,AH,AI,AJ,AK,AL;
Japan:                              25:  45:  AS:   36.24: -139.00:    -9.0:  JA:
    JA,JE,JF,JG,JH,JI,JJ,JK,JL,JM,JN,JO,JP,JQ,JR,JS;
";

    #[tokio::test]
    async fn resolver_enriches_dxcc_fields_on_create() {
        use station_resolver::CtyDbResolver;

        let db = Database::open_in_memory().await.unwrap();
        let repo = Arc::new(SqliteQsoRepository::new(&db));
        let resolver = Arc::new(CtyDbResolver::from_reader(MINI_CTY.as_bytes()).unwrap());
        let svc = LogbookService::with_resolver(repo, resolver);

        let id = svc
            .create_qso(CreateQsoCommand {
                band: Some(Band::M20),
                freq_hz: Some(14_074_000),
                mode: Some(Mode::FT8),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("W1AW").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                )
            })
            .await
            .unwrap();

        let results = svc.search_qsos(QsoSearch::default()).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
        assert_eq!(results[0].dxcc_prefix.as_deref(), Some("W"));
        assert_eq!(results[0].continent.as_deref(), Some("NA"));
        assert_eq!(results[0].cq_zone, Some(5));
        assert_eq!(results[0].dxcc_id, Some(291));
    }

    #[tokio::test]
    async fn pending_uploads_lifecycle() {
        use logbook_domain::QsoRepository;

        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo.clone());

        let id1 = svc
            .create_qso(CreateQsoCommand {
                band: Some(Band::M20),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("W1AW").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                )
            })
            .await
            .unwrap();
        let _id2 = svc
            .create_qso(CreateQsoCommand {
                band: Some(Band::M40),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("VE3XYZ").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 35, 0).unwrap(),
                )
            })
            .await
            .unwrap();

        // Initially both are pending for "lotw".
        let pending = repo.list_pending_uploads("lotw", None).await.unwrap();
        assert_eq!(pending.len(), 2);

        // Mark one uploaded; it drops out of pending.
        repo.mark_uploaded(&id1, "lotw", Utc::now(), Some("batch-42"))
            .await
            .unwrap();
        let pending = repo.list_pending_uploads("lotw", None).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].call.as_str(), "VE3XYZ");

        // Other services are independent.
        let pending_eqsl = repo.list_pending_uploads("eqsl", None).await.unwrap();
        assert_eq!(pending_eqsl.len(), 2);

        // Failure path: mark failed, still pending for retry.
        repo.mark_upload_failed(&id1, "lotw", "TQSL exit code 1")
            .await
            .unwrap();
        let pending = repo.list_pending_uploads("lotw", None).await.unwrap();
        assert_eq!(pending.len(), 2);

        // Mark uploaded again — clears last_error and pending.
        repo.mark_uploaded(&id1, "lotw", Utc::now(), None).await.unwrap();
        let pending = repo.list_pending_uploads("lotw", None).await.unwrap();
        assert_eq!(pending.len(), 1);

        // Mark confirmed — pending unchanged but state recorded.
        repo.mark_confirmed(&id1, "lotw", Utc::now(), Some("lotw-12345"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn count_matching_and_search_full_match_search_qsos() {
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo.clone());

        for (call, band) in [
            ("W1AW", Band::M20),
            ("VE3X", Band::M40),
            ("JA1NUT", Band::M20),
        ] {
            svc.create_qso(CreateQsoCommand {
                band: Some(band),
                ..CreateQsoCommand::minimal(
                    Callsign::parse(call).unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 0, 0).unwrap(),
                )
            })
            .await
            .unwrap();
        }

        let search_20m = QsoSearch {
            band: Some(Band::M20),
            ..Default::default()
        };
        let count = svc.count_matching(search_20m.clone()).await.unwrap();
        assert_eq!(count, 2);
        let summaries = svc.search_qsos(search_20m.clone()).await.unwrap();
        assert_eq!(summaries.len(), 2);
        let full = svc.search_full_qsos(search_20m).await.unwrap();
        assert_eq!(full.len(), 2);
        // Full QSOs should have populated created_at; summaries don't.
        assert!(full.iter().all(|q| q.created_at <= Utc::now()));
    }

    #[tokio::test]
    async fn search_sort_order_descending_then_ascending() {
        use logbook_domain::SortOrder;
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo);

        // Three QSOs at different times.
        for (call, hour) in [("W1AW", 18), ("VE3X", 19), ("JA1NUT", 20)] {
            svc.create_qso(CreateQsoCommand {
                band: Some(Band::M20),
                ..CreateQsoCommand::minimal(
                    Callsign::parse(call).unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, hour, 0, 0).unwrap(),
                )
            })
            .await
            .unwrap();
        }

        let asc = svc
            .search_qsos(QsoSearch {
                sort: SortOrder::QsoBeginAsc,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(asc[0].call.as_str(), "W1AW");
        assert_eq!(asc[2].call.as_str(), "JA1NUT");

        let desc = svc
            .search_qsos(QsoSearch::default())
            .await
            .unwrap();
        assert_eq!(desc[0].call.as_str(), "JA1NUT");
        assert_eq!(desc[2].call.as_str(), "W1AW");
    }

    #[tokio::test]
    async fn bulk_soft_delete_by_search_chains_count_and_action() {
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo);

        for call in ["W1AW", "VE3X", "JA1NUT"] {
            svc.create_qso(CreateQsoCommand {
                band: Some(Band::M40),
                ..CreateQsoCommand::minimal(
                    Callsign::parse(call).unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 0, 0).unwrap(),
                )
            })
            .await
            .unwrap();
        }

        // Delete all 40m QSOs.
        let report = svc
            .bulk_soft_delete_by_search(QsoSearch {
                band: Some(Band::M40),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(report.matched, 3);
        assert_eq!(report.succeeded, 3);

        let remaining = svc.count_matching(QsoSearch::default()).await.unwrap();
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn bulk_soft_delete_removes_all_in_set() {
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo.clone());

        let mut ids = Vec::new();
        for call in ["W1AW", "VE3X", "JA1NUT"] {
            let id = svc
                .create_qso(CreateQsoCommand {
                    band: Some(Band::M20),
                    ..CreateQsoCommand::minimal(
                        Callsign::parse(call).unwrap(),
                        Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                    )
                })
                .await
                .unwrap();
            ids.push(id);
        }

        let deleted = svc.bulk_soft_delete(&ids[..2]).await; // delete first 2
        assert_eq!(deleted, 2);

        let results = svc.search_qsos(QsoSearch::default()).await.unwrap();
        assert_eq!(results.len(), 1, "one QSO survives the bulk delete");
        assert_eq!(results[0].call.as_str(), "JA1NUT");
    }

    #[tokio::test]
    async fn bulk_mark_uploaded_clears_pending_for_all() {
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo.clone());

        let mut ids = Vec::new();
        for call in ["W1AW", "VE3X"] {
            let id = svc
                .create_qso(CreateQsoCommand {
                    band: Some(Band::M20),
                    ..CreateQsoCommand::minimal(
                        Callsign::parse(call).unwrap(),
                        Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                    )
                })
                .await
                .unwrap();
            ids.push(id);
        }

        let marked = svc.bulk_mark_uploaded(&ids, "lotw", Utc::now()).await;
        assert_eq!(marked, 2);

        let pending = repo.list_pending_uploads("lotw", None).await.unwrap();
        assert!(pending.is_empty(), "no pending after bulk-mark-uploaded");
    }

    #[tokio::test]
    async fn search_filters_by_lotw_confirmed() {
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo.clone());

        let id1 = svc
            .create_qso(CreateQsoCommand {
                band: Some(Band::M20),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("W1AW").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                )
            })
            .await
            .unwrap();
        let _id2 = svc
            .create_qso(CreateQsoCommand {
                band: Some(Band::M20),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("VE3X").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 35, 0).unwrap(),
                )
            })
            .await
            .unwrap();

        // Only id1 is confirmed.
        repo.mark_confirmed(&id1, "lotw", Utc::now(), None)
            .await
            .unwrap();

        let confirmed = svc
            .search_qsos(QsoSearch {
                lotw_confirmed: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(confirmed.len(), 1);
        assert_eq!(confirmed[0].id, id1);

        let not_confirmed = svc
            .search_qsos(QsoSearch {
                lotw_confirmed: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(not_confirmed.len(), 1);
        assert_ne!(not_confirmed[0].id, id1);
    }

    #[tokio::test]
    async fn import_dedup_skips_repeats() {
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo);

        // Same shape as a WSJT-X auto-import: station_callsign present,
        // call/date/band/mode unique-determining.
        let make_cmd = || CreateQsoCommand {
            band: Some(Band::M20),
            mode: Some(Mode::FT8),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            ..CreateQsoCommand::minimal(
                Callsign::parse("W1AW").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
            )
        };

        let report = svc.import_qsos(vec![make_cmd()]).await;
        assert_eq!(report.created, 1);
        assert_eq!(report.duplicates, 0);

        // WSJT-X re-broadcast (or repeat ADIF import) of the same QSO.
        let report = svc.import_qsos(vec![make_cmd(), make_cmd()]).await;
        assert_eq!(report.created, 0);
        assert_eq!(report.duplicates, 2);

        // Confirm only one row exists.
        let results = svc.search_qsos(QsoSearch::default()).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn import_keeps_a_second_qso_with_the_same_station_on_the_same_day() {
        let svc = fresh_service().await;
        // Two sessions of the same contest: same station, band and mode,
        // two hours apart. Both are real QSOs.
        let at = |hour| CreateQsoCommand {
            band: Some(Band::M20),
            mode: Some(Mode::CW),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            ..CreateQsoCommand::minimal(
                Callsign::parse("W1AW").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, hour, 5, 0).unwrap(),
            )
        };
        let report = svc.import_qsos(vec![at(13), at(19)]).await;
        assert_eq!(report.created, 2);
        assert_eq!(report.duplicates, 0);

        // Importing the same records again adds nothing.
        let report = svc.import_qsos(vec![at(13), at(19)]).await;
        assert_eq!(report.created, 0);
        assert_eq!(report.duplicates, 2);
    }

    #[tokio::test]
    async fn import_dedup_does_not_collapse_different_bands() {
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo);

        let mk = |band: Band| CreateQsoCommand {
            band: Some(band),
            mode: Some(Mode::FT8),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            ..CreateQsoCommand::minimal(
                Callsign::parse("W1AW").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
            )
        };

        let report = svc.import_qsos(vec![mk(Band::M20), mk(Band::M40)]).await;
        // Same call, same date, same mode, DIFFERENT band — both keepers.
        assert_eq!(report.created, 2);
        assert_eq!(report.duplicates, 0);
    }

    #[tokio::test]
    async fn edit_then_delete_qso_round_trip() {
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo.clone());

        let id = svc
            .create_qso(CreateQsoCommand {
                band: Some(Band::M40),
                rst_sent: Some("599".into()),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("W1AW").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                )
            })
            .await
            .unwrap();

        // Edit: change band, rst, and call.
        svc.update_qso(
            id,
            CreateQsoCommand {
                band: Some(Band::M20),
                rst_sent: Some("57".into()),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("VE3XYZ").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                )
            },
        )
        .await
        .unwrap();

        let qso = svc.get_qso(&id).await.unwrap().unwrap();
        assert_eq!(qso.call.as_str(), "VE3XYZ");
        assert_eq!(qso.band, Some(Band::M20));
        assert_eq!(qso.rst_sent.as_deref(), Some("57"));

        // Delete: soft-delete; doesn't show up in search results anymore.
        svc.delete_qso(&id).await.unwrap();
        let results = svc.search_qsos(QsoSearch::default()).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn upload_verified_state_transitions() {
        use logbook_domain::QsoRepository;

        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo.clone());

        let id = svc
            .create_qso(CreateQsoCommand {
                band: Some(Band::M20),
                station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("W1AW").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                )
            })
            .await
            .unwrap();

        // Path A: verified-without-prior-upload (e.g. first fetch sees a
        // QSO we never tried to push — possible if user manually uploaded
        // outside slogger). The state should still land on 'verified'.
        repo.mark_upload_verified(&id, "lotw", Utc::now(), Some("lotw-A"))
            .await
            .unwrap();
        let pending = repo.list_pending_uploads("lotw", None).await.unwrap();
        assert!(pending.is_empty(), "verified QSO should not be pending");

        // Path B: normal — upload then verify. Verifying again is idempotent.
        repo.mark_uploaded(&id, "lotw", Utc::now(), None).await.unwrap();
        repo.mark_upload_verified(&id, "lotw", Utc::now(), None)
            .await
            .unwrap();

        // Path C: failed → verified (e.g. transient HTTP failure cleared
        // by next sync seeing the QSO landed). last_error must clear.
        let id2 = svc
            .create_qso(CreateQsoCommand {
                band: Some(Band::M40),
                station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("VE3XYZ").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 19, 0, 0).unwrap(),
                )
            })
            .await
            .unwrap();
        repo.mark_upload_failed(&id2, "lotw", "transient HTTP")
            .await
            .unwrap();
        let pending = repo.list_pending_uploads("lotw", None).await.unwrap();
        assert_eq!(pending.len(), 1);
        repo.mark_upload_verified(&id2, "lotw", Utc::now(), None)
            .await
            .unwrap();
        let pending = repo.list_pending_uploads("lotw", None).await.unwrap();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn match_lookup_finds_qso_by_canonical_keys() {
        use logbook_domain::QsoRepository;

        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let svc = LogbookService::new(repo.clone());

        let id = svc
            .create_qso(CreateQsoCommand {
                band: Some(Band::M20),
                mode: Some(Mode::FT8),
                station_callsign: Some(Callsign::parse("K2A").unwrap()),
                ..CreateQsoCommand::minimal(
                    Callsign::parse("W1AW").unwrap(),
                    Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
                )
            })
            .await
            .unwrap();

        let found = repo
            .find_match_for_confirmation("K2A", "W1AW", "2026-05-08", Some("20M"), Some("FT8"))
            .await
            .unwrap();
        assert_eq!(found, Some(id));

        // Wrong band → no match.
        let none = repo
            .find_match_for_confirmation("K2A", "W1AW", "2026-05-08", Some("40M"), Some("FT8"))
            .await
            .unwrap();
        assert!(none.is_none());

        // Band optional — match without it.
        let found_loose = repo
            .find_match_for_confirmation("K2A", "W1AW", "2026-05-08", None, None)
            .await
            .unwrap();
        assert_eq!(found_loose, Some(id));
    }

    #[tokio::test]
    async fn unknown_call_skipped_by_resolver() {
        use station_resolver::CtyDbResolver;

        let db = Database::open_in_memory().await.unwrap();
        let repo = Arc::new(SqliteQsoRepository::new(&db));
        let resolver = Arc::new(CtyDbResolver::from_reader(MINI_CTY.as_bytes()).unwrap());
        let svc = LogbookService::with_resolver(repo, resolver);

        svc.create_qso(CreateQsoCommand {
            band: Some(Band::M20),
            ..CreateQsoCommand::minimal(
                Callsign::parse("ZZ9ZZZ").unwrap(),
                Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap(),
            )
        })
        .await
        .unwrap();

        let results = svc.search_qsos(QsoSearch::default()).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].dxcc_prefix.is_none());
        assert!(results[0].continent.is_none());
    }

    // ---- DXKeeper-shaped import: locations, credit, new columns ----

    use crate::station_repo::SqliteStationRepository;
    use logbook_domain::{ExportOptions, LocationFilter, StationRepository, parse_adif};

    fn f(name: &str, value: &str) -> String {
        format!("<{name}:{}>{value}", value.len())
    }

    /// A synthetic ADIF record from station K2A worked on 20m CW.
    fn rec(call: &str, date: &str, time: &str, extra: &str) -> String {
        format!(
            "{}{}{}{}{}{}{}{extra}<EOR>\n",
            f("CALL", call),
            f("QSO_DATE", date),
            f("TIME_ON", time),
            f("BAND", "20M"),
            f("MODE", "CW"),
            f("STATION_CALLSIGN", "K2A"),
            f("OWNER_CALLSIGN", "K2B"),
        )
    }

    async fn import_text(svc: &LogbookService, adif: &str) -> logbook_domain::ImportReport {
        let outcome = parse_adif(&format!("<EOH>\n{adif}")).unwrap();
        assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
        svc.import_qsos(outcome.commands).await
    }

    async fn count(db: &Database, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(db.pool()).await.unwrap()
    }

    #[tokio::test]
    async fn import_creates_each_qth_once_reuses_existing_and_leaves_rest_unassigned() {
        let (svc, db) = fresh_service_with_db().await;
        let stations = SqliteStationRepository::new(&db);
        let now = Utc::now();
        let existing = radio_core::StationLocation {
            id: StationLocationId::new(),
            name: "Home".into(),
            station_callsign: Some(Callsign::parse("W1AW").unwrap()),
            owner_callsign: None,
            city: None,
            county: None,
            state: None,
            country: None,
            grid: Some("FN31".into()),
            latitude: None,
            longitude: None,
            cq_zone: None,
            itu_zone: None,
            iota: None,
            lotw_station_location: None,
            eqsl_account: None,
            created_at: now,
            updated_at: now,
        };
        stations.insert_location(&existing).await.unwrap();

        let qth = |n: &str| f("APP_DXKEEPER_MY_QTHID", n);
        let adif = [
            rec("W1AW", "20240101", "120000", &qth("Home")),
            rec("K1ABC", "20240101", "120100", &qth("Portable")),
            rec("K1ABD", "20240101", "120200", &qth("Portable")),
            rec("K1ABE", "20240101", "120300", ""),
        ]
        .concat();
        let report = import_text(&svc, &adif).await;
        assert_eq!(report.created, 4);
        assert_eq!(report.locations_created, 1);

        let locations = stations.list_locations().await.unwrap();
        assert_eq!(locations.len(), 2);
        let portable = locations.iter().find(|l| l.name == "Portable").unwrap();
        // Callsigns from the first QSO naming it; nothing else invented.
        assert_eq!(portable.station_callsign.as_ref().unwrap().as_str(), "K2A");
        assert_eq!(portable.owner_callsign.as_ref().unwrap().as_str(), "K2B");
        assert_eq!(portable.grid, None);

        let at = |filter| QsoSearch {
            station_location: filter,
            ..Default::default()
        };
        assert_eq!(svc.count_matching(at(LocationFilter::Is(existing.id))).await.unwrap(), 1);
        assert_eq!(svc.count_matching(at(LocationFilter::Is(portable.id))).await.unwrap(), 2);
        let unassigned = svc.search_qsos(at(LocationFilter::Unassigned)).await.unwrap();
        assert_eq!(unassigned.len(), 1);
        assert_eq!(unassigned[0].call.as_str(), "K1ABE");
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM qso_exchange_fields WHERE field_name = 'APP_DXKEEPER_MY_QTHID'").await,
            0
        );

        // Re-importing the same file creates nothing new.
        let again = import_text(&svc, &adif).await;
        assert_eq!((again.created, again.duplicates, again.locations_created), (0, 4, 0));
    }

    #[tokio::test]
    async fn my_fields_fill_a_new_location_and_differing_values_stay_with_the_qso() {
        let (svc, db) = fresh_service_with_db().await;
        let adif = [
            rec(
                "W1AW",
                "20240101",
                "120000",
                &(f("MY_GRIDSQUARE", "FN31pr") + &f("MY_CITY", "Newington")
                    + &f("MY_CQ_ZONE", "5") + &f("MY_LAT", "N041 42.857")),
            ),
            // Same derived name, different grid square on this QSO.
            rec(
                "K1ABC",
                "20240101",
                "120100",
                &(f("MY_GRIDSQUARE", "FN31pr") + &f("MY_CITY", "Newington")
                    + &f("MY_CQ_ZONE", "4")),
            ),
        ]
        .concat();
        let report = import_text(&svc, &adif).await;
        assert_eq!((report.created, report.locations_created), (2, 1));
        let loc = &SqliteStationRepository::new(&db).list_locations().await.unwrap()[0];
        assert_eq!(loc.name, "Newington FN31pr");
        assert_eq!(loc.grid.as_deref(), Some("FN31pr"));
        assert_eq!(loc.city.as_deref(), Some("Newington"));
        assert_eq!(loc.cq_zone, Some(5));
        assert!((loc.latitude.unwrap() - 41.71428).abs() < 1e-4);
        let extras: Vec<(String, String)> = sqlx::query_as(
            "SELECT field_name, raw_value FROM qso_exchange_fields ORDER BY field_name",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(extras, [("MY_CQ_ZONE".to_string(), "4".to_string())]);
    }

    #[tokio::test]
    async fn bulk_assign_location_covers_every_match_not_one_page() {
        let (svc, db) = fresh_service_with_db().await;
        let adif: String = (0..7)
            .map(|i| rec(&format!("K1A{}", (b'A' + i) as char), "20240101", "120000", ""))
            .collect::<Vec<_>>()
            .concat()
            + &rec("W1AW", "20240102", "120000", &f("APP_DXKEEPER_MY_QTHID", "Home"));
        import_text(&svc, &adif).await;
        let stations = SqliteStationRepository::new(&db);
        let home = stations.list_locations().await.unwrap().remove(0);

        let unassigned = QsoSearch {
            station_location: LocationFilter::Unassigned,
            limit: Some(3),
            ..Default::default()
        };
        assert_eq!(svc.search_qsos(unassigned.clone()).await.unwrap().len(), 3);
        let report = svc.bulk_assign_location_by_search(unassigned.clone(), home.id).await.unwrap();
        assert_eq!(report, logbook_domain::BulkReport { matched: 7, succeeded: 7 });
        assert_eq!(svc.count_matching(unassigned).await.unwrap(), 0);
        let at_home = QsoSearch {
            station_location: LocationFilter::Is(home.id),
            ..Default::default()
        };
        assert_eq!(svc.count_matching(at_home).await.unwrap(), 8);

        // Other filters still narrow the assignment.
        let narrowed = QsoSearch {
            exact_call: Some(Callsign::parse("W1AW").unwrap()),
            ..Default::default()
        };
        let report = svc.bulk_assign_location_by_search(narrowed, home.id).await.unwrap();
        assert_eq!(report.matched, 1);
    }

    #[tokio::test]
    async fn pending_uploads_can_be_counted_by_location_and_listed_for_some_locations() {
        let (svc, db) = fresh_service_with_db().await;
        let qth = |n: &str| f("APP_DXKEEPER_MY_QTHID", n);
        let adif = [
            rec("W1AA", "20240101", "120000", &qth("Home")),
            rec("W1AB", "20240102", "120000", &qth("Home")),
            rec("W1AC", "20240103", "120000", &qth("Home")),
            rec("W1AD", "20240104", "120000", &qth("Portable")),
            rec("W1AE", "20240105", "120000", ""),
        ]
        .concat();
        import_text(&svc, &adif).await;
        sqlx::query("UPDATE qsos SET station_callsign = NULL WHERE call = 'W1AC'")
            .execute(db.pool())
            .await
            .unwrap();
        let repo = SqliteQsoRepository::new(&db);
        let locations = SqliteStationRepository::new(&db).list_locations().await.unwrap();
        let id_of = |name: &str| locations.iter().find(|l| l.name == name).unwrap().id;
        let (home, portable) = (id_of("Home"), id_of("Portable"));

        let mut counts = repo.count_pending_uploads_by_location("lotw").await.unwrap();
        counts.sort_by_key(|c| (c.station_location_id.map(|i| i.as_uuid()), c.has_station_callsign));
        let mut expected = vec![
            PendingUploadCount { station_location_id: None, has_station_callsign: true, count: 1 },
            PendingUploadCount { station_location_id: Some(home), has_station_callsign: false, count: 1 },
            PendingUploadCount { station_location_id: Some(home), has_station_callsign: true, count: 2 },
            PendingUploadCount { station_location_id: Some(portable), has_station_callsign: true, count: 1 },
        ];
        expected.sort_by_key(|c| (c.station_location_id.map(|i| i.as_uuid()), c.has_station_callsign));
        assert_eq!(counts, expected);

        // Only Home's QSOs with a station callsign, newest first, limit
        // applied after that selection.
        let at_home = repo.list_pending_uploads_at("lotw", &[home], Some(1)).await.unwrap();
        assert_eq!(at_home.len(), 1);
        assert_eq!(at_home[0].call.as_str(), "W1AB");
        let both = repo.list_pending_uploads_at("lotw", &[home, portable], None).await.unwrap();
        assert_eq!(both.len(), 3);

        repo.mark_uploaded(&at_home[0].id, "lotw", Utc::now(), None).await.unwrap();
        let at_home = repo.list_pending_uploads_at("lotw", &[home], None).await.unwrap();
        assert_eq!(at_home.iter().map(|q| q.call.as_str()).collect::<Vec<_>>(), ["W1AA"]);
        assert!(repo.list_pending_uploads_at("lotw", &[], None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn credits_land_in_award_tables_and_count_as_credited() {
        let (svc, db) = fresh_service_with_db().await;
        let adif = [
            rec(
                "W1AW",
                "20240101",
                "120000",
                &(f("DXCC", "291") + &f("STATE", "CT") + &f("GRIDSQUARE", "fn31pr")
                    + &f("CREDIT_GRANTED", "DXCC:LOTW, DXCC_BAND:LOTW, DXCC_MODE:LOTW,WAS")
                    + &f("CREDIT_SUBMITTED", "VUCC_BAND")),
            ),
            rec("VE3XYZ", "20240101", "120100", &(f("DXCC", "1") + &f("CREDIT_GRANTED", "DXCC:CARD"))),
            rec("JA1ABC", "20240101", "120200", &f("DXCC", "339")),
        ]
        .concat();
        assert_eq!(import_text(&svc, &adif).await.created, 3);

        assert_eq!(count(&db, "SELECT COUNT(*) FROM award_definitions").await, 5);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM award_credit_imports").await, 6);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM award_credit_links WHERE link_status = 'granted'").await, 5);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM award_credit_links WHERE link_status = 'submitted'").await, 1);
        let units: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT d.name, i.unit, i.qso_date, i.band FROM award_credit_imports i \
             JOIN award_definitions d ON d.id = i.award_id WHERE i.call = 'W1AW' ORDER BY d.name",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        let units: Vec<(&str, &str)> = units.iter().map(|u| (u.0.as_str(), u.1.as_str())).collect();
        assert_eq!(
            units,
            [("DXCC", "291"), ("DXCC_BAND", "291"), ("DXCC_MODE", "291"), ("VUCC_BAND", "FN31"), ("WAS", "CT")]
        );
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM qso_exchange_fields WHERE field_name LIKE 'CREDIT_%'").await,
            0
        );

        let w1aw = svc.search_qsos(QsoSearch {
            exact_call: Some(Callsign::parse("W1AW").unwrap()),
            ..Default::default()
        });
        let id = w1aw.await.unwrap()[0].id;
        let detail = svc.get_qso_detail(&id).await.unwrap().unwrap();
        assert_eq!(detail.credits.len(), 5);
        assert_eq!(detail.credits[0].award, "DXCC");
        assert_eq!(detail.credits[0].medium.as_deref(), Some("LOTW"));
        assert_eq!(detail.credits[3].medium, None);
        assert_eq!(detail.credits[4].status, CreditStatus::Submitted);

        let award_qsos = svc_repo(&db).list_award_qsos().await.unwrap();
        let snap = logbook_domain::snapshot(&award_qsos, 2024);
        assert_eq!(snap.dxcc.worked, 3);
        assert_eq!(snap.dxcc.confirmed, 0);
        assert_eq!(snap.dxcc.credited, 2);
        assert_eq!(snap.dxcc_by_band[&Band::M20].credited, 1);
        assert_eq!(snap.was.credited, 1);
    }

    fn svc_repo(db: &Database) -> SqliteQsoRepository {
        SqliteQsoRepository::new(db)
    }

    #[tokio::test]
    async fn qrz_logid_becomes_remote_id_even_without_an_upload_status() {
        let (svc, db) = fresh_service_with_db().await;
        let adif = [
            rec(
                "W1AW",
                "20240101",
                "120000",
                &(f("QRZCOM_QSO_UPLOAD_STATUS", "Y") + &f("APP_DXKEEPER_QRZCOM_LOGID", "1001")),
            ),
            rec(
                "K1ABC",
                "20240101",
                "120100",
                &(f("QRZCOM_QSO_UPLOAD_STATUS", "M") + &f("APP_DXKEEPER_QRZCOM_LOGID", "1002")),
            ),
        ]
        .concat();
        import_text(&svc, &adif).await;
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT remote_id, upload_state FROM qso_service_state WHERE service = 'qrz' ORDER BY remote_id",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(
            rows,
            [("1001".into(), "uploaded".into()), ("1002".into(), "pending".into())]
        );
        // `M` still means "upload again".
        let pending = svc_repo(&db).list_pending_uploads("qrz", None).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].call.as_str(), "K1ABC");
    }

    /// Everything a QSO carries, minus what is assigned per database
    /// (ids, timestamps) — for comparing two imports of the same data.
    fn fingerprint(q: &Qso, locations: &[radio_core::StationLocation]) -> String {
        let loc = q
            .station_location_id
            .map(|id| locations.iter().find(|l| l.id == id).unwrap())
            .map(|l| (&l.name, &l.grid, &l.city, l.cq_zone, &l.station_callsign));
        let q = Qso {
            id: QsoId::from_uuid(Uuid::nil()),
            station_location_id: None,
            created_at: q.qso_begin,
            updated_at: q.qso_begin,
            ..q.clone()
        };
        format!("{q:?} {loc:?}")
    }

    #[tokio::test]
    async fn export_then_import_into_an_empty_log_reproduces_columns_locations_and_credit() {
        let (svc, db) = fresh_service_with_db().await;
        let full = [
            ("FREQ", "14.025500"),
            ("FREQ_RX", "14.027000"),
            ("BAND_RX", "20M"),
            ("TIME_OFF", "000130"),
            ("RST_SENT", "599"),
            ("RST_RCVD", "579"),
            ("OPERATOR", "K1ABC"),
            ("DXCC", "291"),
            ("APP_DXKEEPER_DXCCPREFIX", "K"),
            ("PFX", "W1"),
            ("CONT", "NA"),
            ("CQZ", "5"),
            ("ITUZ", "8"),
            ("GRIDSQUARE", "FN31pr"),
            ("STATE", "CT"),
            ("CNTY", "CT,Hartford"),
            ("LAT", "N041 42.857"),
            ("LON", "W072 43.698"),
            ("NAME", "Art"),
            ("QTH", "Newington"),
            ("ADDRESS", "1 Main St\r\nNewington CT"),
            ("EMAIL", "art@example.com"),
            ("AGE", "42"),
            ("QSL_VIA", "W1AW"),
            ("COMMENT", "synthetic"),
            ("CONTEST_ID", "cwt"),
            ("STX", "17"),
            ("SRX", "1234"),
            ("STX_STRING", "CHAD 1"),
            ("SRX_STRING", "ART CT"),
            ("ARRL_SECT", "CT"),
            ("TX_PWR", "100"),
            ("RX_PWR", "0"),
            ("DISTANCE", "1234.5"),
            ("ANT_AZ", "87"),
            ("SFI", "150"),
            ("A_INDEX", "7"),
            ("K_INDEX", "2"),
            ("CREDIT_GRANTED", "DXCC:LOTW, DXCC_BAND:LOTW,WAS"),
            ("CREDIT_SUBMITTED", "VUCC_BAND"),
            ("APP_DXKEEPER_MY_QTHID", "Home"),
            ("MY_GRIDSQUARE", "FN31pr"),
            ("MY_CQ_ZONE", "5"),
            ("LOTW_QSL_SENT", "Y"),
            ("QSL_SENT", "N"),
            ("APP_DXKEEPER_QRZCOM_LOGID", "1001"),
        ]
        .iter()
        .map(|(n, v)| f(n, v))
        .collect::<String>();
        let adif = [
            rec("W1AW", "20240101", "235930", &full),
            rec("K1ABE", "20240102", "120000", &f("STX", "5NN")),
        ]
        .concat();
        assert_eq!(import_text(&svc, &adif).await.created, 2);

        let stations = SqliteStationRepository::new(&db);
        let locations = stations.list_locations().await.unwrap();
        let exported = svc
            .export_adif_by_search(QsoSearch::default(), &locations, &ExportOptions::default())
            .await
            .unwrap();

        let (svc2, db2) = fresh_service_with_db().await;
        let outcome = parse_adif(&exported).unwrap();
        assert!(outcome.skipped.is_empty());
        assert_eq!(svc2.import_qsos(outcome.commands).await.created, 2);
        let locations2 = SqliteStationRepository::new(&db2).list_locations().await.unwrap();
        assert_eq!(locations2.len(), 1);

        let mut before: Vec<String> = svc
            .search_full_qsos(QsoSearch::default())
            .await
            .unwrap()
            .iter()
            .map(|q| fingerprint(q, &locations))
            .collect();
        let mut after: Vec<String> = svc2
            .search_full_qsos(QsoSearch::default())
            .await
            .unwrap()
            .iter()
            .map(|q| fingerprint(q, &locations2))
            .collect();
        before.sort();
        after.sort();
        assert_eq!(before, after);

        // Credit, extra fields and service state rebuilt identically.
        for sql in [
            "SELECT d.name || ':' || COALESCE(json_extract(i.raw_json, '$.medium'), '') || ':' \
             || l.link_status || ':' || i.unit FROM award_credit_links l \
             JOIN award_credit_imports i ON i.id = l.credit_import_id \
             JOIN award_definitions d ON d.id = i.award_id ORDER BY 1",
            "SELECT field_name || '=' || raw_value FROM qso_exchange_fields ORDER BY 1",
            "SELECT service || ':' || upload_state || ':' || COALESCE(remote_id, '') \
             FROM qso_service_state ORDER BY 1",
        ] {
            let a: Vec<String> = sqlx::query_scalar(sql).fetch_all(db.pool()).await.unwrap();
            let b: Vec<String> = sqlx::query_scalar(sql).fetch_all(db2.pool()).await.unwrap();
            assert!(!a.is_empty(), "{sql}");
            assert_eq!(a, b, "{sql}");
        }
        let extras: Vec<String> = sqlx::query_scalar(
            "SELECT field_name FROM qso_exchange_fields ORDER BY 1",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(extras, ["APP_DXKEEPER_QRZCOM_LOGID", "LOTW_QSL_SENT", "QSL_SENT", "STX"]);
    }

    #[tokio::test]
    async fn lotw_report_applies_once_per_qso_and_is_idempotent() {
        use logbook_domain::credit::parse_credit_list;
        use logbook_domain::{QsoRepository, ServiceQso, apply_report};

        let (svc, db) = fresh_service_with_db().await;
        let repo = svc_repo(&db);
        let at = |h, m| Utc.with_ymd_and_hms(2026, 5, 8, h, m, 0).unwrap();
        let qso = |call: &str, band, h, m| CreateQsoCommand {
            band: Some(band),
            mode: Some(Mode::CW),
            station_callsign: Some(Callsign::parse("K1ABC").unwrap()),
            dxcc_id: Some(291),
            ..CreateQsoCommand::minimal(Callsign::parse(call).unwrap(), at(h, m))
        };
        // Same station, same band, same day, three minutes apart.
        let first = svc.create_qso(qso("W1AW", Band::M20, 10, 0)).await.unwrap();
        let second = svc.create_qso(qso("W1AW", Band::M20, 10, 3)).await.unwrap();
        let deleted = svc.create_qso(qso("W1AW", Band::M40, 12, 0)).await.unwrap();
        repo.soft_delete_qso(&deleted).await.unwrap();

        let record = |band: &str, h, m, confirmed: bool| ServiceQso {
            station_callsign: Some("K1ABC".into()),
            call: "W1AW".into(),
            start: at(h, m),
            band: Some(band.into()),
            mode: Some("CW".into()),
            mode_group: Some("CW".into()),
            confirmed,
            confirmed_at: confirmed.then(|| at(20, 0)),
            uploaded_at: Some(at(11, 0)),
            credits: Vec::new(),
            dxcc_id: Some(291),
            state: None,
            grid: None,
            detail: Vec::new(),
        };
        let mut second_rec = record("20M", 10, 3, true);
        second_rec.credits = parse_credit_list("DXCC:LOTW", CreditStatus::Submitted);
        let report = vec![
            second_rec.clone(),
            record("20M", 10, 0, false),
            record("40M", 12, 0, true),
        ];
        let state = |id| {
            let repo = &repo;
            async move {
                repo.list_service_states(&id)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|s| (s.upload_state, s.confirmation_state, s.confirmed_at))
                    .collect::<Vec<_>>()
            }
        };

        let out = apply_report(&repo, "lotw", &report, "qsl_since", Some("2026-05-09 01:02:03".into()), at(23, 0))
            .await
            .unwrap();
        assert_eq!((out.newly_verified, out.newly_confirmed, out.credits_changed), (2, 1, 1));
        // The soft-deleted QSO never matches.
        assert_eq!(out.not_in_log, vec![2]);
        assert_eq!(
            state(second).await,
            vec![("verified".into(), "confirmed".into(), Some(at(20, 0)))]
        );
        assert_eq!(state(first).await, vec![("verified".into(), "pending".into(), None)]);
        assert!(state(deleted).await.is_empty());
        assert_eq!(
            repo.get_sync_watermark("lotw", "qsl_since").await.unwrap().as_deref(),
            Some("2026-05-09 01:02:03")
        );
        assert_eq!(repo.get_sync_watermark("lotw", "qso_rx_since").await.unwrap(), None);

        // The same report again (the boundary record always comes back):
        // nothing changes, nothing is duplicated. Without a new watermark
        // the stored one stays.
        let out = apply_report(&repo, "lotw", &report, "qsl_since", None, at(23, 30))
            .await
            .unwrap();
        assert_eq!((out.newly_verified, out.newly_confirmed, out.credits_changed), (0, 0, 0));
        let links = || async {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM award_credit_links")
                .fetch_one(db.pool())
                .await
                .unwrap()
        };
        assert_eq!(links().await, 1);
        assert_eq!(
            repo.get_sync_watermark("lotw", "qsl_since").await.unwrap().as_deref(),
            Some("2026-05-09 01:02:03")
        );

        // Not a confirmation any more (as the uploads report would say):
        // still confirmed, date kept.
        let unconfirmed = vec![record("20M", 10, 3, false)];
        apply_report(&repo, "lotw", &unconfirmed, "qso_rx_since", Some("x".into()), at(23, 45))
            .await
            .unwrap();
        assert_eq!(
            state(second).await,
            vec![("verified".into(), "confirmed".into(), Some(at(20, 0)))]
        );

        // DXCC is granted now: the submitted credit is raised, not doubled.
        second_rec.credits = parse_credit_list("DXCC:LOTW", CreditStatus::Granted);
        let out = apply_report(&repo, "lotw", &[second_rec], "qsl_since", None, at(23, 50))
            .await
            .unwrap();
        assert_eq!(out.credits_changed, 1);
        assert_eq!(links().await, 1);
        let credits = repo.list_credits(&second).await.unwrap();
        assert_eq!(credits.len(), 1);
        assert_eq!(credits[0].award, "DXCC");
        assert_eq!(credits[0].medium.as_deref(), Some("LOTW"));
        assert_eq!(credits[0].status, CreditStatus::Granted);
        let unit: String = sqlx::query_scalar("SELECT unit FROM award_credit_imports")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(unit, "291");
    }

    #[tokio::test]
    async fn lotw_detail_fills_blanks_records_differences_and_stays_idempotent() {
        use logbook_domain::{DetailField, ServiceQso, apply_report};

        let (svc, db) = fresh_service_with_db().await;
        let repo = svc_repo(&db);
        let at = |h, m| Utc.with_ymd_and_hms(2026, 5, 8, h, m, 0).unwrap();
        let mut cmd = CreateQsoCommand {
            band: Some(Band::M20),
            mode: Some(Mode::CW),
            station_callsign: Some(Callsign::parse("K1ABC").unwrap()),
            ..CreateQsoCommand::minimal(Callsign::parse("W9XYZ").unwrap(), at(10, 0))
        };
        cmd.dxcc_id = Some(291);
        cmd.grid = Some("EN61".into());
        cmd.state = Some("IL".into());
        let id = svc.create_qso(cmd.clone()).await.unwrap();
        // Same station on another band: no record for it, never touched.
        cmd.band = Some(Band::M40);
        cmd.grid = None;
        cmd.state = None;
        let untouched = svc.create_qso(cmd).await.unwrap();

        let raw = |pairs: &[(DetailField, &str)]| {
            pairs.iter().map(|(f, v)| (*f, v.to_string())).collect::<Vec<_>>()
        };
        let record = |detail: Vec<(DetailField, String)>| ServiceQso {
            station_callsign: Some("K1ABC".into()),
            call: "W9XYZ".into(),
            start: at(10, 0),
            band: Some("20M".into()),
            mode: Some("CW".into()),
            mode_group: None,
            confirmed: true,
            confirmed_at: Some(at(20, 0)),
            uploaded_at: None,
            credits: Vec::new(),
            dxcc_id: Some(291),
            state: None,
            grid: None,
            detail,
        };
        let detail = raw(&[
            (DetailField::Dxcc, "291"),          // agrees
            (DetailField::Gridsquare, "EN61ax"), // refines EN61
            (DetailField::State, "IN"),          // differs from IL
            (DetailField::Cnty, "IN,Lake"),      // fills
            (DetailField::Cont, "na"),           // fills, upper-cased
            (DetailField::Cqz, "04"),            // fills, as a number
        ]);
        let sync = |detail: Vec<(DetailField, String)>, when| {
            let repo = &repo;
            async move {
                apply_report(repo, "lotw", &[record(detail)], "qsl_since", None, when)
                    .await
                    .unwrap()
            }
        };
        let rows = || async {
            sqlx::query_as::<_, (String, String, Option<String>)>(
                "SELECT field_name, source, source_detail FROM qso_field_provenance \
                 ORDER BY field_name",
            )
            .fetch_all(db.pool())
            .await
            .unwrap()
        };
        let row = |f: &str, s: &str, d: &str| (f.to_string(), s.to_string(), Some(d.to_string()));

        let out = sync(detail.clone(), at(23, 0)).await;
        assert_eq!((out.newly_confirmed, out.qsos_filled, out.qsos_differing), (1, 1, 1));
        let q = repo.get_qso(&id).await.unwrap().unwrap();
        assert_eq!(q.grid.as_deref(), Some("EN61ax"));
        assert_eq!(q.state.as_deref(), Some("IL"), "a differing value is kept");
        assert_eq!(q.county.as_deref(), Some("IN,Lake"));
        assert_eq!(q.continent.as_deref(), Some("NA"));
        assert_eq!(q.cq_zone, Some(4));
        assert_eq!(q.dxcc_id, Some(291));
        assert_eq!(q.itu_zone, None);
        let first_rows = vec![
            row("CNTY", "service_sync:lotw", "IN,Lake"),
            row("CONT", "service_sync:lotw", "NA"),
            row("CQZ", "service_sync:lotw", "4"),
            row("GRIDSQUARE", "service_sync:lotw", "EN61ax"),
            row("STATE", "service_differs:lotw", "IN"),
        ];
        assert_eq!(rows().await, first_rows);
        let other = repo.get_qso(&untouched).await.unwrap().unwrap();
        assert_eq!((other.county, other.grid), (None, None));

        // The detail pane gets the rows.
        let pane = svc.get_qso_detail(&id).await.unwrap().unwrap();
        assert_eq!(pane.provenance.len(), 5);

        // The filter lists exactly the QSO that differs, through all three
        // search paths.
        let differs = QsoSearch {
            differs_from_lotw: true,
            ..Default::default()
        };
        let hits = svc.search_qsos(differs.clone()).await.unwrap();
        assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), vec![id]);
        assert_eq!(svc.count_matching(differs.clone()).await.unwrap(), 1);
        assert_eq!(svc.search_full_qsos(differs.clone()).await.unwrap().len(), 1);
        assert_eq!(svc.count_matching(QsoSearch::default()).await.unwrap(), 2);

        // The uploads report lists the confirmed QSO too, without detail:
        // nothing about the detail changes.
        let out = sync(Vec::new(), at(23, 10)).await;
        assert_eq!((out.qsos_filled, out.qsos_differing), (0, 0));
        assert_eq!(rows().await, first_rows);

        // The same record again: nothing changes, nothing is added.
        let updated = repo.get_qso(&id).await.unwrap().unwrap().updated_at;
        let out = sync(detail.clone(), at(23, 30)).await;
        assert_eq!((out.qsos_filled, out.qsos_differing), (0, 0));
        assert_eq!(rows().await, first_rows);
        assert_eq!(repo.get_qso(&id).await.unwrap().unwrap().updated_at, updated);

        // An edit keeps the rows; the operator corrects the state to
        // LoTW's.
        let mut edit = CreateQsoCommand::from_qso(&repo.get_qso(&id).await.unwrap().unwrap());
        edit.state = Some("in".into());
        svc.update_qso(id, edit).await.unwrap();
        assert_eq!(rows().await, first_rows);
        // The next sync of the record drops the difference; the filter no
        // longer finds it.
        let out = sync(detail.clone(), at(23, 40)).await;
        assert_eq!((out.qsos_filled, out.qsos_differing), (0, 0));
        assert_eq!(rows().await, first_rows[..4].to_vec());
        assert_eq!(svc.count_matching(differs.clone()).await.unwrap(), 0);

        // LoTW's value changes: the one row for the field is rewritten.
        let mut changed = detail.clone();
        changed[3] = (DetailField::Cnty, "IN,Porter".into());
        let out = sync(changed, at(23, 50)).await;
        assert_eq!((out.qsos_filled, out.qsos_differing), (0, 1));
        let now = rows().await;
        assert_eq!(now[0], row("CNTY", "service_differs:lotw", "IN,Porter"));
        assert_eq!(now.len(), 4);
        assert_eq!(
            repo.get_qso(&id).await.unwrap().unwrap().county.as_deref(),
            Some("IN,Lake")
        );

        // Filled values count toward awards (computed live from columns),
        // and export carries them as ordinary fields, not the bookkeeping.
        let award = repo.list_award_qsos().await.unwrap();
        let mine = award.iter().find(|a| a.id == id).unwrap();
        assert_eq!(mine.continent.as_deref(), Some("NA"));
        let adif = svc
            .export_adif_by_search(
                QsoSearch {
                    exact_call: Some(Callsign::parse("W9XYZ").unwrap()),
                    band: Some(Band::M20),
                    ..Default::default()
                },
                &[],
                &logbook_domain::ExportOptions::default(),
            )
            .await
            .unwrap();
        assert!(adif.contains("<CNTY:7>IN,Lake"), "{adif}");
        assert!(!adif.contains("service_") && !adif.contains("IN,Porter"), "{adif}");
    }
}
