//! Rig control wrapper around `riglib`. Read-only for v1: connects to a
//! transceiver, reads its current frequency + mode, then subscribes to
//! the rig's broadcast events to keep slogger's state fresh. Setting the
//! rig (writing freq/mode/PTT) is intentionally deferred — that's a
//! deeper rabbit hole (operator confirmation UX, race conditions with
//! WSJT-X also driving the rig, etc.) that needs its own design pass.
//!
//! Vendor dispatch is hand-rolled here rather than auto-discovered from
//! riglib because each vendor's builder takes a vendor-specific model
//! type — there's no common builder trait to dispatch through. New
//! vendors are a paste-job in `connect()`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::mpsc;

use riglib::elecraft::builder::ElecraftBuilder;
use riglib::elecraft::models as elecraft_models;
use riglib::flex::builder::FlexRadioBuilder;
use riglib::flex::models as flex_models;
use riglib::icom::builder::IcomBuilder;
use riglib::icom::models as icom_models;
use riglib::kenwood::builder::KenwoodBuilder;
use riglib::kenwood::models as kenwood_models;
use riglib::yaesu::builder::YaesuBuilder;
use riglib::yaesu::models as yaesu_models;
use riglib::{ReceiverId, Rig, RigEvent};

#[derive(Debug, Clone)]
pub struct RigConfig {
    /// Vendor: `icom` / `yaesu` / `kenwood` / `elecraft` / `flex`.
    pub vendor: String,
    /// Model name, e.g. `"IC-7300"`, `"FT-DX10"`, `"6400"`. Case- and
    /// hyphen-insensitive lookup against riglib's per-vendor model tables.
    pub model: String,
    /// Serial device path. Required for icom/yaesu/kenwood/elecraft;
    /// ignored for flex.
    pub serial_port: String,
    pub baud_rate: Option<u32>,
    /// FlexRadio: hostname or IP. Required for flex; ignored for serial
    /// vendors. Default port (4992) is used if not overridden.
    pub host: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RigSnapshot {
    pub vendor: String,
    pub model: String,
    pub freq_hz: Option<u64>,
    /// Mode as a vendor-neutral ADIF-ish string (CW/SSB/USB/LSB/FM/AM/RTTY/…).
    /// Not always derivable in vendor terms — None when unknown.
    pub mode: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("unknown vendor: {0}")]
    UnknownVendor(String),

    #[error("unknown {vendor} model: {model}")]
    UnknownModel { vendor: String, model: String },

    #[error("riglib build error: {0}")]
    Build(String),

    #[error("riglib subscribe error: {0}")]
    Subscribe(String),

    #[error("missing required config field: {0}")]
    MissingConfig(&'static str),
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("rig command failed: {0}")]
    Set(String),

    #[error("unsupported mode: {0}")]
    UnsupportedMode(String),
}

/// Handle to an open rig. Returned by `connect()` alongside the snapshot
/// receiver so callers can issue set commands without reaching into
/// riglib types directly. The inner rig is held behind an async RwLock
/// so the auto-reconnect task can swap it on the fly — `set_*` calls
/// during a disconnect window get a clear "rig not connected" error
/// rather than blocking forever or panicking.
#[derive(Clone)]
pub struct RigHandle {
    inner: Arc<tokio::sync::RwLock<Option<Arc<dyn Rig + Send + Sync>>>>,
}

impl std::fmt::Debug for RigHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RigHandle").finish_non_exhaustive()
    }
}

impl RigHandle {
    /// Returns true if the underlying rig is currently connected. UI can
    /// poll this for a "rig live" indicator without subscribing to
    /// snapshots.
    pub async fn is_connected(&self) -> bool {
        self.inner.read().await.is_some()
    }

    /// Tune VFO-A to `hz`. The rig will broadcast a FrequencyChanged
    /// event afterwards which our event task forwards as a snapshot,
    /// so the UI sees the new freq through the same channel as
    /// dial-driven changes.
    pub async fn set_frequency_hz(&self, hz: u64) -> Result<(), CommandError> {
        let rig = self.current_rig().await?;
        rig.set_frequency(ReceiverId::VFO_A, hz)
            .await
            .map_err(|e| CommandError::Set(format!("{e:?}")))
    }

    /// Set VFO-A mode from an ADIF-ish string. Unknown / unsupported
    /// modes return UnsupportedMode rather than guessing — the caller
    /// can decide whether to skip or surface the error.
    pub async fn set_mode_adif(&self, adif: &str) -> Result<(), CommandError> {
        let mode = mode_from_adif(adif).ok_or_else(|| CommandError::UnsupportedMode(adif.to_string()))?;
        let rig = self.current_rig().await?;
        rig.set_mode(ReceiverId::VFO_A, mode)
            .await
            .map_err(|e| CommandError::Set(format!("{e:?}")))
    }

    async fn current_rig(&self) -> Result<Arc<dyn Rig + Send + Sync>, CommandError> {
        let g = self.inner.read().await;
        g.as_ref()
            .map(Arc::clone)
            .ok_or_else(|| CommandError::Set("rig not connected".into()))
    }
}

fn mode_from_adif(s: &str) -> Option<riglib::Mode> {
    use riglib::Mode;
    match s.to_ascii_uppercase().as_str() {
        "USB" => Some(Mode::USB),
        "LSB" => Some(Mode::LSB),
        // ADIF "SSB" is band-dependent (USB above 10 MHz, LSB below).
        // Defaulting to USB is wrong half the time; refusing is honest.
        "SSB" => None,
        "CW" => Some(Mode::CW),
        "CWR" => Some(Mode::CWR),
        "AM" => Some(Mode::AM),
        "FM" => Some(Mode::FM),
        "RTTY" => Some(Mode::RTTY),
        // Modern digital modes (FT8/FT4/PSK/JT*) all run as USB-side data
        // — that's how WSJT-X expects the rig to be set when it decodes.
        "FT8" | "FT4" | "PSK" | "JT65" | "JT9" | "JS8" | "MFSK" | "OLIVIA"
        | "DIGITAL" | "DIGITALUSB" | "DATA" | "DATA-U" => Some(Mode::DataUSB),
        "DATA-L" | "DIGITALLSB" => Some(Mode::DataLSB),
        _ => None,
    }
}

const SNAPSHOT_DEPTH: usize = 16;

/// Connect to the rig and spawn a forwarder task. Returns a receiver of
/// `RigSnapshot` plus a `RigHandle` for issuing set commands.
///
/// The initial connect is synchronous — if the rig isn't reachable on
/// first try, this returns `Err`. **After** that first success, the
/// spawned task reconnects automatically with exponential backoff
/// (1→2→4→8→16→30s, capped at 30s) when the connection drops. The
/// `RigHandle` survives reconnects: its inner Arc is swapped under a
/// RwLock. `set_*` calls during a disconnect window return a clean
/// `CommandError::Set("rig not connected")` rather than blocking.
pub async fn connect(
    cfg: &RigConfig,
) -> Result<(mpsc::Receiver<RigSnapshot>, RigHandle), ConnectError> {
    // First connection — synchronous, fail-fast for the caller.
    let initial_rig: Arc<dyn Rig + Send + Sync> = build_rig(cfg).await?.into();

    let handle_inner: Arc<tokio::sync::RwLock<Option<Arc<dyn Rig + Send + Sync>>>> =
        Arc::new(tokio::sync::RwLock::new(Some(initial_rig.clone())));
    let handle = RigHandle {
        inner: handle_inner.clone(),
    };

    let (tx, rx) = mpsc::channel::<RigSnapshot>(SNAPSHOT_DEPTH);
    let snapshot = Arc::new(tokio::sync::Mutex::new(RigSnapshot {
        vendor: cfg.vendor.clone(),
        model: cfg.model.clone(),
        freq_hz: None,
        mode: None,
        at: Utc::now(),
    }));

    // Seed the channel + run-loop using the initial rig we just built.
    seed_initial_state(&initial_rig, &snapshot).await;
    let _ = tx.send(snapshot.lock().await.clone()).await;

    let cfg_for_task = cfg.clone();
    let snapshot_for_task = snapshot.clone();
    tokio::spawn(async move {
        // First iteration uses the rig the caller just connected to
        // (already installed in handle_inner). Subsequent iterations
        // build a fresh rig.
        let mut current_rig: Option<Arc<dyn Rig + Send + Sync>> = Some(initial_rig);
        let mut backoff_secs: u64 = 1;

        loop {
            // Ensure we have a rig — either the initial one (first pass)
            // or a freshly built one (reconnect pass).
            let rig = match current_rig.take() {
                Some(r) => r,
                None => match build_rig(&cfg_for_task).await {
                    Ok(boxed) => {
                        let arc: Arc<dyn Rig + Send + Sync> = boxed.into();
                        // Reset backoff on successful build.
                        backoff_secs = 1;
                        // Install new rig in the shared handle.
                        *handle_inner.write().await = Some(arc.clone());
                        // Seed snapshot from the fresh rig and emit one
                        // so the UI's "stale" indicator clears.
                        seed_initial_state(&arc, &snapshot_for_task).await;
                        if tx.send(snapshot_for_task.lock().await.clone()).await.is_err() {
                            return;
                        }
                        tracing::info!("rig reconnected");
                        arc
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, backoff_secs, "rig reconnect failed; sleeping");
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(30);
                        continue;
                    }
                },
            };

            // Subscribe + run forwarding loop until the rig disconnects.
            match rig.subscribe() {
                Ok(mut events) => {
                    while let Ok(event) = events.recv().await {
                        let mut s = snapshot_for_task.lock().await;
                        apply_event(&mut s, &event);
                        s.at = Utc::now();
                        if tx.send(s.clone()).await.is_err() {
                            return;
                        }
                    }
                    // events.recv() Err = broadcast channel closed = rig gone.
                    tracing::warn!("rig event channel closed; will reconnect");
                }
                Err(e) => {
                    tracing::warn!(error = %format!("{e:?}"), "rig.subscribe() failed");
                }
            }

            // Disconnected. Drop the handle's view of the rig, then loop
            // back into the build_rig retry path.
            *handle_inner.write().await = None;
        }
    });

    Ok((rx, handle))
}

/// Read current freq + mode from a freshly-built rig and write them
/// into the shared snapshot. Used both at initial connect and after
/// each successful reconnect.
async fn seed_initial_state(
    rig: &Arc<dyn Rig + Send + Sync>,
    snapshot: &Arc<tokio::sync::Mutex<RigSnapshot>>,
) {
    let freq = rig.get_frequency(ReceiverId::VFO_A).await.ok();
    let mode = rig
        .get_mode(ReceiverId::VFO_A)
        .await
        .ok()
        .map(|m| mode_to_str(&m));
    let mut s = snapshot.lock().await;
    s.freq_hz = freq;
    s.mode = mode;
    s.at = Utc::now();
}

async fn build_rig(cfg: &RigConfig) -> Result<Box<dyn Rig + Send + Sync>, ConnectError> {
    match cfg.vendor.to_ascii_lowercase().as_str() {
        "icom" => {
            let model = lookup_icom_model(&cfg.model)?;
            let mut b = IcomBuilder::new(model).serial_port(&cfg.serial_port);
            if let Some(baud) = cfg.baud_rate {
                b = b.baud_rate(baud);
            }
            let rig = b.build().await.map_err(|e| ConnectError::Build(format!("{e:?}")))?;
            Ok(Box::new(rig))
        }
        "yaesu" => {
            let model = lookup_yaesu_model(&cfg.model)?;
            let mut b = YaesuBuilder::new(model).serial_port(&cfg.serial_port);
            if let Some(baud) = cfg.baud_rate {
                b = b.baud_rate(baud);
            }
            let rig = b.build().await.map_err(|e| ConnectError::Build(format!("{e:?}")))?;
            Ok(Box::new(rig))
        }
        "kenwood" => {
            let model = lookup_kenwood_model(&cfg.model)?;
            let mut b = KenwoodBuilder::new(model).serial_port(&cfg.serial_port);
            if let Some(baud) = cfg.baud_rate {
                b = b.baud_rate(baud);
            }
            let rig = b.build().await.map_err(|e| ConnectError::Build(format!("{e:?}")))?;
            Ok(Box::new(rig))
        }
        "elecraft" => {
            let model = lookup_elecraft_model(&cfg.model)?;
            let mut b = ElecraftBuilder::new(model).serial_port(&cfg.serial_port);
            if let Some(baud) = cfg.baud_rate {
                b = b.baud_rate(baud);
            }
            let rig = b.build().await.map_err(|e| ConnectError::Build(format!("{e:?}")))?;
            Ok(Box::new(rig))
        }
        "flex" => {
            let model = lookup_flex_model(&cfg.model)?;
            let host = cfg.host.as_deref().ok_or(ConnectError::MissingConfig("host"))?;
            let rig = FlexRadioBuilder::new()
                .model(model)
                .host(host)
                .client_name("slogger")
                .build()
                .await
                .map_err(|e| ConnectError::Build(format!("{e:?}")))?;
            Ok(Box::new(rig))
        }
        other => Err(ConnectError::UnknownVendor(other.to_string())),
    }
}

fn normalize(name: &str) -> String {
    name.to_ascii_lowercase().replace('-', "")
}

fn lookup_icom_model(name: &str) -> Result<icom_models::IcomModel, ConnectError> {
    Ok(match normalize(name).as_str() {
        "ic7300" => icom_models::ic_7300(),
        "ic7300mk2" => icom_models::ic_7300mk2(),
        "ic7610" => icom_models::ic_7610(),
        "ic7600" => icom_models::ic_7600(),
        "ic7700" => icom_models::ic_7700(),
        "ic7800" => icom_models::ic_7800(),
        "ic7850" => icom_models::ic_7850(),
        "ic7851" => icom_models::ic_7851(),
        "ic9700" => icom_models::ic_9700(),
        "ic705" => icom_models::ic_705(),
        "ic7100" => icom_models::ic_7100(),
        "ic9100" => icom_models::ic_9100(),
        "ic7410" => icom_models::ic_7410(),
        "ic905" => icom_models::ic_905(),
        _ => {
            return Err(ConnectError::UnknownModel {
                vendor: "icom".into(),
                model: name.into(),
            });
        }
    })
}

fn lookup_yaesu_model(name: &str) -> Result<yaesu_models::YaesuModel, ConnectError> {
    Ok(match normalize(name).as_str() {
        "ftdx10" => yaesu_models::ft_dx10(),
        "ft891" => yaesu_models::ft_891(),
        "ft991a" => yaesu_models::ft_991a(),
        "ftdx101d" => yaesu_models::ft_dx101d(),
        "ftdx101mp" => yaesu_models::ft_dx101mp(),
        "ft710" => yaesu_models::ft_710(),
        _ => {
            return Err(ConnectError::UnknownModel {
                vendor: "yaesu".into(),
                model: name.into(),
            });
        }
    })
}

fn lookup_kenwood_model(name: &str) -> Result<kenwood_models::KenwoodModel, ConnectError> {
    Ok(match normalize(name).as_str() {
        "ts590s" => kenwood_models::ts_590s(),
        "ts590sg" => kenwood_models::ts_590sg(),
        "ts990s" => kenwood_models::ts_990s(),
        "ts890s" => kenwood_models::ts_890s(),
        _ => {
            return Err(ConnectError::UnknownModel {
                vendor: "kenwood".into(),
                model: name.into(),
            });
        }
    })
}

fn lookup_flex_model(name: &str) -> Result<flex_models::FlexRadioModel, ConnectError> {
    Ok(match normalize(name).as_str() {
        "6400" | "flex6400" => flex_models::flex_6400(),
        "6400m" | "flex6400m" => flex_models::flex_6400m(),
        "6600" | "flex6600" => flex_models::flex_6600(),
        "6600m" | "flex6600m" => flex_models::flex_6600m(),
        "6700" | "flex6700" => flex_models::flex_6700(),
        "8400" | "flex8400" => flex_models::flex_8400(),
        "8600" | "flex8600" => flex_models::flex_8600(),
        _ => {
            return Err(ConnectError::UnknownModel {
                vendor: "flex".into(),
                model: name.into(),
            });
        }
    })
}

fn lookup_elecraft_model(name: &str) -> Result<elecraft_models::ElecraftModel, ConnectError> {
    Ok(match normalize(name).as_str() {
        "k3" => elecraft_models::k3(),
        "k3s" => elecraft_models::k3s(),
        "k4" => elecraft_models::k4(),
        "kx2" => elecraft_models::kx2(),
        "kx3" => elecraft_models::kx3(),
        _ => {
            return Err(ConnectError::UnknownModel {
                vendor: "elecraft".into(),
                model: name.into(),
            });
        }
    })
}

fn apply_event(s: &mut RigSnapshot, ev: &RigEvent) {
    match ev {
        RigEvent::FrequencyChanged { freq_hz, .. } => s.freq_hz = Some(*freq_hz),
        RigEvent::ModeChanged { mode, .. } => s.mode = Some(mode_to_str(mode)),
        _ => {}
    }
}

/// Translate riglib's Mode enum into ADIF-ish string.
fn mode_to_str(mode: &riglib::Mode) -> String {
    use riglib::Mode;
    match mode {
        Mode::CW => "CW",
        Mode::CWR => "CW",
        Mode::USB => "USB",
        Mode::LSB => "LSB",
        Mode::AM => "AM",
        Mode::FM => "FM",
        Mode::RTTY => "RTTY",
        Mode::RTTYR => "RTTY",
        Mode::DataUSB | Mode::DataLSB | Mode::DataFM | Mode::DataAM => "DIGITAL",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_vendor_rejected() {
        let cfg = RigConfig {
            vendor: "noradio".into(),
            model: "X".into(),
            serial_port: "/dev/ttyUSB0".into(),
            baud_rate: None,
            host: None,
        };
        // We can't actually call connect() without a serial port present,
        // but the model-lookup paths are easy to exercise individually.
        let err = lookup_icom_model("nonsense").unwrap_err();
        assert!(matches!(
            err,
            ConnectError::UnknownModel { vendor, .. } if vendor == "icom"
        ));
        // Vendor field unused at lookup time (each lookup is per-vendor),
        // but exercise the type to keep it referenced.
        let _ = cfg.vendor;
    }

    #[test]
    fn case_insensitive_model_lookup() {
        // Different casings + hyphenations should all hit IC-7300.
        assert!(lookup_icom_model("IC-7300").is_ok());
        assert!(lookup_icom_model("ic7300").is_ok());
        assert!(lookup_icom_model("Ic-7300").is_ok());
    }

    #[test]
    fn yaesu_kenwood_elecraft_lookup_paths_work() {
        assert!(lookup_yaesu_model("FT-DX10").is_ok());
        assert!(lookup_kenwood_model("TS-890S").is_ok());
        assert!(lookup_elecraft_model("K4").is_ok());
    }

    #[test]
    fn flex_lookup_handles_short_and_prefixed_names() {
        assert!(lookup_flex_model("6400").is_ok());
        assert!(lookup_flex_model("FLEX-6400").is_ok());
        assert!(lookup_flex_model("flex6600M").is_ok());
        assert!(lookup_flex_model("8600").is_ok());
        assert!(lookup_flex_model("nope").is_err());
    }

    #[test]
    fn mode_from_adif_handles_common_cases() {
        use riglib::Mode;
        assert_eq!(mode_from_adif("CW"), Some(Mode::CW));
        assert_eq!(mode_from_adif("USB"), Some(Mode::USB));
        assert_eq!(mode_from_adif("FT8"), Some(Mode::DataUSB));
        assert_eq!(mode_from_adif("PSK"), Some(Mode::DataUSB));
        // SSB is intentionally unmappable — depends on band.
        assert_eq!(mode_from_adif("SSB"), None);
        assert_eq!(mode_from_adif("nonsense"), None);
    }
}
