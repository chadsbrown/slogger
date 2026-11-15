//! What a confirming station told a QSL service about its own location,
//! set against the log.
//!
//! LoTW's confirmations (`qso_qsldetail=yes`) carry the other station's
//! DXCC entity, zones, grid, state, county and so on, as that station
//! gave them to LoTW. Slogger uses them to fill in what the log leaves
//! blank, and never to overwrite what it holds: the log may be right, or
//! may record what the operator actually heard. Where the two disagree
//! the difference is recorded for the operator to look at.
//!
//! Both are kept in `qso_field_provenance`, at most one row per QSO,
//! field and service:
//!
//! - `field_name` — the ADIF field the service sent (`DXCC`, `CONT`,
//!   `CQZ`, `ITUZ`, `IOTA`, `GRIDSQUARE`, `STATE`, `CNTY`, `PFX`), which
//!   is also what the operator recognises;
//! - `source` — `service_sync:<service>` (`FieldSource::ServiceSync`)
//!   when the column was filled, or a grid made more precise, from the
//!   service; `service_differs:<service>` (`FieldSource::ServiceDiffers`)
//!   when the service reports something else and the log's value was kept;
//! - `source_detail` — the service's value, normalised as stored.
//!
//! Everything here is pure; `qsl_sync` loads the inputs and writes the
//! result with the rest of a report.

use chrono::{DateTime, Utc};

use radio_core::{FieldSource, Qso, QsoId};

use crate::repository::{DetailFill, ProvenanceWrite};

/// The QSO columns a confirmation's detail can fill, named by the ADIF
/// field that carries them. `dxcc_prefix` and `province` are not among
/// them: LoTW sends neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetailField {
    /// `dxcc_id`
    Dxcc,
    /// `continent`
    Cont,
    /// `cq_zone`
    Cqz,
    /// `itu_zone`
    Ituz,
    /// `iota`
    Iota,
    /// `grid`
    Gridsquare,
    /// `state`
    State,
    /// `county`
    Cnty,
    /// `wpx_prefix`
    Pfx,
}

impl DetailField {
    /// Every field, in the order they are shown.
    pub const ALL: [DetailField; 9] = [
        Self::Dxcc,
        Self::Cont,
        Self::Cqz,
        Self::Ituz,
        Self::Iota,
        Self::Gridsquare,
        Self::State,
        Self::Cnty,
        Self::Pfx,
    ];

    /// The ADIF field name: what the service sends and what
    /// `qso_field_provenance.field_name` holds.
    pub fn adif_name(self) -> &'static str {
        match self {
            Self::Dxcc => "DXCC",
            Self::Cont => "CONT",
            Self::Cqz => "CQZ",
            Self::Ituz => "ITUZ",
            Self::Iota => "IOTA",
            Self::Gridsquare => "GRIDSQUARE",
            Self::State => "STATE",
            Self::Cnty => "CNTY",
            Self::Pfx => "PFX",
        }
    }

    /// Case-insensitive, as ADIF field names are.
    pub fn from_adif_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|f| f.adif_name().eq_ignore_ascii_case(name.trim()))
    }

    /// Stored as an integer column, so compared as a number.
    pub fn is_numeric(self) -> bool {
        matches!(self, Self::Dxcc | Self::Cqz | Self::Ituz)
    }
}

/// The nine columns of one QSO that `DetailField` names, typed as the
/// `Qso` holds them. Also what a fill writes: there, only the fields to
/// set are `Some`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocationDetail {
    pub dxcc_id: Option<u16>,
    pub continent: Option<String>,
    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    pub iota: Option<String>,
    pub grid: Option<String>,
    pub state: Option<String>,
    pub county: Option<String>,
    pub wpx_prefix: Option<String>,
}

impl LocationDetail {
    pub fn from_qso(q: &Qso) -> Self {
        Self {
            dxcc_id: q.dxcc_id,
            continent: q.continent.clone(),
            cq_zone: q.cq_zone,
            itu_zone: q.itu_zone,
            iota: q.iota.clone(),
            grid: q.grid.clone(),
            state: q.state.clone(),
            county: q.county.clone(),
            wpx_prefix: q.wpx_prefix.clone(),
        }
    }

    /// `field`'s value as text (numbers in decimal); `None` when blank.
    pub fn get(&self, field: DetailField) -> Option<String> {
        let value = match field {
            DetailField::Dxcc => self.dxcc_id.map(|v| v.to_string()),
            DetailField::Cont => self.continent.clone(),
            DetailField::Cqz => self.cq_zone.map(|v| v.to_string()),
            DetailField::Ituz => self.itu_zone.map(|v| v.to_string()),
            DetailField::Iota => self.iota.clone(),
            DetailField::Gridsquare => self.grid.clone(),
            DetailField::State => self.state.clone(),
            DetailField::Cnty => self.county.clone(),
            DetailField::Pfx => self.wpx_prefix.clone(),
        };
        value.filter(|v| !v.trim().is_empty())
    }

    /// Set `field` to a value `normalize` produced (so a numeric field's
    /// value parses).
    fn set(&mut self, field: DetailField, value: &str) {
        let text = || Some(value.to_string());
        match field {
            DetailField::Dxcc => self.dxcc_id = value.parse().ok(),
            DetailField::Cont => self.continent = text(),
            DetailField::Cqz => self.cq_zone = value.parse().ok(),
            DetailField::Ituz => self.itu_zone = value.parse().ok(),
            DetailField::Iota => self.iota = text(),
            DetailField::Gridsquare => self.grid = text(),
            DetailField::State => self.state = text(),
            DetailField::Cnty => self.county = text(),
            DetailField::Pfx => self.wpx_prefix = text(),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A service's raw value for `field` as the ADIF importer would store it
/// (see `record_to_command`): `DXCC` / `CQZ` / `ITUZ` as integers,
/// `CONT` and `PFX` upper-cased, the rest as given. Surrounding
/// whitespace is dropped. `None` when blank or, for a number, unreadable
/// — the importer would not fill the column from it either.
pub fn normalize(field: DetailField, raw: &str) -> Option<String> {
    let v = raw.trim();
    if v.is_empty() {
        return None;
    }
    match field {
        DetailField::Dxcc => v.parse::<u16>().ok().map(|n| n.to_string()),
        DetailField::Cqz | DetailField::Ituz => v.parse::<u8>().ok().map(|n| n.to_string()),
        DetailField::Cont | DetailField::Pfx => Some(v.to_ascii_uppercase()),
        _ => Some(v.to_string()),
    }
}

/// How the log's value of a field stands against a service's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    /// The log has nothing: take the service's value.
    Fill,
    /// The log's grid is a less precise form of the service's (`EM69`
    /// against `EM69xv`): take the service's — it adds detail, it
    /// doesn't contradict.
    Refine,
    /// The same value (ignoring case and surrounding space; numbers
    /// numerically; a grid the service gives less precisely than the log
    /// counts as the same).
    Agree,
    /// Something else. The log's value is kept.
    Differ,
}

/// Compare the log's value of `field` (`None` or blank: nothing) with
/// `reported`, a value `normalize` produced.
pub fn compare(field: DetailField, local: Option<&str>, reported: &str) -> Comparison {
    let Some(local) = local.map(str::trim).filter(|v| !v.is_empty()) else {
        return Comparison::Fill;
    };
    let reported = reported.trim();
    if field.is_numeric() {
        return match (local.parse::<u32>(), reported.parse::<u32>()) {
            (Ok(a), Ok(b)) if a == b => Comparison::Agree,
            _ => Comparison::Differ,
        };
    }
    if local.eq_ignore_ascii_case(reported) {
        return Comparison::Agree;
    }
    if field == DetailField::Gridsquare {
        let (l, r) = (local.to_ascii_uppercase(), reported.to_ascii_uppercase());
        if r.starts_with(&l) {
            return Comparison::Refine;
        }
        if l.starts_with(&r) {
            return Comparison::Agree;
        }
    }
    Comparison::Differ
}

/// One stored `qso_field_provenance` row.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldProvenance {
    pub field_name: String,
    pub source: FieldSource,
    pub source_detail: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// What one confirmation changes on one QSO.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DetailPlan {
    /// Columns to write; `None` when there are none.
    pub fill: Option<DetailFill>,
    pub provenance: Vec<ProvenanceWrite>,
    /// A newly recorded difference, or one whose service value changed.
    pub new_difference: bool,
}

/// Bring one QSO in line with what `service` reports about the other
/// station, given the QSO's current values and the service's rows
/// already stored for it (`existing`; rows from other sources are
/// ignored).
///
/// Per field the service gives a usable value for:
/// - log blank, or its grid a prefix of the service's: write the
///   service's value, and a `ServiceSync` row holding it;
/// - log different: keep it, and a `ServiceDiffers` row holding the
///   service's value;
/// - the same: no row is needed, but a `ServiceSync` row from an earlier
///   fill stays while the log still holds that value (it is where the
///   value came from), and a `ServiceDiffers` row goes — the operator has
///   brought the log into line.
///
/// A field the service no longer gives keeps a `ServiceSync` row (the
/// value still came from there) and loses a `ServiceDiffers` one (there
/// is no longer anything to differ from). That reading needs a record
/// that carries detail at all; `plan_sync` only calls this for those. Only rows that change are
/// written, so applying the same record twice changes nothing.
pub fn plan_detail(
    service: &str,
    qso_id: QsoId,
    local: &LocationDetail,
    reported: &[(DetailField, String)],
    existing: &[FieldProvenance],
) -> DetailPlan {
    let synced = FieldSource::ServiceSync(service.to_string());
    let differs = FieldSource::ServiceDiffers(service.to_string());
    let mut fill = LocationDetail::default();
    let mut plan = DetailPlan::default();

    for field in DetailField::ALL {
        let name = field.adif_name();
        let have = existing
            .iter()
            .find(|p| p.field_name == name && (p.source == synced || p.source == differs));
        let current = local.get(field);
        // First usable value wins, like a repeated field at import.
        let value = reported
            .iter()
            .filter(|(f, _)| *f == field)
            .find_map(|(_, raw)| normalize(field, raw));

        let keep_synced = || {
            have.filter(|p| {
                p.source == synced
                    && p.source_detail
                        .as_deref()
                        .is_some_and(|d| compare(field, current.as_deref(), d) == Comparison::Agree)
            })
            .map(|p| {
                (
                    p.source.clone(),
                    p.source_detail.clone().unwrap_or_default(),
                )
            })
        };
        let want: Option<(FieldSource, String)> = match value {
            None => have.filter(|p| p.source == synced).map(|p| {
                (
                    p.source.clone(),
                    p.source_detail.clone().unwrap_or_default(),
                )
            }),
            Some(v) => match compare(field, current.as_deref(), &v) {
                Comparison::Fill | Comparison::Refine => {
                    fill.set(field, &v);
                    Some((synced.clone(), v))
                }
                Comparison::Agree => keep_synced(),
                Comparison::Differ => Some((differs.clone(), v)),
            },
        };

        let had = have.map(|p| {
            (
                p.source.clone(),
                p.source_detail.clone().unwrap_or_default(),
            )
        });
        if want == had {
            continue;
        }
        if want.as_ref().is_some_and(|(s, _)| *s == differs) {
            plan.new_difference = true;
        }
        plan.provenance.push(ProvenanceWrite {
            qso_id,
            field_name: name.to_string(),
            replaces: have.map(|p| p.source.clone()),
            row: want,
        });
    }
    if !fill.is_empty() {
        plan.fill = Some(DetailFill {
            qso_id,
            values: fill,
        });
    }
    plan
}

/// How one of a service's stored rows reads against the QSO as it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceDetail {
    /// The log's value came from the service and is still that value.
    Filled { value: String },
    /// The service reports `reported`; the log has `local` (`None`: blank).
    Differs {
        reported: String,
        local: Option<String>,
    },
}

/// `service`'s rows for a QSO, for display, in `DetailField::ALL` order.
///
/// Read against the QSO's current values rather than taken at their word,
/// because an edit since the last sync may have changed the column (a
/// corrected call re-derives the entity fields): a filled value the log
/// no longer holds shows as a difference, and a difference the operator
/// has since resolved isn't shown. The rows themselves catch up the next
/// time the service's record for the QSO is synced.
pub fn service_details(
    service: &str,
    qso: &Qso,
    rows: &[FieldProvenance],
) -> Vec<(DetailField, ServiceDetail)> {
    let synced = FieldSource::ServiceSync(service.to_string());
    let differs = FieldSource::ServiceDiffers(service.to_string());
    let local = LocationDetail::from_qso(qso);
    let mut out = Vec::new();
    for field in DetailField::ALL {
        let Some(row) = rows.iter().find(|p| {
            p.field_name == field.adif_name() && (p.source == synced || p.source == differs)
        }) else {
            continue;
        };
        let Some(reported) = row.source_detail.clone().filter(|d| !d.trim().is_empty()) else {
            continue;
        };
        let current = local.get(field);
        let agrees = compare(field, current.as_deref(), &reported) == Comparison::Agree;
        let shown = match (row.source == synced, agrees) {
            (true, true) => ServiceDetail::Filled { value: reported },
            (false, true) => continue,
            (_, false) => ServiceDetail::Differs {
                reported,
                local: current,
            },
        };
        out.push((field, shown));
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::commands::CreateQsoCommand;
    use radio_core::Callsign;

    use DetailField::*;

    fn t() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 8, 12, 0, 0).unwrap()
    }

    #[test]
    fn field_names_are_the_adif_ones_both_ways() {
        let names: Vec<&str> = DetailField::ALL.iter().map(|f| f.adif_name()).collect();
        assert_eq!(
            names,
            [
                "DXCC",
                "CONT",
                "CQZ",
                "ITUZ",
                "IOTA",
                "GRIDSQUARE",
                "STATE",
                "CNTY",
                "PFX"
            ]
        );
        for f in DetailField::ALL {
            assert_eq!(DetailField::from_adif_name(f.adif_name()), Some(f));
        }
        assert_eq!(DetailField::from_adif_name("gridsquare"), Some(Gridsquare));
        assert_eq!(DetailField::from_adif_name("VUCC_GRIDS"), None);
        assert_eq!(DetailField::from_adif_name("APP_LoTW_NPSUNIT"), None);
    }

    #[test]
    fn normalize_matches_the_importer() {
        assert_eq!(normalize(Dxcc, " 291 ").as_deref(), Some("291"));
        assert_eq!(normalize(Dxcc, "0291").as_deref(), Some("291"));
        assert_eq!(normalize(Dxcc, "USA"), None);
        assert_eq!(normalize(Cqz, "05").as_deref(), Some("5"));
        assert_eq!(normalize(Ituz, "8").as_deref(), Some("8"));
        assert_eq!(normalize(Cqz, "300"), None, "doesn't fit the column");
        assert_eq!(normalize(Cont, "na").as_deref(), Some("NA"));
        assert_eq!(normalize(Pfx, "w1").as_deref(), Some("W1"));
        // As given (only trimmed): the importer doesn't touch these.
        assert_eq!(normalize(Gridsquare, "EM69xv").as_deref(), Some("EM69xv"));
        assert_eq!(normalize(State, "in").as_deref(), Some("in"));
        assert_eq!(normalize(Cnty, "IN,Marion").as_deref(), Some("IN,Marion"));
        assert_eq!(normalize(Iota, "NA-046").as_deref(), Some("NA-046"));
        assert_eq!(normalize(State, "   "), None);
        assert_eq!(normalize(Iota, ""), None);
    }

    #[test]
    fn compare_fills_blanks_and_keeps_what_the_log_has() {
        assert_eq!(compare(State, None, "IN"), Comparison::Fill);
        assert_eq!(compare(State, Some("  "), "IN"), Comparison::Fill);
        assert_eq!(compare(State, Some("IN"), "IN"), Comparison::Agree);
        assert_eq!(compare(State, Some(" in "), "IN"), Comparison::Agree);
        assert_eq!(compare(State, Some("IL"), "IN"), Comparison::Differ);
        assert_eq!(
            compare(Cnty, Some("in,marion"), "IN,Marion"),
            Comparison::Agree
        );
        assert_eq!(
            compare(Cnty, Some("IN,Marion"), "IN,Lake"),
            Comparison::Differ
        );
        assert_eq!(compare(Cont, Some("NA"), "SA"), Comparison::Differ);
        // Numbers numerically.
        assert_eq!(compare(Cqz, Some("5"), "05"), Comparison::Agree);
        assert_eq!(compare(Dxcc, Some("291"), "291"), Comparison::Agree);
        assert_eq!(compare(Dxcc, Some("291"), "1"), Comparison::Differ);
        assert_eq!(compare(Ituz, Some("8"), "7"), Comparison::Differ);
        // A prefix only means something for grids.
        assert_eq!(compare(Iota, Some("NA-04"), "NA-046"), Comparison::Differ);
        assert_eq!(compare(Pfx, Some("W"), "W1"), Comparison::Differ);
    }

    #[test]
    fn compare_grid_prefixes() {
        // Less precise locally: refine.
        assert_eq!(
            compare(Gridsquare, Some("EM69"), "EM69xv"),
            Comparison::Refine
        );
        assert_eq!(
            compare(Gridsquare, Some("em69"), "EM69XV"),
            Comparison::Refine
        );
        assert_eq!(compare(Gridsquare, Some("EM"), "EM69"), Comparison::Refine);
        // Less precise at the service: they agree.
        assert_eq!(
            compare(Gridsquare, Some("EM69xv"), "EM69"),
            Comparison::Agree
        );
        assert_eq!(
            compare(Gridsquare, Some("EM69XV"), "em69xv"),
            Comparison::Agree
        );
        // Another square, at either precision.
        assert_eq!(
            compare(Gridsquare, Some("EM69"), "EM79"),
            Comparison::Differ
        );
        assert_eq!(
            compare(Gridsquare, Some("EM69xv"), "EM69xw"),
            Comparison::Differ
        );
        assert_eq!(
            compare(Gridsquare, Some("EM69xv"), "EM70"),
            Comparison::Differ
        );
        assert_eq!(compare(Gridsquare, None, "EM69xv"), Comparison::Fill);
    }

    fn reported(pairs: &[(DetailField, &str)]) -> Vec<(DetailField, String)> {
        pairs.iter().map(|(f, v)| (*f, v.to_string())).collect()
    }

    fn row(field: &str, source: FieldSource, detail: &str) -> FieldProvenance {
        FieldProvenance {
            field_name: field.into(),
            source,
            source_detail: Some(detail.into()),
            created_at: t(),
        }
    }

    fn synced() -> FieldSource {
        FieldSource::ServiceSync("lotw".into())
    }

    fn differs() -> FieldSource {
        FieldSource::ServiceDiffers("lotw".into())
    }

    /// (field, replaces, new row) for each write, to compare compactly.
    fn writes(plan: &DetailPlan) -> Vec<(&str, Option<FieldSource>, Option<(FieldSource, &str)>)> {
        plan.provenance
            .iter()
            .map(|w| {
                (
                    w.field_name.as_str(),
                    w.replaces.clone(),
                    w.row.as_ref().map(|(s, d)| (s.clone(), d.as_str())),
                )
            })
            .collect()
    }

    #[test]
    fn plan_fills_refines_agrees_and_records_differences() {
        let id = QsoId::new();
        let local = LocationDetail {
            dxcc_id: Some(291),
            continent: Some("NA".into()),
            cq_zone: Some(4),
            grid: Some("EM69".into()),
            state: Some("IL".into()),
            ..Default::default()
        };
        let lotw = reported(&[
            (Dxcc, "291"),          // agrees
            (Cont, "na"),           // agrees once normalised
            (Cqz, "5"),             // differs
            (Ituz, "08"),           // fills, as a number
            (Gridsquare, "EM69xv"), // refines
            (State, "IN"),          // differs
            (Cnty, "IN,Marion"),    // fills
            (Pfx, "w9"),            // fills, upper-cased
            (Iota, " "),            // blank: nothing
        ]);
        let plan = plan_detail("lotw", id, &local, &lotw, &[]);
        assert_eq!(
            plan.fill,
            Some(DetailFill {
                qso_id: id,
                values: LocationDetail {
                    itu_zone: Some(8),
                    grid: Some("EM69xv".into()),
                    county: Some("IN,Marion".into()),
                    wpx_prefix: Some("W9".into()),
                    ..Default::default()
                },
            })
        );
        assert_eq!(
            writes(&plan),
            vec![
                ("CQZ", None, Some((differs(), "5"))),
                ("ITUZ", None, Some((synced(), "8"))),
                ("GRIDSQUARE", None, Some((synced(), "EM69xv"))),
                ("STATE", None, Some((differs(), "IN"))),
                ("CNTY", None, Some((synced(), "IN,Marion"))),
                ("PFX", None, Some((synced(), "W9"))),
            ]
        );
        assert!(plan.new_difference);
        assert!(plan.provenance.iter().all(|w| w.qso_id == id));
    }

    #[test]
    fn plan_is_idempotent_once_applied() {
        let id = QsoId::new();
        // The log after the first plan above was written.
        let local = LocationDetail {
            dxcc_id: Some(291),
            continent: Some("NA".into()),
            cq_zone: Some(4),
            itu_zone: Some(8),
            grid: Some("EM69xv".into()),
            state: Some("IL".into()),
            county: Some("IN,Marion".into()),
            wpx_prefix: Some("W9".into()),
            ..Default::default()
        };
        let rows = vec![
            row("CQZ", differs(), "5"),
            row("ITUZ", synced(), "8"),
            row("GRIDSQUARE", synced(), "EM69xv"),
            row("STATE", differs(), "IN"),
            row("CNTY", synced(), "IN,Marion"),
            row("PFX", synced(), "W9"),
        ];
        let lotw = reported(&[
            (Dxcc, "291"),
            (Cont, "NA"),
            (Cqz, "5"),
            (Ituz, "8"),
            (Gridsquare, "EM69xv"),
            (State, "IN"),
            (Cnty, "IN,Marion"),
            (Pfx, "W9"),
        ]);
        assert_eq!(
            plan_detail("lotw", id, &local, &lotw, &rows),
            DetailPlan::default()
        );
        // Other services' rows and other sources don't count as LoTW's.
        let mut others = rows.clone();
        others.push(row(
            "STATE",
            FieldSource::ServiceDiffers("eqsl".into()),
            "OH",
        ));
        others.push(row("IOTA", FieldSource::ImportedAdif, "NA-046"));
        assert_eq!(
            plan_detail("lotw", id, &local, &lotw, &others),
            DetailPlan::default()
        );
    }

    #[test]
    fn plan_updates_or_drops_rows_when_things_change() {
        let id = QsoId::new();
        let local = LocationDetail {
            state: Some("IN".into()),
            cq_zone: Some(4),
            grid: Some("EM69xv".into()),
            county: Some("IN,Marion".into()),
            ..Default::default()
        };
        let rows = vec![
            // The operator has since corrected the state to LoTW's.
            row("STATE", differs(), "IN"),
            // LoTW's zone changed.
            row("CQZ", differs(), "5"),
            // Filled earlier; LoTW now gives it less precisely: still ours.
            row("GRIDSQUARE", synced(), "EM69xv"),
            // Filled earlier; LoTW now says another county.
            row("CNTY", synced(), "IN,Marion"),
        ];
        let lotw = reported(&[
            (State, "IN"),
            (Cqz, "3"),
            (Gridsquare, "EM69"),
            (Cnty, "IN,Hamilton"),
        ]);
        let plan = plan_detail("lotw", id, &local, &lotw, &rows);
        assert_eq!(plan.fill, None, "nothing the log holds is overwritten");
        assert_eq!(
            writes(&plan),
            vec![
                ("CQZ", Some(differs()), Some((differs(), "3"))),
                ("STATE", Some(differs()), None),
                ("CNTY", Some(synced()), Some((differs(), "IN,Hamilton"))),
            ]
        );
        assert!(plan.new_difference);

        // Only a difference going away: not a new one.
        let plan = plan_detail("lotw", id, &local, &reported(&[(State, "IN")]), &rows);
        assert!(!plan.new_difference);
    }

    #[test]
    fn plan_when_the_service_stops_giving_a_field() {
        let id = QsoId::new();
        let local = LocationDetail {
            state: Some("IL".into()),
            county: Some("IN,Marion".into()),
            ..Default::default()
        };
        let rows = vec![
            row("STATE", differs(), "IN"),
            row("CNTY", synced(), "IN,Marion"),
        ];
        // A record with no detail, or only unusable detail.
        let plan = plan_detail("lotw", id, &local, &reported(&[(Cqz, "x")]), &rows);
        assert_eq!(writes(&plan), vec![("STATE", Some(differs()), None)]);
        assert_eq!(plan.fill, None);
    }

    #[test]
    fn plan_takes_the_first_usable_value_and_refills_a_cleared_column() {
        let id = QsoId::new();
        let lotw = reported(&[(Cqz, "zz"), (Cqz, "5"), (Cqz, "6")]);
        let plan = plan_detail("lotw", id, &LocationDetail::default(), &lotw, &[]);
        assert_eq!(plan.fill.unwrap().values.cq_zone, Some(5));

        // Cleared after a fill: filled again, the row is already right.
        let rows = vec![row("CQZ", synced(), "5")];
        let plan = plan_detail("lotw", id, &LocationDetail::default(), &lotw, &rows);
        assert_eq!(plan.fill.unwrap().values.cq_zone, Some(5));
        assert!(plan.provenance.is_empty());
    }

    fn qso(state: Option<&str>, grid: Option<&str>) -> Qso {
        let mut cmd = CreateQsoCommand::minimal(Callsign::parse("W1AW").unwrap(), t());
        cmd.state = state.map(String::from);
        cmd.grid = grid.map(String::from);
        cmd.cq_zone = Some(5);
        crate::service::qso_from_command(QsoId::new(), cmd, t(), t()).0
    }

    #[test]
    fn details_read_rows_against_the_qso_as_it_is_now() {
        let rows = vec![
            row("GRIDSQUARE", synced(), "EM69xv"),
            row("STATE", differs(), "IN"),
            row("CQZ", differs(), "4"),
            row("CNTY", synced(), "IN,Marion"),
            row("DXCC", FieldSource::ServiceDiffers("eqsl".into()), "1"),
        ];
        let shown = service_details("lotw", &qso(Some("IL"), Some("EM69XV")), &rows);
        assert_eq!(
            shown,
            vec![
                (
                    Cqz,
                    ServiceDetail::Differs {
                        reported: "4".into(),
                        local: Some("5".into())
                    }
                ),
                (
                    Gridsquare,
                    ServiceDetail::Filled {
                        value: "EM69xv".into()
                    }
                ),
                (
                    State,
                    ServiceDetail::Differs {
                        reported: "IN".into(),
                        local: Some("IL".into())
                    }
                ),
                // Filled, then the column was cleared.
                (
                    Cnty,
                    ServiceDetail::Differs {
                        reported: "IN,Marion".into(),
                        local: None
                    }
                ),
            ]
        );
        // The operator corrected the state to LoTW's: nothing to show.
        let shown = service_details("lotw", &qso(Some("in"), None), &rows);
        assert!(!shown.iter().any(|(f, _)| *f == State));
        assert!(service_details("lotw", &qso(None, None), &[]).is_empty());
    }
}
