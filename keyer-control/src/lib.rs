//! CW keyer wrapper around `winkey`. Mirrors the rig-control pattern:
//! initial-connect synchronous, then auto-reconnect with exponential
//! backoff on disconnect; `KeyerHandle` survives reconnects via a
//! shared RwLock; commands during a disconnect window return a clean
//! "keyer not connected" error.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::mpsc;

use winkey::{Keyer, KeyerEvent, WinKeyerBuilder};

#[derive(Debug, Clone)]
pub struct KeyerConfig {
    /// Serial device path (e.g. `/dev/ttyUSB1`).
    pub serial_port: String,
    /// Initial CW speed in WPM. Operators usually set this once at boot
    /// and adjust mid-QSO via the speed pot or set_wpm. Default 25.
    pub initial_wpm: u8,
}

#[derive(Debug, Clone)]
pub struct KeyerSnapshot {
    pub wpm: u8,
    /// True while the keyer is actively transmitting (between busy=true
    /// and busy=false StatusChanged events).
    pub keying: bool,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("winkey build error: {0}")]
    Build(String),
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("keyer command failed: {0}")]
    Command(String),
}

/// Cheap-to-clone handle. The inner `Arc<dyn Keyer>` is held under an
/// async RwLock so the auto-reconnect task can swap it on the fly.
#[derive(Clone)]
pub struct KeyerHandle {
    inner: Arc<tokio::sync::RwLock<Option<Arc<dyn Keyer + Send + Sync>>>>,
}

impl std::fmt::Debug for KeyerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyerHandle").finish_non_exhaustive()
    }
}

impl KeyerHandle {
    pub async fn is_connected(&self) -> bool {
        self.inner.read().await.is_some()
    }

    /// Queue CW for transmission. Blocks if the keyer's XOFF flow control
    /// is asserted. For long messages, prefer breaking into chunks so the
    /// operator can abort cleanly.
    pub async fn send_message(&self, text: &str) -> Result<(), CommandError> {
        let k = self.current().await?;
        k.send_message(text)
            .await
            .map_err(|e| CommandError::Command(format!("{e:?}")))
    }

    /// Immediately stop sending and clear the buffer.
    pub async fn abort(&self) -> Result<(), CommandError> {
        let k = self.current().await?;
        k.abort()
            .await
            .map_err(|e| CommandError::Command(format!("{e:?}")))
    }

    pub async fn set_wpm(&self, wpm: u8) -> Result<(), CommandError> {
        let k = self.current().await?;
        k.set_speed(wpm)
            .await
            .map_err(|e| CommandError::Command(format!("{e:?}")))
    }

    /// Hold the key down (test transmit, antenna tuning, etc.). Caller
    /// must turn it off explicitly with `set_tune(false)`.
    pub async fn set_tune(&self, on: bool) -> Result<(), CommandError> {
        let k = self.current().await?;
        k.set_tune(on)
            .await
            .map_err(|e| CommandError::Command(format!("{e:?}")))
    }

    async fn current(&self) -> Result<Arc<dyn Keyer + Send + Sync>, CommandError> {
        let g = self.inner.read().await;
        g.as_ref()
            .map(Arc::clone)
            .ok_or_else(|| CommandError::Command("keyer not connected".into()))
    }
}

const SNAPSHOT_DEPTH: usize = 16;

pub async fn connect(
    cfg: &KeyerConfig,
) -> Result<(mpsc::Receiver<KeyerSnapshot>, KeyerHandle), ConnectError> {
    let initial = build_keyer(cfg).await?;
    let initial_wpm = cfg.initial_wpm.max(5).min(50);

    let handle_inner: Arc<tokio::sync::RwLock<Option<Arc<dyn Keyer + Send + Sync>>>> =
        Arc::new(tokio::sync::RwLock::new(Some(initial.clone())));
    let handle = KeyerHandle {
        inner: handle_inner.clone(),
    };

    let (tx, rx) = mpsc::channel::<KeyerSnapshot>(SNAPSHOT_DEPTH);
    let snapshot = Arc::new(tokio::sync::Mutex::new(KeyerSnapshot {
        wpm: initial_wpm,
        keying: false,
        at: Utc::now(),
    }));
    // Best-effort initial wpm read.
    if let Ok(actual) = initial.get_speed().await {
        snapshot.lock().await.wpm = actual;
    }
    let _ = tx.send(snapshot.lock().await.clone()).await;

    let cfg_for_task = cfg.clone();
    let snapshot_for_task = snapshot.clone();
    tokio::spawn(async move {
        let mut current_keyer: Option<Arc<dyn Keyer + Send + Sync>> = Some(initial);
        let mut backoff_secs: u64 = 1;
        loop {
            let keyer = match current_keyer.take() {
                Some(k) => k,
                None => match build_keyer(&cfg_for_task).await {
                    Ok(k) => {
                        backoff_secs = 1;
                        *handle_inner.write().await = Some(k.clone());
                        if let Ok(wpm) = k.get_speed().await {
                            snapshot_for_task.lock().await.wpm = wpm;
                        }
                        snapshot_for_task.lock().await.at = Utc::now();
                        if tx.send(snapshot_for_task.lock().await.clone()).await.is_err() {
                            return;
                        }
                        tracing::info!("keyer reconnected");
                        k
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, backoff_secs, "keyer reconnect failed; sleeping");
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(30);
                        continue;
                    }
                },
            };

            let mut events = keyer.subscribe();
            // Forward events until disconnect.
            loop {
                match events.recv().await {
                    Ok(KeyerEvent::StatusChanged(status)) => {
                        let mut s = snapshot_for_task.lock().await;
                        s.keying = status.busy;
                        s.at = Utc::now();
                        if tx.send(s.clone()).await.is_err() {
                            return;
                        }
                    }
                    Ok(KeyerEvent::SpeedPotChanged { wpm }) => {
                        let mut s = snapshot_for_task.lock().await;
                        s.wpm = wpm;
                        s.at = Utc::now();
                        if tx.send(s.clone()).await.is_err() {
                            return;
                        }
                    }
                    Ok(_) => {
                        // Other events (CharacterSent etc.) don't change
                        // the snapshot we surface to the UI. winkey's
                        // listeners that need them can subscribe directly.
                    }
                    Err(_) => {
                        tracing::warn!("keyer event channel closed; will reconnect");
                        break;
                    }
                }
            }

            *handle_inner.write().await = None;
        }
    });

    Ok((rx, handle))
}

async fn build_keyer(cfg: &KeyerConfig) -> Result<Arc<dyn Keyer + Send + Sync>, ConnectError> {
    let initial_wpm = cfg.initial_wpm.max(5).min(50);
    let keyer = WinKeyerBuilder::new(&cfg.serial_port)
        .speed(initial_wpm)
        .build()
        .await
        .map_err(|e| ConnectError::Build(format!("{e:?}")))?;
    Ok(Arc::new(keyer) as Arc<dyn Keyer + Send + Sync>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_carries_port_and_speed() {
        let cfg = KeyerConfig {
            serial_port: "/dev/ttyUSB1".into(),
            initial_wpm: 28,
        };
        assert_eq!(cfg.initial_wpm, 28);
    }
}
