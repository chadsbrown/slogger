//! DX cluster spot feed — wraps the `dxfeed` crate behind a slim API.
//!
//! Concrete spot semantics belong here, not in the UI. Consumers get an
//! `mpsc::Receiver<SpotEvent>` and drain it however they like (iced
//! subscription, log writer, scoring engine).

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
    pub spotted_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum SpotEvent {
    /// New or updated spot.
    Spot(Spot),
    /// Cluster withdrew a spot (DX*Sh entry expired or replaced).
    Withdrawn { call: String },
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
        while let Some(event) = feed.next_event().await {
            for translated in translate(event) {
                if tx.send(translated).await.is_err() {
                    return; // receiver dropped
                }
            }
        }
        tracing::info!("dxfeed event stream ended");
    });

    Ok(rx)
}

fn translate(event: dxfeed::model::DxEvent) -> Vec<SpotEvent> {
    use dxfeed::model::{DxEvent, SpotEventKind};
    match event {
        DxEvent::Spot(spot_event) => match spot_event.kind {
            SpotEventKind::New | SpotEventKind::Update => {
                vec![SpotEvent::Spot(Spot {
                    call: spot_event.spot.dx_call,
                    freq_hz: spot_event.spot.freq_hz,
                    mode: dxmode_to_str(spot_event.spot.mode).map(String::from),
                    comment: spot_event.spot.comment,
                    spotted_at: spot_event.spot.last_seen,
                })]
            }
            SpotEventKind::Withdraw => {
                vec![SpotEvent::Withdrawn {
                    call: spot_event.spot.dx_call,
                }]
            }
        },
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
