use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use app_config::{
    CondXConfig as CondXConfigToml, Config, DxClusterConfig, KeyerConfig as KeyerConfigToml,
    RigConfig as RigConfigToml, So2rConfig as So2rConfigToml,
};
use app_persistence::{Database, SqliteQsoRepository, SqliteStationRepository};
use chrono::{Datelike, Utc};
use condx_feed::{CondXConfig as CondXFeedConfig, spawn_condx_feed};
use keyer_control::{
    KeyerConfig as KeyerClientConfig, connect as connect_keyer,
};
use pota_feed::{PotaConfig as PotaFeedConfig, spawn_pota_feed};
use logbook_domain::{
    CreateQsoCommand, ImportReport, LogbookService, QsoRepository, QsoSearch,
    StationRepository, parse_adif, snapshot as awards_snapshot,
};
use radio_core::{QsoId, StationLocation};
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
use super::subscriptions::{CONDX_RX, KEYER_RX, POTA_RX, RIG_RX, SO2R_RX, SPOT_RX, WSJTX_RX};
use super::types::{BootBundle, RigEntry, TaggedRigSnapshot};

pub(super) async fn boot_app() -> Result<BootBundle, String> {
    let dir = data_dir()?;
    let db_path = dir.join("slogger.sqlite");
    tracing::info!(path = %db_path.display(), "opening logbook db");
    let db = Database::open(&db_path).await.map_err(|e| e.to_string())?;
    let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
    let station_repo: Arc<dyn StationRepository> = Arc::new(SqliteStationRepository::new(&db));

    let (resolver, cty_loaded) = load_resolver(&dir);
    let service = Arc::new(LogbookService::with_resolver(
        repo.clone(),
        resolver.clone(),
    ));

    let config = Config::load_default().map_err(|e| e.to_string())?;
    if let Some(name) = &config.lotw.station_location {
        tracing::warn!(
            station_location = %name,
            "[lotw] station_location is deprecated and ignored: set each station location's \
             TQSL location in the station editor instead"
        );
    }

    let backups = super::backup::Backups::from_config(
        &config.backup,
        &dir,
        db.clone(),
        service.clone(),
        station_repo.clone(),
    );
    match &backups {
        Some(b) => tracing::info!(directory = %b.directory.display(), "logbook backups on"),
        None => tracing::info!("logbook backups switched off in config"),
    }

    let station_locations = station_repo
        .list_locations()
        .await
        .map_err(|e| e.to_string())?;
    let active_location = station_locations.first().cloned();
    tracing::info!(
        location = %active_location.as_ref().map(|l| l.name.as_str()).unwrap_or("(none)"),
        "active operating location"
    );

    // Loaded here, ahead of the spot feed, because it remembers which
    // spot filter was last chosen. (std::fs is cheap enough to do inside
    // the async boot.)
    let panel_layout = load_layout();
    let spot_filter = match panel_layout.as_ref().and_then(|saved| saved.spot_filter.as_deref()) {
        Some(saved) => super::spot_filter::from_saved(saved),
        None => config.dxcluster.filter_file.clone(),
    };
    let mut filter_control = None;
    let mut spot_filter_problem = None;

    let spots_active = if let Some(mut feed_cfg) = build_spot_feed_config(&config.dxcluster) {
        feed_cfg.filter_path = spot_filter.clone();
        // The country file also serves the spot filter's continent,
        // zone and entity rules. Without one those rules have nothing
        // to go on, so none is passed and the caller is warned.
        let entities: Option<Arc<dyn spot_feed::EntityLookup>> =
            cty_loaded.then(|| Arc::new(CountryFile(resolver.clone())) as _);
        match spawn_spot_feed(&feed_cfg, entities).await {
            Ok((rx, control, problem)) => {
                let _ = SPOT_RX.set(Mutex::new(Some(rx)));
                filter_control = Some(control);
                if let Some(problem) = problem {
                    tracing::warn!(error = %problem, "spot filter unusable; no cluster spots shown");
                    spot_filter_problem = Some(problem.to_string());
                }
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

    // Off unless asked for: no request reaches hamqsl.com otherwise.
    let condx_active = match spawn_condx_feed(&build_condx_feed_config(&config.condx)) {
        Some(rx) => {
            let _ = CONDX_RX.set(Mutex::new(Some(rx)));
            tracing::info!("condx feed started");
            true
        }
        None => {
            tracing::info!("condx feed disabled in config");
            false
        }
    };

    // Hardware connects in the order rigs, keyer, SO2R switch — the order
    // clogger uses on the same libraries and hardware. slogger used to
    // connect the switch first and keyed stray CW ("? ?") at startup on a
    // real station, which clogger does not; the order was the only
    // difference between the two, so it is the suspected cause.
    // Multi-rig: iterate every configured rig, connect each, build entries.
    // A single shared mpsc channel multiplexes all rigs' snapshots; per-rig
    // forwarder tasks tag snapshots with the rig index before pushing.
    let rigs = build_rig_entries(&config.rigs).await;

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

    // (The saved layout was loaded above; `None` falls back to the
    // default split in `App::init`.)

    // Silent until switched on: no request reaches pota.app while off.
    let pota_on = panel_layout
        .as_ref()
        .and_then(|saved| saved.pota)
        .unwrap_or(config.pota.enabled);
    let (pota_rx, pota_control) = spawn_pota_feed(&PotaFeedConfig {
        enabled: pota_on,
        refresh_interval_secs: config.pota.refresh_interval_secs,
    });
    let _ = POTA_RX.set(Mutex::new(Some(pota_rx)));
    tracing::info!(on = pota_on, "pota feed ready");

    Ok(BootBundle {
        service,
        repo,
        station_repo,
        resolver,
        config,
        station_locations,
        active_location,
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
        condx_active,
        pota_on,
        pota_control,
        panel_layout,
        backups,
        cty_loaded,
        filter_control,
        spot_filter,
        spot_filter_problem,
    })
}

fn build_condx_feed_config(cfg: &CondXConfigToml) -> CondXFeedConfig {
    CondXFeedConfig {
        enabled: cfg.enabled,
        refresh_interval_secs: cfg.refresh_interval_secs,
    }
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
        sidetone: cfg.sidetone.unwrap_or(true),
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
        transceive: cfg.transceive,
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

/// slogger's country file as the spot feed wants it.
struct CountryFile(Arc<dyn Resolver>);

impl spot_feed::EntityLookup for CountryFile {
    fn entity(&self, call: &str) -> Option<spot_feed::Entity> {
        // A skimmer's "-#" or "-2" is not part of its callsign.
        let call = call.split('-').next().unwrap_or(call);
        let found = self.0.resolve(&radio_core::Callsign::parse(call).ok()?)?;
        Some(spot_feed::Entity {
            name: found.country?,
            continent: found.continent?,
            cq_zone: found.cq_zone,
            itu_zone: found.itu_zone,
            prefix: found.dxcc_prefix.unwrap_or_default(),
        })
    }
}

/// The callsign resolver, and whether it has a country file behind it.
fn load_resolver(dir: &std::path::Path) -> (Arc<dyn Resolver>, bool) {
    let cty_path = dir.join("cty.dat");
    match CtyDbResolver::from_path(&cty_path) {
        Ok(r) => {
            tracing::info!(path = %cty_path.display(), "loaded cty.dat for callsign resolution");
            (Arc::new(r), true)
        }
        Err(e) => {
            tracing::warn!(
                path = %cty_path.display(),
                error = %e,
                "no cty.dat available; QSOs will not be auto-enriched with DXCC info"
            );
            (Arc::new(NoOpResolver), false)
        }
    }
}

pub(super) async fn refresh(
    repo: Arc<dyn QsoRepository>,
    station_repo: Arc<dyn StationRepository>,
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
    let station_locations = station_repo
        .list_locations()
        .await
        .map_err(|e| e.to_string())?;
    Ok(RefreshSnapshot {
        recent,
        pending_lotw: pending_lotw.len(),
        pending_eqsl: pending_eqsl.len(),
        pending_clublog: pending_clublog.len(),
        pending_qrz: pending_qrz.len(),
        pending_hrdlog: pending_hrdlog.len(),
        awards,
        station_locations,
    })
}

pub(super) async fn create_qso(
    svc: Arc<LogbookService>,
    cmd: CreateQsoCommand,
) -> Result<QsoId, String> {
    svc.create_qso(cmd).await.map_err(|e| e.to_string())
}

/// Writes a location from the station editor: an update when `is_new`
/// is false, otherwise an insert.
pub(super) async fn save_location(
    repo: Arc<dyn StationRepository>,
    loc: StationLocation,
    is_new: bool,
) -> Result<StationLocation, String> {
    let result = if is_new {
        repo.insert_location(&loc).await
    } else {
        repo.update_location(&loc).await
    };
    result.map_err(|e| e.to_string())?;
    Ok(loc)
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
        duplicates: report.duplicates,
        locations_created: report.locations_created,
        skipped: report.skipped,
        parse_errors,
        first_errors: report.errors.into_iter().take(3).collect(),
    })
}

#[cfg(test)]
mod country_file_tests {
    use super::*;
    use spot_feed::EntityLookup;

    const MINI_CTY: &str = "United States:                      05:  08:  NA:   37.53:   97.00:     5.0:  K:
    K,W,N,AA,AB,AC,AD,AE,AF,AG,AH,AI,AJ,AK,AL;
Fed. Rep. of Germany:               14:  28:  EU:   51.00:  -10.00:    -1.0:  DL:
    DA,DB,DC,DD,DE,DF,DG,DH,DI,DJ,DK,DL,DM,DN,DO,DP,DQ,DR;
";

    #[test]
    fn the_spot_filter_sees_what_cty_dat_says() {
        let cty = CountryFile(Arc::new(CtyDbResolver::from_reader(MINI_CTY.as_bytes()).unwrap()));
        let de = cty.entity("DL1ABC").unwrap();
        assert_eq!(
            (de.name.as_str(), de.continent.as_str(), de.cq_zone, de.itu_zone, de.prefix.as_str()),
            ("Fed. Rep. of Germany", "EU", Some(14), Some(28), "DL")
        );
        // A skimmer's suffix is not part of its call.
        for skimmer in ["K3LR-#", "W3LPL-2", "K3LR"] {
            assert_eq!(cty.entity(skimmer).unwrap().continent, "NA", "{skimmer}");
        }
        assert!(cty.entity("ZZ9ZZZ").is_none());
    }
}
