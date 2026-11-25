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
    /// Where the spot came from, for the operator: who reported it and
    /// how. Shown when hovering a row; not used for anything else.
    pub origin: Option<String>,
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

/// Keys in a filter file that dxfeed does not recognise, as dotted
/// paths ("mode_allow", "rf.modes"). dxfeed ignores such keys without
/// complaint, so a misspelt or misplaced one filters nothing; callers
/// report these so the operator finds out. Empty for a file that can't
/// be read or parsed: [`load_filter_file`] reports that.
pub fn unknown_filter_keys(path: &Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    unknown_keys_in(&raw)
}

fn unknown_keys_in(raw: &str) -> Vec<String> {
    let Ok(given) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    // Every key dxfeed knows appears when its defaults are written out.
    let Ok(known) = serde_json::to_value(dxfeed::filter::config::FilterConfigSerde::default())
    else {
        return Vec::new();
    };
    let mut unknown = Vec::new();
    collect_unknown(&given, &known, "", &mut unknown);
    unknown
}

fn collect_unknown(
    given: &serde_json::Value,
    known: &serde_json::Value,
    prefix: &str,
    out: &mut Vec<String>,
) {
    let (Some(given), Some(known)) = (given.as_object(), known.as_object()) else {
        return;
    };
    for (key, value) in given {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match known.get(key) {
            Some(known_value) => collect_unknown(value, known_value, &path, out),
            None => out.push(path),
        }
    }
}

const CHANNEL_DEPTH: usize = 256;

/// Where a callsign is, as the caller's country file (cty.dat) has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    /// Entity name exactly as the country file spells it ("Japan",
    /// "Fed. Rep. of Germany"). `geo.*.entity_allow` / `entity_deny`
    /// in a filter file match against this.
    pub name: String,
    /// Two-letter continent: AF, AN, AS, EU, NA, OC or SA.
    pub continent: String,
    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    /// The entity's primary prefix ("JA", "DL").
    pub prefix: String,
}

/// Looks up where a callsign is. Given to [`spawn_spot_feed`] so a
/// filter file's `geo` rules (continent, CQ and ITU zone, entity) have
/// something to work on, for the spotted station and the spotter alike.
pub trait EntityLookup: Send + Sync {
    /// `call` is upper-case and may carry a skimmer's `-#` or `-2`
    /// suffix, or a portable prefix or suffix.
    fn entity(&self, call: &str) -> Option<Entity>;
}

/// Adapts an [`EntityLookup`] to what dxfeed's aggregator takes.
pub fn entity_resolver(
    lookup: Box<dyn EntityLookup>,
) -> Box<dyn dxfeed::resolver::entity::EntityResolver> {
    struct Adapter(Box<dyn EntityLookup>);
    impl dxfeed::resolver::entity::EntityResolver for Adapter {
        fn resolve(&self, callsign: &str) -> Option<dxfeed::resolver::entity::EntityInfo> {
            use dxfeed::domain::Continent;
            let entity = self.0.entity(callsign)?;
            let continent = match entity.continent.to_ascii_uppercase().as_str() {
                "AF" => Continent::AF,
                "AN" => Continent::AN,
                "AS" => Continent::AS,
                "EU" => Continent::EU,
                "NA" => Continent::NA,
                "OC" => Continent::OC,
                "SA" => Continent::SA,
                _ => Continent::Unknown,
            };
            Some(dxfeed::resolver::entity::EntityInfo {
                entity_name: entity.name,
                continent,
                // Zone 0 does not exist, so an unknown zone matches no
                // allow list and no deny list.
                cq_zone: entity.cq_zone.unwrap_or(0),
                itu_zone: entity.itu_zone.unwrap_or(0),
                lat: 0.0,
                lon: 0.0,
                primary_prefix: entity.prefix,
            })
        }
    }
    Box::new(Adapter(lookup))
}

/// Whether a filter file has rules that need an [`EntityLookup`]: any
/// `geo` allow or deny list. Without a lookup such an allow list hides
/// every spot and a deny list hides none. False when the file cannot be
/// read or parsed: [`load_filter_file`] reports that.
pub fn filter_uses_geography(path: &Path) -> bool {
    load_filter_file(path)
        .ok()
        .and_then(|filter| filter.validate_and_compile().ok())
        .is_some_and(|f| f.geo.dx.has_any_filters() || f.geo.spotter.has_any_filters())
}

/// Starts the cluster feed. `entities` is the country-file lookup for a
/// filter's `geo` rules; pass `None` when there is no country file.
pub async fn spawn_spot_feed(
    config: &SpotFeedConfig,
    entities: Option<Box<dyn EntityLookup>>,
) -> Result<mpsc::Receiver<SpotEvent>, FeedError> {
    let mut builder = dxfeed::feed::DxFeedBuilder::new();
    if let Some(lookup) = entities {
        builder = builder.entity_resolver(entity_resolver(lookup));
    }
    for (i, src) in config.sources.iter().enumerate() {
        let cluster = dxfeed::source::cluster::ClusterSourceConfig::new(
            &src.host,
            src.port,
            &config.my_callsign,
            dxfeed::model::SourceId(format!("cluster-{i}")),
        );
        builder = builder.add_source(dxfeed::source::supervisor::SourceConfig::Cluster(cluster));
    }

    let mut modes = ModeRule::default();
    if let Some(path) = &config.filter_path {
        let filter = load_filter_file(path)?;
        modes = ModeRule {
            file: Some(path.display().to_string()),
            allow: filter.rf.mode_allow.iter().copied().collect(),
            deny: filter.rf.mode_deny.iter().copied().collect(),
        };
        tracing::info!(
            path = %path.display(),
            modes = %modes.describe(),
            "spot feed: applying filter"
        );
        builder = builder.set_filter(filter);
    }

    let mut feed = builder
        .build()
        .map_err(|e| FeedError::Build(format!("{e:?}")))?;

    let (tx, rx) = mpsc::channel::<SpotEvent>(CHANNEL_DEPTH);
    tokio::spawn(async move {
        let mut tracker = SpotTracker { modes, ..SpotTracker::default() };
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
    /// The filter file's mode rule, checked again here. dxfeed applies
    /// it too; a spot that reaches this point against the rule is
    /// dropped and logged, so the panel never shows one.
    modes: ModeRule,
}

/// The modes a filter file lets through.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ModeRule {
    /// The filter file the rule came from; `None` when the config
    /// names no filter file, so nothing is filtered at all.
    file: Option<String>,
    /// Empty means every mode not denied.
    allow: Vec<dxfeed::domain::DxMode>,
    deny: Vec<dxfeed::domain::DxMode>,
}

impl ModeRule {
    fn permits(&self, mode: dxfeed::domain::DxMode) -> bool {
        !self.deny.contains(&mode) && (self.allow.is_empty() || self.allow.contains(&mode))
    }

    /// For the log and the spot tooltip: "CW only", "any mode", ...
    fn describe(&self) -> String {
        let Some(file) = &self.file else {
            return "none (no filter_file under [dxcluster] in config.toml)".to_string();
        };
        format!("{} ({file})", self.rule())
    }

    fn rule(&self) -> String {
        let list = |modes: &[dxfeed::domain::DxMode]| {
            modes.iter().map(|m| format!("{m:?}")).collect::<Vec<_>>().join(", ")
        };
        match (self.allow.is_empty(), self.deny.is_empty()) {
            (true, true) => "any mode".to_string(),
            (false, true) => format!("{} only", list(&self.allow)),
            (true, false) => format!("not {}", list(&self.deny)),
            (false, false) => format!("{} only, not {}", list(&self.allow), list(&self.deny)),
        }
    }
}

impl SpotTracker {
    fn translate(&mut self, event: dxfeed::model::DxEvent) -> Vec<SpotEvent> {
        use dxfeed::model::{DxEvent, SpotEventKind};
        match event {
            DxEvent::Spot(spot_event) => {
                let key = spot_event.spot.spot_key.clone();
                match spot_event.kind {
                    SpotEventKind::New | SpotEventKind::Update => {
                        if !self.modes.permits(spot_event.spot.mode) {
                            tracing::warn!(
                                call = %spot_event.spot.dx_call,
                                freq_hz = spot_event.spot.freq_hz,
                                mode = ?spot_event.spot.mode,
                                filter = %self.modes.describe(),
                                "dxfeed sent a spot the filter's mode rule excludes; dropped"
                            );
                            return Vec::new();
                        }
                        let origin = Some(format!(
                            "{}  ·  filter: {}",
                            describe_origin(&spot_event.spot),
                            self.modes.describe()
                        ));
                        let spot = Spot {
                            origin,
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

/// Who reported a cluster spot and what dxfeed made of it, e.g.
/// "cluster: KU8Y (human) · 1 report · dxfeed mode DIG". The newest
/// three reporters are named.
fn describe_origin(spot: &dxfeed::model::DxSpot) -> String {
    use dxfeed::domain::OriginatorKind;
    let mut reporters: Vec<String> = Vec::new();
    for obs in spot.observations.iter().rev() {
        let kind = match obs.originator_kind {
            OriginatorKind::Human => "human",
            OriginatorKind::Skimmer => "skimmer",
            OriginatorKind::Unknown => "unknown",
        };
        let entry = format!("{} ({kind})", obs.originator.0);
        if !reporters.contains(&entry) {
            reporters.push(entry);
        }
        if reporters.len() == 3 {
            break;
        }
    }
    let count = spot.observations.len();
    format!(
        "cluster: {}  ·  {count} report{}  ·  dxfeed mode {:?}",
        reporters.join(", "),
        if count == 1 { "" } else { "s" },
        spot.mode,
    )
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
    fn a_spot_outside_the_filters_modes_is_never_shown() {
        use dxfeed::domain::{Band, DxMode};
        use dxfeed::model::SpotEventKind;
        let cw_only = ModeRule { file: Some("f.json".into()), allow: vec![DxMode::CW], deny: vec![] };
        assert_eq!(cw_only.describe(), "CW only (f.json)");
        assert!(ModeRule::default().describe().starts_with("none"));
        assert_eq!(ModeRule { file: Some("f.json".into()), ..ModeRule::default() }.describe(), "any mode (f.json)");
        assert!(ModeRule::default().permits(DxMode::SSB));

        // The test events are CW: shown under a CW-only rule, and under
        // an SSB-only one neither shown nor remembered.
        let mut t = SpotTracker { modes: cw_only, ..SpotTracker::default() };
        let shown =
            t.translate(spot_event("K1ABC", Band::B20, 14_025_000, 14_025_000, SpotEventKind::New));
        assert_eq!(shown.len(), 1);
        let SpotEvent::Spot(spot) = &shown[0] else { panic!("expected a spot") };
        assert!(spot.origin.as_deref().unwrap().ends_with("filter: CW only (f.json)"), "{:?}", spot.origin);

        let ssb_only = ModeRule { file: Some("f.json".into()), allow: vec![DxMode::SSB], deny: vec![] };
        let mut t = SpotTracker { modes: ssb_only, ..SpotTracker::default() };
        assert!(
            t.translate(spot_event("K1ABC", Band::B20, 14_025_000, 14_025_000, SpotEventKind::New))
                .is_empty()
        );
        assert!(
            t.translate(spot_event("K1ABC", Band::B20, 14_025_000, 14_025_000, SpotEventKind::Withdraw))
                .is_empty()
        );
    }

    #[test]
    fn the_shipped_cw_filter_has_no_unknown_keys() {
        let raw = include_str!("../../docs/spot-filter-cw.json");
        assert_eq!(unknown_keys_in(raw), Vec::<String>::new());
    }

    #[test]
    fn misplaced_and_misspelt_filter_keys_are_found() {
        let raw = r#"{ "mode_allow": ["CW"], "rf": { "modes": ["CW"], "band_allow": [] } }"#;
        assert_eq!(unknown_keys_in(raw), ["mode_allow", "rf.modes"]);
    }

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
