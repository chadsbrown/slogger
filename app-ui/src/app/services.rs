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
use qrz_sync::{BookStatus, InsertOutcome, QrzClient, QrzConfig as QrzClientConfig, QrzRecord};
use radio_core::{Qso, StationLocation};

use super::constants::{
    CLUBLOG_SERVICE, EQSL_SERVICE, HRDLOG_SERVICE, LOTW_SERVICE, QRZ_SERVICE,
};
use super::lotw_upload;
use super::message::{
    ClubLogUpdateSummary, EqslUpdateSummary, FetchSummary, HrdlogUpdateSummary,
    MultiUpdateSummary, QrzFetchSummary, QrzUpdateSummary, UpdateSummary, UploadSummary,
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
    tracing::info!(
        report = kind.label(),
        records = records.len(),
        "LoTW: report read; matching against the log"
    );
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
    tracing::info!(report = kind.label(), "LoTW: results stored");
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

/// Most QSOs sent to QRZ in one update. Each is its own request.
const QRZ_UPLOAD_LIMIT: usize = 500;

/// Where QRZ's "modified since" date is kept between syncs.
const QRZ_WATERMARK_KEY: &str = "modified_since";

/// QRZ's part of an update: check the key, read back what the logbook
/// holds and has confirmed since the last sync, then send what's still
/// pending.
///
/// Reading comes first so that QSOs already in the logbook — uploaded
/// from another program, or before a database was rebuilt — are seen to
/// be there and not sent again as duplicates.
async fn run_qrz_update(repo: Arc<dyn QsoRepository>, cfg: QrzConfig) -> QrzUpdateSummary {
    let mut summary = QrzUpdateSummary::default();
    let Some(api_key) = cfg.api_key.clone() else {
        summary.upload_skipped_reason = Some("missing [qrz].api_key".into());
        return summary;
    };
    qrz_update_with(repo.as_ref(), &QrzClient::new(QrzClientConfig::new(api_key))).await
}

async fn qrz_update_with(repo: &dyn QsoRepository, client: &QrzClient) -> QrzUpdateSummary {
    let mut summary = QrzUpdateSummary::default();

    // A bad key or a lapsed subscription stops here, with QRZ's reason,
    // rather than failing every QSO in turn.
    let book = match client.status().await {
        Ok(book) => book,
        Err(e) => {
            summary.upload_error = Some(e.to_string());
            return summary;
        }
    };
    summary.book_callsign = book.callsign.clone();

    match run_qrz_sync(repo, client).await {
        Ok(f) => summary.fetch = Some(f),
        Err(e) => {
            tracing::warn!(error = %e, "QRZ sync failed");
            summary.fetch_error = Some(e);
        }
    }

    run_qrz_upload(repo, client, &book, &mut summary).await;
    summary
}

/// Read the logbook's records modified since the last sync (all of them
/// the first time) and apply them: QSOs found there become
/// upload-verified, and those QRZ has matched become confirmed. The new
/// sync point is stored in the same transaction as the results.
async fn run_qrz_sync(
    repo: &dyn QsoRepository,
    client: &QrzClient,
) -> Result<QrzFetchSummary, String> {
    let since = repo
        .get_sync_watermark(QRZ_SERVICE, QRZ_WATERMARK_KEY)
        .await
        .map_err(|e| format!("reading last sync point: {e}"))?
        .and_then(|d| chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok());
    let started = Utc::now();
    let report = client
        .fetch_modified_since(since)
        .await
        .map_err(|e| e.to_string())?;
    let records: Vec<ServiceQso> = report.records.into_iter().map(qrz_service_qso).collect();
    tracing::info!(records = records.len(), "QRZ: logbook read; matching against the log");

    // QRZ compares whole dates, and not necessarily in UTC, so the next
    // sync starts a day before this one did. It will see a few records
    // again; applying a record twice changes nothing.
    let next_since = (started.date_naive() - chrono::Days::new(1))
        .format("%Y-%m-%d")
        .to_string();
    let outcome = apply_report(
        repo,
        QRZ_SERVICE,
        &records,
        QRZ_WATERMARK_KEY,
        Some(next_since),
        started,
    )
    .await
    .map_err(|e| format!("storing results: {e}"))?;
    Ok(QrzFetchSummary {
        full: since.is_none(),
        records: records.len(),
        new_confirmations: outcome.newly_confirmed,
        uploads_verified: outcome.newly_verified,
        not_in_log: outcome.not_in_log.len(),
        unreadable: report.unreadable,
    })
}

fn qrz_service_qso(r: QrzRecord) -> ServiceQso {
    ServiceQso {
        station_callsign: r.station_callsign,
        call: r.call,
        start: r.start,
        band: r.band,
        mode: r.mode,
        mode_group: None,
        confirmed: r.confirmed,
        confirmed_at: r.confirmed_at,
        uploaded_at: r.uploaded_at,
        // QRZ confirmations carry no award credit, and its copy of the
        // other station's details is a callbook lookup, not something
        // that station logged.
        credits: Vec::new(),
        dxcc_id: None,
        state: None,
        grid: None,
        detail: Vec::new(),
    }
}

/// Whether `book` is where `qso` belongs. A QRZ logbook serves one
/// station callsign — `K1ABC` and `K1ABC/P` are different books — over a
/// range of dates, and rejects everything else.
fn qrz_book_takes(book: &BookStatus, qso: &Qso) -> bool {
    let Some(station) = &qso.station_callsign else {
        // QRZ needs to know who made the QSO.
        return false;
    };
    if let Some(callsign) = &book.callsign
        && !station.as_str().eq_ignore_ascii_case(callsign)
    {
        return false;
    }
    let day = qso.qso_begin.date_naive();
    book.start_date.is_none_or(|start| day >= start) && book.end_date.is_none_or(|end| day <= end)
}

/// Send the pending QSOs that belong in `book`, one request each, and
/// record what became of each one as its answer arrives — so whatever
/// stops the run, the QSOs already sent are accounted for.
async fn run_qrz_upload(
    repo: &dyn QsoRepository,
    client: &QrzClient,
    book: &BookStatus,
    summary: &mut QrzUpdateSummary,
) {
    let pending = match repo.list_pending_uploads(QRZ_SERVICE, None).await {
        Ok(p) => p,
        Err(e) => {
            summary.upload_error = Some(e.to_string());
            return;
        }
    };
    let (ours, others): (Vec<&Qso>, Vec<&Qso>) =
        pending.iter().partition(|q| qrz_book_takes(book, q));
    summary.held_back = others.len();

    for qso in ours.into_iter().take(QRZ_UPLOAD_LIMIT) {
        let adif = export_adif(std::slice::from_ref(qso), &ExportOptions::default());
        let marked = match client.insert(&adif).await {
            Ok(InsertOutcome::Inserted { logid } | InsertOutcome::Replaced { logid }) => {
                summary.uploaded += 1;
                let logid = logid.map(|id| id.to_string());
                repo.mark_uploaded(&qso.id, QRZ_SERVICE, Utc::now(), logid.as_deref())
                    .await
            }
            Ok(InsertOutcome::Duplicate) => {
                // Already there, so there is nothing left to send.
                summary.duplicates += 1;
                repo.mark_uploaded(&qso.id, QRZ_SERVICE, Utc::now(), None).await
            }
            Ok(InsertOutcome::Rejected(reason)) => {
                summary.rejected += 1;
                if summary.first_errors.len() < 3 {
                    summary.first_errors.push(format!("{}: {reason}", qso.call.as_str()));
                }
                repo.mark_upload_failed(&qso.id, QRZ_SERVICE, &reason).await
            }
            Err(e) => {
                // The key stopped working or the connection failed; the
                // rest would only fail the same way.
                summary.upload_error = Some(e.to_string());
                return;
            }
        };
        if let Err(e) = marked {
            tracing::warn!(qso_id = %qso.id, error = %e, "failed to record QRZ upload result");
        }
    }
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
    if let Some(r) = &s.upload_skipped_reason {
        return format!("upload skip ({r})");
    }
    let mut parts = Vec::new();
    if let Some(f) = &s.fetch {
        parts.push(format!("sync: {}", format_qrz_fetch(f)));
    } else if let Some(e) = &s.fetch_error {
        parts.push(format!("sync error: {e}"));
    }
    if s.uploaded + s.duplicates + s.rejected > 0 {
        let mut msg = format!("upload {} ok", s.uploaded);
        if s.duplicates > 0 {
            msg.push_str(&format!(" / {} already there", s.duplicates));
        }
        if s.rejected > 0 {
            msg.push_str(&format!(" / {} rejected", s.rejected));
        }
        if let Some(first) = s.first_errors.first() {
            msg.push_str(&format!(" — first: {first}"));
        }
        parts.push(msg);
    }
    if s.held_back > 0 {
        let book = s.book_callsign.as_deref().unwrap_or("this");
        parts.push(format!("{} not for the {book} logbook", s.held_back));
    }
    if let Some(e) = &s.upload_error {
        parts.push(format!("error: {e}"));
    }
    if parts.is_empty() {
        "nothing to do".into()
    } else {
        parts.join(", ")
    }
}

/// One line for a QRZ sync, e.g. `2 new confirmations · 5 uploads
/// verified · 1 not in log`. "Not in log" is QSOs QRZ holds that match
/// nothing here.
pub(super) fn format_qrz_fetch(f: &QrzFetchSummary) -> String {
    let mut parts = Vec::new();
    if f.new_confirmations > 0 {
        parts.push(format!("{} new confirmations", f.new_confirmations));
    }
    if f.uploads_verified > 0 {
        parts.push(format!("{} uploads verified", f.uploads_verified));
    }
    if f.not_in_log > 0 {
        parts.push(format!("{} not in log", f.not_in_log));
    }
    if f.unreadable > 0 {
        parts.push(format!("{} unreadable", f.unreadable));
    }
    if parts.is_empty() {
        parts.push("no changes".into());
    }
    let scope = if f.full { "whole logbook" } else { "since last sync" };
    format!("{} ({} records read, {scope})", parts.join(" · "), f.records)
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

    // ---- QRZ ----

    use app_persistence::{Database, SqliteQsoRepository};
    use chrono::TimeZone;
    use logbook_domain::{CreateQsoCommand, LogbookService};
    use radio_core::{Band, Callsign, QsoId};
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A stand-in for QRZ that answers by what it is asked: the status
    /// of K1ABC's logbook, a fetch holding one confirmed QSO with W1AW,
    /// and inserts that succeed, duplicate or fail by the call worked.
    /// Returns a client for it and the request bodies it received.
    async fn fake_qrz(valid_key: bool) -> (QrzClient, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let body = loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).into_owned();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let wanted = head
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:")?.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if body.len() >= wanted {
                            break body.to_string();
                        }
                    }
                    if n == 0 {
                        break String::new();
                    }
                };
                seen2.lock().unwrap().push(body.clone());
                let (status, answer) = if !valid_key {
                    (401, "STATUS=AUTH&RESULT=AUTH&REASON=invalid api key\n&EXTENDED=".to_string())
                } else if body.contains("ACTION=STATUS") {
                    (200, "RESULT=OK&CALLSIGN=K1ABC&START_DATE=2020-01-01&END_DATE=2030-01-01".into())
                } else if body.contains("ACTION=FETCH") {
                    (200, "COUNT=1&RESULT=OK&ADIF=&lt;app_qrzlog_logid:2&gt;41&lt;station_callsign:5&gt;K1ABC\
                        &lt;call:4&gt;W1AW&lt;qso_date:8&gt;20260508&lt;time_on:4&gt;1830&lt;band:3&gt;20M\
                        &lt;app_qrzlog_status:1&gt;C&lt;app_qrzlog_qsldate:8&gt;20260601&lt;eor&gt;".into())
                } else if body.contains("JA1NUT") {
                    (200, "RESULT=OK&LOGID=55&COUNT=1".into())
                } else if body.contains("DL1XYZ") {
                    (200, "RESULT=FAIL&REASON=Unable to add QSO to database: duplicate&EXTENDED=".into())
                } else {
                    (200, "RESULT=FAIL&REASON=wrong station_callsign for this logbook".into())
                };
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        let config = QrzClientConfig::new("KEY-123").with_api_url(format!("http://{addr}/api"));
        (QrzClient::new(config), seen)
    }

    /// A log of five QSOs on 2026-05-08: W1AW (which the fake QRZ already
    /// holds, confirmed), JA1NUT, DL1XYZ and F5XYZ as K1ABC, and G4ABC
    /// as K1ABC/P. Returns the repository and each QSO's id by call.
    async fn qrz_log() -> (Arc<SqliteQsoRepository>, impl Fn(&str) -> QsoId) {
        let db = Database::open_in_memory().await.unwrap();
        let repo = Arc::new(SqliteQsoRepository::new(&db));
        let service = LogbookService::new(repo.clone());
        let mut ids = Vec::new();
        for (minute, call, station) in [
            (30, "W1AW", "K1ABC"),
            (31, "JA1NUT", "K1ABC"),
            (32, "DL1XYZ", "K1ABC"),
            (33, "F5XYZ", "K1ABC"),
            (34, "G4ABC", "K1ABC/P"),
        ] {
            let begin = Utc.with_ymd_and_hms(2026, 5, 8, 18, minute, 0).unwrap();
            let mut command = CreateQsoCommand::minimal(Callsign::parse(call).unwrap(), begin);
            command.station_callsign = Some(Callsign::parse(station).unwrap());
            command.band = Some(Band::M20);
            ids.push((call, service.create_qso(command).await.unwrap()));
        }
        (repo, move |call: &str| ids.iter().find(|(c, _)| *c == call).unwrap().1)
    }

    #[tokio::test]
    async fn qrz_update_syncs_then_uploads_and_records_each_qso_by_its_own_answer() {
        let (repo, id) = qrz_log().await;
        let (client, seen_by) = fake_qrz(true).await;

        let summary = qrz_update_with(repo.as_ref(), &client).await;

        assert_eq!(summary.book_callsign.as_deref(), Some("K1ABC"));
        let fetch = summary.fetch.as_ref().expect("sync ran");
        assert!(fetch.full);
        assert_eq!((fetch.records, fetch.new_confirmations, fetch.uploads_verified), (1, 1, 1));
        assert_eq!(
            (summary.uploaded, summary.duplicates, summary.rejected, summary.held_back),
            (1, 1, 1, 1)
        );
        assert_eq!(summary.upload_error, None);
        assert_eq!(
            summary.first_errors,
            ["F5XYZ: wrong station_callsign for this logbook"]
        );

        // W1AW was already at QRZ, so it was never sent.
        let seen = seen_by.lock().unwrap().clone();
        let inserts: Vec<&String> = seen.iter().filter(|b| b.contains("ACTION=INSERT")).collect();
        assert_eq!(inserts.len(), 3);
        assert!(inserts.iter().all(|b| !b.contains("W1AW") && !b.contains("G4ABC")));

        // Still pending: the rejection (to retry) and the other logbook's QSO.
        let mut pending: Vec<QsoId> = repo
            .list_pending_uploads(QRZ_SERVICE, None)
            .await
            .unwrap()
            .iter()
            .map(|q| q.id)
            .collect();
        pending.sort_by_key(|q| q.as_uuid());
        let mut expected = vec![id("F5XYZ"), id("G4ABC")];
        expected.sort_by_key(|q| q.as_uuid());
        assert_eq!(pending, expected);

        let state = |call: &'static str| {
            let repo = repo.clone();
            let qso = id(call);
            async move {
                let states = repo.list_service_states(&qso).await.unwrap();
                states.into_iter().find(|s| s.service == QRZ_SERVICE).unwrap()
            }
        };
        assert_eq!(state("JA1NUT").await.upload_state, "uploaded");
        assert_eq!(state("DL1XYZ").await.upload_state, "uploaded");
        let w1aw = state("W1AW").await;
        assert_eq!(w1aw.upload_state, "verified");
        assert_eq!(w1aw.confirmed_at.unwrap().to_rfc3339(), "2026-06-01T00:00:00+00:00");
        assert_eq!(state("F5XYZ").await.upload_state, "failed");
        assert_eq!(
            state("F5XYZ").await.last_error.as_deref(),
            Some("wrong station_callsign for this logbook")
        );

        // The next update only asks for what changed since.
        let again = qrz_update_with(repo.as_ref(), &client).await;
        assert!(!again.fetch.as_ref().unwrap().full);
        assert_eq!(again.fetch.as_ref().unwrap().new_confirmations, 0);
        let fetches: Vec<String> = seen_by
            .lock()
            .unwrap()
            .iter()
            .filter(|b| b.contains("ACTION=FETCH"))
            .cloned()
            .collect();
        assert_eq!(fetches.len(), 2);
        assert!(!fetches[0].contains("MODSINCE"));
        assert!(fetches[1].contains("OPTION=MODSINCE%3A"));
    }

    #[tokio::test]
    async fn qrz_update_stops_at_a_refused_key_without_touching_the_log() {
        let (repo, _) = qrz_log().await;
        let (client, seen) = fake_qrz(false).await;

        let summary = qrz_update_with(repo.as_ref(), &client).await;

        assert_eq!(
            summary.upload_error.as_deref(),
            Some("QRZ refused the API key: invalid api key")
        );
        assert!(summary.fetch.is_none() && summary.fetch_error.is_none());
        assert_eq!(seen.lock().unwrap().len(), 1);
        let pending = repo.list_pending_uploads(QRZ_SERVICE, None).await.unwrap();
        assert_eq!(pending.len(), 5);
    }

    #[tokio::test]
    async fn a_qrz_logbook_takes_only_its_own_callsign_and_dates() {
        let db = Database::open_in_memory().await.unwrap();
        let repo = Arc::new(SqliteQsoRepository::new(&db));
        let service = LogbookService::new(repo.clone());
        let mut qsos = Vec::new();
        for (station, year) in [
            (Some("K1ABC"), 2026),
            (Some("K1ABC/P"), 2026),
            (None, 2026),
            (Some("K1ABC"), 2019),
            (Some("K1ABC"), 2031),
        ] {
            let begin = Utc.with_ymd_and_hms(year, 5, 8, 18, 30, 0).unwrap();
            let mut c = CreateQsoCommand::minimal(Callsign::parse("W1AW").unwrap(), begin);
            c.station_callsign = station.map(|s| Callsign::parse(s).unwrap());
            let id = service.create_qso(c).await.unwrap();
            qsos.push(repo.get_qso(&id).await.unwrap().unwrap());
        }
        let book = BookStatus {
            callsign: Some("k1abc".into()),
            start_date: chrono::NaiveDate::from_ymd_opt(2020, 1, 1),
            end_date: chrono::NaiveDate::from_ymd_opt(2030, 1, 1),
            ..Default::default()
        };
        let taken: Vec<bool> = qsos.iter().map(|q| qrz_book_takes(&book, q)).collect();
        assert_eq!(taken, [true, false, false, false, false]);
        // A status that names no callsign or dates holds back only a
        // QSO with no station callsign.
        let open = BookStatus::default();
        let taken: Vec<bool> = qsos.iter().map(|q| qrz_book_takes(&open, q)).collect();
        assert_eq!(taken, [true, true, false, true, true]);
    }
}
