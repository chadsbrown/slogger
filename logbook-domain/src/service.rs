use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use radio_core::{FieldSource, Qso, QsoExchangeField, QsoId, StationLocation, StationLocationId};
use station_resolver::{NoOpResolver, Resolver};

use crate::commands::CreateQsoCommand;
use crate::credit::AwardCredit;
use crate::export::{ExportOptions, ExportRecord, export_adif_records};
use crate::my_station::{StationLocationRef, fill_location, location_covers};
use crate::queries::{QsoSearch, QsoSummary};
use crate::repository::{
    DedupKey, ImportBatch, ImportedCredit, ImportedServiceState, QsoDetail, QsoRepository,
    RepoResult, RepositoryError,
};

#[derive(Debug)]
pub struct LogbookService {
    repo: Arc<dyn QsoRepository>,
    resolver: Arc<dyn Resolver>,
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub created: usize,
    /// QSOs already in the log (matched on station_callsign + call +
    /// start time + band + mode). Skipped without error. Lets repeated WSJT-X
    /// broadcasts and re-imported ADIFs converge on a stable count.
    pub duplicates: usize,
    pub skipped: usize,
    /// Station locations the import created for QTH names it hadn't seen.
    pub locations_created: usize,
    pub errors: Vec<String>,
}

/// The parts of a `CreateQsoCommand` that aren't columns of the QSO.
struct CommandExtras {
    exchange_fields: Vec<QsoExchangeField>,
    station_location_ref: Option<StationLocationRef>,
    credits: Vec<AwardCredit>,
}

/// Split a command into the QSO record (not yet enriched) and the rest.
fn qso_from_command(
    id: QsoId,
    command: CreateQsoCommand,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> (Qso, CommandExtras) {
    let c = command;
    let qso = Qso {
        id,
        call: c.call,
        qso_begin: c.qso_begin,
        qso_end: c.qso_end,
        band: c.band,
        freq_hz: c.freq_hz,
        mode: c.mode,
        submode: c.submode,
        rst_sent: c.rst_sent,
        rst_rcvd: c.rst_rcvd,
        operator_id: c.operator_id,
        station_location_id: c.station_location_id,
        station_callsign: c.station_callsign,
        owner_callsign: c.owner_callsign,
        operator_callsign: c.operator_callsign,
        dxcc_id: c.dxcc_id,
        dxcc_prefix: c.dxcc_prefix,
        wpx_prefix: c.wpx_prefix,
        continent: c.continent,
        cq_zone: c.cq_zone,
        itu_zone: c.itu_zone,
        grid: c.grid,
        state: c.state,
        county: c.county,
        province: c.province,
        iota: c.iota,
        latitude: c.latitude,
        longitude: c.longitude,
        name: c.name,
        qth: c.qth,
        address: c.address,
        email: c.email,
        age: c.age,
        qsl_via: c.qsl_via,
        comment: c.comment,
        contest_id: c.contest_id,
        serial_sent: c.serial_sent,
        serial_rcvd: c.serial_rcvd,
        exchange_sent: c.exchange_sent,
        exchange_rcvd: c.exchange_rcvd,
        arrl_section: c.arrl_section,
        band_rx: c.band_rx,
        freq_rx_hz: c.freq_rx_hz,
        tx_power_w: c.tx_power_w,
        rx_power_w: c.rx_power_w,
        propagation_mode: c.propagation_mode,
        sat_name: c.sat_name,
        sat_mode: c.sat_mode,
        distance_km: c.distance_km,
        bearing_deg: c.bearing_deg,
        sfi: c.sfi,
        a_index: c.a_index,
        k_index: c.k_index,
        created_at,
        updated_at,
    };
    let extras = CommandExtras {
        exchange_fields: c.exchange_fields,
        station_location_ref: c.station_location_ref,
        credits: c.credits,
    };
    (qso, extras)
}

impl LogbookService {
    pub fn new(repo: Arc<dyn QsoRepository>) -> Self {
        Self {
            repo,
            resolver: Arc::new(NoOpResolver),
        }
    }

    pub fn with_resolver(repo: Arc<dyn QsoRepository>, resolver: Arc<dyn Resolver>) -> Self {
        Self { repo, resolver }
    }

    pub async fn create_qso(&self, command: CreateQsoCommand) -> RepoResult<QsoId> {
        let (qso, extras) = self.build_qso_from_command(command);
        let id = qso.id;
        self.repo.insert_qso(&qso).await?;
        for field in extras.exchange_fields {
            self.repo.add_exchange_field(&id, &field).await?;
        }
        Ok(id)
    }

    /// Consume a `CreateQsoCommand` and return the fully-built Qso
    /// (with resolver enrichment applied) plus the command's non-column
    /// parts. Shared by single-record `create_qso` and the batched
    /// `import_qsos` path.
    fn build_qso_from_command(&self, command: CreateQsoCommand) -> (Qso, CommandExtras) {
        let now = Utc::now();
        let (mut qso, extras) = qso_from_command(QsoId::new(), command, now, now);
        self.enrich(&mut qso);
        (qso, extras)
    }

    /// Fill DXCC details the record lacks from the resolver.
    /// Operator-supplied / imported values win; the resolver only fills
    /// blanks.
    fn enrich(&self, qso: &mut Qso) {
        if let Some(res) = self.resolver.resolve(&qso.call) {
            qso.dxcc_id = qso.dxcc_id.or(res.dxcc_id);
            qso.dxcc_prefix = qso.dxcc_prefix.take().or(res.dxcc_prefix);
            qso.continent = qso.continent.take().or(res.continent);
            qso.cq_zone = qso.cq_zone.or(res.cq_zone);
            qso.itu_zone = qso.itu_zone.or(res.itu_zone);
        }
    }

    /// Batched import (ADIF files, WSJT-X logged QSOs). Besides the QSOs
    /// themselves this links each one to the station location its record
    /// names — reusing a location of that exact name, or creating one the
    /// first time a name is seen — and records its award credit. A record
    /// that names no location is left unlinked for the operator to assign
    /// (see `bulk_assign_location_by_search`); slogger does not guess.
    pub async fn import_qsos(
        &self,
        commands: impl IntoIterator<Item = CreateQsoCommand>,
    ) -> ImportReport {
        let mut report = ImportReport::default();

        // Preload existing dedup keys into a HashSet for O(1) lookups.
        // Per-record SELECTs against the growing qsos table are the
        // quadratic-time killer they replace.
        let mut seen: HashSet<DedupKey> = match self.repo.load_dedup_keys().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "dedup preload failed; importing without dedup");
                HashSet::new()
            }
        };

        // Same idea for station locations: resolve names against one
        // up-front load, plus the ones this import creates.
        let mut locations: HashMap<String, StationLocation> = HashMap::new();
        match self.repo.load_station_locations().await {
            Ok(existing) => {
                for loc in existing {
                    locations.entry(loc.name.clone()).or_insert(loc);
                }
            }
            Err(e) => {
                report.errors.push(format!("loading station locations failed: {e}"));
                return report;
            }
        }

        let mut batch = ImportBatch::default();

        for cmd in commands {
            // Intra-import dedup: HashSet::insert returns false on
            // collision, catching both existing-log and same-batch
            // duplicates with the same code path.
            if let Some(key) = dedup_key(&cmd) {
                if !seen.insert(key) {
                    report.duplicates += 1;
                    continue;
                }
            }

            let (mut qso, extras) = self.build_qso_from_command(cmd);
            let id = qso.id;
            let mut exchange_fields = extras.exchange_fields;

            if let Some(r) = extras.station_location_ref {
                if qso.station_location_id.is_some() {
                    // Caller already chose the location; keep what the
                    // record said about it rather than dropping it.
                    exchange_fields.extend(r.my_fields.into_iter().map(imported_field));
                } else {
                    let loc = locations.entry(r.name.clone()).or_insert_with(|| {
                        let loc = new_location(&r, &qso);
                        batch.locations.push(loc.clone());
                        loc
                    });
                    qso.station_location_id = Some(loc.id);
                    // MY_* values the location doesn't reproduce (this QSO
                    // was made from a different grid than the location's,
                    // say) stay with the QSO.
                    exchange_fields.extend(
                        r.my_fields
                            .into_iter()
                            .filter(|(n, v)| !location_covers(loc, n, v))
                            .map(imported_field),
                    );
                }
            }

            batch
                .service_states
                .extend(extract_service_states(id, &exchange_fields));
            batch.credits.extend(
                extras
                    .credits
                    .into_iter()
                    .map(|credit| imported_credit(&qso, credit)),
            );
            batch
                .exchange_fields
                .extend(exchange_fields.into_iter().map(|f| (id, f)));
            batch.qsos.push(qso);
        }

        let created = batch.qsos.len();
        match self.repo.insert_qsos_batch(&batch).await {
            Ok(()) => {
                report.created = created;
                report.locations_created = batch.locations.len();
            }
            Err(e) => {
                report.skipped = created;
                report.errors.push(format!("batch insert failed: {e}"));
            }
        }
        report
    }

    pub async fn search_qsos(&self, query: QsoSearch) -> RepoResult<Vec<QsoSummary>> {
        self.repo.search_qsos(query).await
    }

    pub async fn get_qso(&self, id: &QsoId) -> RepoResult<Option<Qso>> {
        self.repo.get_qso(id).await
    }

    /// The QSO plus its extra fields and per-service QSL state — what a
    /// detail view needs. `None` if the QSO doesn't exist.
    pub async fn get_qso_detail(&self, id: &QsoId) -> RepoResult<Option<QsoDetail>> {
        let Some(qso) = self.repo.get_qso(id).await? else {
            return Ok(None);
        };
        Ok(Some(QsoDetail {
            qso,
            exchange_fields: self.repo.list_exchange_fields(id).await?,
            service_states: self.repo.list_service_states(id).await?,
            credits: self.repo.list_credits(id).await?,
        }))
    }

    /// Apply edits from `command` onto an existing QSO. Re-runs resolver
    /// enrichment so a corrected callsign picks up its DXCC info too.
    /// Operator-supplied values still win over resolver fills.
    pub async fn update_qso(&self, id: QsoId, command: CreateQsoCommand) -> RepoResult<()> {
        let existing = self
            .repo
            .get_qso(&id)
            .await?
            .ok_or(RepositoryError::NotFound)?;
        // Credits and location references only come from import; an
        // edit carries neither, so the split-off parts are dropped.
        let (mut qso, _) = qso_from_command(id, command, existing.created_at, Utc::now());
        self.enrich(&mut qso);

        self.repo.update_qso(&qso).await
        // Note: this leaves existing exchange_fields in place. Re-syncing
        // them on every edit would risk losing imported ADIF metadata.
        // Edits to specific exchange fields would be a separate command.
    }

    pub async fn delete_qso(&self, id: &QsoId) -> RepoResult<()> {
        self.repo.soft_delete_qso(id).await
    }

    /// Soft-delete every QSO in `ids`. Returns the count successfully
    /// deleted. Bookkeeping for partial failures is logged via tracing
    /// — bulk failure isn't a hard stop because per-id errors are
    /// usually transient (FK constraint races, concurrent edits) and
    /// the operator can re-run.
    pub async fn bulk_soft_delete(&self, ids: &[QsoId]) -> usize {
        let mut deleted = 0;
        for id in ids {
            match self.repo.soft_delete_qso(id).await {
                Ok(()) => deleted += 1,
                Err(e) => {
                    tracing::warn!(qso = %id, error = %e, "bulk soft-delete: per-id failed");
                }
            }
        }
        deleted
    }

    /// Mark every QSO in `ids` as uploaded to `service` at `at`. Useful
    /// for batch-applying a manual confirmation: e.g. operator uploaded
    /// to LotW outside slogger and wants the local DB to reflect it.
    pub async fn bulk_mark_uploaded(
        &self,
        ids: &[QsoId],
        service: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> usize {
        let mut marked = 0;
        for id in ids {
            match self.repo.mark_uploaded(id, service, at, None).await {
                Ok(()) => marked += 1,
                Err(e) => {
                    tracing::warn!(qso = %id, %service, error = %e, "bulk mark_uploaded: per-id failed");
                }
            }
        }
        marked
    }

    /// Mark every QSO in `ids` as confirmed by `service` at `at`. For
    /// bulk paper-QSL or external-tooled confirmation imports.
    pub async fn bulk_mark_confirmed(
        &self,
        ids: &[QsoId],
        service: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> usize {
        let mut marked = 0;
        for id in ids {
            match self.repo.mark_confirmed(id, service, at, None).await {
                Ok(()) => marked += 1,
                Err(e) => {
                    tracing::warn!(qso = %id, %service, error = %e, "bulk mark_confirmed: per-id failed");
                }
            }
        }
        marked
    }

    /// Run a search and apply soft-delete to every match. Returns
    /// `BulkReport { matched, succeeded }` so callers can show the
    /// operator both the size of the operation and the success count.
    pub async fn bulk_soft_delete_by_search(
        &self,
        search: QsoSearch,
    ) -> RepoResult<BulkReport> {
        let summaries = self.repo.search_qsos(search).await?;
        let ids: Vec<QsoId> = summaries.iter().map(|s| s.id).collect();
        let succeeded = self.bulk_soft_delete(&ids).await;
        Ok(BulkReport {
            matched: ids.len(),
            succeeded,
        })
    }

    /// Run a search and apply mark-uploaded to every match.
    pub async fn bulk_mark_uploaded_by_search(
        &self,
        search: QsoSearch,
        service: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> RepoResult<BulkReport> {
        let summaries = self.repo.search_qsos(search).await?;
        let ids: Vec<QsoId> = summaries.iter().map(|s| s.id).collect();
        let succeeded = self.bulk_mark_uploaded(&ids, service, at).await;
        Ok(BulkReport {
            matched: ids.len(),
            succeeded,
        })
    }

    /// Run a search and apply mark-confirmed to every match.
    pub async fn bulk_mark_confirmed_by_search(
        &self,
        search: QsoSearch,
        service: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> RepoResult<BulkReport> {
        let summaries = self.repo.search_qsos(search).await?;
        let ids: Vec<QsoId> = summaries.iter().map(|s| s.id).collect();
        let succeeded = self.bulk_mark_confirmed(&ids, service, at).await;
        Ok(BulkReport {
            matched: ids.len(),
            succeeded,
        })
    }

    /// Link every QSO the search matches — all of them, not just the
    /// page the search's limit would return — to `location`. The fix-up
    /// for QSOs imported without a station location: search for
    /// unassigned ones (narrowed by date, say) and assign them.
    pub async fn bulk_assign_location_by_search(
        &self,
        search: QsoSearch,
        location: StationLocationId,
    ) -> RepoResult<BulkReport> {
        let matched = self.repo.count_matching(search.clone()).await?;
        let succeeded = self.repo.assign_station_location(search, location).await?;
        Ok(BulkReport { matched, succeeded })
    }

    /// Full-fidelity ADIF for the QSOs `search` matches: every column,
    /// the station location (by name and `MY_*` details), award credit and
    /// extra fields, so importing the result into an empty log rebuilds
    /// the same records. `locations` are the station locations to name
    /// (QSOs linked to one not in the list are written without it).
    pub async fn export_adif_by_search(
        &self,
        search: QsoSearch,
        locations: &[StationLocation],
        opts: &ExportOptions,
    ) -> RepoResult<String> {
        let qsos = self.repo.search_full_qsos(search).await?;
        let mut extras = Vec::with_capacity(qsos.len());
        for q in &qsos {
            extras.push((
                self.repo.list_exchange_fields(&q.id).await?,
                self.repo.list_credits(&q.id).await?,
            ));
        }
        let records: Vec<ExportRecord<'_>> = qsos
            .iter()
            .zip(&extras)
            .map(|(qso, (fields, credits))| ExportRecord {
                qso,
                location: qso
                    .station_location_id
                    .and_then(|id| locations.iter().find(|l| l.id == id)),
                credits,
                extra_fields: fields,
            })
            .collect();
        Ok(export_adif_records(&records, opts))
    }

    /// Count without fetching — for "Delete N QSOs?" confirmations.
    pub async fn count_matching(&self, search: QsoSearch) -> RepoResult<usize> {
        self.repo.count_matching(search).await
    }

    /// Full QSO records (not summaries) for export, edit, etc.
    pub async fn search_full_qsos(&self, search: QsoSearch) -> RepoResult<Vec<radio_core::Qso>> {
        self.repo.search_full_qsos(search).await
    }
}

/// Result shape for search-based bulk operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkReport {
    /// QSOs the search matched.
    pub matched: usize,
    /// Of those, how many the action succeeded on. `matched - succeeded`
    /// is the per-id failure count (logged via tracing, not returned
    /// individually).
    pub succeeded: usize,
}

/// Build the dedup key for an inbound `CreateQsoCommand`: station call,
/// worked call, start time to the second, band and mode.
///
/// The start time, not just the date, is part of the key. Working the
/// same station twice in a day on one band and mode is routine (two
/// sessions of a weekly contest, a sprint followed by a ragchew), and a
/// date-only key silently dropped about a thousand such QSOs from a real
/// 48k-record log. Re-importing the same file still dedups exactly.
///
/// Returns `None` when `station_callsign` is missing: without a station
/// call we can't tell whose log it belongs to, so we don't claim a
/// duplicate.
fn dedup_key(cmd: &CreateQsoCommand) -> Option<DedupKey> {
    let station = cmd.station_callsign.as_ref()?.as_str().to_string();
    let call = cmd.call.as_str().to_string();
    // Same shape as the first 19 characters of the stored RFC 3339
    // timestamp, which is what `load_dedup_keys` reads back.
    let begin = cmd.qso_begin.format("%Y-%m-%dT%H:%M:%S").to_string();
    let band = cmd.band.map(|b| b.as_adif().to_string());
    let mode = cmd.mode.as_ref().map(|m| m.as_adif().to_string());
    Some((station, call, begin, band, mode))
}

/// A location created by import for a QTH name not seen before. It takes
/// its callsigns from the first QSO that names it and whatever `MY_*`
/// details that record carries; the rest is left for the operator.
fn new_location(r: &StationLocationRef, qso: &Qso) -> StationLocation {
    let now = Utc::now();
    let mut loc = StationLocation {
        id: StationLocationId::new(),
        name: r.name.clone(),
        station_callsign: qso.station_callsign.clone(),
        owner_callsign: qso.owner_callsign.clone(),
        city: None,
        county: None,
        state: None,
        country: None,
        grid: None,
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
    fill_location(&mut loc, &r.my_fields);
    loc
}

fn imported_field((name, raw_value): (String, String)) -> QsoExchangeField {
    QsoExchangeField {
        name,
        raw_value,
        normalized_value: None,
        source: FieldSource::ImportedAdif,
    }
}

/// The credit row for `credit` on `qso`, with the QSO's details as the
/// award sponsor would list them.
fn imported_credit(qso: &Qso, credit: AwardCredit) -> ImportedCredit {
    ImportedCredit {
        qso_id: qso.id,
        unit: credit_unit(
            &credit.award,
            qso.dxcc_id,
            qso.state.as_deref(),
            qso.grid.as_deref(),
        ),
        credit,
        call: qso.call.as_str().to_string(),
        qso_date: qso.qso_begin.format("%Y-%m-%d").to_string(),
        band: qso.band.map(|b| b.as_adif().to_string()),
        mode: qso.mode.as_ref().map(|m| m.as_adif().to_string()),
    }
}

/// What an award counts: the DXCC entity for the DXCC family (DXCC,
/// DXCC_BAND, DXCC_MODE), the state for WAS, the four-character grid
/// square for VUCC. Empty for awards slogger has no unit for.
pub(crate) fn credit_unit(
    award: &str,
    dxcc_id: Option<u16>,
    state: Option<&str>,
    grid: Option<&str>,
) -> String {
    let unit = if award.starts_with("DXCC") {
        dxcc_id.map(|id| id.to_string())
    } else if award == "WAS" {
        state.map(String::from)
    } else if award.starts_with("VUCC") {
        grid.map(|g| g.chars().take(4).collect::<String>().to_ascii_uppercase())
    } else {
        None
    };
    unit.unwrap_or_default()
}

fn parse_adif_date(s: &str) -> Option<DateTime<Utc>> {
    let nd = NaiveDate::parse_from_str(s.trim(), "%Y%m%d").ok()?;
    let dt = nd.and_hms_opt(0, 0, 0)?;
    Some(Utc.from_utc_datetime(&dt))
}

/// Look up an ADIF field by name (case-insensitive) in a parsed
/// command's exchange_fields. ADIF field names are uppercased by
/// `parse_adif` before storage, so the comparison is effectively
/// already aligned — `eq_ignore_ascii_case` is belt-and-suspenders.
fn get_field<'a>(fields: &'a [QsoExchangeField], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(name))
        .map(|f| f.raw_value.as_str())
}

/// Synthesize `qso_service_state` rows from ADIF service fields that
/// `parse_adif` parked in `exchange_fields`. Pure function — easy to
/// unit-test. The mapping table is:
///
/// - LotW: `LOTW_QSL_SENT in {Y,V}` → uploaded (verified if `_SENT=V`
///   or `APP_DXKEEPER_LOTW_VERIFIED=V`); `LOTW_QSL_RCVD in {Y,V}` →
///   confirmed. Dates from `LOTW_QSLSDATE` / `LOTW_QSLRDATE`.
/// - eQSL: `EQSL_QSL_SENT=Y` → uploaded; `EQSL_QSL_RCVD=Y` →
///   confirmed. Dates from `EQSL_QSLSDATE` / `EQSL_QSLRDATE`.
/// - Club Log: `CLUBLOG_QSO_UPLOAD_STATUS=Y` → uploaded (no date in
///   the ADIF spec; uploaded_at left as None). `=M` (modified, needs
///   re-upload) emits no row — defaults to pending.
/// - QRZ: `QRZCOM_QSO_UPLOAD_STATUS=Y` → uploaded (date from
///   `QRZCOM_QSO_UPLOAD_DATE`); `QRZCOM_QSO_DOWNLOAD_STATUS=Y` →
///   confirmed (date from `QRZCOM_QSO_DOWNLOAD_DATE`). DXKeeper's
///   `APP_DXKEEPER_QRZCOM_LOGID` becomes the row's `remote_id`.
/// - HRDLog: `HRDLOG_QSO_UPLOAD_STATUS=Y` → uploaded (date from
///   `HRDLOG_QSO_UPLOAD_DATE`).
///
/// Fields are read non-destructively — they remain in
/// `exchange_fields` so an export round-trip still emits them.
pub(crate) fn extract_service_states(
    qso_id: QsoId,
    fields: &[QsoExchangeField],
) -> Vec<ImportedServiceState> {
    let mut out = Vec::new();

    // ---- LotW ----
    {
        let sent = get_field(fields, "LOTW_QSL_SENT")
            .map(|s| s.trim().to_ascii_uppercase());
        let rcvd = get_field(fields, "LOTW_QSL_RCVD")
            .map(|s| s.trim().to_ascii_uppercase());
        let verified = get_field(fields, "APP_DXKEEPER_LOTW_VERIFIED")
            .map(|s| s.trim().to_ascii_uppercase());
        let send_date = get_field(fields, "LOTW_QSLSDATE").and_then(parse_adif_date);
        let recv_date = get_field(fields, "LOTW_QSLRDATE").and_then(parse_adif_date);

        let upload = match sent.as_deref() {
            Some("V") => Some("verified"),
            Some("Y") if verified.as_deref() == Some("V") => Some("verified"),
            Some("Y") => Some("uploaded"),
            _ => None,
        };
        let confirm = matches!(rcvd.as_deref(), Some("Y") | Some("V")).then_some("confirmed");

        if upload.is_some() || confirm.is_some() {
            out.push(ImportedServiceState {
                qso_id,
                service: "lotw",
                upload_state: upload,
                confirmation_state: confirm,
                uploaded_at: if upload.is_some() { send_date } else { None },
                confirmed_at: if confirm.is_some() { recv_date } else { None },
                remote_id: None,
            });
        }
    }

    // ---- eQSL ----
    {
        let sent = get_field(fields, "EQSL_QSL_SENT")
            .map(|s| s.trim().to_ascii_uppercase());
        let rcvd = get_field(fields, "EQSL_QSL_RCVD")
            .map(|s| s.trim().to_ascii_uppercase());
        let send_date = get_field(fields, "EQSL_QSLSDATE").and_then(parse_adif_date);
        let recv_date = get_field(fields, "EQSL_QSLRDATE").and_then(parse_adif_date);

        let upload = (sent.as_deref() == Some("Y")).then_some("uploaded");
        let confirm = (rcvd.as_deref() == Some("Y")).then_some("confirmed");
        if upload.is_some() || confirm.is_some() {
            out.push(ImportedServiceState {
                qso_id,
                service: "eqsl",
                upload_state: upload,
                confirmation_state: confirm,
                uploaded_at: if upload.is_some() { send_date } else { None },
                confirmed_at: if confirm.is_some() { recv_date } else { None },
                remote_id: None,
            });
        }
    }

    // ---- Club Log ----
    {
        let status = get_field(fields, "CLUBLOG_QSO_UPLOAD_STATUS")
            .map(|s| s.trim().to_ascii_uppercase());
        if status.as_deref() == Some("Y") {
            out.push(ImportedServiceState {
                qso_id,
                service: "clublog",
                upload_state: Some("uploaded"),
                confirmation_state: None,
                uploaded_at: None,
                confirmed_at: None,
                remote_id: None,
            });
        }
        // status = "M" (modified — re-upload needed) emits no row;
        // default of no row = pending matches that semantic.
    }

    // ---- QRZ ----
    {
        let up_status = get_field(fields, "QRZCOM_QSO_UPLOAD_STATUS")
            .map(|s| s.trim().to_ascii_uppercase());
        let up_date = get_field(fields, "QRZCOM_QSO_UPLOAD_DATE").and_then(parse_adif_date);
        let dn_status = get_field(fields, "QRZCOM_QSO_DOWNLOAD_STATUS")
            .map(|s| s.trim().to_ascii_uppercase());
        let dn_date = get_field(fields, "QRZCOM_QSO_DOWNLOAD_DATE").and_then(parse_adif_date);

        let log_id = get_field(fields, "APP_DXKEEPER_QRZCOM_LOGID")
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let upload = (up_status.as_deref() == Some("Y")).then_some("uploaded");
        let confirm = (dn_status.as_deref() == Some("Y")).then_some("confirmed");
        // A QRZ logbook id alone still gets a row, to hold the id; with
        // no status signal it stays pending (e.g. `M`, modified since
        // upload), as it would without the id.
        if upload.is_some() || confirm.is_some() || log_id.is_some() {
            out.push(ImportedServiceState {
                qso_id,
                service: "qrz",
                upload_state: upload,
                confirmation_state: confirm,
                uploaded_at: if upload.is_some() { up_date } else { None },
                confirmed_at: if confirm.is_some() { dn_date } else { None },
                remote_id: log_id.map(String::from),
            });
        }
    }

    // ---- HRDLog ----
    {
        let status = get_field(fields, "HRDLOG_QSO_UPLOAD_STATUS")
            .map(|s| s.trim().to_ascii_uppercase());
        let date = get_field(fields, "HRDLOG_QSO_UPLOAD_DATE").and_then(parse_adif_date);
        if status.as_deref() == Some("Y") {
            out.push(ImportedServiceState {
                qso_id,
                service: "hrdlog",
                upload_state: Some("uploaded"),
                confirmation_state: None,
                uploaded_at: date,
                confirmed_at: None,
                remote_id: None,
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use radio_core::FieldSource;

    fn field(name: &str, value: &str) -> QsoExchangeField {
        QsoExchangeField {
            name: name.to_string(),
            raw_value: value.to_string(),
            normalized_value: None,
            source: FieldSource::ImportedAdif,
        }
    }

    #[test]
    fn lotw_sent_y_emits_uploaded_with_date() {
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[
                field("LOTW_QSL_SENT", "Y"),
                field("LOTW_QSLSDATE", "20240101"),
            ],
        );
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].service, "lotw");
        assert_eq!(states[0].upload_state, Some("uploaded"));
        assert!(states[0].uploaded_at.is_some());
        assert_eq!(states[0].confirmation_state, None);
    }

    #[test]
    fn lotw_sent_v_emits_verified() {
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[
                field("LOTW_QSL_SENT", "V"),
                field("LOTW_QSLSDATE", "20240101"),
            ],
        );
        assert_eq!(states[0].upload_state, Some("verified"));
    }

    #[test]
    fn lotw_dxkeeper_verified_promotes_to_verified() {
        // DXKeeper sometimes leaves LOTW_QSL_SENT=Y but emits the
        // companion APP_DXKEEPER_LOTW_VERIFIED=V to signal verification.
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[
                field("LOTW_QSL_SENT", "Y"),
                field("APP_DXKEEPER_LOTW_VERIFIED", "V"),
                field("LOTW_QSLSDATE", "20240101"),
            ],
        );
        assert_eq!(states[0].upload_state, Some("verified"));
    }

    #[test]
    fn lotw_rcvd_v_emits_confirmed_with_date() {
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[
                field("LOTW_QSL_RCVD", "V"),
                field("LOTW_QSLRDATE", "20240115"),
            ],
        );
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].confirmation_state, Some("confirmed"));
        assert!(states[0].confirmed_at.is_some());
    }

    #[test]
    fn lotw_no_signal_emits_nothing() {
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[
                field("LOTW_QSL_SENT", "N"),
                field("LOTW_QSL_RCVD", "N"),
            ],
        );
        assert!(states.iter().all(|s| s.service != "lotw"));
    }

    #[test]
    fn eqsl_sent_and_rcvd_emit_one_combined_row() {
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[
                field("EQSL_QSL_SENT", "Y"),
                field("EQSL_QSL_RCVD", "Y"),
                field("EQSL_QSLSDATE", "20240101"),
                field("EQSL_QSLRDATE", "20240105"),
            ],
        );
        let eqsl: Vec<_> = states.iter().filter(|s| s.service == "eqsl").collect();
        assert_eq!(eqsl.len(), 1);
        assert_eq!(eqsl[0].upload_state, Some("uploaded"));
        assert_eq!(eqsl[0].confirmation_state, Some("confirmed"));
    }

    #[test]
    fn clublog_m_emits_no_row() {
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[field("CLUBLOG_QSO_UPLOAD_STATUS", "M")],
        );
        assert!(states.iter().all(|s| s.service != "clublog"));
    }

    #[test]
    fn clublog_y_emits_uploaded_no_date() {
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[field("CLUBLOG_QSO_UPLOAD_STATUS", "Y")],
        );
        let clublog: Vec<_> = states.iter().filter(|s| s.service == "clublog").collect();
        assert_eq!(clublog.len(), 1);
        assert_eq!(clublog[0].upload_state, Some("uploaded"));
        assert!(clublog[0].uploaded_at.is_none());
    }

    #[test]
    fn qrz_upload_and_download_emit_one_row() {
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[
                field("QRZCOM_QSO_UPLOAD_STATUS", "Y"),
                field("QRZCOM_QSO_UPLOAD_DATE", "20240506"),
                field("QRZCOM_QSO_DOWNLOAD_STATUS", "Y"),
                field("QRZCOM_QSO_DOWNLOAD_DATE", "20240507"),
            ],
        );
        let qrz: Vec<_> = states.iter().filter(|s| s.service == "qrz").collect();
        assert_eq!(qrz.len(), 1);
        assert_eq!(qrz[0].upload_state, Some("uploaded"));
        assert_eq!(qrz[0].confirmation_state, Some("confirmed"));
        assert!(qrz[0].uploaded_at.is_some());
        assert!(qrz[0].confirmed_at.is_some());
    }

    #[test]
    fn hrdlog_y_emits_row() {
        let id = QsoId::new();
        let states = extract_service_states(
            id,
            &[
                field("HRDLOG_QSO_UPLOAD_STATUS", "Y"),
                field("HRDLOG_QSO_UPLOAD_DATE", "20240301"),
            ],
        );
        let hrd: Vec<_> = states.iter().filter(|s| s.service == "hrdlog").collect();
        assert_eq!(hrd.len(), 1);
        assert_eq!(hrd[0].upload_state, Some("uploaded"));
        assert!(hrd[0].uploaded_at.is_some());
    }

    #[test]
    fn empty_fields_emit_empty_vec() {
        let states = extract_service_states(QsoId::new(), &[]);
        assert!(states.is_empty());
    }

    #[test]
    fn field_name_case_insensitive() {
        // parse_adif normalizes names to uppercase but harden the
        // helper against case drift from other callers.
        let states = extract_service_states(
            QsoId::new(),
            &[field("lotw_qsl_sent", "Y"), field("lotw_qslsdate", "20240101")],
        );
        assert_eq!(states[0].upload_state, Some("uploaded"));
    }
}
