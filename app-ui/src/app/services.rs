use std::collections::HashSet;
use std::sync::Arc;

use app_config::{ClubLogConfig, Config, EqslConfig, HrdlogConfig, LotwConfig, QrzConfig};
use chrono::{DateTime, Utc};
use clublog_sync::{ClubLogConfig as ClubLogClientConfig, ClubLogUploadClient};
use eqsl_sync::{EqslFetchClient, EqslFetchConfig, EqslUploadClient, EqslUploadConfig};
use hrdlog_sync::{HrdlogConfig as HrdlogClientConfig, HrdlogUploadClient};
use logbook_domain::credit::parse_credit_list;
use logbook_domain::{
    CreditStatus, DetailField, ExportOptions, QsoRepository, ServiceQso, StationRepository,
    SyncOutcome, apply_report, export_adif,
};
use lotw_sync::{LotwFetchClient, LotwFetchConfig, LotwRecord, LotwReportKind, SINCE_BEGINNING};
use qrz_sync::{QrzConfig as QrzClientConfig, QrzUploadClient};
use radio_core::StationLocation;

use super::constants::{
    CLUBLOG_SERVICE, EQSL_SERVICE, HRDLOG_SERVICE, LOTW_SERVICE, QRZ_SERVICE,
};
use super::lotw_upload;
use super::message::{
    ClubLogUpdateSummary, EqslUpdateSummary, FetchSummary, HrdlogUpdateSummary,
    MultiUpdateSummary, QrzUpdateSummary, UpdateSummary, UploadSummary,
};

/// One user gesture spans all configured services. Each service's
/// upload + fetch are run sequentially; failures in one phase don't stop
/// the next phase or the next service.
pub(super) async fn run_services_update(
    repo: Arc<dyn QsoRepository>,
    station_repo: Arc<dyn StationRepository>,
    cfg: Config,
) -> MultiUpdateSummary {
    let lotw = match station_repo.list_locations().await {
        Ok(locations)
            if lotw_upload::upload_configured(&locations) || cfg.lotw.is_configured_for_fetch() =>
        {
            Some(run_lotw_update(repo.clone(), &locations, cfg.lotw).await)
        }
        Ok(_) => None,
        Err(e) => Some(UpdateSummary {
            upload_error: Some(format!("reading station locations: {e}")),
            ..Default::default()
        }),
    };
    let eqsl = if cfg.eqsl.is_configured() {
        Some(run_eqsl_update(repo.clone(), cfg.eqsl).await)
    } else {
        None
    };
    let clublog = if cfg.clublog.is_configured() {
        Some(run_clublog_update(repo.clone(), cfg.clublog).await)
    } else {
        None
    };
    let qrz = if cfg.qrz.is_configured() {
        Some(run_qrz_update(repo.clone(), cfg.qrz).await)
    } else {
        None
    };
    let hrdlog = if cfg.hrdlog.is_configured() {
        Some(run_hrdlog_update(repo, cfg.hrdlog).await)
    } else {
        None
    };
    MultiUpdateSummary {
        lotw,
        eqsl,
        clublog,
        qrz,
        hrdlog,
    }
}

/// LoTW's part of an update: upload what's pending, then read back what
/// LoTW has accepted and confirmed since the last sync.
async fn run_lotw_update(
    repo: Arc<dyn QsoRepository>,
    locations: &[StationLocation],
    cfg: LotwConfig,
) -> UpdateSummary {
    let mut summary = UpdateSummary::default();

    if lotw_upload::upload_configured(locations) {
        match lotw_upload::run_lotw_upload(repo.clone(), locations, &cfg).await {
            Ok(u) => summary.upload = Some(u),
            Err(e) => summary.upload_error = Some(e),
        }
    } else {
        summary.upload_skipped_reason =
            Some("no station location has a TQSL location set".into());
    }

    if cfg.is_configured_for_fetch() {
        match run_lotw_sync(repo, &cfg, false).await {
            Ok(f) => summary.fetch = Some(f),
            Err(e) => summary.fetch_error = Some(e),
        }
    } else {
        summary.fetch_skipped_reason = Some("no [lotw].username/password".into());
    }

    summary
}

/// Read both LoTW reports and apply them. Incremental unless
/// `full_resync`, which ignores the stored watermarks and reads
/// everything (then stores fresh ones).
///
/// The reports are independent: one failing leaves the other's results
/// and watermark in place. `Err` only when both fail.
pub(super) async fn run_lotw_sync(
    repo: Arc<dyn QsoRepository>,
    cfg: &LotwConfig,
    full_resync: bool,
) -> Result<FetchSummary, String> {
    let (Some(username), Some(password)) = (cfg.username.clone(), cfg.password.clone()) else {
        return Err("missing [lotw].username/password".into());
    };
    let client = LotwFetchClient::new(LotwFetchConfig::new(username, password));

    let mut summary = FetchSummary {
        full_resync,
        ..Default::default()
    };
    // A QSO LoTW has but the log doesn't can turn up in both reports;
    // count it once.
    let mut not_in_log: HashSet<(Option<String>, String, Option<String>, DateTime<Utc>)> =
        HashSet::new();
    for kind in [LotwReportKind::Confirmations, LotwReportKind::Uploads] {
        match sync_lotw_report(repo.as_ref(), &client, kind, full_resync).await {
            Ok((records, outcome, unreadable)) => {
                summary.new_confirmations += outcome.newly_confirmed;
                summary.uploads_verified += outcome.newly_verified;
                summary.credits += outcome.credits_changed;
                summary.qsos_filled += outcome.qsos_filled;
                summary.qsos_differing += outcome.qsos_differing;
                summary.unreadable += unreadable;
                for &i in &outcome.not_in_log {
                    let r = &records[i];
                    not_in_log.insert((
                        r.station_callsign.clone(),
                        r.call.clone(),
                        r.band.clone(),
                        r.start,
                    ));
                }
            }
            Err(e) => {
                tracing::warn!(report = kind.label(), error = %e, "LoTW sync report failed");
                summary.errors.push(format!("{} report failed: {e}", kind.label()));
            }
        }
    }
    summary.not_in_log = not_in_log.len();
    if summary.errors.len() == 2 {
        return Err(summary.errors.join("; "));
    }
    Ok(summary)
}

/// Fetch one report since its watermark and apply it; the new watermark
/// is stored in the same transaction as the results. Returns the records
/// as matched, what applying them did, and how many were unreadable.
async fn sync_lotw_report(
    repo: &dyn QsoRepository,
    client: &LotwFetchClient,
    kind: LotwReportKind,
    full_resync: bool,
) -> Result<(Vec<ServiceQso>, SyncOutcome, usize), String> {
    let stored = if full_resync {
        None
    } else {
        repo.get_sync_watermark(LOTW_SERVICE, kind.watermark_key())
            .await
            .map_err(|e| format!("reading last sync point: {e}"))?
    };
    let since = stored.as_deref().unwrap_or(SINCE_BEGINNING);
    let report = client.fetch(kind, since).await.map_err(|e| e.to_string())?;
    let records: Vec<ServiceQso> = report.records.into_iter().map(service_qso).collect();
    let outcome = apply_report(
        repo,
        LOTW_SERVICE,
        &records,
        kind.watermark_key(),
        report.watermark,
        Utc::now(),
    )
    .await
    .map_err(|e| format!("storing results: {e}"))?;
    Ok((records, outcome, report.unreadable))
}

/// An LoTW record in the shape the matcher takes. Credit lists use the
/// ADIF award enumeration the importer reads too.
fn service_qso(r: LotwRecord) -> ServiceQso {
    let mut credits = Vec::new();
    if let Some(list) = &r.credit_granted {
        credits.extend(parse_credit_list(list, CreditStatus::Granted));
    }
    if let Some(list) = &r.credit_submitted {
        credits.extend(parse_credit_list(list, CreditStatus::Submitted));
    }
    ServiceQso {
        station_callsign: r.station_callsign,
        call: r.worked_callsign,
        start: r.qso_start,
        band: r.band,
        mode: r.mode,
        mode_group: r.mode_group,
        confirmed: r.qsl_rcvd,
        confirmed_at: r.qsl_rdate,
        uploaded_at: r.rx_qso,
        credits,
        dxcc_id: r.dxcc,
        state: r.state,
        grid: r.gridsquare,
        detail: r
            .qsl_detail
            .into_iter()
            .filter_map(|(name, value)| Some((DetailField::from_adif_name(&name)?, value)))
            .collect(),
    }
}

async fn run_eqsl_update(repo: Arc<dyn QsoRepository>, cfg: EqslConfig) -> EqslUpdateSummary {
    let mut summary = EqslUpdateSummary::default();
    let (Some(username), Some(password)) = (cfg.username.clone(), cfg.password.clone()) else {
        summary.upload_skipped_reason = Some("missing [eqsl] username/password".into());
        summary.fetch_skipped_reason = Some("missing [eqsl] username/password".into());
        return summary;
    };

    // Upload phase.
    match run_eqsl_upload(repo.clone(), &username, &password).await {
        Ok(u) => summary.upload = Some(u),
        Err(e) => summary.upload_error = Some(e),
    }

    // Fetch phase.
    match run_eqsl_fetch(repo, &username, &password).await {
        Ok((fetched, confirmed, unmatched)) => {
            summary.fetched = fetched;
            summary.confirmed = confirmed;
            summary.unmatched = unmatched;
        }
        Err(e) => summary.fetch_error = Some(e),
    }
    summary
}

async fn run_eqsl_upload(
    repo: Arc<dyn QsoRepository>,
    username: &str,
    password: &str,
) -> Result<UploadSummary, String> {
    let pending = repo
        .list_pending_uploads(EQSL_SERVICE, Some(500))
        .await
        .map_err(|e| e.to_string())?;
    if pending.is_empty() {
        return Ok(UploadSummary {
            uploaded: 0,
            note: "no pending QSOs".into(),
        });
    }
    let adif = export_adif(&pending, &ExportOptions::default());
    let client = EqslUploadClient::new(EqslUploadConfig::new(username, password));
    let outcome = client.upload_adif(&adif).await.map_err(|e| e.to_string())?;
    let now = Utc::now();
    let mut marked = 0usize;
    for qso in &pending {
        if let Err(e) = repo
            .mark_uploaded(&qso.id, EQSL_SERVICE, now, None)
            .await
        {
            tracing::warn!(qso_id = %qso.id, error = %e, "failed to mark eqsl uploaded");
        } else {
            marked += 1;
        }
    }
    let note = if outcome.accepted {
        "accepted by eQSL"
    } else {
        "uploaded; eQSL response unclear — verify on eqsl.cc"
    };
    Ok(UploadSummary {
        uploaded: marked,
        note: note.into(),
    })
}

async fn run_eqsl_fetch(
    repo: Arc<dyn QsoRepository>,
    username: &str,
    password: &str,
) -> Result<(usize, usize, usize), String> {
    let client = EqslFetchClient::new(EqslFetchConfig::new(username, password));
    let records = client.fetch(None).await.map_err(|e| e.to_string())?;
    let fetched = records.len();
    let now = Utc::now();
    let mut confirmed = 0usize;
    let mut unmatched = 0usize;
    for rec in records {
        let Some(station) = rec.station_callsign.as_deref() else {
            unmatched += 1;
            continue;
        };
        let qso_id = repo
            .find_match_for_confirmation(
                station,
                &rec.worked_callsign,
                &rec.qso_date,
                rec.band.as_deref(),
                rec.mode.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;
        match qso_id {
            Some(id) => {
                if let Err(e) = repo.mark_confirmed(&id, EQSL_SERVICE, now, None).await {
                    tracing::warn!(qso_id = %id, error = %e, "failed to mark eqsl confirmed");
                    unmatched += 1;
                } else {
                    confirmed += 1;
                }
            }
            None => unmatched += 1,
        }
    }
    Ok((fetched, confirmed, unmatched))
}

async fn run_clublog_update(
    repo: Arc<dyn QsoRepository>,
    cfg: ClubLogConfig,
) -> ClubLogUpdateSummary {
    let mut summary = ClubLogUpdateSummary::default();
    let (Some(email), Some(password), Some(callsign)) =
        (cfg.email.clone(), cfg.password.clone(), cfg.callsign.clone())
    else {
        summary.upload_skipped_reason = Some("missing [clublog] fields".into());
        return summary;
    };

    let pending = match repo.list_pending_uploads(CLUBLOG_SERVICE, Some(500)).await {
        Ok(p) => p,
        Err(e) => {
            summary.upload_error = Some(e.to_string());
            return summary;
        }
    };
    if pending.is_empty() {
        summary.upload = Some(UploadSummary {
            uploaded: 0,
            note: "no pending QSOs".into(),
        });
        return summary;
    }
    let adif = export_adif(&pending, &ExportOptions::default());
    let client = ClubLogUploadClient::new(ClubLogClientConfig::new(email, password, callsign));
    match client.upload_adif(&adif).await {
        Ok(outcome) => {
            let now = Utc::now();
            let mut marked = 0usize;
            for qso in &pending {
                if let Err(e) = repo
                    .mark_uploaded(&qso.id, CLUBLOG_SERVICE, now, None)
                    .await
                {
                    tracing::warn!(qso_id = %qso.id, error = %e, "failed to mark clublog uploaded");
                } else {
                    marked += 1;
                }
            }
            let note = if outcome.accepted {
                "accepted by Club Log"
            } else {
                "uploaded; Club Log response unclear — verify on clublog.org"
            };
            summary.upload = Some(UploadSummary {
                uploaded: marked,
                note: note.into(),
            });
        }
        Err(e) => summary.upload_error = Some(e.to_string()),
    }
    summary
}

async fn run_qrz_update(repo: Arc<dyn QsoRepository>, cfg: QrzConfig) -> QrzUpdateSummary {
    let mut summary = QrzUpdateSummary::default();
    let Some(api_key) = cfg.api_key.clone() else {
        summary.upload_skipped_reason = Some("missing [qrz].api_key".into());
        return summary;
    };
    let pending = match repo.list_pending_uploads(QRZ_SERVICE, Some(500)).await {
        Ok(p) => p,
        Err(e) => {
            summary.upload_error = Some(e.to_string());
            return summary;
        }
    };
    if pending.is_empty() {
        return summary;
    }
    let adif = export_adif(&pending, &ExportOptions::default());
    let client = QrzUploadClient::new(QrzClientConfig::new(api_key));
    match client.upload_adif(&adif).await {
        Ok(outcome) => {
            let now = Utc::now();
            // QRZ accepts records sequentially; "accepted" count tells us
            // how many returned RESULT=OK or RESULT=REPLACE. Mark only
            // those as uploaded, not the rejected ones.
            //
            // The current API doesn't surface per-record correlation back
            // to caller, so we mark the FIRST `accepted` pending QSOs.
            // This matches QRZ's record order (which is the ADIF record
            // order, which is `pending`'s order).
            for qso in pending.iter().take(outcome.accepted) {
                if let Err(e) = repo.mark_uploaded(&qso.id, QRZ_SERVICE, now, None).await {
                    tracing::warn!(qso_id = %qso.id, error = %e, "failed to mark qrz uploaded");
                }
            }
            for qso in pending.iter().skip(outcome.accepted).take(outcome.rejected) {
                let err_msg = outcome
                    .errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "unknown error".into());
                if let Err(e) = repo.mark_upload_failed(&qso.id, QRZ_SERVICE, &err_msg).await {
                    tracing::warn!(qso_id = %qso.id, error = %e, "failed to mark qrz failed");
                }
            }
            summary.uploaded = outcome.accepted;
            summary.rejected = outcome.rejected;
            summary.first_errors = outcome.errors.into_iter().take(3).collect();
        }
        Err(e) => summary.upload_error = Some(e.to_string()),
    }
    summary
}

async fn run_hrdlog_update(
    repo: Arc<dyn QsoRepository>,
    cfg: HrdlogConfig,
) -> HrdlogUpdateSummary {
    let mut summary = HrdlogUpdateSummary::default();
    let (Some(callsign), Some(code)) = (cfg.callsign.clone(), cfg.code.clone()) else {
        summary.upload_skipped_reason = Some("missing [hrdlog] callsign/code".into());
        return summary;
    };
    let pending = match repo.list_pending_uploads(HRDLOG_SERVICE, Some(500)).await {
        Ok(p) => p,
        Err(e) => {
            summary.upload_error = Some(e.to_string());
            return summary;
        }
    };
    if pending.is_empty() {
        summary.upload = Some(UploadSummary {
            uploaded: 0,
            note: "no pending QSOs".into(),
        });
        return summary;
    }
    let adif = export_adif(&pending, &ExportOptions::default());
    let client = HrdlogUploadClient::new(HrdlogClientConfig::new(callsign, code));
    match client.upload_adif(&adif).await {
        Ok(outcome) => {
            let now = Utc::now();
            let mut marked = 0usize;
            for qso in &pending {
                if let Err(e) = repo
                    .mark_uploaded(&qso.id, HRDLOG_SERVICE, now, None)
                    .await
                {
                    tracing::warn!(qso_id = %qso.id, error = %e, "failed to mark hrdlog uploaded");
                } else {
                    marked += 1;
                }
            }
            let note = if outcome.accepted {
                "accepted by HRDLog"
            } else {
                "uploaded; HRDLog response unclear — verify on hrdlog.net"
            };
            summary.upload = Some(UploadSummary {
                uploaded: marked,
                note: note.into(),
            });
        }
        Err(e) => summary.upload_error = Some(e.to_string()),
    }
    summary
}

pub(super) fn format_multi_summary(s: &MultiUpdateSummary) -> String {
    let mut parts = Vec::new();
    match &s.lotw {
        Some(u) => parts.push(format!("lotw: {}", format_lotw_summary(u))),
        None => parts.push("lotw: skipped (not configured)".into()),
    }
    match &s.eqsl {
        Some(u) => parts.push(format!("eqsl: {}", format_eqsl_summary(u))),
        None => parts.push("eqsl: skipped (not configured)".into()),
    }
    match &s.clublog {
        Some(u) => parts.push(format!("clublog: {}", format_clublog_summary(u))),
        None => parts.push("clublog: skipped (not configured)".into()),
    }
    match &s.qrz {
        Some(u) => parts.push(format!("qrz: {}", format_qrz_summary(u))),
        None => parts.push("qrz: skipped (not configured)".into()),
    }
    match &s.hrdlog {
        Some(u) => parts.push(format!("hrdlog: {}", format_hrdlog_summary(u))),
        None => parts.push("hrdlog: skipped (not configured)".into()),
    }
    parts.join(" | ")
}

fn format_lotw_summary(s: &UpdateSummary) -> String {
    let mut parts = Vec::new();
    if let Some(u) = &s.upload {
        parts.push(format!("upload: {}", lotw_upload::format_lotw_upload(u)));
    } else if let Some(e) = &s.upload_error {
        parts.push(format!("upload error: {e}"));
    } else if let Some(r) = &s.upload_skipped_reason {
        parts.push(format!("upload skip ({r})"));
    }
    if let Some(f) = &s.fetch {
        parts.push(format!("sync: {}", format_lotw_fetch(f)));
    } else if let Some(e) = &s.fetch_error {
        parts.push(format!("sync error: {e}"));
    } else if let Some(r) = &s.fetch_skipped_reason {
        parts.push(format!("sync skip ({r})"));
    }
    if parts.is_empty() {
        "nothing to do".into()
    } else {
        parts.join(", ")
    }
}

/// One line for a LoTW sync, e.g. `3 new confirmations · 12 uploads
/// verified · 1 not in log`. "Not in log" is QSOs LoTW holds that match
/// nothing here — expected when QSOs were uploaded from elsewhere.
/// "Filled in" and "differ" count QSOs, not fields: those whose blank
/// location fields took the other station's values from its
/// confirmation, and those where it newly reports something the log
/// doesn't hold (listed by the Logbook's "Differs from LoTW" filter).
pub(super) fn format_lotw_fetch(f: &FetchSummary) -> String {
    let count = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = Vec::new();
    if f.full_resync {
        parts.push("full resync".to_string());
    }
    parts.push(count(f.new_confirmations, "new confirmation", "new confirmations"));
    parts.push(count(f.uploads_verified, "upload verified", "uploads verified"));
    if f.not_in_log > 0 {
        parts.push(format!("{} not in log", f.not_in_log));
    }
    if f.credits > 0 {
        parts.push(count(f.credits, "award credit updated", "award credits updated"));
    }
    if f.qsos_filled > 0 {
        parts.push(count(f.qsos_filled, "QSO filled in from LoTW", "QSOs filled in from LoTW"));
    }
    if f.qsos_differing > 0 {
        parts.push(count(f.qsos_differing, "differs from LoTW", "differ from LoTW"));
    }
    if f.unreadable > 0 {
        parts.push(count(f.unreadable, "record unreadable", "records unreadable"));
    }
    parts.extend(f.errors.iter().cloned());
    parts.join(" · ")
}

fn format_eqsl_summary(s: &EqslUpdateSummary) -> String {
    let mut parts = Vec::new();
    if let Some(u) = &s.upload {
        parts.push(format!("upload {} qso ({})", u.uploaded, u.note));
    } else if let Some(e) = &s.upload_error {
        parts.push(format!("upload error: {e}"));
    } else if let Some(r) = &s.upload_skipped_reason {
        parts.push(format!("upload skip ({r})"));
    }
    if s.fetched > 0 || s.confirmed > 0 || s.unmatched > 0 {
        parts.push(format!(
            "fetch {} records, {} confirmed, {} unmatched",
            s.fetched, s.confirmed, s.unmatched
        ));
    } else if let Some(e) = &s.fetch_error {
        parts.push(format!("fetch error: {e}"));
    } else if let Some(r) = &s.fetch_skipped_reason {
        parts.push(format!("fetch skip ({r})"));
    }
    if parts.is_empty() {
        "nothing to do".into()
    } else {
        parts.join(", ")
    }
}

fn format_clublog_summary(s: &ClubLogUpdateSummary) -> String {
    if let Some(u) = &s.upload {
        format!("upload {} qso ({})", u.uploaded, u.note)
    } else if let Some(e) = &s.upload_error {
        format!("upload error: {e}")
    } else if let Some(r) = &s.upload_skipped_reason {
        format!("upload skip ({r})")
    } else {
        "nothing to do".into()
    }
}

fn format_qrz_summary(s: &QrzUpdateSummary) -> String {
    if let Some(e) = &s.upload_error {
        return format!("upload error: {e}");
    }
    if let Some(r) = &s.upload_skipped_reason {
        return format!("upload skip ({r})");
    }
    if s.uploaded == 0 && s.rejected == 0 {
        return "nothing to do".into();
    }
    let mut msg = format!("upload {} ok / {} rejected", s.uploaded, s.rejected);
    if let Some(first) = s.first_errors.first() {
        msg.push_str(&format!(" — first: {first}"));
    }
    msg
}

fn format_hrdlog_summary(s: &HrdlogUpdateSummary) -> String {
    if let Some(u) = &s.upload {
        format!("upload {} qso ({})", u.uploaded, u.note)
    } else if let Some(e) = &s.upload_error {
        format!("upload error: {e}")
    } else if let Some(r) = &s.upload_skipped_reason {
        format!("upload skip ({r})")
    } else {
        "nothing to do".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lotw_record_credit_lists_become_award_credit() {
        let body = "<EOH><STATION_CALLSIGN:5>K1ABC<CALL:4>W1AW<BAND:3>20M<MODE:2>CW\
            <QSO_DATE:8>20260508<TIME_ON:4>1830<QSL_RCVD:1>Y\
            <CREDIT_GRANTED:19>DXCC:LOTW,DXCC_BAND<CREDIT_SUBMITTED:8>WAS:LOTW\
            <APP_LoTW_CREDIT_GRANTED:4>DXCC<EOR><APP_LoTW_EOF>";
        let report = lotw_sync::fetch::parse_report(body, LotwReportKind::Confirmations).unwrap();
        let q = service_qso(report.records.into_iter().next().unwrap());
        assert!(q.confirmed);
        assert!(q.detail.is_empty());
        let got: Vec<(&str, Option<&str>, CreditStatus)> = q
            .credits
            .iter()
            .map(|c| (c.award.as_str(), c.medium.as_deref(), c.status))
            .collect();
        assert_eq!(
            got,
            vec![
                ("DXCC", Some("LOTW"), CreditStatus::Granted),
                ("DXCC_BAND", None, CreditStatus::Granted),
                ("WAS", Some("LOTW"), CreditStatus::Submitted),
            ]
        );
    }

    #[test]
    fn lotw_summary_says_what_happened_in_plain_terms() {
        let mut f = FetchSummary {
            new_confirmations: 3,
            uploads_verified: 12,
            not_in_log: 1,
            ..Default::default()
        };
        assert_eq!(
            format_lotw_fetch(&f),
            "3 new confirmations · 12 uploads verified · 1 not in log"
        );
        f.full_resync = true;
        f.new_confirmations = 1;
        f.uploads_verified = 0;
        f.credits = 2;
        f.qsos_filled = 41;
        f.qsos_differing = 3;
        f.errors.push("uploads report failed: HTTP error: timed out".into());
        assert_eq!(
            format_lotw_fetch(&f),
            "full resync · 1 new confirmation · 0 uploads verified · 1 not in log · \
             2 award credits updated · 41 QSOs filled in from LoTW · 3 differ from LoTW · \
             uploads report failed: HTTP error: timed out"
        );
        f.qsos_filled = 1;
        f.qsos_differing = 1;
        f.errors.clear();
        assert!(format_lotw_fetch(&f).ends_with("1 QSO filled in from LoTW · 1 differs from LoTW"));
    }

    #[test]
    fn lotw_record_detail_reaches_the_matcher_by_field() {
        let body = "<EOH><STATION_CALLSIGN:5>K1ABC<CALL:4>W1AW<BAND:3>20M<MODE:2>CW\
            <QSO_DATE:8>20260508<TIME_ON:4>1830<QSL_RCVD:1>Y<DXCC:3>291<PFX:2>W1\
            <STATE:2>ct<GRIDSQUARE:6>FN31pr<COUNTRY:3>USA<EOR><APP_LoTW_EOF>";
        let report = lotw_sync::fetch::parse_report(body, LotwReportKind::Confirmations).unwrap();
        let q = service_qso(report.records.into_iter().next().unwrap());
        assert_eq!(
            q.detail,
            vec![
                (DetailField::Dxcc, "291".to_string()),
                (DetailField::Gridsquare, "FN31pr".to_string()),
                (DetailField::State, "ct".to_string()),
                (DetailField::Pfx, "W1".to_string()),
            ]
        );
        // The credit unit keeps its own, upper-cased copy.
        assert_eq!(q.state.as_deref(), Some("CT"));
    }
}
