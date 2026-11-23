//! Parks on the Air spots in the Spots pane.
//!
//! POTA spots are shown alongside DX cluster spots and marked as POTA.
//! An activator the cluster also has on the same band is one row: the
//! cluster's, with the park added. Nothing about parks is stored, and
//! POTA spots carry no worked/needed status of their own.

use app_config::PotaConfig;
use chrono::{DateTime, Utc};
use pota_feed::PotaSpot;
use radio_core::Band;
use spot_feed::Spot;

/// A POTA spot currently shown, with when its activator first appeared
/// on that band. The feed's own time moves every time the activator is
/// re-spotted; this does not, so the row keeps its place in the table.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Activation {
    pub spot: PotaSpot,
    pub first_seen: DateTime<Utc>,
}

impl Activation {
    fn band(&self) -> Option<Band> {
        Band::from_freq_hz(self.spot.freq_hz as i64)
    }

    /// What the Spots table shows for the park: reference, name, region.
    pub(super) fn park_label(&self) -> String {
        let mut label = self.spot.reference.clone();
        if let Some(name) = &self.spot.park_name {
            label.push(' ');
            label.push_str(name);
        }
        if let Some(location) = &self.spot.location {
            label.push_str(&format!(" ({location})"));
        }
        label
    }

    /// This spot as a row of its own, for when the cluster doesn't have it.
    fn as_spot(&self) -> Spot {
        Spot {
            call: self.spot.call.clone(),
            freq_hz: self.spot.freq_hz,
            mode: self.spot.mode.clone(),
            comment: self.spot.comment.clone(),
            spotted_at: self.spot.spotted_at,
            first_seen: self.first_seen,
            origin: Some("POTA spots feed (not the DX cluster)".into()),
        }
    }
}

/// The `[pota]` band and mode filters. Empty means "all".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PotaFilter {
    bands: Vec<Band>,
    modes: Vec<String>,
}

impl PotaFilter {
    /// Reads the filters from the config. A band name slogger doesn't
    /// know is left out and reported, so a typo narrows nothing silently.
    pub(super) fn from_config(cfg: &PotaConfig) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut bands = Vec::new();
        for name in &cfg.bands {
            match Band::from_adif(name.trim()) {
                Some(band) => bands.push(band),
                None => problems.push(format!("[pota] bands: \"{name}\" is not a band")),
            }
        }
        let modes = cfg
            .modes
            .iter()
            .map(|m| m.trim().to_ascii_uppercase())
            .filter(|m| !m.is_empty())
            .collect();
        (Self { bands, modes }, problems)
    }

    pub(super) fn accepts(&self, spot: &PotaSpot) -> bool {
        let band_ok = self.bands.is_empty()
            || Band::from_freq_hz(spot.freq_hz as i64).is_some_and(|b| self.bands.contains(&b));
        let mode_ok = self.modes.is_empty()
            || spot
                .mode
                .as_ref()
                .is_some_and(|m| self.modes.iter().any(|want| want.eq_ignore_ascii_case(m)));
        band_ok && mode_ok
    }
}

/// The list to show after a poll: `incoming` without QRT spots and
/// without anything the filter excludes. An activator already shown on
/// the same band keeps its `first_seen`.
pub(super) fn refresh(
    previous: &[Activation],
    incoming: Vec<PotaSpot>,
    filter: &PotaFilter,
) -> Vec<Activation> {
    incoming
        .into_iter()
        .filter(|spot| !spot.qrt && filter.accepts(spot))
        .map(|spot| {
            let band = Band::from_freq_hz(spot.freq_hz as i64);
            let first_seen = previous
                .iter()
                .find(|old| old.spot.call == spot.call && old.band() == band)
                .map_or(spot.spotted_at, |old| old.first_seen);
            Activation { spot, first_seen }
        })
        .collect()
}

/// The activation behind a row, if any: same callsign, same band.
pub(super) fn activation_for<'a>(
    spot: &Spot,
    activations: &[&'a Activation],
) -> Option<&'a Activation> {
    let band = Band::from_freq_hz(spot.freq_hz as i64);
    activations
        .iter()
        .copied()
        .find(|a| a.spot.call.eq_ignore_ascii_case(&spot.call) && a.band() == band)
}

/// Rows for the activations the cluster doesn't have: every activation
/// with no cluster spot for the same callsign on the same band.
pub(super) fn unmatched(activations: &[&Activation], cluster: &[&Spot]) -> Vec<Spot> {
    activations
        .iter()
        .filter(|a| {
            !cluster.iter().any(|s| {
                s.call.eq_ignore_ascii_case(&a.spot.call)
                    && Band::from_freq_hz(s.freq_hz as i64) == a.band()
            })
        })
        .map(|a| a.as_spot())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes_ago: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-23T18:00:00Z").unwrap().to_utc()
            - chrono::Duration::minutes(minutes_ago)
    }

    fn pota(call: &str, khz: u64, mode: &str, minutes_ago: i64) -> PotaSpot {
        PotaSpot {
            call: call.into(),
            freq_hz: khz * 1000,
            mode: Some(mode.into()),
            reference: "US-8390".into(),
            park_name: Some("Mystic Lakes State Park".into()),
            location: Some("US-MA".into()),
            comment: None,
            spotted_at: at(minutes_ago),
            qrt: false,
        }
    }

    fn cluster(call: &str, khz: u64) -> Spot {
        Spot {
            call: call.into(),
            freq_hz: khz * 1000,
            mode: None,
            comment: None,
            spotted_at: at(0),
            first_seen: at(0),
            origin: None,
        }
    }

    fn filter(bands: &[&str], modes: &[&str]) -> (PotaFilter, Vec<String>) {
        PotaFilter::from_config(&PotaConfig {
            enabled: true,
            bands: bands.iter().map(|s| s.to_string()).collect(),
            modes: modes.iter().map(|s| s.to_string()).collect(),
            refresh_interval_secs: None,
        })
    }

    fn calls(list: &[Activation]) -> Vec<&str> {
        list.iter().map(|a| a.spot.call.as_str()).collect()
    }

    #[test]
    fn no_filters_means_everything() {
        let (f, problems) = filter(&[], &[]);
        assert!(problems.is_empty());
        assert!(f.accepts(&pota("WC1D", 18_072, "CW", 0)));
        assert!(f.accepts(&PotaSpot { mode: None, ..pota("WC1D", 18_072, "CW", 0) }));
    }

    #[test]
    fn filters_by_band_and_mode_in_any_case() {
        let (f, problems) = filter(&["20M", "40m"], &["cw"]);
        assert!(problems.is_empty());
        assert!(f.accepts(&pota("A", 14_030, "CW", 0)));
        assert!(f.accepts(&pota("A", 7_030, "CW", 0)));
        assert!(!f.accepts(&pota("A", 18_072, "CW", 0)), "17 m is not listed");
        assert!(!f.accepts(&pota("A", 14_250, "SSB", 0)), "SSB is not listed");
        // With a mode filter, a spot that names no mode can't match it.
        assert!(!f.accepts(&PotaSpot { mode: None, ..pota("A", 14_030, "CW", 0) }));
    }

    #[test]
    fn an_unknown_band_name_is_reported_and_ignored() {
        let (f, problems) = filter(&["20m", "twenty"], &[]);
        assert_eq!(problems, ["[pota] bands: \"twenty\" is not a band"]);
        assert!(f.accepts(&pota("A", 14_030, "CW", 0)));
        assert!(!f.accepts(&pota("A", 7_030, "CW", 0)));
    }

    #[test]
    fn refresh_drops_qrt_and_filtered_spots() {
        let (f, _) = filter(&[], &["CW"]);
        let incoming = vec![
            pota("KEEP", 14_030, "CW", 1),
            PotaSpot { qrt: true, ..pota("GONE", 14_040, "CW", 1) },
            pota("PHONE", 14_250, "SSB", 1),
        ];
        assert_eq!(calls(&refresh(&[], incoming, &f)), ["KEEP"]);
    }

    #[test]
    fn a_re_spotted_activator_keeps_its_first_seen() {
        let f = PotaFilter::default();
        let first = refresh(&[], vec![pota("WC1D", 18_072, "CW", 20)], &f);
        assert_eq!(first[0].first_seen, at(20));

        // Re-spotted a little up the band: same activation.
        let again = refresh(&first, vec![pota("WC1D", 18_074, "CW", 2)], &f);
        assert_eq!(again[0].spot.spotted_at, at(2));
        assert_eq!(again[0].first_seen, at(20));

        // Moved to another band: a new activation as far as the table goes.
        let moved = refresh(&again, vec![pota("WC1D", 14_030, "CW", 1)], &f);
        assert_eq!(moved[0].first_seen, at(1));
    }

    #[test]
    fn a_cluster_spot_on_the_same_band_takes_the_park() {
        let list = refresh(&[], vec![pota("WC1D", 18_072, "CW", 5)], &PotaFilter::default());
        let live: Vec<&Activation> = list.iter().collect();

        // RBN heard them 200 Hz away: still the same station.
        let same_band = cluster("wc1d", 18_072);
        let found = activation_for(&same_band, &live).expect("merged");
        assert_eq!(found.park_label(), "US-8390 Mystic Lakes State Park (US-MA)");
        assert!(unmatched(&live, &[&same_band]).is_empty(), "one row, not two");

        // The cluster has them on another band only: two separate rows.
        let other_band = cluster("WC1D", 14_030);
        assert!(activation_for(&other_band, &live).is_none());
        let alone = unmatched(&live, &[&other_band]);
        assert_eq!(alone.len(), 1);
        assert_eq!((alone[0].call.as_str(), alone[0].freq_hz), ("WC1D", 18_072_000));
        assert_eq!(alone[0].first_seen, at(5));
    }

    #[test]
    fn a_merged_row_loses_its_park_when_the_activator_goes_qrt() {
        let f = PotaFilter::default();
        let before = refresh(&[], vec![pota("WC1D", 18_072, "CW", 5)], &f);
        let after = refresh(
            &before,
            vec![PotaSpot { qrt: true, ..pota("WC1D", 18_072, "CW", 0) }],
            &f,
        );
        let live: Vec<&Activation> = after.iter().collect();
        let row = cluster("WC1D", 18_072);
        // The cluster row stays (it is the cluster's); the park is gone,
        // and no POTA-only row appears either.
        assert!(activation_for(&row, &live).is_none());
        assert!(unmatched(&live, &[&row]).is_empty());
        assert!(unmatched(&live, &[]).is_empty());
    }

    #[test]
    fn park_label_copes_with_missing_parts() {
        let bare = Activation {
            spot: PotaSpot { park_name: None, location: None, ..pota("A", 14_030, "CW", 0) },
            first_seen: at(0),
        };
        assert_eq!(bare.park_label(), "US-8390");
    }
}
