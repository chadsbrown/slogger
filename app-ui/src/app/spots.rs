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
        SpotStatus::Unknown => "",
        SpotStatus::Worked => "wkd",
        SpotStatus::NeededBand => "NEW",
    }
}

/// Everything the Spots table shows about a spot beyond the raw feed data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SpotInfo {
    pub status: SpotStatus,
    pub band: Option<Band>,
    pub dxcc_id: Option<u16>,
    pub country: Option<String>,
    /// The park, when this is a POTA activator: "US-8390 Park Name (US-MA)".
    pub pota: Option<String>,
}

/// Resolves a spot once and derives its band, entity and need status.
pub(super) fn describe_spot(
    spot: &Spot,
    resolver: Option<&dyn Resolver>,
    worked_by_band: &std::collections::BTreeMap<Band, std::collections::HashSet<u16>>,
) -> SpotInfo {
    let band = Band::from_freq_hz(spot.freq_hz as i64);
    let resolution = resolver.and_then(|r| {
        let call = Callsign::parse(&spot.call).ok()?;
        r.resolve(&call)
    });
    let dxcc_id = resolution.as_ref().and_then(|r| r.dxcc_id);
    let country = resolution.and_then(|r| r.country);
    let status = match (dxcc_id, band) {
        (Some(id), Some(band)) => {
            if worked_by_band.get(&band).is_some_and(|set| set.contains(&id)) {
                SpotStatus::Worked
            } else {
                SpotStatus::NeededBand
            }
        }
        _ => SpotStatus::Unknown,
    };
    SpotInfo {
        status,
        band,
        dxcc_id,
        country,
        pota: None,
    }
}

/// A column of the Spots table the operator can sort by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotColumn {
    Status,
    /// POTA activators first when ascending.
    Pota,
    Age,
    Call,
    Freq,
    Band,
    Mode,
    Entity,
    Comment,
}

/// The Spots table's sort: a column and a direction. `None` in
/// `App::spots_sort` means the default order (see [`sort_spots`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpotSort {
    pub column: SpotColumn,
    pub ascending: bool,
}

impl SpotSort {
    /// What clicking `column`'s header does: ascending, then descending,
    /// then back to the default order.
    pub(super) fn clicked(current: Option<SpotSort>, column: SpotColumn) -> Option<SpotSort> {
        match current {
            Some(s) if s.column == column && s.ascending => Some(SpotSort {
                column,
                ascending: false,
            }),
            Some(s) if s.column == column => None,
            _ => Some(SpotSort {
                column,
                ascending: true,
            }),
        }
    }
}

/// Orders the Spots table.
///
/// With no sort chosen, spots are listed by when they first appeared,
/// newest first. That order is stable: a spot that gets re-reported
/// keeps its row and only its age changes. Sorting by a column orders by
/// that column, with the default order breaking ties. Sorting by age is
/// the one choice that moves a row when its spot is re-reported.
pub(super) fn sort_spots(rows: &mut [(&Spot, SpotInfo)], sort: Option<SpotSort>) {
    use std::cmp::Ordering;
    let default_order = |a: &(&Spot, SpotInfo), b: &(&Spot, SpotInfo)| {
        b.0.first_seen
            .cmp(&a.0.first_seen)
            .then_with(|| a.0.call.cmp(&b.0.call))
            .then_with(|| a.0.freq_hz.cmp(&b.0.freq_hz))
    };
    let Some(sort) = sort else {
        rows.sort_by(default_order);
        return;
    };
    // NEW first when ascending: the rows the operator is hunting for.
    let status_rank = |s: SpotStatus| match s {
        SpotStatus::NeededBand => 0,
        SpotStatus::Unknown => 1,
        SpotStatus::Worked => 2,
    };
    rows.sort_by(|a, b| {
        let by_column = match sort.column {
            SpotColumn::Status => status_rank(a.1.status).cmp(&status_rank(b.1.status)),
            SpotColumn::Pota => a.1.pota.is_none().cmp(&b.1.pota.is_none()),
            // Ascending age = youngest first = latest report first.
            SpotColumn::Age => b.0.spotted_at.cmp(&a.0.spotted_at),
            SpotColumn::Call => a.0.call.cmp(&b.0.call),
            SpotColumn::Freq => a.0.freq_hz.cmp(&b.0.freq_hz),
            SpotColumn::Band => a.1.band.cmp(&b.1.band),
            SpotColumn::Mode => a.0.mode.cmp(&b.0.mode),
            SpotColumn::Entity => a.1.country.cmp(&b.1.country),
            SpotColumn::Comment => a.0.comment.cmp(&b.0.comment),
        };
        let by_column = if sort.ascending {
            by_column
        } else {
            by_column.reverse()
        };
        match by_column {
            Ordering::Equal => default_order(a, b),
            other => other,
        }
    });
}

#[cfg(test)]
pub(super) fn annotate_spot(
    spot: &Spot,
    resolver: &dyn Resolver,
    worked_by_band: &std::collections::BTreeMap<Band, std::collections::HashSet<u16>>,
) -> SpotStatus {
    describe_spot(spot, Some(resolver), worked_by_band).status
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
            first_seen: chrono::Utc::now(),
            origin: None,
        }
    }

    /// A spot first seen `first_min` minutes ago and last reported
    /// `last_min` minutes ago.
    fn timed(call: &str, freq_hz: u64, first_min: i64, last_min: i64) -> Spot {
        let now = chrono::Utc::now();
        Spot {
            first_seen: now - chrono::Duration::minutes(first_min),
            spotted_at: now - chrono::Duration::minutes(last_min),
            ..spot(call, freq_hz)
        }
    }

    fn info() -> SpotInfo {
        SpotInfo {
            status: SpotStatus::Unknown,
            band: None,
            dxcc_id: None,
            country: None,
            pota: None,
        }
    }

    fn calls(rows: &[(&Spot, SpotInfo)]) -> Vec<String> {
        rows.iter().map(|(s, _)| s.call.clone()).collect()
    }

    #[test]
    fn default_order_ignores_re_reports() {
        // B appeared most recently; A is older but was just re-reported.
        let a = timed("A1A", 14_010_000, 10, 0);
        let b = timed("B1B", 14_020_000, 2, 2);
        let c = timed("C1C", 14_030_000, 5, 1);
        let mut rows = vec![(&a, info()), (&b, info()), (&c, info())];
        sort_spots(&mut rows, None);
        assert_eq!(calls(&rows), ["B1B", "C1C", "A1A"]);
    }

    #[test]
    fn sorts_by_a_column_in_either_direction() {
        let a = timed("A1A", 14_030_000, 10, 0);
        let b = timed("B1B", 14_010_000, 2, 2);
        let c = timed("C1C", 14_020_000, 5, 1);
        let mut rows = vec![(&a, info()), (&b, info()), (&c, info())];

        let by = |column, ascending| Some(SpotSort { column, ascending });
        sort_spots(&mut rows, by(SpotColumn::Freq, true));
        assert_eq!(calls(&rows), ["B1B", "C1C", "A1A"]);
        sort_spots(&mut rows, by(SpotColumn::Freq, false));
        assert_eq!(calls(&rows), ["A1A", "C1C", "B1B"]);
        sort_spots(&mut rows, by(SpotColumn::Call, true));
        assert_eq!(calls(&rows), ["A1A", "B1B", "C1C"]);
        // Youngest report first.
        sort_spots(&mut rows, by(SpotColumn::Age, true));
        assert_eq!(calls(&rows), ["A1A", "C1C", "B1B"]);
    }

    #[test]
    fn header_click_cycles_ascending_descending_default() {
        let once = SpotSort::clicked(None, SpotColumn::Call);
        assert_eq!(once, Some(SpotSort { column: SpotColumn::Call, ascending: true }));
        let twice = SpotSort::clicked(once, SpotColumn::Call);
        assert_eq!(twice, Some(SpotSort { column: SpotColumn::Call, ascending: false }));
        assert_eq!(SpotSort::clicked(twice, SpotColumn::Call), None);
        // A different column starts over at ascending.
        assert_eq!(
            SpotSort::clicked(twice, SpotColumn::Freq),
            Some(SpotSort { column: SpotColumn::Freq, ascending: true })
        );
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
