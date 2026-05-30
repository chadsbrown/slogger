use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use radio_core::{Qso, QsoExchangeField, QsoId};
use station_resolver::{NoOpResolver, Resolver};

use crate::commands::CreateQsoCommand;
use crate::queries::{QsoSearch, QsoSummary};
use crate::repository::{
    DedupKey, ImportedServiceState, QsoRepository, RepoResult, RepositoryError,
};

#[derive(Debug)]
pub struct LogbookService {
    repo: Arc<dyn QsoRepository>,
    resolver: Arc<dyn Resolver>,
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub created: usize,
    /// QSOs already in the log (matched on station_callsign + call + date
    /// + band + mode). Skipped without error. Lets repeated WSJT-X
    /// broadcasts and re-imported ADIFs converge on a stable count.
    pub duplicates: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
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
        let (qso, exchange_fields) = self.build_qso_from_command(command);
        let id = qso.id;
        self.repo.insert_qso(&qso).await?;
        for field in exchange_fields {
            self.repo.add_exchange_field(&id, &field).await?;
        }
        Ok(id)
    }

    /// Consume a `CreateQsoCommand` and return the fully-built Qso
    /// (with resolver enrichment applied) plus its exchange_fields.
    /// Shared by single-record `create_qso` and the batched
    /// `import_qsos` path.
    fn build_qso_from_command(
        &self,
        command: CreateQsoCommand,
    ) -> (Qso, Vec<QsoExchangeField>) {
        let now = Utc::now();
        let id = QsoId::new();
        let resolution = self.resolver.resolve(&command.call);

        let mut qso = Qso {
            id,
            call: command.call,
            qso_begin: command.qso_begin,
            qso_end: command.qso_end,
            band: command.band,
            freq_hz: command.freq_hz,
            mode: command.mode,
            submode: command.submode,
            rst_sent: command.rst_sent,
            rst_rcvd: command.rst_rcvd,
            operator_id: command.operator_id,
            station_location_id: command.station_location_id,
            station_callsign: command.station_callsign,
            owner_callsign: command.owner_callsign,
            dxcc_id: command.dxcc_id,
            dxcc_prefix: command.dxcc_prefix,
            continent: command.continent,
            cq_zone: command.cq_zone,
            itu_zone: command.itu_zone,
            grid: command.grid,
            state: command.state,
            county: command.county,
            province: command.province,
            iota: command.iota,
            tx_power_w: command.tx_power_w,
            rx_power_w: command.rx_power_w,
            propagation_mode: command.propagation_mode,
            sat_name: command.sat_name,
            sat_mode: command.sat_mode,
            distance_km: None,
            bearing_deg: None,
            created_at: now,
            updated_at: now,
        };

        if let Some(res) = resolution {
            // Operator-supplied values win; resolver only fills blanks.
            qso.dxcc_id = qso.dxcc_id.or(res.dxcc_id);
            qso.dxcc_prefix = qso.dxcc_prefix.or(res.dxcc_prefix);
            qso.continent = qso.continent.or(res.continent);
            qso.cq_zone = qso.cq_zone.or(res.cq_zone);
            qso.itu_zone = qso.itu_zone.or(res.itu_zone);
        }

        (qso, command.exchange_fields)
    }

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

        let mut new_qsos: Vec<Qso> = Vec::new();
        let mut new_fields: Vec<(QsoId, QsoExchangeField)> = Vec::new();
        let mut new_service_states: Vec<ImportedServiceState> = Vec::new();

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

            let (qso, exchange_fields) = self.build_qso_from_command(cmd);
            let id = qso.id;
            new_service_states.extend(extract_service_states(id, &exchange_fields));
            for f in exchange_fields {
                new_fields.push((id, f));
            }
            new_qsos.push(qso);
        }

        let created = new_qsos.len();
        match self
            .repo
            .insert_qsos_batch(&new_qsos, &new_fields, &new_service_states)
            .await
        {
            Ok(()) => report.created = created,
            Err(e) => {
                report.skipped = created;
                report.errors.push(format!("batch insert failed: {e}"));
            }
        }
        report
    }

    /// Returns true if a QSO matching the command's canonical keys
    /// (station_callsign + call + UTC date + band + mode) already exists.
    /// Skips the check when station_callsign is missing — without it we
    /// can't match correctly and would get false positives across
    /// different operators.
    #[allow(dead_code)]
    async fn is_duplicate(&self, cmd: &CreateQsoCommand) -> RepoResult<bool> {
        let Some(station) = cmd.station_callsign.as_ref() else {
            return Ok(false);
        };
        let date = cmd.qso_begin.format("%Y-%m-%d").to_string();
        let band = cmd.band.map(|b| b.as_adif());
        let mode = cmd.mode.as_ref().map(|m| m.as_adif().to_string());
        let mode_ref = mode.as_deref();
        self.repo
            .find_match_for_confirmation(
                station.as_str(),
                cmd.call.as_str(),
                &date,
                band,
                mode_ref,
            )
            .await
            .map(|m| m.is_some())
    }

    pub async fn search_qsos(&self, query: QsoSearch) -> RepoResult<Vec<QsoSummary>> {
        self.repo.search_qsos(query).await
    }

    pub async fn get_qso(&self, id: &QsoId) -> RepoResult<Option<Qso>> {
        self.repo.get_qso(id).await
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
        let resolution = self.resolver.resolve(&command.call);

        let mut qso = Qso {
            id,
            call: command.call,
            qso_begin: command.qso_begin,
            qso_end: command.qso_end,
            band: command.band,
            freq_hz: command.freq_hz,
            mode: command.mode,
            submode: command.submode,
            rst_sent: command.rst_sent,
            rst_rcvd: command.rst_rcvd,
            operator_id: command.operator_id,
            station_location_id: command.station_location_id,
            station_callsign: command.station_callsign,
            owner_callsign: command.owner_callsign,
            dxcc_id: command.dxcc_id,
            dxcc_prefix: command.dxcc_prefix,
            continent: command.continent,
            cq_zone: command.cq_zone,
            itu_zone: command.itu_zone,
            grid: command.grid,
            state: command.state,
            county: command.county,
            province: command.province,
            iota: command.iota,
            tx_power_w: command.tx_power_w,
            rx_power_w: command.rx_power_w,
            propagation_mode: command.propagation_mode,
            sat_name: command.sat_name,
            sat_mode: command.sat_mode,
            distance_km: existing.distance_km,
            bearing_deg: existing.bearing_deg,
            created_at: existing.created_at,
            updated_at: Utc::now(),
        };

        if let Some(res) = resolution {
            qso.dxcc_id = qso.dxcc_id.or(res.dxcc_id);
            qso.dxcc_prefix = qso.dxcc_prefix.or(res.dxcc_prefix);
            qso.continent = qso.continent.or(res.continent);
            qso.cq_zone = qso.cq_zone.or(res.cq_zone);
            qso.itu_zone = qso.itu_zone.or(res.itu_zone);
        }

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

/// Build the dedup key for an inbound `CreateQsoCommand`. Returns
/// `None` when `station_callsign` is missing, matching the
/// `is_duplicate` semantics: without a station call we can't tell
/// whose log it belongs to so we don't claim a duplicate.
fn dedup_key(cmd: &CreateQsoCommand) -> Option<DedupKey> {
    let station = cmd.station_callsign.as_ref()?.as_str().to_string();
    let call = cmd.call.as_str().to_string();
    let date = cmd.qso_begin.format("%Y-%m-%d").to_string();
    let band = cmd.band.map(|b| b.as_adif().to_string());
    let mode = cmd.mode.as_ref().map(|m| m.as_adif().to_string());
    Some((station, call, date, band, mode))
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
///   confirmed (date from `QRZCOM_QSO_DOWNLOAD_DATE`).
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

        let upload = (up_status.as_deref() == Some("Y")).then_some("uploaded");
        let confirm = (dn_status.as_deref() == Some("Y")).then_some("confirmed");
        if upload.is_some() || confirm.is_some() {
            out.push(ImportedServiceState {
                qso_id,
                service: "qrz",
                upload_state: upload,
                confirmation_state: confirm,
                uploaded_at: if upload.is_some() { up_date } else { None },
                confirmed_at: if confirm.is_some() { dn_date } else { None },
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
