//! The data shaping behind the Operating view's CondX panel, kept apart
//! from the widgets so it can be tested.

use condx_feed::BandCondition;

/// How a band rating reads at a glance. The view maps these onto the
/// active theme's success / warning / danger colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RatingTone {
    Good,
    Fair,
    Poor,
}

/// Traffic-light reading of a hamqsl rating. Anything unrecognised gets
/// no tone (shown muted) rather than a guess.
pub(super) fn rating_tone(rating: &str) -> Option<RatingTone> {
    let lower = rating.to_ascii_lowercase();
    if lower.contains("excellent") || lower.contains("good") {
        Some(RatingTone::Good)
    } else if lower.contains("fair") {
        Some(RatingTone::Fair)
    } else if lower.contains("poor") {
        Some(RatingTone::Poor)
    } else {
        None
    }
}

/// One line of the Band / Day / Night table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BandRow {
    pub band: String,
    pub day: Option<String>,
    pub night: Option<String>,
}

/// Collapses hamqsl's flat condition list — each band group appears
/// twice, once for `time="day"` and once for `time="night"` — into one
/// row per band, in first-seen order so the table matches the feed.
pub(super) fn group_by_band(conds: &[BandCondition]) -> Vec<BandRow> {
    let mut out: Vec<BandRow> = Vec::new();
    for c in conds {
        let i = match out.iter().position(|r| r.band == c.band) {
            Some(i) => i,
            None => {
                out.push(BandRow {
                    band: c.band.clone(),
                    day: None,
                    night: None,
                });
                out.len() - 1
            }
        };
        let slot = if c.time.eq_ignore_ascii_case("day") {
            &mut out[i].day
        } else {
            &mut out[i].night
        };
        *slot = Some(c.rating.clone());
    }
    out
}

/// A figure for the stats grid, with a dash when the feed omitted it.
pub(super) fn number_or_dash(v: Option<u32>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "—".into())
}

/// A raw feed string for display, with a dash when the feed omitted it.
pub(super) fn text_or_dash(s: &str) -> String {
    if s.trim().is_empty() {
        "—".into()
    } else {
        s.to_string()
    }
}

/// The "Updated …" footer. hamqsl stamps its readings in GMT; amateur
/// radio says UTC. `None` when the feed sent no timestamp.
pub(super) fn updated_label(updated: &str) -> Option<String> {
    let updated = updated.trim();
    (!updated.is_empty()).then(|| format!("Updated {}", updated.replace("GMT", "UTC")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond(band: &str, time: &str, rating: &str) -> BandCondition {
        BandCondition {
            band: band.into(),
            time: time.into(),
            rating: rating.into(),
        }
    }

    #[test]
    fn ratings_map_to_traffic_lights() {
        assert_eq!(rating_tone("Good"), Some(RatingTone::Good));
        assert_eq!(rating_tone("EXCELLENT"), Some(RatingTone::Good));
        assert_eq!(rating_tone("Fair"), Some(RatingTone::Fair));
        assert_eq!(rating_tone("poor"), Some(RatingTone::Poor));
        assert_eq!(rating_tone("Band Closed"), None);
        assert_eq!(rating_tone("—"), None);
    }

    #[test]
    fn day_and_night_ratings_share_one_row_per_band_in_feed_order() {
        let rows = group_by_band(&[
            cond("80m-40m", "day", "Poor"),
            cond("30m-20m", "day", "Good"),
            cond("80m-40m", "night", "Fair"),
            cond("30m-20m", "night", "Good"),
        ]);
        assert_eq!(
            rows,
            vec![
                BandRow {
                    band: "80m-40m".into(),
                    day: Some("Poor".into()),
                    night: Some("Fair".into()),
                },
                BandRow {
                    band: "30m-20m".into(),
                    day: Some("Good".into()),
                    night: Some("Good".into()),
                },
            ]
        );
    }

    #[test]
    fn a_band_listed_for_only_night_leaves_day_empty() {
        let rows = group_by_band(&[cond("12m-10m", "night", "Poor")]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].day, None);
        assert_eq!(rows[0].night.as_deref(), Some("Poor"));
    }

    #[test]
    fn missing_values_show_a_dash() {
        assert_eq!(number_or_dash(Some(107)), "107");
        assert_eq!(number_or_dash(None), "—");
        assert_eq!(text_or_dash("B3.2"), "B3.2");
        assert_eq!(text_or_dash(""), "—");
    }

    #[test]
    fn updated_line_says_utc() {
        assert_eq!(
            updated_label("18 Apr 2026 2057 GMT").as_deref(),
            Some("Updated 18 Apr 2026 2057 UTC")
        );
        assert_eq!(updated_label("  "), None);
    }
}
