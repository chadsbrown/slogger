//! Parks on the Air activator spots from the public feed at pota.app.
//!
//! Polls `spot/activator` on an interval, but only while switched on:
//! when off, no request is ever issued. Each successful poll pushes the
//! whole current list down an `mpsc::Receiver`, so the consumer replaces
//! what it had rather than tracking additions and removals.
//!
//! The endpoint is the one the POTA website uses. It needs no key, but
//! it is not a versioned public API, so every field is optional here and
//! a spot missing what we need is skipped rather than failing the poll.

use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::{mpsc, watch};

const POTA_URL: &str = "https://api.pota.app/spot/activator";
const DEFAULT_REFRESH_SECS: u64 = 60;
/// Floor on the poll cadence so a typo can't hammer pota.app.
const MIN_REFRESH_SECS: u64 = 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default)]
pub struct PotaConfig {
    /// Whether polling starts switched on.
    pub enabled: bool,
    /// Override the default 60-second poll cadence. Values below 60 s
    /// are raised to 60.
    pub refresh_interval_secs: Option<u64>,
}

impl PotaConfig {
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

/// One activator spot.
#[derive(Debug, Clone, PartialEq)]
pub struct PotaSpot {
    /// The activator's callsign, upper-cased.
    pub call: String,
    pub freq_hz: u64,
    /// As the feed gives it, upper-cased: `CW`, `SSB`, `FT8`, …
    pub mode: Option<String>,
    /// Park reference, e.g. `US-8390`.
    pub reference: String,
    pub park_name: Option<String>,
    /// e.g. `US-MA`.
    pub location: Option<String>,
    pub comment: Option<String>,
    pub spotted_at: DateTime<Utc>,
    /// The spot says the activator has stopped.
    pub qrt: bool,
}

#[derive(Debug, Error)]
pub enum PotaError {
    #[error("GET {POTA_URL}: {0}")]
    Http(#[from] reqwest::Error),

    #[error("POTA spots parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Switches polling on and off while the task runs. Cheap to clone.
#[derive(Debug, Clone)]
pub struct PotaControl {
    enabled: watch::Sender<bool>,
}

impl PotaControl {
    /// Switching on polls straight away; switching off stops requests.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.send_replace(on);
    }
}

/// Spawns the polling task. Must be called from within a tokio runtime.
/// The task idles, making no requests, whenever it is switched off.
pub fn spawn_pota_feed(config: &PotaConfig) -> (mpsc::Receiver<Vec<PotaSpot>>, PotaControl) {
    let (tx, rx) = mpsc::channel::<Vec<PotaSpot>>(4);
    let (enabled_tx, mut enabled) = watch::channel(config.enabled);
    let interval = config.refresh_interval();

    tokio::spawn(async move {
        // Built once so reqwest reuses its connection pool across polls.
        let client = match reqwest::Client::builder().timeout(HTTP_TIMEOUT).build() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "pota: failed to build HTTP client; feed disabled");
                return;
            }
        };

        loop {
            // Wait here, silent, until switched on.
            if enabled.wait_for(|on| *on).await.is_err() {
                return; // control dropped
            }
            match fetch(&client).await {
                Ok(spots) => {
                    tracing::debug!(count = spots.len(), "pota: fetched spots");
                    if tx.send(spots).await.is_err() {
                        return; // receiver dropped
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        retry_secs = interval.as_secs(),
                        "pota: fetch failed; will retry next interval"
                    );
                }
            }
            // Sleep out the interval, but wake at once if switched off
            // (and then, above, at once when switched back on).
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                changed = enabled.wait_for(|on| !*on) => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
        }
    });
    (rx, PotaControl { enabled: enabled_tx })
}

async fn fetch(client: &reqwest::Client) -> Result<Vec<PotaSpot>, PotaError> {
    let body = client
        .get(POTA_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_spots(&body)
}

/// A spot as the feed sends it. Everything is optional so an added,
/// removed or null field never fails the whole list.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSpot {
    activator: Option<String>,
    /// Kilohertz, as a string: `"14250"`, `"18082.5"`.
    frequency: Option<String>,
    mode: Option<String>,
    reference: Option<String>,
    name: Option<String>,
    location_desc: Option<String>,
    comments: Option<String>,
    /// UTC with no zone: `"2026-11-23T17:29:08"`.
    spot_time: Option<String>,
    invalid: Option<serde_json::Value>,
}

/// Parses the feed's JSON list. Spots without a callsign, a usable
/// frequency, a park reference or a time, and spots the feed marks
/// invalid, are skipped.
pub fn parse_spots(json: &str) -> Result<Vec<PotaSpot>, PotaError> {
    let raw: Vec<RawSpot> = serde_json::from_str(json)?;
    Ok(raw.into_iter().filter_map(spot_from_raw).collect())
}

fn spot_from_raw(raw: RawSpot) -> Option<PotaSpot> {
    if raw.invalid.is_some_and(|v| !v.is_null() && v != false) {
        return None;
    }
    let call = non_empty(raw.activator)?.to_ascii_uppercase();
    let khz: f64 = raw.frequency?.trim().parse().ok()?;
    if !(khz.is_finite() && khz > 0.0) {
        return None;
    }
    let spotted_at = NaiveDateTime::parse_from_str(raw.spot_time?.trim(), "%Y-%m-%dT%H:%M:%S%.f")
        .ok()?
        .and_utc();
    let comment = non_empty(raw.comments);
    Some(PotaSpot {
        call,
        freq_hz: (khz * 1000.0).round() as u64,
        mode: non_empty(raw.mode).map(|m| m.to_ascii_uppercase()),
        reference: non_empty(raw.reference)?,
        park_name: non_empty(raw.name),
        location: non_empty(raw.location_desc),
        qrt: comment.as_deref().is_some_and(says_qrt),
        comment,
        spotted_at,
    })
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Whether a comment says the activator has stopped: "QRT" as a word of
/// its own, in any case ("QRT", "qrt, thanks all", "going QRT soon").
fn says_qrt(comment: &str) -> bool {
    comment
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case("QRT"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two spots copied from the live feed, plus the shapes that must be
    /// skipped or tolerated.
    const SAMPLE: &str = r#"[
      {"spotId": 61588948, "activator": "WC1D", "frequency": "18072", "mode": "CW",
       "reference": "US-8390", "parkName": null, "spotTime": "2026-11-23T17:29:08",
       "spotter": "K9IMM-#", "comments": "RBN 6 dB 22 WPM via K9IMM-#", "source": "RBN",
       "invalid": null, "name": "Mystic Lakes State Park", "locationDesc": "US-MA",
       "grid4": "FN42", "grid6": "FN42kk", "latitude": 42.4317, "longitude": -71.1477,
       "count": 50, "expire": 1781},
      {"activator": "k4abc", "frequency": "18082.5", "mode": "ssb", "reference": "US-0001",
       "spotTime": "2026-11-23T17:01:32", "comments": "QRT", "name": "", "newField": 7},
      {"activator": "N0CALL", "frequency": "", "reference": "US-0002",
       "spotTime": "2026-11-23T17:01:32"},
      {"activator": "N0REF", "frequency": "7030", "spotTime": "2026-11-23T17:01:32"},
      {"activator": "N0BAD", "frequency": "7030", "reference": "US-0003",
       "spotTime": "2026-11-23T17:01:32", "invalid": true}
    ]"#;

    #[test]
    fn parses_live_shaped_spots_and_skips_unusable_ones() {
        let spots = parse_spots(SAMPLE).unwrap();
        assert_eq!(spots.len(), 2, "{spots:?}");

        let first = &spots[0];
        assert_eq!(first.call, "WC1D");
        assert_eq!(first.freq_hz, 18_072_000);
        assert_eq!(first.mode.as_deref(), Some("CW"));
        assert_eq!(first.reference, "US-8390");
        assert_eq!(first.park_name.as_deref(), Some("Mystic Lakes State Park"));
        assert_eq!(first.location.as_deref(), Some("US-MA"));
        assert_eq!(first.spotted_at.to_rfc3339(), "2026-11-23T17:29:08+00:00");
        assert!(!first.qrt);

        let second = &spots[1];
        assert_eq!(second.call, "K4ABC");
        assert_eq!(second.freq_hz, 18_082_500);
        assert_eq!(second.mode.as_deref(), Some("SSB"));
        assert_eq!(second.park_name, None);
        assert!(second.qrt);
    }

    #[test]
    fn something_other_than_a_list_is_an_error() {
        assert!(parse_spots("{\"message\": \"nope\"}").is_err());
        assert!(parse_spots("<html>").is_err());
        assert_eq!(parse_spots("[]").unwrap(), Vec::new());
    }

    #[test]
    fn qrt_is_matched_as_a_word() {
        assert!(says_qrt("QRT"));
        assert!(says_qrt("qrt, thanks all"));
        assert!(says_qrt("Going QRT/next park"));
        assert!(!says_qrt("QRTP 5w"));
        assert!(!says_qrt("RBN 6 dB 22 WPM"));
    }

    #[test]
    fn refresh_interval_has_a_floor() {
        let cfg = |secs| PotaConfig { enabled: true, refresh_interval_secs: secs };
        assert_eq!(cfg(None).refresh_interval(), Duration::from_secs(60));
        assert_eq!(cfg(Some(5)).refresh_interval(), Duration::from_secs(60));
        assert_eq!(cfg(Some(300)).refresh_interval(), Duration::from_secs(300));
    }

    /// Fetches the real feed. Run by hand to check the endpoint still
    /// looks the way this crate expects:
    /// `cargo test -p pota-feed live_feed -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "makes a network request to pota.app"]
    async fn live_feed_parses() {
        let client = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build().unwrap();
        let body = client.get(POTA_URL).send().await.unwrap().text().await.unwrap();
        let listed = serde_json::from_str::<Vec<serde_json::Value>>(&body).unwrap().len();
        let spots = parse_spots(&body).unwrap();
        println!("{} of {listed} spots usable, {} QRT", spots.len(), spots.iter().filter(|s| s.qrt).count());
        for spot in spots.iter().take(5) {
            println!("{spot:?}");
        }
        // Nearly every listed spot should be usable; far fewer means the
        // feed's shape has changed.
        assert!(listed == 0 || spots.len() * 10 >= listed * 9);
    }
}
