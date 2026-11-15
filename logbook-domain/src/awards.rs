//! Award progress derived from the QSO log.
//!
//! All five computations consume `&[AwardQso]` (one repository fetch) and
//! return per-unit buckets. No materialization, no triggers — for slogger's
//! target log sizes (thousands to ~100k QSOs) recomputing on demand is
//! cheap and always correct.

use std::collections::BTreeMap;

use chrono::Datelike;

use radio_core::{Band, wpx_prefix};

use crate::repository::AwardQso;

/// One unit's status against an award (e.g. one DXCC entity, one US state,
/// one IOTA reference, one WPX prefix).
#[derive(Debug, Clone)]
pub struct AwardUnit {
    pub key: String,
    pub worked_count: usize,
    pub confirmed: bool,
    /// The award sponsor has granted credit for this unit (a QSO for it
    /// carries `CREDIT_GRANTED` for the award). Independent of
    /// `confirmed`: credit is granted on confirmations, but slogger may
    /// not hold the confirmation (a paper card, say).
    pub credited: bool,
}

/// Aggregate counts for an award, plus the per-unit detail.
#[derive(Debug, Clone, Default)]
pub struct AwardProgress {
    pub worked: usize,
    pub confirmed: usize,
    pub credited: usize,
    pub units: Vec<AwardUnit>,
}

impl AwardProgress {
    fn from_buckets(buckets: BTreeMap<String, BucketAccum>) -> Self {
        let mut units: Vec<AwardUnit> = buckets
            .into_iter()
            .map(|(k, b)| AwardUnit {
                key: k,
                worked_count: b.worked,
                confirmed: b.confirmed,
                credited: b.credited,
            })
            .collect();
        units.sort_by(|a, b| a.key.cmp(&b.key));
        let confirmed = units.iter().filter(|u| u.confirmed).count();
        let credited = units.iter().filter(|u| u.credited).count();
        Self {
            worked: units.len(),
            confirmed,
            credited,
            units,
        }
    }
}

#[derive(Default)]
struct BucketAccum {
    worked: usize,
    confirmed: bool,
    credited: bool,
}

impl BucketAccum {
    /// Count `q` towards this unit. `credit_award` names the award whose
    /// granted credit marks the unit credited (`None`: the award has no
    /// credit slogger tracks).
    fn add(&mut self, q: &AwardQso, credit_award: Option<&str>) {
        self.worked += 1;
        if q.lotw_confirmed {
            self.confirmed = true;
        }
        if credit_award.is_some_and(|a| q.has_granted_credit(a)) {
            self.credited = true;
        }
    }
}

fn bucket_by<F>(
    qsos: &[AwardQso],
    credit_award: Option<&str>,
    key_of: F,
) -> BTreeMap<String, BucketAccum>
where
    F: Fn(&AwardQso) -> Option<String>,
{
    let mut map: BTreeMap<String, BucketAccum> = BTreeMap::new();
    for q in qsos {
        if let Some(k) = key_of(q) {
            map.entry(k).or_default().add(q, credit_award);
        }
    }
    map
}

/// DXCC: distinct DXCC entity IDs worked. Requires `dxcc_id` populated —
/// QSOs with no resolver hit are excluded. Awards code can't make up data.
pub fn dxcc_progress(qsos: &[AwardQso]) -> AwardProgress {
    let buckets = bucket_by(qsos, Some("DXCC"), |q| q.dxcc_id.map(|id| id.to_string()));
    AwardProgress::from_buckets(buckets)
}

/// WAS: distinct US states worked. Filters to USA QSOs (DXCC 291) so a
/// stray `state = "ON"` on a Canadian QSO doesn't count.
pub fn was_progress(qsos: &[AwardQso]) -> AwardProgress {
    let buckets = bucket_by(qsos, Some("WAS"), |q| {
        if q.dxcc_id == Some(291) {
            q.state.clone().filter(|s| !s.is_empty())
        } else {
            None
        }
    });
    AwardProgress::from_buckets(buckets)
}

/// WPX: distinct callsign prefixes worked. Derived from `call` per CQ WPX
/// rules — see `radio_core::wpx_prefix`.
pub fn wpx_progress(qsos: &[AwardQso]) -> AwardProgress {
    let buckets = bucket_by(qsos, Some("WPX"), |q| wpx_prefix(q.call.as_str()));
    AwardProgress::from_buckets(buckets)
}

/// IOTA: distinct IOTA references worked. Honors whatever was logged in
/// the `iota` field — no rule validation.
pub fn iota_progress(qsos: &[AwardQso]) -> AwardProgress {
    let buckets = bucket_by(qsos, Some("IOTA"), |q| q.iota.clone().filter(|s| !s.is_empty()));
    AwardProgress::from_buckets(buckets)
}

/// CQ Marathon: distinct DXCC entities + CQ zones worked in a calendar
/// year. Returns separate counts so callers can display "31 entities, 4
/// zones" — Marathon score combines the two.
#[derive(Debug, Clone, Default)]
pub struct MarathonProgress {
    pub year: i32,
    pub entities: AwardProgress,
    pub zones: AwardProgress,
    pub qso_count: usize,
}

pub fn marathon_progress(qsos: &[AwardQso], year: i32) -> MarathonProgress {
    let in_year: Vec<&AwardQso> = qsos
        .iter()
        .filter(|q| q.qso_begin.year() == year)
        .collect();

    let entities = bucket_by_ref(&in_year, |q| q.dxcc_id.map(|id| id.to_string()));
    // CQ zones: derived from the AwardQso's `continent` + the rule that we
    // don't carry zone in the slim shape. For now compute entities only;
    // zone tracking requires extending AwardQso. Marathon-zones here is a
    // placeholder — callers can ignore until we wire cq_zone through.
    let zones: BTreeMap<String, BucketAccum> = BTreeMap::new();

    MarathonProgress {
        year,
        entities: AwardProgress::from_buckets(entities),
        zones: AwardProgress::from_buckets(zones),
        qso_count: in_year.len(),
    }
}

fn bucket_by_ref<F>(qsos: &[&AwardQso], key_of: F) -> BTreeMap<String, BucketAccum>
where
    F: Fn(&AwardQso) -> Option<String>,
{
    let mut map: BTreeMap<String, BucketAccum> = BTreeMap::new();
    for q in qsos {
        if let Some(k) = key_of(q) {
            // Marathon is a yearly competition, not an award with credit.
            map.entry(k).or_default().add(q, None);
        }
    }
    map
}

/// Filter QSOs to a band and run DXCC progress against the slice. Useful
/// for "DXCC on 20m" style breakdowns.
pub fn dxcc_progress_on_band(qsos: &[AwardQso], band: Band) -> AwardProgress {
    let mut buckets: BTreeMap<String, BucketAccum> = BTreeMap::new();
    for q in qsos.iter().filter(|q| q.band == Some(band)) {
        if let Some(id) = q.dxcc_id {
            buckets
                .entry(id.to_string())
                .or_default()
                .add(q, Some("DXCC_BAND"));
        }
    }
    AwardProgress::from_buckets(buckets)
}

/// All bands that appear in `qsos`, sorted lowest-frequency-first by ADIF
/// label order — handy for stable display.
fn bands_present(qsos: &[AwardQso]) -> Vec<Band> {
    let mut set: std::collections::BTreeSet<Band> = std::collections::BTreeSet::new();
    for q in qsos {
        if let Some(b) = q.band {
            set.insert(b);
        }
    }
    set.into_iter().collect()
}

/// Convenience: compute all five awards from one slice. Useful for the UI.
#[derive(Debug, Clone, Default)]
pub struct AwardsSnapshot {
    pub total_qsos: usize,
    pub dxcc: AwardProgress,
    pub was: AwardProgress,
    pub wpx: AwardProgress,
    pub iota: AwardProgress,
    pub marathon: MarathonProgress,
    /// DXCC entity counts split by band — only bands that have any QSO
    /// appear here. Use ordering of `Band` enum for stable display.
    pub dxcc_by_band: BTreeMap<Band, AwardProgress>,
}

pub fn snapshot(qsos: &[AwardQso], marathon_year: i32) -> AwardsSnapshot {
    let mut dxcc_by_band = BTreeMap::new();
    for band in bands_present(qsos) {
        dxcc_by_band.insert(band, dxcc_progress_on_band(qsos, band));
    }
    AwardsSnapshot {
        total_qsos: qsos.len(),
        dxcc: dxcc_progress(qsos),
        was: was_progress(qsos),
        wpx: wpx_progress(qsos),
        iota: iota_progress(qsos),
        marathon: marathon_progress(qsos, marathon_year),
        dxcc_by_band,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono::Utc;
    use radio_core::{Band, Callsign, Mode, QsoId};

    fn aq(
        call: &str,
        dxcc_id: Option<u16>,
        state: Option<&str>,
        iota: Option<&str>,
        confirmed: bool,
    ) -> AwardQso {
        AwardQso {
            id: QsoId::new(),
            call: Callsign::parse(call).unwrap(),
            qso_begin: Utc.with_ymd_and_hms(2026, 5, 8, 18, 0, 0).unwrap(),
            band: Some(Band::M20),
            mode: Some(Mode::FT8),
            dxcc_id,
            dxcc_prefix: None,
            continent: None,
            state: state.map(String::from),
            iota: iota.map(String::from),
            lotw_confirmed: confirmed,
            granted_credits: Vec::new(),
        }
    }

    #[test]
    fn dxcc_counts_distinct_entities() {
        let qsos = vec![
            aq("W1AW", Some(291), None, None, true),
            aq("W2A", Some(291), None, None, false), // same entity
            aq("VE3X", Some(1), None, None, false),
            aq("JA1", Some(339), None, None, true),
            aq("ZZ1", None, None, None, false), // no entity → excluded
        ];
        let p = dxcc_progress(&qsos);
        assert_eq!(p.worked, 3);
        assert_eq!(p.confirmed, 2);
    }

    #[test]
    fn was_filters_to_usa_only() {
        let qsos = vec![
            aq("W1AW", Some(291), Some("CT"), None, true),
            aq("K2A", Some(291), Some("NY"), None, false),
            aq("VE3X", Some(1), Some("ON"), None, false),  // Canadian, NOT a state
            aq("W3X", Some(291), Some("CT"), None, false), // same state
        ];
        let p = was_progress(&qsos);
        assert_eq!(p.worked, 2);
        assert_eq!(p.confirmed, 1);
    }

    #[test]
    fn wpx_counts_distinct_prefixes() {
        let qsos = vec![
            aq("W1AW", Some(291), None, None, true),
            aq("W1XYZ", Some(291), None, None, false), // same prefix W1
            aq("K2A", Some(291), None, None, false),
            aq("VE3XYZ", Some(1), None, None, true),
            aq("9V1A", Some(381), None, None, false),
        ];
        let p = wpx_progress(&qsos);
        assert_eq!(p.worked, 4);
        assert_eq!(p.confirmed, 2);
    }

    #[test]
    fn iota_counts_present_only() {
        let qsos = vec![
            aq("W1AW", Some(291), None, Some("NA-001"), true),
            aq("VE3X", Some(1), None, None, false),
            aq("EA8X", Some(29), None, Some("AF-004"), false),
        ];
        let p = iota_progress(&qsos);
        assert_eq!(p.worked, 2);
    }

    #[test]
    fn marathon_filters_by_year() {
        let mut qsos = vec![
            aq("W1AW", Some(291), None, None, true),
            aq("VE3X", Some(1), None, None, false),
            aq("JA1", Some(339), None, None, true),
        ];
        // Backdate one to a different year.
        qsos[0].qso_begin = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 0).unwrap();
        let m = marathon_progress(&qsos, 2026);
        assert_eq!(m.year, 2026);
        assert_eq!(m.qso_count, 2);
        assert_eq!(m.entities.worked, 2); // VE3X (1) + JA1 (339)
    }

    #[test]
    fn snapshot_runs_all_five() {
        let qsos = vec![
            aq("W1AW", Some(291), Some("CT"), None, true),
            aq("VE3X", Some(1), None, None, false),
            aq("JA1", Some(339), None, Some("AS-007"), true),
        ];
        let snap = snapshot(&qsos, 2026);
        assert_eq!(snap.total_qsos, 3);
        assert_eq!(snap.dxcc.worked, 3);
        assert_eq!(snap.dxcc.confirmed, 2);
        assert_eq!(snap.was.worked, 1);
        assert_eq!(snap.iota.worked, 1);
        assert!(snap.wpx.worked >= 3);
        assert_eq!(snap.marathon.entities.worked, 3);
    }

    #[test]
    fn dxcc_by_band_splits_correctly() {
        let mut qsos = vec![
            aq("W1AW", Some(291), None, None, true),
            aq("VE3X", Some(1), None, None, false),
            aq("JA1", Some(339), None, None, true),
        ];
        // First two on 20m, third on 40m.
        qsos[2].band = Some(Band::M40);
        let snap = snapshot(&qsos, 2026);
        assert_eq!(snap.dxcc_by_band.len(), 2);
        assert_eq!(snap.dxcc_by_band[&Band::M20].worked, 2);
        assert_eq!(snap.dxcc_by_band[&Band::M20].confirmed, 1);
        assert_eq!(snap.dxcc_by_band[&Band::M40].worked, 1);
        assert_eq!(snap.dxcc_by_band[&Band::M40].confirmed, 1);
    }

    #[test]
    fn credited_counts_units_with_granted_credit_for_the_award() {
        let mut qsos = vec![
            aq("W1AW", Some(291), Some("CT"), None, true),
            aq("K2A", Some(291), Some("NY"), None, false),
            aq("VE3X", Some(1), None, None, false),
            aq("JA1", Some(339), None, None, false),
        ];
        qsos[0].granted_credits = vec!["DXCC".into(), "DXCC_BAND".into(), "WAS".into()];
        // Credited without a LoTW confirmation (e.g. a paper card).
        qsos[2].granted_credits = vec!["DXCC".into()];
        // WAS credit only: not a DXCC credit.
        qsos[1].granted_credits = vec!["WAS".into()];
        qsos[3].band = Some(Band::M40);
        qsos[3].granted_credits = vec!["DXCC_BAND".into()];

        let snap = snapshot(&qsos, 2026);
        assert_eq!(snap.dxcc.worked, 3);
        assert_eq!(snap.dxcc.confirmed, 1, "confirmed semantics unchanged");
        assert_eq!(snap.dxcc.credited, 2); // 291 and 1; 339 only has band credit
        assert!(snap.dxcc.units.iter().find(|u| u.key == "1").unwrap().credited);
        assert!(!snap.dxcc.units.iter().find(|u| u.key == "339").unwrap().credited);
        assert_eq!(snap.was.credited, 2);
        assert_eq!(snap.dxcc_by_band[&Band::M20].credited, 1);
        assert_eq!(snap.dxcc_by_band[&Band::M40].credited, 1);
        assert_eq!(snap.wpx.credited, 0);
        assert_eq!(snap.marathon.entities.credited, 0);
    }
}
