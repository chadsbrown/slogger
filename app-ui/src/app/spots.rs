use logbook_domain::AwardsSnapshot;
use radio_core::{Band, Callsign};
use spot_feed::Spot;
use station_resolver::Resolver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpotStatus {
    /// Couldn't resolve callsign or derive band — be conservative.
    Unknown,
    /// Entity worked on this band already.
    Worked,
    /// Entity not yet worked on this band — DX-hunting target.
    NeededBand,
}

pub(super) fn status_label(s: SpotStatus) -> &'static str {
    match s {
        SpotStatus::Unknown => "  ?",
        SpotStatus::Worked => "wkd",
        SpotStatus::NeededBand => "NEW",
    }
}

pub(super) fn annotate_spot(
    spot: &Spot,
    resolver: &dyn Resolver,
    worked_by_band: &std::collections::BTreeMap<Band, std::collections::HashSet<u16>>,
) -> SpotStatus {
    let Ok(call) = Callsign::parse(&spot.call) else {
        return SpotStatus::Unknown;
    };
    let Some(res) = resolver.resolve(&call) else {
        return SpotStatus::Unknown;
    };
    let Some(dxcc_id) = res.dxcc_id else {
        return SpotStatus::Unknown;
    };
    let Some(band) = Band::from_freq_hz(spot.freq_hz as i64) else {
        return SpotStatus::Unknown;
    };
    let worked = worked_by_band
        .get(&band)
        .is_some_and(|set| set.contains(&dxcc_id));
    if worked {
        SpotStatus::Worked
    } else {
        SpotStatus::NeededBand
    }
}

pub(super) fn build_worked_by_band(
    snap: &AwardsSnapshot,
) -> std::collections::BTreeMap<Band, std::collections::HashSet<u16>> {
    let mut m = std::collections::BTreeMap::new();
    for (band, prog) in &snap.dxcc_by_band {
        let set: std::collections::HashSet<u16> = prog
            .units
            .iter()
            .filter_map(|u| u.key.parse::<u16>().ok())
            .collect();
        m.insert(*band, set);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashSet};

    /// Test resolver: maps a fixed set of calls to fixed dxcc_ids.
    #[derive(Debug)]
    struct StubResolver;

    impl Resolver for StubResolver {
        fn resolve(&self, call: &Callsign) -> Option<station_resolver::Resolution> {
            let id = match call.as_str() {
                "W1AW" => 291,
                "JA1NUT" => 339,
                "VE3X" => 1,
                _ => return None,
            };
            Some(station_resolver::Resolution {
                dxcc_id: Some(id),
                dxcc_prefix: None,
                country: None,
                continent: None,
                cq_zone: None,
                itu_zone: None,
                latitude: None,
                longitude: None,
            })
        }
    }

    fn spot(call: &str, freq_hz: u64) -> Spot {
        Spot {
            call: call.to_string(),
            freq_hz,
            mode: None,
            comment: None,
            spotted_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn worked_entity_on_same_band_is_marked_worked() {
        let mut worked = BTreeMap::new();
        let mut s = HashSet::new();
        s.insert(291u16);
        worked.insert(Band::M20, s);
        let status = annotate_spot(&spot("W1AW", 14_074_000), &StubResolver, &worked);
        assert_eq!(status, SpotStatus::Worked);
    }

    #[test]
    fn worked_entity_on_different_band_is_marked_needed() {
        let mut worked = BTreeMap::new();
        let mut s = HashSet::new();
        s.insert(291u16);
        worked.insert(Band::M20, s);
        // Same entity, but on 40m — needed for band-DXCC.
        let status = annotate_spot(&spot("W1AW", 7_074_000), &StubResolver, &worked);
        assert_eq!(status, SpotStatus::NeededBand);
    }

    #[test]
    fn unknown_callsign_is_unknown_not_needed() {
        let worked = BTreeMap::new();
        let status = annotate_spot(&spot("ZZ9ZZZ", 14_074_000), &StubResolver, &worked);
        assert_eq!(status, SpotStatus::Unknown);
    }

    #[test]
    fn out_of_band_freq_is_unknown() {
        let worked = BTreeMap::new();
        // 42 MHz is not a ham band.
        let status = annotate_spot(&spot("W1AW", 42_000_000), &StubResolver, &worked);
        assert_eq!(status, SpotStatus::Unknown);
    }
}
