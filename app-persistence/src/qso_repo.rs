use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use logbook_domain::{
    AwardQso, DedupKey, ImportedServiceState, QsoRepository, QsoSearch, QsoSummary, RepoResult,
    RepositoryError,
};
use std::collections::HashSet;

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
    if let Some(loc_id) = query.station_location_id {
        sql.push_str(" AND q.station_location_id = ?");
        binds.push(DynBind::Text(fmt_id(loc_id.as_uuid())));
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

    (sql, binds)
}
use radio_core::{
    Band, Callsign, FieldSource, Mode, OperatingSessionId, OperatorId, PropagationMode, Qso,
    QsoExchangeField, QsoId, StationLocationId,
};

use crate::db::Database;

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
    let operating_session_id: Option<String> =
        row.try_get("operating_session_id").map_err(map_err)?;
    let station_callsign: Option<String> = row.try_get("station_callsign").map_err(map_err)?;
    let owner_callsign: Option<String> = row.try_get("owner_callsign").map_err(map_err)?;
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
        operating_session_id: operating_session_id
            .as_deref()
            .map(parse_uuid)
            .transpose()?
            .map(OperatingSessionId::from_uuid),
        station_callsign: station_callsign.as_deref().map(parse_callsign).transpose()?,
        owner_callsign: owner_callsign.as_deref().map(parse_callsign).transpose()?,
        dxcc_id: dxcc_id.map(|v| v as u16),
        dxcc_prefix,
        continent,
        cq_zone: cq_zone.map(|v| v as u8),
        itu_zone: itu_zone.map(|v| v as u8),
        grid,
        state,
        county,
        province,
        iota,
        tx_power_w: tx_power_w.map(|v| v as f32),
        rx_power_w: rx_power_w.map(|v| v as f32),
        propagation_mode: propagation_mode.as_deref().map(propagation_from_str),
        sat_name,
        sat_mode,
        distance_km,
        bearing_deg,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

#[async_trait]
impl QsoRepository for SqliteQsoRepository {
    async fn insert_qso(&self, qso: &Qso) -> RepoResult<()> {
        sqlx::query(
            r#"
            INSERT INTO qsos (
                id, call, qso_begin, qso_end,
                band, freq_hz, mode, submode,
                rst_sent, rst_rcvd,
                operator_id, station_location_id, operating_session_id,
                station_callsign, owner_callsign,
                dxcc_id, dxcc_prefix, continent, cq_zone, itu_zone,
                grid, state, county, province, iota,
                tx_power_w, rx_power_w,
                propagation_mode, sat_name, sat_mode,
                distance_km, bearing_deg,
                created_at, updated_at
            ) VALUES (
                ?, ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?,
                ?, ?, ?,
                ?, ?,
                ?, ?, ?, ?, ?,
                ?, ?, ?, ?, ?,
                ?, ?,
                ?, ?, ?,
                ?, ?,
                ?, ?
            )
            "#,
        )
        .bind(fmt_id(qso.id.as_uuid()))
        .bind(qso.call.as_str())
        .bind(fmt_dt(&qso.qso_begin))
        .bind(qso.qso_end.as_ref().map(fmt_dt))
        .bind(qso.band.map(|b| b.as_adif().to_string()))
        .bind(qso.freq_hz)
        .bind(qso.mode.as_ref().map(|m| m.as_adif().to_string()))
        .bind(&qso.submode)
        .bind(&qso.rst_sent)
        .bind(&qso.rst_rcvd)
        .bind(qso.operator_id.map(|id| fmt_id(id.as_uuid())))
        .bind(qso.station_location_id.map(|id| fmt_id(id.as_uuid())))
        .bind(qso.operating_session_id.map(|id| fmt_id(id.as_uuid())))
        .bind(qso.station_callsign.as_ref().map(|c| c.as_str().to_string()))
        .bind(qso.owner_callsign.as_ref().map(|c| c.as_str().to_string()))
        .bind(qso.dxcc_id.map(|v| v as i64))
        .bind(&qso.dxcc_prefix)
        .bind(&qso.continent)
        .bind(qso.cq_zone.map(|v| v as i64))
        .bind(qso.itu_zone.map(|v| v as i64))
        .bind(&qso.grid)
        .bind(&qso.state)
        .bind(&qso.county)
        .bind(&qso.province)
        .bind(&qso.iota)
        .bind(qso.tx_power_w.map(|v| v as f64))
        .bind(qso.rx_power_w.map(|v| v as f64))
        .bind(qso.propagation_mode.as_ref().map(propagation_to_str))
        .bind(&qso.sat_name)
        .bind(&qso.sat_mode)
        .bind(qso.distance_km)
        .bind(qso.bearing_deg)
        .bind(fmt_dt(&qso.created_at))
        .bind(fmt_dt(&qso.updated_at))
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn update_qso(&self, qso: &Qso) -> RepoResult<()> {
        sqlx::query(
            r#"
            UPDATE qsos SET
                call = ?, qso_begin = ?, qso_end = ?,
                band = ?, freq_hz = ?, mode = ?, submode = ?,
                rst_sent = ?, rst_rcvd = ?,
                operator_id = ?, station_location_id = ?, operating_session_id = ?,
                station_callsign = ?, owner_callsign = ?,
                dxcc_id = ?, dxcc_prefix = ?, continent = ?, cq_zone = ?, itu_zone = ?,
                grid = ?, state = ?, county = ?, province = ?, iota = ?,
                tx_power_w = ?, rx_power_w = ?,
                propagation_mode = ?, sat_name = ?, sat_mode = ?,
                distance_km = ?, bearing_deg = ?,
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(qso.call.as_str())
        .bind(fmt_dt(&qso.qso_begin))
        .bind(qso.qso_end.as_ref().map(fmt_dt))
        .bind(qso.band.map(|b| b.as_adif().to_string()))
        .bind(qso.freq_hz)
        .bind(qso.mode.as_ref().map(|m| m.as_adif().to_string()))
        .bind(&qso.submode)
        .bind(&qso.rst_sent)
        .bind(&qso.rst_rcvd)
        .bind(qso.operator_id.map(|id| fmt_id(id.as_uuid())))
        .bind(qso.station_location_id.map(|id| fmt_id(id.as_uuid())))
        .bind(qso.operating_session_id.map(|id| fmt_id(id.as_uuid())))
        .bind(qso.station_callsign.as_ref().map(|c| c.as_str().to_string()))
        .bind(qso.owner_callsign.as_ref().map(|c| c.as_str().to_string()))
        .bind(qso.dxcc_id.map(|v| v as i64))
        .bind(&qso.dxcc_prefix)
        .bind(&qso.continent)
        .bind(qso.cq_zone.map(|v| v as i64))
        .bind(qso.itu_zone.map(|v| v as i64))
        .bind(&qso.grid)
        .bind(&qso.state)
        .bind(&qso.county)
        .bind(&qso.province)
        .bind(&qso.iota)
        .bind(qso.tx_power_w.map(|v| v as f64))
        .bind(qso.rx_power_w.map(|v| v as f64))
        .bind(qso.propagation_mode.as_ref().map(propagation_to_str))
        .bind(&qso.sat_name)
        .bind(&qso.sat_mode)
        .bind(qso.distance_km)
        .bind(qso.bearing_deg)
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
                          COALESCE(s.confirmation_state = 'confirmed', 0) AS lotw_confirmed
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
        // Mirrors the keys used by find_match_for_confirmation: station
        // callsign + worked call + UTC date (yyyy-mm-dd) + band + mode.
        // Only QSOs with station_callsign set qualify (the per-record
        // dedup path skips when station is None, see service.rs:142-160).
        let rows = sqlx::query(
            r#"
            SELECT station_callsign, call,
                   substr(qso_begin, 1, 10) AS date_only,
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
            let date: String = row.try_get("date_only").map_err(map_err)?;
            let band: Option<String> = row.try_get("band").map_err(map_err)?;
            let mode: Option<String> = row.try_get("mode").map_err(map_err)?;
            out.insert((station, call, date, band, mode));
        }
        Ok(out)
    }

    async fn insert_qsos_batch(
        &self,
        qsos: &[Qso],
        exchange_fields: &[(QsoId, QsoExchangeField)],
        service_states: &[ImportedServiceState],
    ) -> RepoResult<()> {
        // One transaction, three INSERT loops. Per-row binds are the
        // same as the corresponding single-row methods (insert_qso,
        // add_exchange_field, mark_uploaded/mark_upload_verified/
        // mark_confirmed) — we just swap the executor from `&self.pool`
        // for `&mut tx`. ~50k QSOs → one commit instead of ~1.5M.
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        let now = fmt_dt(&Utc::now());

        for q in qsos {
            sqlx::query(
                r#"
                INSERT INTO qsos (
                    id, call, qso_begin, qso_end,
                    band, freq_hz, mode, submode,
                    rst_sent, rst_rcvd,
                    operator_id, station_location_id, operating_session_id,
                    station_callsign, owner_callsign,
                    dxcc_id, dxcc_prefix, continent, cq_zone, itu_zone,
                    grid, state, county, province, iota,
                    tx_power_w, rx_power_w,
                    propagation_mode, sat_name, sat_mode,
                    distance_km, bearing_deg,
                    created_at, updated_at
                ) VALUES (
                    ?, ?, ?, ?,
                    ?, ?, ?, ?,
                    ?, ?,
                    ?, ?, ?,
                    ?, ?,
                    ?, ?, ?, ?, ?,
                    ?, ?, ?, ?, ?,
                    ?, ?,
                    ?, ?, ?,
                    ?, ?,
                    ?, ?
                )
                "#,
            )
            .bind(fmt_id(q.id.as_uuid()))
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
            .bind(q.operating_session_id.map(|id| fmt_id(id.as_uuid())))
            .bind(q.station_callsign.as_ref().map(|c| c.as_str().to_string()))
            .bind(q.owner_callsign.as_ref().map(|c| c.as_str().to_string()))
            .bind(q.dxcc_id.map(|v| v as i64))
            .bind(&q.dxcc_prefix)
            .bind(&q.continent)
            .bind(q.cq_zone.map(|v| v as i64))
            .bind(q.itu_zone.map(|v| v as i64))
            .bind(&q.grid)
            .bind(&q.state)
            .bind(&q.county)
            .bind(&q.province)
            .bind(&q.iota)
            .bind(q.tx_power_w.map(|v| v as f64))
            .bind(q.rx_power_w.map(|v| v as f64))
            .bind(q.propagation_mode.as_ref().map(propagation_to_str))
            .bind(&q.sat_name)
            .bind(&q.sat_mode)
            .bind(q.distance_km)
            .bind(q.bearing_deg)
            .bind(fmt_dt(&q.created_at))
            .bind(fmt_dt(&q.updated_at))
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }

        for (qso_id, field) in exchange_fields {
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

        for s in service_states {
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
                     uploaded_at, confirmed_at,
                     created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(fmt_id(row_id))
            .bind(fmt_id(s.qso_id.as_uuid()))
            .bind(s.service)
            .bind(upload_state)
            .bind(confirmation_state)
            .bind(s.uploaded_at.as_ref().map(fmt_dt))
            .bind(s.confirmed_at.as_ref().map(fmt_dt))
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        }

        tx.commit().await.map_err(map_err)?;
        Ok(())
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
                Utc.with_ymd_and_hms(2026, 5, 8, 19, 0, 0).unwrap(),
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
                Utc.with_ymd_and_hms(2026, 5, 8, 20, 0, 0).unwrap(),
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
}
