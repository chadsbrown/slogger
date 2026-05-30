use std::sync::{Mutex, OnceLock};

use iced::Subscription;
use iced::futures::SinkExt;
use keyer_control::KeyerSnapshot;
use so2r_control::So2rSnapshot;
use spot_feed::SpotEvent;
use tokio::sync::mpsc;
use wsjtx_bridge::WsjtxMessage;

use super::message::Message;
use super::state::App;
use super::types::TaggedRigSnapshot;

/// Receiver-stash for the spot feed. iced 0.14's `Subscription::run`
/// takes a `fn` pointer, so the streaming closure can't capture state —
/// it has to fetch the receiver from a static. Set by `boot_app` when the
/// `[dxcluster]` config is present; consumed once by `spot_events_stream`
/// on the first subscription tick.
pub(super) static SPOT_RX: OnceLock<Mutex<Option<mpsc::Receiver<SpotEvent>>>> = OnceLock::new();

/// Same pattern as `SPOT_RX` but for the WSJT-X bridge.
pub(super) static WSJTX_RX: OnceLock<Mutex<Option<mpsc::Receiver<WsjtxMessage>>>> =
    OnceLock::new();

/// Unified snapshot channel for all rigs. Each rig's per-rig forwarder
/// task wraps its `RigSnapshot` with its index and pushes here.
pub(super) static RIG_RX: OnceLock<Mutex<Option<mpsc::Receiver<TaggedRigSnapshot>>>> =
    OnceLock::new();

/// And for the keyer snapshot stream.
pub(super) static KEYER_RX: OnceLock<Mutex<Option<mpsc::Receiver<KeyerSnapshot>>>> =
    OnceLock::new();

/// And for the SO2R switch snapshot stream.
pub(super) static SO2R_RX: OnceLock<Mutex<Option<mpsc::Receiver<So2rSnapshot>>>> =
    OnceLock::new();

impl App {
    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = Vec::with_capacity(3);
        if self.spots_active {
            subs.push(Subscription::run(spot_events_stream).map(Message::SpotEvent));
        }
        if self.wsjtx_active {
            subs.push(Subscription::run(wsjtx_events_stream).map(Message::WsjtxMessage));
        }
        if !self.rigs.is_empty() {
            subs.push(Subscription::run(rig_events_stream).map(Message::RigSnapshot));
        }
        if self.keyer_active {
            subs.push(Subscription::run(keyer_events_stream).map(Message::KeyerSnapshot));
        }
        if self.so2r_active {
            subs.push(Subscription::run(so2r_events_stream).map(Message::So2rSnapshotMsg));
        }
        Subscription::batch(subs)
    }
}

fn spot_events_stream() -> impl iced::futures::Stream<Item = SpotEvent> {
    iced::stream::channel(64, async |mut output| {
        let Some(slot) = SPOT_RX.get() else {
            tracing::warn!("SPOT_RX not initialized; spot subscription idle");
            std::future::pending::<()>().await;
            return;
        };
        let Some(mut rx) = slot.lock().ok().and_then(|mut g| g.take()) else {
            tracing::warn!("SPOT_RX already taken; spot subscription idle");
            std::future::pending::<()>().await;
            return;
        };
        while let Some(ev) = rx.recv().await {
            if output.send(ev).await.is_err() {
                break;
            }
        }
    })
}

fn wsjtx_events_stream() -> impl iced::futures::Stream<Item = WsjtxMessage> {
    iced::stream::channel(64, async |mut output| {
        let Some(slot) = WSJTX_RX.get() else {
            std::future::pending::<()>().await;
            return;
        };
        let Some(mut rx) = slot.lock().ok().and_then(|mut g| g.take()) else {
            std::future::pending::<()>().await;
            return;
        };
        while let Some(ev) = rx.recv().await {
            if output.send(ev).await.is_err() {
                break;
            }
        }
    })
}

fn so2r_events_stream() -> impl iced::futures::Stream<Item = So2rSnapshot> {
    iced::stream::channel(16, async |mut output| {
        let Some(slot) = SO2R_RX.get() else {
            std::future::pending::<()>().await;
            return;
        };
        let Some(mut rx) = slot.lock().ok().and_then(|mut g| g.take()) else {
            std::future::pending::<()>().await;
            return;
        };
        while let Some(s) = rx.recv().await {
            if output.send(s).await.is_err() {
                break;
            }
        }
    })
}

fn keyer_events_stream() -> impl iced::futures::Stream<Item = KeyerSnapshot> {
    iced::stream::channel(16, async |mut output| {
        let Some(slot) = KEYER_RX.get() else {
            std::future::pending::<()>().await;
            return;
        };
        let Some(mut rx) = slot.lock().ok().and_then(|mut g| g.take()) else {
            std::future::pending::<()>().await;
            return;
        };
        while let Some(s) = rx.recv().await {
            if output.send(s).await.is_err() {
                break;
            }
        }
    })
}

fn rig_events_stream() -> impl iced::futures::Stream<Item = TaggedRigSnapshot> {
    iced::stream::channel(64, async |mut output| {
        let Some(slot) = RIG_RX.get() else {
            std::future::pending::<()>().await;
            return;
        };
        let Some(mut rx) = slot.lock().ok().and_then(|mut g| g.take()) else {
            std::future::pending::<()>().await;
            return;
        };
        while let Some(s) = rx.recv().await {
            if output.send(s).await.is_err() {
                break;
            }
        }
    })
}
