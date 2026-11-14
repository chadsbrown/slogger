//! DX cluster spot feed — wraps the `dxfeed` crate behind a slim API.
//!
//! Concrete spot semantics belong here, not in the UI. Consumers get an
//! `mpsc::Receiver<SpotEvent>` and drain it however they like (iced
//! subscription, log writer, scoring engine).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct Spot {
    pub call: String,
    pub freq_hz: u64,
    pub mode: Option<String>,
    pub comment: Option<String>,
    /// When the spot was most recently reported — what "age" counts from.
    pub spotted_at: DateTime<Utc>,
    /// When the spot first appeared. Unlike `spotted_at` it never changes
    /// as the spot is re-reported, so it gives a stable display order.
    pub first_seen: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum SpotEvent {
    /// New or updated spot.
    Spot(Spot),
    /// A spot previously sent as [`SpotEvent::Spot`] is gone (expired,
    /// moved, evicted, or about to be replaced by an updated copy).
    /// Identifies exactly that spot: the same call on another band or
    /// frequency is a different spot and stays.
    Withdrawn { call: String, freq_hz: u64 },
    /// Source connection state change. Useful for "are we live?" UI badge.
    SourceStatus { source_id: String, status: String },
    /// Soft error from the feed. Don't crash the adapter on these.
    Error { message: String },
}

#[derive(Debug, Clone)]
pub struct ClusterSource {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct SpotFeedConfig {
    /// Login callsign sent to clusters. Required — clusters reject anonymous
    /// connections. Doesn't have to match `[station] default_callsign`.
    pub my_callsign: String,
    pub sources: Vec<ClusterSource>,
    /// Path to a dxfeed filter JSON file. When set, spots are filtered
    /// before reaching the channel — useful for restricting to specific
    /// bands/modes/continents. See dxfeed docs for the JSON schema.
    pub filter_path: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum FeedError {
    #[error("dxfeed build error: {0}")]
    Build(String),

    #[error("filter file IO error at {path}: {error}")]
    FilterIo { path: PathBuf, error: String },

    #[error("filter file parse error at {path}: {error}")]
    FilterParse { path: PathBuf, error: String },
}

/// Load + parse a dxfeed filter file. Public so callers can validate
/// configs early (e.g. at startup) and surface a clean error before
/// spawning the feed.
pub fn load_filter_file(
    path: &Path,
) -> Result<dxfeed::filter::config::FilterConfigSerde, FeedError> {
    let raw = std::fs::read_to_string(path).map_err(|e| FeedError::FilterIo {
        path: path.to_path_buf(),
        error: e.to_string(),
    })?;
    serde_json::from_str(&raw).map_err(|e| FeedError::FilterParse {
        path: path.to_path_buf(),
        error: e.to_string(),
    })
}

const CHANNEL_DEPTH: usize = 256;

pub async fn spawn_spot_feed(config: &SpotFeedConfig) -> Result<mpsc::Receiver<SpotEvent>, FeedError> {
    let mut builder = dxfeed::feed::DxFeedBuilder::new();
    for (i, src) in config.sources.iter().enumerate() {
        let cluster = dxfeed::source::cluster::ClusterSourceConfig::new(
            &src.host,
            src.port,
            &config.my_callsign,
            dxfeed::model::SourceId(format!("cluster-{i}")),
        );
        builder = builder.add_source(dxfeed::source::supervisor::SourceConfig::Cluster(cluster));
    }

    if let Some(path) = &config.filter_path {
        let filter = load_filter_file(path)?;
        tracing::info!(path = %path.display(), "spot feed: applying filter");
        builder = builder.set_filter(filter);
    }

    let mut feed = builder
        .build()
        .map_err(|e| FeedError::Build(format!("{e:?}")))?;

    let (tx, rx) = mpsc::channel::<SpotEvent>(CHANNEL_DEPTH);
    tokio::spawn(async move {
        let mut tracker = SpotTracker::default();
        while let Some(event) = feed.next_event().await {
            for translated in tracker.translate(event) {
                if tx.send(translated).await.is_err() {
                    return; // receiver dropped
                }
            }
        }
        tracing::info!("dxfeed event stream ended");
    });

    Ok(rx)
}

/// Translates dxfeed events into panel events.
///
/// dxfeed identifies a spot by call, band, mode and a frequency anchor,
/// and guarantees that every `New` is followed by exactly one `Withdraw`
/// for the same key. The panel knows a spot only by call and frequency,
/// and the frequency on an `Update` or `Withdraw` is the latest one
/// reported, which can differ from the one sent earlier. So this
/// remembers what was sent for each key and withdraws exactly that: the
/// panel holds one row per dxfeed spot, and a call spotted on two bands
/// keeps its other row when one goes away.
#[derive(Default)]
struct SpotTracker {
    /// Call and frequency last sent to the panel for each live key.
    sent: HashMap<dxfeed::model::SpotKey, (String, u64)>,
}

impl SpotTracker {
    fn translate(&mut self, event: dxfeed::model::DxEvent) -> Vec<SpotEvent> {
        use dxfeed::model::{DxEvent, SpotEventKind};
        match event {
            DxEvent::Spot(spot_event) => {
                let key = spot_event.spot.spot_key.clone();
                match spot_event.kind {
                    SpotEventKind::New | SpotEventKind::Update => {
                        let spot = Spot {
                            call: spot_event.spot.dx_call,
                            freq_hz: spot_event.spot.freq_hz,
                            mode: dxmode_to_str(spot_event.spot.mode).map(String::from),
                            comment: spot_event.spot.comment,
                            spotted_at: spot_event.spot.last_seen,
                            first_seen: spot_event.spot.first_seen,
                        };
                        let previous = self.sent.insert(key, (spot.call.clone(), spot.freq_hz));
                        let mut out = Vec::new();
                        // Replace rather than add, so an updated spot
                        // does not pile up duplicate rows.
                        if let Some((call, freq_hz)) = previous {
                            out.push(SpotEvent::Withdrawn { call, freq_hz });
                        }
                        out.push(SpotEvent::Spot(spot));
                        out
                    }
                    SpotEventKind::Withdraw => {
                        tracing::debug!(
                            call = %spot_event.spot.dx_call,
                            freq_hz = spot_event.spot.freq_hz,
                            reason = ?spot_event.withdraw_reason,
                            "dxfeed withdraw"
                        );
                        match self.sent.remove(&key) {
                            Some((call, freq_hz)) => {
                                vec![SpotEvent::Withdrawn { call, freq_hz }]
                            }
                            // Never sent to the panel: nothing to remove.
                            None => Vec::new(),
                        }
                    }
                }
            }
            DxEvent::SourceStatus(status) => vec![SpotEvent::SourceStatus {
                source_id: status.source_id.0,
                status: format!("{:?}", status.state),
            }],
            DxEvent::Error(err) => vec![SpotEvent::Error {
                message: err.message,
            }],
            _ => Vec::new(),
        }
    }
}

fn dxmode_to_str(mode: dxfeed::domain::DxMode) -> Option<&'static str> {
    use dxfeed::domain::DxMode;
    match mode {
        DxMode::CW => Some("CW"),
        DxMode::SSB => Some("SSB"),
        DxMode::DIG => Some("FT8"), // dxfeed lumps all digital under DIG; FT8 is the right default for DX clusters today
        DxMode::AM => Some("AM"),
        DxMode::FM => Some("FM"),
        DxMode::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dxfeed::domain::{Band, DxMode};
    use dxfeed::model::{
        DxEvent, DxSpot, DxSpotEvent, SpotConfidence, SpotEventKind, SpotKey, WithdrawReason,
    };

    /// A dxfeed event for `call` whose key is anchored at `anchor_hz` and
    /// whose latest reported frequency is `freq_hz`.
    fn spot_event(
        call: &str,
        band: Band,
        anchor_hz: u64,
        freq_hz: u64,
        kind: SpotEventKind,
    ) -> DxEvent {
        let now = Utc::now();
        DxEvent::Spot(DxSpotEvent {
            spot: DxSpot {
                spot_key: SpotKey {
                    dx_call_norm: call.to_string(),
                    band,
                    mode: DxMode::CW,
                    freq_bucket_hz: anchor_hz,
                },
                dx_call: call.to_string(),
                freq_hz,
                band,
                mode: DxMode::CW,
                first_seen: now,
                last_seen: now,
                spot_time: None,
                comment: None,
                observations: Vec::new(),
                unique_originators: 1,
                unique_sources: 1,
                skim_quality: None,
                quality_detail: None,
                confidence: SpotConfidence::default(),
            },
            kind,
            revision: 1,
            withdraw_reason: (kind == SpotEventKind::Withdraw).then_some(WithdrawReason::Expired),
        })
    }

    /// Compact view of translated events: `+CALL@hz` / `-CALL@hz`.
    fn summary(events: Vec<SpotEvent>) -> Vec<String> {
        events
            .into_iter()
            .map(|e| match e {
                SpotEvent::Spot(s) => format!("+{}@{}", s.call, s.freq_hz),
                SpotEvent::Withdrawn { call, freq_hz } => format!("-{call}@{freq_hz}"),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn update_replaces_the_row_it_updates() {
        let mut t = SpotTracker::default();
        let new = t.translate(spot_event("K1ABC", Band::B20, 14_025_000, 14_025_000, SpotEventKind::New));
        assert_eq!(summary(new), ["+K1ABC@14025000"]);
        // Same key, slightly different reported frequency.
        let update =
            t.translate(spot_event("K1ABC", Band::B20, 14_025_000, 14_025_100, SpotEventKind::Update));
        assert_eq!(summary(update), ["-K1ABC@14025000", "+K1ABC@14025100"]);
    }

    #[test]
    fn withdraw_removes_only_that_spot() {
        let mut t = SpotTracker::default();
        t.translate(spot_event("K1ABC", Band::B20, 14_025_000, 14_025_000, SpotEventKind::New));
        t.translate(spot_event("K1ABC", Band::B40, 7_010_000, 7_010_000, SpotEventKind::New));
        // The withdraw reports a drifted frequency; the row sent earlier
        // is the one that must go.
        let gone =
            t.translate(spot_event("K1ABC", Band::B20, 14_025_000, 14_025_200, SpotEventKind::Withdraw));
        assert_eq!(summary(gone), ["-K1ABC@14025000"]);
        // The 40m spot is still tracked and withdraws on its own.
        let gone =
            t.translate(spot_event("K1ABC", Band::B40, 7_010_000, 7_010_000, SpotEventKind::Withdraw));
        assert_eq!(summary(gone), ["-K1ABC@7010000"]);
    }

    #[test]
    fn withdraw_of_an_unknown_spot_is_ignored() {
        let mut t = SpotTracker::default();
        let gone =
            t.translate(spot_event("K1ABC", Band::B20, 14_025_000, 14_025_000, SpotEventKind::Withdraw));
        assert!(gone.is_empty());
    }

    #[test]
    fn config_carries_sources() {
        let cfg = SpotFeedConfig {
            my_callsign: "W1ABC".into(),
            sources: vec![
                ClusterSource { host: "dxc.kbx.org".into(), port: 7300 },
                ClusterSource { host: "n1nr.org".into(), port: 7300 },
            ],
            filter_path: None,
        };
        assert_eq!(cfg.sources.len(), 2);
        assert_eq!(cfg.my_callsign, "W1ABC");
    }

    #[test]
    fn load_filter_file_parses_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("filter.json");
        // Empty filter object — accepts everything. Validates the path
        // round-trip and JSON parse without depending on dxfeed's schema.
        std::fs::write(&path, "{}").unwrap();
        let _filter = load_filter_file(&path).expect("default filter should parse");
    }

    #[test]
    fn load_filter_file_missing_returns_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.json");
        let err = load_filter_file(&path).unwrap_err();
        assert!(matches!(err, FeedError::FilterIo { .. }), "got {err:?}");
    }

    #[test]
    fn load_filter_file_bad_json_returns_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        let err = load_filter_file(&path).unwrap_err();
        assert!(matches!(err, FeedError::FilterParse { .. }), "got {err:?}");
    }
}
