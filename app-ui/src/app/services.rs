use std::sync::Arc;

use app_config::{ClubLogConfig, Config, EqslConfig, HrdlogConfig, LotwConfig, QrzConfig};
use chrono::Utc;
use clublog_sync::{ClubLogConfig as ClubLogClientConfig, ClubLogUploadClient};
use eqsl_sync::{EqslFetchClient, EqslFetchConfig, EqslUploadClient, EqslUploadConfig};
use hrdlog_sync::{HrdlogConfig as HrdlogClientConfig, HrdlogUploadClient};
use logbook_domain::{ExportOptions, QsoRepository, export_adif};
use lotw_sync::{LotwFetchClient, LotwFetchConfig, LotwUploadClient, LotwUploadConfig};
use qrz_sync::{QrzConfig as QrzClientConfig, QrzUploadClient};

use super::constants::{
    CLUBLOG_SERVICE, EQSL_SERVICE, HRDLOG_SERVICE, LOTW_SERVICE, QRZ_SERVICE,
};
use super::message::{
    ClubLogUpdateSummary, EqslUpdateSummary, FetchSummary, HrdlogUpdateSummary,
    MultiUpdateSummary, QrzUpdateSummary, UpdateSummary, UploadSummary,
};

/// One user gesture spans all configured services. Each service's
/// upload + fetch are run sequentially; failures in one phase don't stop
/// the next phase or the next service.
pub(super) async fn run_services_update(
    repo: Arc<dyn QsoRepository>,
    cfg: Config,
) -> MultiUpdateSummary {
    let lotw = if cfg.lotw.is_configured_for_upload() || cfg.lotw.is_configured_for_fetch() {
        Some(run_lotw_update(repo.clone(), cfg.lotw).await)
    } else {
        None
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

async fn run_lotw_upload(
    repo: Arc<dyn QsoRepository>,
    cfg: LotwConfig,
) -> Result<UploadSummary, String> {
    let pending = repo
        .list_pending_uploads(LOTW_SERVICE, Some(500))
        .await
        .map_err(|e| e.to_string())?;
    if pending.is_empty() {
        return Ok(UploadSummary {
            uploaded: 0,
            note: "no pending QSOs".into(),
        });
    }

    let adif = export_adif(&pending, &ExportOptions::default());

    let mut upload_cfg = LotwUploadConfig::new(
        cfg.station_location
            .clone()
            .ok_or_else(|| "missing [lotw].station_location".to_string())?,
    );
    if let Some(p) = cfg.tqsl_path.clone() {
        upload_cfg = upload_cfg.with_tqsl_path(p);
    }
    let client = LotwUploadClient::new(upload_cfg);
    let outcome = client.upload_adif(&adif).await.map_err(|e| e.to_string())?;

    let now = Utc::now();
    let mut marked = 0usize;
    for qso in &pending {
        if let Err(e) = repo
            .mark_uploaded(&qso.id, LOTW_SERVICE, now, None)
            .await
        {
            tracing::warn!(qso_id = %qso.id, error = %e, "failed to mark qso uploaded");
        } else {
            marked += 1;
        }
    }
    let note = if outcome.accepted {
        "accepted by LotW"
    } else {
        "uploaded; LotW response unclear — verify in LotW account"
    };
    Ok(UploadSummary {
        uploaded: marked,
        note: note.into(),
    })
}

/// One round-trip to LotW that updates both `upload_state` (to verified)
/// and `confirmation_state` (where qsl_rcvd=Y). LotW returns every QSO in
/// our account; presence alone proves the upload landed, and the
/// `QSL_RCVD` per-record flag tells us whether the other station also has
/// matching QSOs.
async fn run_lotw_update(repo: Arc<dyn QsoRepository>, cfg: LotwConfig) -> UpdateSummary {
    let mut summary = UpdateSummary::default();

    if cfg.is_configured_for_upload() {
        match run_lotw_upload(repo.clone(), cfg.clone()).await {
            Ok(u) => summary.upload = Some(u),
            Err(e) => summary.upload_error = Some(e),
        }
    } else {
        summary.upload_skipped_reason = Some("no [lotw].station_location".into());
    }

    if cfg.is_configured_for_fetch() {
        match run_lotw_sync(repo, cfg).await {
            Ok(f) => summary.fetch = Some(f),
            Err(e) => summary.fetch_error = Some(e),
        }
    } else {
        summary.fetch_skipped_reason = Some("no [lotw].username/password".into());
    }

    summary
}

async fn run_lotw_sync(
    repo: Arc<dyn QsoRepository>,
    cfg: LotwConfig,
) -> Result<FetchSummary, String> {
    let username = cfg
        .username
        .clone()
        .ok_or_else(|| "missing [lotw].username".to_string())?;
    let password = cfg
        .password
        .clone()
        .ok_or_else(|| "missing [lotw].password".to_string())?;
    let client = LotwFetchClient::new(LotwFetchConfig::new(username, password));
    // only_confirmed=false → return all QSOs at LotW so we can verify uploads.
    let records = client
        .fetch(None, false)
        .await
        .map_err(|e| e.to_string())?;

    let fetched = records.len();
    let now = Utc::now();
    let mut verified = 0usize;
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
                if let Err(e) = repo
                    .mark_upload_verified(&id, LOTW_SERVICE, now, None)
                    .await
                {
                    tracing::warn!(qso_id = %id, error = %e, "failed to mark verified");
                    unmatched += 1;
                    continue;
                }
                verified += 1;
                if rec.qsl_rcvd {
                    let confirmed_at = rec.qsl_rdate.unwrap_or(now);
                    if let Err(e) = repo
                        .mark_confirmed(&id, LOTW_SERVICE, confirmed_at, None)
                        .await
                    {
                        tracing::warn!(qso_id = %id, error = %e, "failed to mark confirmed");
                    } else {
                        confirmed += 1;
                    }
                }
            }
            None => unmatched += 1,
        }
    }
    Ok(FetchSummary {
        fetched,
        verified,
        confirmed,
        unmatched,
    })
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
        parts.push(format!("upload {} qso ({})", u.uploaded, u.note));
    } else if let Some(e) = &s.upload_error {
        parts.push(format!("upload error: {e}"));
    } else if let Some(r) = &s.upload_skipped_reason {
        parts.push(format!("upload skip ({r})"));
    }
    if let Some(f) = &s.fetch {
        parts.push(format!(
            "sync {}/{} verified, {} confirmed, {} unmatched",
            f.verified, f.fetched, f.confirmed, f.unmatched
        ));
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
