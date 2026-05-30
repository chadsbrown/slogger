use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use app_config::{
    Config, DxClusterConfig, KeyerConfig as KeyerConfigToml, RigConfig as RigConfigToml,
    So2rConfig as So2rConfigToml,
};
use app_persistence::{Database, SqliteQsoRepository, SqliteStationRepository};
use chrono::{Datelike, Utc};
use keyer_control::{
    KeyerConfig as KeyerClientConfig, connect as connect_keyer,
};
use logbook_domain::{
    CreateQsoCommand, ImportReport, LogbookService, QsoRepository, QsoSearch,
    StationRepository, parse_adif, snapshot as awards_snapshot,
};
use radio_core::{OperatingSessionId, QsoId, StationLocation, StationLocationId};
use rig_control::{RigConfig as RigClientConfig, connect as connect_rig};
use so2r_control::{So2rConfig as So2rClientConfig, connect as connect_so2r};
use spot_feed::{ClusterSource as SpotClusterSource, SpotFeedConfig, spawn_spot_feed};
use station_resolver::{CtyDbResolver, NoOpResolver, Resolver};
use tokio::sync::mpsc;
use wsjtx_bridge::spawn_bridge as spawn_wsjtx;

use super::constants::{
    CLUBLOG_SERVICE, EQSL_SERVICE, HRDLOG_SERVICE, LOTW_SERVICE, QRZ_SERVICE,
};
use super::layout::load_layout;
use super::message::{ImportSummary, RefreshSnapshot};
use super::subscriptions::{KEYER_RX, RIG_RX, SO2R_RX, SPOT_RX, WSJTX_RX};
use super::types::{BootBundle, RigEntry, TaggedRigSnapshot};

pub(super) async fn boot_app() -> Result<BootBundle, String> {
    let dir = data_dir()?;
    let db_path = dir.join("slogger.sqlite");
    tracing::info!(path = %db_path.display(), "opening logbook db");
    let db = Database::open(&db_path).await.map_err(|e| e.to_string())?;
    let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
    let station_repo: Arc<dyn StationRepository> = Arc::new(SqliteStationRepository::new(&db));

    let resolver: Arc<dyn Resolver> = load_resolver(&dir);
    let service = Arc::new(LogbookService::with_resolver(
        repo.clone(),
        resolver.clone(),
    ));

    let config = Config::load_default().map_err(|e| e.to_string())?;

    // Close any orphaned sessions from prior runs before opening today's.
    // Without this, every boot leaves a session with ended_at = NULL in
    // the table; over time that's just clutter. Cheap UPDATE.
    match station_repo.close_open_sessions().await {
        Ok(n) if n > 0 => tracing::info!(closed = n, "closed orphaned sessions from prior runs"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "failed to close stale sessions"),
    }

    let station_locations = station_repo
        .list_locations()
        .await
        .map_err(|e| e.to_string())?;
    let active_location = station_locations.first().cloned();

    let active_session = station_repo
        .start_session(
            None,
            active_location.as_ref().map(|l| &l.id),
            Some("slogger session"),
        )
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        session = %active_session,
        location = %active_location.as_ref().map(|l| l.name.as_str()).unwrap_or("(none)"),
        "started operating session"
    );

    let spots_active = if let Some(feed_cfg) = build_spot_feed_config(&config.dxcluster) {
        match spawn_spot_feed(&feed_cfg).await {
            Ok(rx) => {
                let _ = SPOT_RX.set(Mutex::new(Some(rx)));
                tracing::info!(
                    sources = feed_cfg.sources.len(),
                    "dxcluster spot feed started"
                );
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "dxcluster spot feed failed to start");
                false
            }
        }
    } else {
        tracing::info!("no [dxcluster] config; spot feed disabled");
        false
    };

    let (wsjtx_active, wsjtx_bind_addr) = if config.wsjtx.enabled {
        match config.wsjtx.bind_addr.parse::<std::net::SocketAddr>() {
            Ok(addr) => match spawn_wsjtx(addr).await {
                Ok((bound, rx)) => {
                    let _ = WSJTX_RX.set(Mutex::new(Some(rx)));
                    tracing::info!(addr = %bound, "wsjtx bridge listening");
                    (true, Some(bound.to_string()))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "wsjtx bridge failed to bind");
                    (false, None)
                }
            },
            Err(e) => {
                tracing::warn!(addr = %config.wsjtx.bind_addr, error = %e, "wsjtx bind_addr unparseable");
                (false, None)
            }
        }
    } else {
        tracing::info!("wsjtx bridge disabled in config");
        (false, None)
    };

    let (so2r_active, so2r_status, so2r_handle) = if config.so2r.is_configured() {
        match build_so2r_client_config(&config.so2r) {
            Some(sc) => match connect_so2r(&sc).await {
                Ok((rx, handle)) => {
                    let _ = SO2R_RX.set(Mutex::new(Some(rx)));
                    let label = format!("OTRSP on {}", sc.serial_port);
                    tracing::info!(so2r = %label, "so2r switch connected");
                    (
                        true,
                        Some(format!("SO2R: connected to {label}")),
                        Some(handle),
                    )
                }
                Err(e) => {
                    tracing::warn!(error = %e, "so2r connection failed");
                    (false, Some(format!("SO2R: connect failed — {e}")), None)
                }
            },
            None => (false, Some("SO2R: config incomplete".into()), None),
        }
    } else {
        (false, None, None)
    };

    let (keyer_active, keyer_status, keyer_handle) = if config.keyer.is_configured() {
        match build_keyer_client_config(&config.keyer) {
            Some(kc) => match connect_keyer(&kc).await {
                Ok((rx, handle)) => {
                    let _ = KEYER_RX.set(Mutex::new(Some(rx)));
                    let label = format!("WinKeyer on {}", kc.serial_port);
                    tracing::info!(keyer = %label, "keyer connected");
                    (true, Some(format!("Keyer: connected to {label}")), Some(handle))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "keyer connection failed");
                    (false, Some(format!("Keyer: connect failed — {e}")), None)
                }
            },
            None => (false, Some("Keyer: config incomplete".into()), None),
        }
    } else {
        (false, None, None)
    };

    // Multi-rig: iterate every configured rig, connect each, build entries.
    // A single shared mpsc channel multiplexes all rigs' snapshots; per-rig
    // forwarder tasks tag snapshots with the rig index before pushing.
    let rigs = build_rig_entries(&config.rigs).await;

    // Best-effort layout restoration: None falls back to the default
    // split in `App::init`. We do this synchronously inside the async
    // boot since std::fs::read_to_string is cheap and avoids a separate
    // await point in update().
    let pane_layout = load_layout();

    Ok(BootBundle {
        service,
        repo,
        station_repo,
        resolver,
        config,
        station_locations,
        active_location,
        active_session,
        spots_active,
        wsjtx_active,
        wsjtx_bind_addr,
        rigs,
        keyer_active,
        keyer_status,
        keyer_handle,
        so2r_active,
        so2r_status,
        so2r_handle,
        pane_layout,
    })
}

fn build_so2r_client_config(cfg: &So2rConfigToml) -> Option<So2rClientConfig> {
    Some(So2rClientConfig {
        serial_port: cfg.serial_port.clone()?,
        initial_tx: cfg.initial_tx,
        initial_rx_mode: cfg.initial_rx_mode.clone(),
    })
}

async fn build_rig_entries(cfgs: &[RigConfigToml]) -> Vec<RigEntry> {
    if cfgs.is_empty() {
        return Vec::new();
    }
    // Set up the shared snapshot channel exactly once.
    let (unified_tx, unified_rx) = mpsc::channel::<TaggedRigSnapshot>(64);
    let _ = RIG_RX.set(Mutex::new(Some(unified_rx)));

    let mut out = Vec::with_capacity(cfgs.len());
    for (idx, cfg) in cfgs.iter().enumerate() {
        let label = derive_rig_label(cfg, idx);
        if !cfg.is_configured() {
            out.push(RigEntry {
                label: label.clone(),
                config: cfg.clone(),
                handle: None,
                snapshot: None,
                status: "disabled or config incomplete".into(),
            });
            continue;
        }
        let Some(rc) = build_rig_client_config(cfg) else {
            out.push(RigEntry {
                label: label.clone(),
                config: cfg.clone(),
                handle: None,
                snapshot: None,
                status: "config incomplete".into(),
            });
            continue;
        };
        match connect_rig(&rc).await {
            Ok((mut rx, handle)) => {
                let label_for_log = label.clone();
                let port = rc.serial_port.clone();
                tracing::info!(rig = %label, %port, "rig connected");
                let tx = unified_tx.clone();
                tokio::spawn(async move {
                    while let Some(snap) = rx.recv().await {
                        if tx
                            .send(TaggedRigSnapshot {
                                rig_index: idx,
                                snapshot: snap,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    tracing::info!(rig = %label_for_log, "rig snapshot stream ended");
                });
                out.push(RigEntry {
                    label,
                    config: cfg.clone(),
                    handle: Some(handle),
                    snapshot: None,
                    status: "connected".into(),
                });
            }
            Err(e) => {
                tracing::warn!(rig = %label, error = %e, "rig connection failed");
                out.push(RigEntry {
                    label,
                    config: cfg.clone(),
                    handle: None,
                    snapshot: None,
                    status: format!("connect failed: {e}"),
                });
            }
        }
    }
    out
}

fn derive_rig_label(cfg: &RigConfigToml, idx: usize) -> String {
    cfg.label.clone().unwrap_or_else(|| {
        cfg.model
            .clone()
            .unwrap_or_else(|| format!("Rig {}", idx + 1))
    })
}

fn build_keyer_client_config(cfg: &KeyerConfigToml) -> Option<KeyerClientConfig> {
    Some(KeyerClientConfig {
        serial_port: cfg.serial_port.clone()?,
        initial_wpm: cfg.initial_wpm.unwrap_or(25),
    })
}

fn build_rig_client_config(cfg: &RigConfigToml) -> Option<RigClientConfig> {
    let vendor = cfg.vendor.clone()?;
    Some(RigClientConfig {
        // For flex, serial_port is unused but RigClientConfig still
        // wants a String — pass an empty placeholder so the dispatch
        // doesn't have to special-case missing-required-field.
        serial_port: cfg.serial_port.clone().unwrap_or_default(),
        vendor,
        model: cfg.model.clone()?,
        baud_rate: cfg.baud_rate,
        host: cfg.host.clone(),
    })
}

fn build_spot_feed_config(cfg: &DxClusterConfig) -> Option<SpotFeedConfig> {
    if !cfg.is_configured() {
        return None;
    }
    Some(SpotFeedConfig {
        my_callsign: cfg.my_callsign.clone()?,
        sources: cfg
            .sources
            .iter()
            .map(|s| SpotClusterSource {
                host: s.host.clone(),
                port: s.port,
            })
            .collect(),
        filter_path: cfg.filter_file.clone(),
    })
}

fn data_dir() -> Result<PathBuf, String> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| "no data_local_dir on this platform".to_string())?
        .join("slogger");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn load_resolver(dir: &std::path::Path) -> Arc<dyn Resolver> {
    let cty_path = dir.join("cty.dat");
    match CtyDbResolver::from_path(&cty_path) {
        Ok(r) => {
            tracing::info!(path = %cty_path.display(), "loaded cty.dat for callsign resolution");
            Arc::new(r)
        }
        Err(e) => {
            tracing::warn!(
                path = %cty_path.display(),
                error = %e,
                "no cty.dat available; QSOs will not be auto-enriched with DXCC info"
            );
            Arc::new(NoOpResolver)
        }
    }
}

pub(super) async fn refresh(
    repo: Arc<dyn QsoRepository>,
    svc: Arc<LogbookService>,
) -> Result<RefreshSnapshot, String> {
    let recent = svc
        .search_qsos(QsoSearch {
            limit: Some(100),
            ..Default::default()
        })
        .await
        .map_err(|e| e.to_string())?;
    let pending_lotw = repo
        .list_pending_uploads(LOTW_SERVICE, None)
        .await
        .map_err(|e| e.to_string())?;
    let pending_eqsl = repo
        .list_pending_uploads(EQSL_SERVICE, None)
        .await
        .map_err(|e| e.to_string())?;
    let pending_clublog = repo
        .list_pending_uploads(CLUBLOG_SERVICE, None)
        .await
        .map_err(|e| e.to_string())?;
    let pending_qrz = repo
        .list_pending_uploads(QRZ_SERVICE, None)
        .await
        .map_err(|e| e.to_string())?;
    let pending_hrdlog = repo
        .list_pending_uploads(HRDLOG_SERVICE, None)
        .await
        .map_err(|e| e.to_string())?;
    let award_qsos = repo
        .list_award_qsos()
        .await
        .map_err(|e| e.to_string())?;
    let year = Utc::now().year();
    let awards = awards_snapshot(&award_qsos, year);
    Ok(RefreshSnapshot {
        recent,
        pending_lotw: pending_lotw.len(),
        pending_eqsl: pending_eqsl.len(),
        pending_clublog: pending_clublog.len(),
        pending_qrz: pending_qrz.len(),
        pending_hrdlog: pending_hrdlog.len(),
        awards,
    })
}

pub(super) async fn create_qso(
    svc: Arc<LogbookService>,
    cmd: CreateQsoCommand,
) -> Result<QsoId, String> {
    svc.create_qso(cmd).await.map_err(|e| e.to_string())
}

pub(super) async fn insert_location(
    repo: Arc<dyn StationRepository>,
    loc: StationLocation,
) -> Result<StationLocation, String> {
    repo.insert_location(&loc).await.map_err(|e| e.to_string())?;
    Ok(loc)
}

pub(super) async fn retarget_session(
    repo: Arc<dyn StationRepository>,
    session: OperatingSessionId,
    location: StationLocationId,
) -> Result<(), String> {
    repo.set_session_station_location(&session, Some(&location))
        .await
        .map_err(|e| e.to_string())
}

pub(super) async fn import_adif_file(
    svc: Arc<LogbookService>,
    path: String,
) -> Result<ImportSummary, String> {
    let contents = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("read {path}: {e}"))?;
    let outcome = parse_adif(&contents).map_err(|e| e.to_string())?;
    let parse_errors = outcome.skipped.len();
    let report: ImportReport = svc.import_qsos(outcome.commands).await;
    Ok(ImportSummary {
        created: report.created,
        skipped: report.skipped,
        parse_errors,
        first_errors: report.errors.into_iter().take(3).collect(),
    })
}
