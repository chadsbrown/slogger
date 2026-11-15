//! Solar / propagation conditions from N0NBH's feed at hamqsl.com.
//!
//! Polls `solarxml.php` on an interval and pushes each parsed
//! [`CondXSnapshot`] down an `mpsc::Receiver` for the UI to drain.
//!
//! Opt-in: when the config says disabled, no HTTP request is ever issued.
//! Failures (network, parse) are logged and retried on the next interval —
//! the operator sees a stale timestamp rather than a crash.

use std::time::Duration;

use quick_xml::Reader;
use quick_xml::events::Event;
use thiserror::Error;
use tokio::sync::mpsc;

const HAMQSL_URL: &str = "https://www.hamqsl.com/solarxml.php";
/// hamqsl updates roughly hourly, so polling faster buys nothing.
const DEFAULT_REFRESH_SECS: u64 = 3600;
/// Floor on the poll cadence so a typo can't hammer hamqsl.com.
const MIN_REFRESH_SECS: u64 = 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default)]
pub struct CondXConfig {
    pub enabled: bool,
    /// Override the default 60-minute poll cadence. Values below 60 s are
    /// clamped up.
    pub refresh_interval_secs: Option<u64>,
}

impl CondXConfig {
    /// The poll cadence actually used: the configured value or the
    /// default, never below the floor.
    pub fn refresh_interval(&self) -> Duration {
        Duration::from_secs(
            self.refresh_interval_secs
                .unwrap_or(DEFAULT_REFRESH_SECS)
                .max(MIN_REFRESH_SECS),
        )
    }
}

/// Parsed snapshot of the N0NBH feed. String fields hold the raw feed
/// value (e.g. `"UNSETTLD"`, `"B3.2"`, `"S2-S3"`) for display without
/// reinterpretation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CondXSnapshot {
    pub updated: String,
    pub sfi: Option<u32>,
    pub sunspots: Option<u32>,
    pub a_index: Option<u32>,
    pub k_index: Option<u32>,
    pub xray: String,
    pub geomag_field: String,
    pub signal_noise: String,
    pub conditions: Vec<BandCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandCondition {
    /// e.g. `"80m-40m"`
    pub band: String,
    /// `"day"` or `"night"`
    pub time: String,
    /// `"Poor"`, `"Fair"`, `"Good"`, or whatever the feed reports.
    pub rating: String,
}

#[derive(Debug, Error)]
pub enum CondXError {
    #[error("GET {HAMQSL_URL}: {0}")]
    Http(#[from] reqwest::Error),

    #[error("hamqsl XML parse error: {0}")]
    Parse(String),
}

/// Spawns the polling task. Returns `None` when the feature is disabled,
/// so the caller can skip subscribing at all. Must be called from within
/// a tokio runtime.
pub fn spawn_condx_feed(config: &CondXConfig) -> Option<mpsc::Receiver<CondXSnapshot>> {
    if !config.enabled {
        return None;
    }
    let (tx, rx) = mpsc::channel::<CondXSnapshot>(4);
    let interval = config.refresh_interval();

    tokio::spawn(async move {
        // Built once so reqwest reuses its connection pool across polls.
        let client = match reqwest::Client::builder().timeout(HTTP_TIMEOUT).build() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "condx: failed to build HTTP client; feed disabled");
                return;
            }
        };

        // The first tick fires immediately, so the panel fills at startup
        // rather than an hour later.
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            match fetch_and_parse(&client).await {
                Ok(snapshot) => {
                    tracing::debug!(sfi = ?snapshot.sfi, "condx: fetched snapshot");
                    if tx.send(snapshot).await.is_err() {
                        return; // receiver dropped
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        retry_secs = interval.as_secs(),
                        "condx: fetch failed; will retry next interval"
                    );
                }
            }
        }
    });
    Some(rx)
}

async fn fetch_and_parse(client: &reqwest::Client) -> Result<CondXSnapshot, CondXError> {
    let body = client
        .get(HAMQSL_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_solar_xml(&body)
}

fn parse_err(e: impl std::fmt::Display) -> CondXError {
    CondXError::Parse(e.to_string())
}

/// Parses the hamqsl `<solar><solardata>...</solardata></solar>` feed.
/// Unknown elements are skipped, which keeps us forward-compatible if
/// hamqsl adds fields. Malformed XML is an error.
fn parse_solar_xml(xml: &str) -> Result<CondXSnapshot, CondXError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut snapshot = CondXSnapshot::default();
    let mut buf = Vec::new();
    // The schema is flat within <solardata> except for the
    // <calculatedconditions><band name=… time=…/> list, so the open
    // element plus a band's attributes is all the state needed.
    let mut current_tag: Option<String> = None;
    let mut band_name: Option<String> = None;
    let mut band_time: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = std::str::from_utf8(e.name().as_ref())
                    .map_err(parse_err)?
                    .to_string();
                if name == "band" {
                    for attr in e.attributes().flatten() {
                        let key = std::str::from_utf8(attr.key.as_ref()).map_err(parse_err)?;
                        let val = attr.unescape_value().map_err(parse_err)?.into_owned();
                        match key {
                            "name" => band_name = Some(val),
                            "time" => band_time = Some(val),
                            _ => {}
                        }
                    }
                }
                current_tag = Some(name);
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"band" {
                    band_name = None;
                    band_time = None;
                }
                current_tag = None;
            }
            Ok(Event::Text(t)) => {
                let Some(tag) = &current_tag else { continue };
                let text = t.unescape().map_err(parse_err)?.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                apply_field(&mut snapshot, tag, &text, band_name.as_deref(), band_time.as_deref());
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(CondXError::Parse(format!(
                    "at byte {}: {e}",
                    reader.buffer_position()
                )));
            }
        }
        buf.clear();
    }

    Ok(snapshot)
}

fn apply_field(
    s: &mut CondXSnapshot,
    tag: &str,
    text: &str,
    band_name: Option<&str>,
    band_time: Option<&str>,
) {
    match tag {
        "updated" => s.updated = text.to_string(),
        "solarflux" => s.sfi = text.parse().ok(),
        "sunspots" => s.sunspots = text.parse().ok(),
        "aindex" => s.a_index = text.parse().ok(),
        "kindex" => s.k_index = text.parse().ok(),
        "xray" => s.xray = text.to_string(),
        "geomagfield" => s.geomag_field = text.to_string(),
        "signalnoise" => s.signal_noise = text.to_string(),
        "band" => {
            if let (Some(name), Some(time)) = (band_name, band_time) {
                s.conditions.push(BandCondition {
                    band: name.to_string(),
                    time: time.to_string(),
                    rating: text.to_string(),
                });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<solar>
  <solardata>
    <source url="http://www.hamqsl.com/solar.html">N0NBH</source>
    <updated>18 Apr 2026 2057 GMT</updated>
    <solarflux>107</solarflux>
    <aindex>4</aindex>
    <kindex>3</kindex>
    <xray>B3.2</xray>
    <sunspots>44</sunspots>
    <signalnoise>S2-S3</signalnoise>
    <geomagfield>UNSETTLD</geomagfield>
    <calculatedconditions>
      <band name="80m-40m" time="day">Poor</band>
      <band name="30m-20m" time="day">Good</band>
      <band name="17m-15m" time="day">Fair</band>
      <band name="12m-10m" time="day">Poor</band>
      <band name="80m-40m" time="night">Fair</band>
      <band name="30m-20m" time="night">Good</band>
      <band name="17m-15m" time="night">Fair</band>
      <band name="12m-10m" time="night">Poor</band>
    </calculatedconditions>
  </solardata>
</solar>
"#;

    #[test]
    fn parses_scalar_fields() {
        let s = parse_solar_xml(SAMPLE).unwrap();
        assert_eq!(s.updated, "18 Apr 2026 2057 GMT");
        assert_eq!(s.sfi, Some(107));
        assert_eq!(s.sunspots, Some(44));
        assert_eq!(s.a_index, Some(4));
        assert_eq!(s.k_index, Some(3));
        assert_eq!(s.xray, "B3.2");
        assert_eq!(s.geomag_field, "UNSETTLD");
        assert_eq!(s.signal_noise, "S2-S3");
    }

    #[test]
    fn parses_all_band_conditions() {
        let s = parse_solar_xml(SAMPLE).unwrap();
        assert_eq!(s.conditions.len(), 8);
        let get = |band: &str, time: &str| -> String {
            s.conditions
                .iter()
                .find(|c| c.band == band && c.time == time)
                .map(|c| c.rating.clone())
                .unwrap_or_default()
        };
        assert_eq!(get("80m-40m", "day"), "Poor");
        assert_eq!(get("80m-40m", "night"), "Fair");
        assert_eq!(get("30m-20m", "day"), "Good");
        assert_eq!(get("12m-10m", "night"), "Poor");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let xml = r#"<solar><solardata>
            <solarflux>99</solarflux>
            <totallynewfield>xyz</totallynewfield>
            <kindex>5</kindex>
        </solardata></solar>"#;
        let s = parse_solar_xml(xml).unwrap();
        assert_eq!(s.sfi, Some(99));
        assert_eq!(s.k_index, Some(5));
    }

    #[test]
    fn missing_numeric_field_leaves_none() {
        let xml = r#"<solar><solardata>
            <solarflux>nope</solarflux>
        </solardata></solar>"#;
        let s = parse_solar_xml(xml).unwrap();
        assert_eq!(s.sfi, None);
    }

    #[test]
    fn malformed_xml_is_an_error() {
        let xml = "<solar><solardata><solarflux>99</kindex></solardata></solar>";
        assert!(matches!(parse_solar_xml(xml), Err(CondXError::Parse(_))));
    }

    #[test]
    fn refresh_interval_defaults_to_an_hour_and_is_clamped() {
        let cfg = |secs| CondXConfig {
            enabled: true,
            refresh_interval_secs: secs,
        };
        assert_eq!(cfg(None).refresh_interval(), Duration::from_secs(3600));
        assert_eq!(cfg(Some(900)).refresh_interval(), Duration::from_secs(900));
        assert_eq!(cfg(Some(5)).refresh_interval(), Duration::from_secs(60));
    }

    /// Disabled must mean no task and no request. Called outside a tokio
    /// runtime: if it tried to spawn the poller this would panic.
    #[test]
    fn disabled_feed_spawns_nothing() {
        let cfg = CondXConfig {
            enabled: false,
            refresh_interval_secs: Some(60),
        };
        assert!(spawn_condx_feed(&cfg).is_none());
    }
}
