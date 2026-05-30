//! SO2R switch wrapper around `otrsp`. Mirrors keyer-control / rig-control:
//! initial-connect synchronous, auto-reconnect with exponential backoff
//! after first success; `So2rHandle` survives reconnects via shared
//! RwLock; commands during a disconnect window return a clean
//! "switch not connected" error.
//!
//! In an SO2R operating workflow the OTRSP switch is the hardware
//! bridge between two rigs: it routes PTT, key, mic, and headphone
//! audio between Radio 1 and Radio 2. Slogger holds the handle but
//! doesn't auto-couple it to the multi-rig `active_rig` index — the
//! coupling policy is an operator-facing UX decision that the future
//! UI redesign owns.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::mpsc;

use otrsp::{OtrspBuilder, Radio, RxMode, So2rSwitch, SwitchEvent};

#[derive(Debug, Clone)]
pub struct So2rConfig {
    pub serial_port: String,
    /// Initial TX radio, 1 or 2. Defaults to 1.
    pub initial_tx: Option<u8>,
    /// Initial RX audio mode: "mono" / "stereo" / "reverse_stereo".
    pub initial_rx_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub struct So2rSnapshot {
    /// Current TX radio (1 or 2).
    pub tx_radio: u8,
    /// Current RX-focus radio (1 or 2). In Stereo / ReverseStereo modes
    /// both radios are audible; this is the primary-focus radio.
    pub rx_radio: u8,
    /// "mono" / "stereo" / "reverse_stereo" — matches `set_rx_audio`.
    pub rx_mode: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("otrsp build error: {0}")]
    Build(String),
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("so2r command failed: {0}")]
    Command(String),

    #[error("invalid radio index: {0} (expected 1 or 2)")]
    InvalidRadio(u8),

    #[error("invalid rx mode: {0} (expected mono / stereo / reverse_stereo)")]
    InvalidRxMode(String),
}

#[derive(Clone)]
pub struct So2rHandle {
    inner: Arc<tokio::sync::RwLock<Option<Arc<dyn So2rSwitch + Send + Sync>>>>,
}

impl std::fmt::Debug for So2rHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("So2rHandle").finish_non_exhaustive()
    }
}

impl So2rHandle {
    pub async fn is_connected(&self) -> bool {
        self.inner.read().await.is_some()
    }

    /// Route TX (PTT/key/mic) to radio 1 or 2.
    pub async fn set_tx_radio(&self, radio: u8) -> Result<(), CommandError> {
        let r = parse_radio(radio)?;
        let s = self.current().await?;
        s.set_tx(r)
            .await
            .map_err(|e| CommandError::Command(format!("{e:?}")))
    }

    /// Set RX-focus radio + audio mode. `mode` is case-insensitive
    /// "mono" / "stereo" / "reverse_stereo".
    pub async fn set_rx_audio(&self, radio: u8, mode: &str) -> Result<(), CommandError> {
        let r = parse_radio(radio)?;
        let m = parse_rx_mode(mode)?;
        let s = self.current().await?;
        s.set_rx(r, m)
            .await
            .map_err(|e| CommandError::Command(format!("{e:?}")))
    }

    /// Set BCD band-decoder output on `port` (typically 1 or 2) to
    /// `value` (0–15 — interpreted as a 4-bit BCD code by the switch).
    pub async fn set_aux(&self, port: u8, value: u8) -> Result<(), CommandError> {
        let s = self.current().await?;
        s.set_aux(port, value)
            .await
            .map_err(|e| CommandError::Command(format!("{e:?}")))
    }

    async fn current(&self) -> Result<Arc<dyn So2rSwitch + Send + Sync>, CommandError> {
        let g = self.inner.read().await;
        g.as_ref()
            .map(Arc::clone)
            .ok_or_else(|| CommandError::Command("switch not connected".into()))
    }
}

fn parse_radio(idx: u8) -> Result<Radio, CommandError> {
    match idx {
        1 => Ok(Radio::Radio1),
        2 => Ok(Radio::Radio2),
        other => Err(CommandError::InvalidRadio(other)),
    }
}

fn parse_rx_mode(s: &str) -> Result<RxMode, CommandError> {
    match s.to_ascii_lowercase().as_str() {
        "mono" => Ok(RxMode::Mono),
        "stereo" => Ok(RxMode::Stereo),
        "reverse_stereo" | "reverse-stereo" | "reverse stereo" | "rev_stereo" => {
            Ok(RxMode::ReverseStereo)
        }
        _ => Err(CommandError::InvalidRxMode(s.to_string())),
    }
}

fn radio_to_idx(r: Radio) -> u8 {
    match r {
        Radio::Radio1 => 1,
        Radio::Radio2 => 2,
    }
}

fn rx_mode_to_str(m: RxMode) -> &'static str {
    match m {
        RxMode::Mono => "mono",
        RxMode::Stereo => "stereo",
        RxMode::ReverseStereo => "reverse_stereo",
    }
}

const SNAPSHOT_DEPTH: usize = 16;

pub async fn connect(
    cfg: &So2rConfig,
) -> Result<(mpsc::Receiver<So2rSnapshot>, So2rHandle), ConnectError> {
    let initial = build_switch(cfg).await?;

    let handle_inner: Arc<tokio::sync::RwLock<Option<Arc<dyn So2rSwitch + Send + Sync>>>> =
        Arc::new(tokio::sync::RwLock::new(Some(initial.clone())));
    let handle = So2rHandle {
        inner: handle_inner.clone(),
    };

    let (tx, rx) = mpsc::channel::<So2rSnapshot>(SNAPSHOT_DEPTH);
    let initial_snap = So2rSnapshot {
        tx_radio: cfg.initial_tx.unwrap_or(1),
        rx_radio: cfg.initial_tx.unwrap_or(1),
        rx_mode: cfg.initial_rx_mode.clone().unwrap_or_else(|| "mono".into()),
        at: Utc::now(),
    };
    let snapshot = Arc::new(tokio::sync::Mutex::new(initial_snap.clone()));
    let _ = tx.send(initial_snap).await;

    // Apply initial settings on connect (best-effort).
    apply_initial(&initial, cfg).await;

    let cfg_for_task = cfg.clone();
    let snapshot_for_task = snapshot.clone();
    tokio::spawn(async move {
        let mut current_switch: Option<Arc<dyn So2rSwitch + Send + Sync>> = Some(initial);
        let mut backoff_secs: u64 = 1;
        loop {
            let switch = match current_switch.take() {
                Some(s) => s,
                None => match build_switch(&cfg_for_task).await {
                    Ok(s) => {
                        backoff_secs = 1;
                        *handle_inner.write().await = Some(s.clone());
                        apply_initial(&s, &cfg_for_task).await;
                        snapshot_for_task.lock().await.at = Utc::now();
                        if tx.send(snapshot_for_task.lock().await.clone()).await.is_err() {
                            return;
                        }
                        tracing::info!("so2r switch reconnected");
                        s
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, backoff_secs, "so2r reconnect failed; sleeping");
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(30);
                        continue;
                    }
                },
            };

            let mut events = switch.subscribe();
            loop {
                match events.recv().await {
                    Ok(SwitchEvent::TxChanged { radio }) => {
                        let mut s = snapshot_for_task.lock().await;
                        s.tx_radio = radio_to_idx(radio);
                        s.at = Utc::now();
                        if tx.send(s.clone()).await.is_err() {
                            return;
                        }
                    }
                    Ok(SwitchEvent::RxChanged { radio, mode }) => {
                        let mut s = snapshot_for_task.lock().await;
                        s.rx_radio = radio_to_idx(radio);
                        s.rx_mode = rx_mode_to_str(mode).into();
                        s.at = Utc::now();
                        if tx.send(s.clone()).await.is_err() {
                            return;
                        }
                    }
                    Ok(SwitchEvent::AuxChanged { .. })
                    | Ok(SwitchEvent::Connected)
                    | Ok(SwitchEvent::Disconnected) => {
                        // AUX/Connected/Disconnected don't change the
                        // operator-facing snapshot; logged at debug level
                        // upstream if needed.
                    }
                    Err(_) => {
                        tracing::warn!("so2r event channel closed; will reconnect");
                        break;
                    }
                }
            }

            *handle_inner.write().await = None;
        }
    });

    Ok((rx, handle))
}

async fn build_switch(
    cfg: &So2rConfig,
) -> Result<Arc<dyn So2rSwitch + Send + Sync>, ConnectError> {
    let dev = OtrspBuilder::new(&cfg.serial_port)
        .build()
        .await
        .map_err(|e| ConnectError::Build(format!("{e:?}")))?;
    Ok(Arc::new(dev) as Arc<dyn So2rSwitch + Send + Sync>)
}

/// Apply the initial TX + RX settings best-effort. Errors are logged
/// but not surfaced — if the device is in a sane default at connect
/// time, the operator can adjust manually.
async fn apply_initial(switch: &Arc<dyn So2rSwitch + Send + Sync>, cfg: &So2rConfig) {
    if let Some(idx) = cfg.initial_tx {
        if let Ok(r) = parse_radio(idx) {
            if let Err(e) = switch.set_tx(r).await {
                tracing::warn!(error = %format!("{e:?}"), "so2r initial set_tx failed");
            }
        }
    }
    if let Some(mode_str) = cfg.initial_rx_mode.as_deref() {
        if let Ok(m) = parse_rx_mode(mode_str) {
            let r = cfg
                .initial_tx
                .and_then(|i| parse_radio(i).ok())
                .unwrap_or(Radio::Radio1);
            if let Err(e) = switch.set_rx(r, m).await {
                tracing::warn!(error = %format!("{e:?}"), "so2r initial set_rx failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_radio_accepts_1_and_2() {
        assert!(matches!(parse_radio(1), Ok(Radio::Radio1)));
        assert!(matches!(parse_radio(2), Ok(Radio::Radio2)));
        assert!(matches!(parse_radio(3), Err(CommandError::InvalidRadio(3))));
    }

    #[test]
    fn parse_rx_mode_handles_variants() {
        assert!(matches!(parse_rx_mode("mono"), Ok(RxMode::Mono)));
        assert!(matches!(parse_rx_mode("STEREO"), Ok(RxMode::Stereo)));
        assert!(matches!(
            parse_rx_mode("reverse_stereo"),
            Ok(RxMode::ReverseStereo)
        ));
        assert!(matches!(
            parse_rx_mode("reverse-stereo"),
            Ok(RxMode::ReverseStereo)
        ));
        assert!(matches!(parse_rx_mode("nonsense"), Err(_)));
    }

    #[test]
    fn radio_index_round_trip() {
        assert_eq!(radio_to_idx(Radio::Radio1), 1);
        assert_eq!(radio_to_idx(Radio::Radio2), 2);
    }
}
