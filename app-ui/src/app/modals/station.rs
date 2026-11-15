//! Station location editor: every field of a slogger station location,
//! plus the TQSL station location its QSOs are signed with for LoTW.
//!
//! TQSL's own list of locations is read when the editor opens. With it,
//! the TQSL field is a pick list and "Fill from TQSL" can copy details
//! across; without it (TQSL not installed, or its file somewhere else) the
//! field is free text and TQSL itself will refuse a name it doesn't know.
//! Nothing here writes the mapping on its own: a name preselected because
//! it matches is only stored when the operator presses Save.

use chrono::{DateTime, Utc};
use iced::widget::{
    Space, button, column, container, pick_list, row, rule, scrollable, text, text_input,
};
use iced::{Element, Length};
use lotw_sync::{TqslStationData, TqslStationLocation};
use radio_core::{Callsign, StationLocation, StationLocationId};

use super::super::message::Message;
use super::super::theme as style;

/// Widget id of the Name field, focused when the editor opens so typing
/// lands in the editor rather than the entry form underneath.
pub(in crate::app) const NAME_INPUT_ID: &str = "station-editor-name";

/// A text field of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    StationCall,
    OwnerCall,
    Grid,
    City,
    County,
    State,
    Country,
    CqZone,
    ItuZone,
    Iota,
    Latitude,
    Longitude,
    /// Free-text TQSL location name, used only when TQSL's list is unreadable.
    TqslName,
}

/// The form as typed. Kept as text so a half-typed value never has to be
/// rejected mid-keystroke; [`StationForm::to_location`] validates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StationForm {
    pub name: String,
    pub station_call: String,
    pub owner_call: String,
    pub grid: String,
    pub city: String,
    pub county: String,
    pub state: String,
    pub country: String,
    pub cq_zone: String,
    pub itu_zone: String,
    pub iota: String,
    pub latitude: String,
    pub longitude: String,
    /// TQSL station location name; empty means none.
    pub tqsl_name: String,
}

impl StationForm {
    pub fn from_location(loc: &StationLocation) -> Self {
        let s = |v: &Option<String>| v.clone().unwrap_or_default();
        let call = |c: &Option<Callsign>| {
            c.as_ref()
                .map(|c| c.as_str().to_string())
                .unwrap_or_default()
        };
        let num = |n: Option<u8>| n.map(|n| n.to_string()).unwrap_or_default();
        let deg = |d: Option<f64>| d.map(|d| d.to_string()).unwrap_or_default();
        Self {
            name: loc.name.clone(),
            station_call: call(&loc.station_callsign),
            owner_call: call(&loc.owner_callsign),
            grid: s(&loc.grid),
            city: s(&loc.city),
            county: s(&loc.county),
            state: s(&loc.state),
            country: s(&loc.country),
            cq_zone: num(loc.cq_zone),
            itu_zone: num(loc.itu_zone),
            iota: s(&loc.iota),
            latitude: deg(loc.latitude),
            longitude: deg(loc.longitude),
            tqsl_name: s(&loc.lotw_station_location),
        }
    }

    fn get(&self, field: Field) -> &str {
        match field {
            Field::Name => &self.name,
            Field::StationCall => &self.station_call,
            Field::OwnerCall => &self.owner_call,
            Field::Grid => &self.grid,
            Field::City => &self.city,
            Field::County => &self.county,
            Field::State => &self.state,
            Field::Country => &self.country,
            Field::CqZone => &self.cq_zone,
            Field::ItuZone => &self.itu_zone,
            Field::Iota => &self.iota,
            Field::Latitude => &self.latitude,
            Field::Longitude => &self.longitude,
            Field::TqslName => &self.tqsl_name,
        }
    }

    fn set(&mut self, field: Field, value: String) {
        let slot = match field {
            Field::Name => &mut self.name,
            Field::StationCall => &mut self.station_call,
            Field::OwnerCall => &mut self.owner_call,
            Field::Grid => &mut self.grid,
            Field::City => &mut self.city,
            Field::County => &mut self.county,
            Field::State => &mut self.state,
            Field::Country => &mut self.country,
            Field::CqZone => &mut self.cq_zone,
            Field::ItuZone => &mut self.itu_zone,
            Field::Iota => &mut self.iota,
            Field::Latitude => &mut self.latitude,
            Field::Longitude => &mut self.longitude,
            Field::TqslName => &mut self.tqsl_name,
        };
        *slot = value;
    }

    /// Validates the form and builds the location to store. `existing` is
    /// the row being edited (its id, creation time and the eQSL account
    /// the form doesn't show are kept); `None` makes a new location.
    pub fn to_location(
        &self,
        existing: Option<&StationLocation>,
        now: DateTime<Utc>,
    ) -> Result<StationLocation, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Name is required".into());
        }
        let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        let call = |label: &str, s: &str| {
            opt(s)
                .map(|c| Callsign::parse(&c).map_err(|e| format!("{label}: {e}")))
                .transpose()
        };
        let zone = |label: &str, s: &str, max: u8| {
            opt(s)
                .map(|z| match z.parse::<u8>() {
                    Ok(n) if (1..=max).contains(&n) => Ok(n),
                    _ => Err(format!("{label} must be a number from 1 to {max}")),
                })
                .transpose()
        };
        let degrees = |label: &str, s: &str, max: f64| {
            opt(s)
                .map(|d| match d.parse::<f64>() {
                    Ok(v) if v.abs() <= max => Ok(v),
                    _ => Err(format!(
                        "{label} must be decimal degrees between -{max} and {max}"
                    )),
                })
                .transpose()
        };
        Ok(StationLocation {
            id: existing
                .map(|e| e.id)
                .unwrap_or_else(StationLocationId::new),
            name: name.to_string(),
            station_callsign: call("Station callsign", &self.station_call)?,
            owner_callsign: call("Owner callsign", &self.owner_call)?,
            city: opt(&self.city),
            county: opt(&self.county),
            state: opt(&self.state),
            country: opt(&self.country),
            grid: opt(&self.grid),
            latitude: degrees("Latitude", &self.latitude, 90.0)?,
            longitude: degrees("Longitude", &self.longitude, 180.0)?,
            cq_zone: zone("CQ zone", &self.cq_zone, 40)?,
            itu_zone: zone("ITU zone", &self.itu_zone, 90)?,
            iota: opt(&self.iota),
            lotw_station_location: opt(&self.tqsl_name),
            eqsl_account: existing.and_then(|e| e.eqsl_account.clone()),
            created_at: existing.map(|e| e.created_at).unwrap_or(now),
            updated_at: now,
        })
    }

    /// Copies the TQSL location's details into the fields that are still
    /// empty — never over what the operator typed. Returns how many
    /// fields were filled.
    pub fn fill_from_tqsl(&mut self, tqsl: &TqslStationLocation) -> usize {
        let candidates = [
            (Field::StationCall, tqsl.call.clone()),
            (Field::Grid, tqsl.grid.clone()),
            (Field::County, tqsl.county.clone()),
            (Field::State, tqsl.state.clone()),
            (Field::CqZone, tqsl.cq_zone.map(|z| z.to_string())),
            (Field::ItuZone, tqsl.itu_zone.map(|z| z.to_string())),
            (Field::Iota, tqsl.iota.clone()),
        ];
        let mut filled = 0;
        for (field, value) in candidates {
            if let Some(value) = value.filter(|v| !v.trim().is_empty())
                && self.get(field).trim().is_empty()
            {
                self.set(field, value);
                filled += 1;
            }
        }
        filled
    }
}

/// The TQSL name to propose for a location whose TQSL field is empty: the
/// TQSL location named exactly like the slogger one, if there is one.
pub fn preselect_tqsl(name: &str, stored: &str, tqsl: &TqslStationData) -> Option<String> {
    if !stored.trim().is_empty() {
        return None;
    }
    tqsl.find(name.trim()).map(|l| l.name.clone())
}

/// Where the form and the chosen TQSL location disagree on callsign or
/// grid. Only a warning: a /P call or a finer grid square can be right.
/// Grids agree when one is a prefix of the other (`EM69` vs `EM69xv`).
pub fn tqsl_mismatches(form: &StationForm, tqsl: &TqslStationLocation) -> Vec<String> {
    let mut out = Vec::new();
    let call = form.station_call.trim();
    if let Some(t) = tqsl.call.as_deref()
        && !call.is_empty()
        && !call.eq_ignore_ascii_case(t.trim())
    {
        out.push(format!(
            "callsign {} differs from TQSL's {}",
            call.to_ascii_uppercase(),
            t
        ));
    }
    let grid = form.grid.trim().to_ascii_uppercase();
    if let Some(t) = tqsl.grid.as_deref() {
        let t_up = t.trim().to_ascii_uppercase();
        if !grid.is_empty() && !grid.starts_with(&t_up) && !t_up.starts_with(&grid) {
            out.push(format!(
                "grid {} differs from TQSL's {}",
                form.grid.trim(),
                t
            ));
        }
    }
    out
}

/// One entry in the TQSL pick list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TqslChoice {
    None,
    Name(String),
    /// The stored name, which TQSL's list doesn't contain.
    Missing(String),
}

impl std::fmt::Display for TqslChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("(none)"),
            Self::Name(n) => f.write_str(n),
            Self::Missing(n) => write!(f, "{n} (not found in TQSL)"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum StationEditorMsg {
    Changed(Field, String),
    TqslPicked(TqslChoice),
    FillFromTqsl,
    Save,
}

/// The editor's state, held in `Modal::StationEditor` while it is open.
#[derive(Debug, Clone)]
pub struct StationEditor {
    /// The location being edited; `None` when creating one.
    pub editing: Option<StationLocation>,
    pub form: StationForm,
    pub tqsl: TqslStationData,
    /// The operator has chosen in the TQSL field themselves; from then on
    /// renaming the location no longer proposes a TQSL match.
    tqsl_touched: bool,
    /// The TQSL field holds a proposal from [`preselect_tqsl`], not yet
    /// saved.
    preselected: bool,
    pub error: Option<String>,
    /// A transient note, e.g. what "Fill from TQSL" did.
    note: Option<String>,
    pub saving: bool,
}

impl StationEditor {
    pub fn new(editing: Option<StationLocation>, tqsl: TqslStationData) -> Self {
        let mut form = editing
            .as_ref()
            .map(StationForm::from_location)
            .unwrap_or_default();
        let preselect = preselect_tqsl(&form.name, &form.tqsl_name, &tqsl);
        let preselected = preselect.is_some();
        if let Some(name) = preselect {
            form.tqsl_name = name;
        }
        Self {
            editing,
            form,
            tqsl,
            tqsl_touched: false,
            preselected,
            error: None,
            note: None,
            saving: false,
        }
    }

    /// Applies every message except `Save`, which the app handles because
    /// it needs the repository.
    pub fn update(&mut self, msg: StationEditorMsg) {
        self.note = None;
        match msg {
            StationEditorMsg::Changed(field, value) => {
                if field == Field::TqslName {
                    self.tqsl_touched = true;
                    self.preselected = false;
                }
                self.form.set(field, value);
                if field == Field::Name
                    && !self.tqsl_touched
                    && (self.preselected || self.form.tqsl_name.is_empty())
                {
                    let proposal = preselect_tqsl(&self.form.name, "", &self.tqsl);
                    self.preselected = proposal.is_some();
                    self.form.tqsl_name = proposal.unwrap_or_default();
                }
            }
            StationEditorMsg::TqslPicked(choice) => {
                self.tqsl_touched = true;
                self.preselected = false;
                self.form.tqsl_name = match choice {
                    TqslChoice::None => String::new(),
                    TqslChoice::Name(n) | TqslChoice::Missing(n) => n,
                };
            }
            StationEditorMsg::FillFromTqsl => {
                if let Some(t) = self.selected_tqsl().cloned() {
                    let n = self.form.fill_from_tqsl(&t);
                    self.note = Some(match n {
                        0 => "nothing to fill: those fields already have values".into(),
                        1 => "filled 1 empty field from TQSL".into(),
                        n => format!("filled {n} empty fields from TQSL"),
                    });
                }
            }
            StationEditorMsg::Save => {}
        }
    }

    /// The TQSL location the form currently names, if TQSL has it.
    fn selected_tqsl(&self) -> Option<&TqslStationLocation> {
        self.tqsl.find(self.form.tqsl_name.trim())
    }

    fn choices(&self) -> (Vec<TqslChoice>, TqslChoice) {
        let names = self.tqsl.locations().unwrap_or_default();
        let mut options = vec![TqslChoice::None];
        options.extend(names.iter().map(|l| TqslChoice::Name(l.name.clone())));
        let current = self.form.tqsl_name.trim();
        let selected = if current.is_empty() {
            TqslChoice::None
        } else if names.iter().any(|l| l.name == current) {
            TqslChoice::Name(current.to_string())
        } else {
            let missing = TqslChoice::Missing(current.to_string());
            options.push(missing.clone());
            missing
        };
        (options, selected)
    }
}

pub(super) fn view(editor: &StationEditor) -> Element<'_, Message> {
    let field = |caption: &'static str, f: Field| -> Element<'_, Message> {
        let mut input = text_input("", editor.form.get(f))
            .on_input(move |s| Message::StationEditor(StationEditorMsg::Changed(f, s)))
            .on_submit(Message::StationEditor(StationEditorMsg::Save))
            .size(style::ROW_TEXT)
            .padding([3, 6]);
        if f == Field::Name {
            input = input.id(NAME_INPUT_ID);
        }
        column![style::label(caption), input]
            .spacing(2)
            .width(Length::Fill)
            .into()
    };

    let mut body = column![
        field("NAME", Field::Name),
        row![
            field("STATION CALLSIGN", Field::StationCall),
            field("OWNER CALLSIGN", Field::OwnerCall)
        ]
        .spacing(12),
        row![field("GRID", Field::Grid), field("IOTA", Field::Iota)].spacing(12),
        row![field("CITY", Field::City), field("COUNTY", Field::County)].spacing(12),
        row![
            field("STATE", Field::State),
            field("COUNTRY", Field::Country)
        ]
        .spacing(12),
        row![
            field("CQ ZONE", Field::CqZone),
            field("ITU ZONE", Field::ItuZone)
        ]
        .spacing(12),
        row![
            field("LATITUDE", Field::Latitude),
            field("LONGITUDE", Field::Longitude)
        ]
        .spacing(12),
        rule::horizontal(1),
        style::label("TQSL LOCATION (used to sign this location's QSOs for LoTW)"),
    ]
    .spacing(10);

    let fill = button(text("Fill from TQSL").size(style::ROW_TEXT))
        .padding([4, 10])
        .style(button::secondary)
        .on_press_maybe(
            editor
                .selected_tqsl()
                .map(|_| Message::StationEditor(StationEditorMsg::FillFromTqsl)),
        );

    let tqsl_input: Element<'_, Message> = match &editor.tqsl {
        TqslStationData::Read { .. } => {
            let (options, selected) = editor.choices();
            pick_list(options, Some(selected), |c| {
                Message::StationEditor(StationEditorMsg::TqslPicked(c))
            })
            .text_size(style::ROW_TEXT)
            .padding([3, 6])
            .width(Length::Fill)
            .into()
        }
        TqslStationData::Unavailable { .. } => {
            text_input("TQSL station location name", &editor.form.tqsl_name)
                .on_input(|s| Message::StationEditor(StationEditorMsg::Changed(Field::TqslName, s)))
                .on_submit(Message::StationEditor(StationEditorMsg::Save))
                .size(style::ROW_TEXT)
                .padding([3, 6])
                .width(Length::Fill)
                .into()
        }
    };
    body = body.push(
        row![tqsl_input, fill]
            .spacing(8)
            .align_y(iced::Alignment::Center),
    );

    let path = editor
        .tqsl
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(no default path)".into());
    let small = |s: String| text(s).size(style::LABEL_TEXT);
    match &editor.tqsl {
        TqslStationData::Read { locations, .. } => {
            body = body.push(
                small(format!(
                    "{} TQSL locations read from {path}",
                    locations.len()
                ))
                .style(style::muted),
            );
            let current = editor.form.tqsl_name.trim();
            if !current.is_empty() && editor.selected_tqsl().is_none() {
                body = body.push(
                    small(format!(
                        "\"{current}\" is not found in TQSL — this location's QSOs will be held back from LoTW"
                    ))
                    .style(text::danger),
                );
            }
        }
        TqslStationData::Unavailable { reason, .. } => {
            body = body.push(
                small(format!(
                    "TQSL's location list could not be read ({path}: {reason}). Type the name \
                     exactly as TQSL shows it; set [lotw] tqsl_station_data if TQSL keeps it elsewhere."
                ))
                .style(text::warning),
            );
        }
    }
    if editor.preselected {
        body = body.push(
            small(
                "Proposed because TQSL has a location with this name — press Save to use it."
                    .into(),
            )
            .style(style::muted),
        );
    }
    if editor.form.tqsl_name.trim().is_empty() {
        body = body.push(
            small("Without a TQSL location this location's QSOs are held back from LoTW.".into())
                .style(style::muted),
        );
    }
    if let Some(t) = editor.selected_tqsl() {
        for warning in tqsl_mismatches(&editor.form, t) {
            body = body.push(small(format!("Note: {warning}")).style(text::warning));
        }
    }
    if let Some(note) = &editor.note {
        body = body.push(small(note.clone()).style(style::muted));
    }
    if let Some(err) = &editor.error {
        body = body.push(text(err.as_str()).size(style::ROW_TEXT).style(text::danger));
    }

    let body = scrollable(container(body).padding(iced::Padding {
        right: 14.0,
        ..iced::Padding::ZERO
    }))
    .width(Length::Fill)
    .height(Length::Fill);

    let save = button(text(if editor.saving { "Saving…" } else { "Save" }).size(style::ROW_TEXT))
        .padding([4, 14])
        .style(button::primary)
        .on_press_maybe((!editor.saving).then_some(Message::StationEditor(StationEditorMsg::Save)));
    let cancel = button(text("Cancel").size(style::ROW_TEXT))
        .padding([4, 14])
        .style(button::secondary)
        .on_press(Message::CloseModal);
    let footer = row![Space::new().width(Length::Fill), cancel, save]
        .spacing(8)
        .align_y(iced::Alignment::Center);

    let title = if editor.editing.is_some() {
        "Edit station location"
    } else {
        "New station location"
    };
    super::frame(title, body.into(), Some(footer.into()), 640.0, Some(640.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tqsl_loc(name: &str) -> TqslStationLocation {
        TqslStationLocation {
            name: name.into(),
            call: Some("W1ABC".into()),
            grid: Some("FN31pr".into()),
            dxcc: Some(291),
            cq_zone: Some(5),
            itu_zone: Some(8),
            state: Some("CT".into()),
            county: Some("Hartford".into()),
            iota: None,
        }
    }

    fn read(names: &[&str]) -> TqslStationData {
        TqslStationData::Read {
            path: PathBuf::from("/x/station_data"),
            locations: names.iter().map(|n| tqsl_loc(n)).collect(),
        }
    }

    fn unavailable() -> TqslStationData {
        TqslStationData::Unavailable {
            path: Some(PathBuf::from("/x/station_data")),
            reason: "not found".into(),
        }
    }

    fn location(name: &str) -> StationLocation {
        let now = Utc::now();
        StationLocation {
            id: StationLocationId::new(),
            name: name.into(),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            owner_callsign: None,
            city: None,
            county: None,
            state: None,
            country: None,
            grid: None,
            latitude: None,
            longitude: None,
            cq_zone: None,
            itu_zone: None,
            iota: None,
            lotw_station_location: None,
            eqsl_account: Some("W1ABC-eqsl".into()),
            created_at: now - chrono::Duration::days(3),
            updated_at: now - chrono::Duration::days(3),
        }
    }

    #[test]
    fn form_round_trips_and_keeps_what_it_does_not_show() {
        let mut loc = location("Home");
        loc.grid = Some("FN31pr".into());
        loc.cq_zone = Some(5);
        loc.latitude = Some(41.5);
        loc.lotw_station_location = Some("Home QTH".into());
        let form = StationForm::from_location(&loc);
        let now = Utc::now();
        let back = form.to_location(Some(&loc), now).unwrap();
        assert_eq!(back.id, loc.id);
        assert_eq!(back.created_at, loc.created_at);
        assert_eq!(back.updated_at, now);
        assert_eq!(back.eqsl_account.as_deref(), Some("W1ABC-eqsl"));
        assert_eq!(back.grid.as_deref(), Some("FN31pr"));
        assert_eq!(back.cq_zone, Some(5));
        assert_eq!(back.latitude, Some(41.5));
        assert_eq!(back.lotw_station_location.as_deref(), Some("Home QTH"));
    }

    #[test]
    fn form_trims_uppercases_calls_and_turns_blanks_into_none() {
        let form = StationForm {
            name: "  Field Day ".into(),
            station_call: " n9unx/p ".into(),
            grid: "  ".into(),
            tqsl_name: " ".into(),
            ..Default::default()
        };
        let loc = form.to_location(None, Utc::now()).unwrap();
        assert_eq!(loc.name, "Field Day");
        assert_eq!(loc.station_callsign.unwrap().as_str(), "N9UNX/P");
        assert_eq!(loc.owner_callsign, None);
        assert_eq!(loc.grid, None);
        assert_eq!(loc.lotw_station_location, None);
        assert_eq!(loc.eqsl_account, None);
    }

    #[test]
    fn form_rejects_bad_values_with_a_message() {
        let base = StationForm {
            name: "Home".into(),
            ..Default::default()
        };
        let err = |f: StationForm| f.to_location(None, Utc::now()).unwrap_err();
        assert!(err(StationForm::default()).contains("Name"));
        assert!(
            err(StationForm {
                station_call: "W1 ABC".into(),
                ..base.clone()
            })
            .starts_with("Station callsign")
        );
        assert!(
            err(StationForm {
                owner_call: "W1-ABC".into(),
                ..base.clone()
            })
            .starts_with("Owner callsign")
        );
        assert!(
            err(StationForm {
                cq_zone: "five".into(),
                ..base.clone()
            })
            .contains("CQ zone")
        );
        assert!(
            err(StationForm {
                itu_zone: "91".into(),
                ..base.clone()
            })
            .contains("ITU zone")
        );
        assert!(
            err(StationForm {
                latitude: "95".into(),
                ..base.clone()
            })
            .contains("Latitude")
        );
        assert!(
            err(StationForm {
                longitude: "x".into(),
                ..base.clone()
            })
            .contains("Longitude")
        );
    }

    #[test]
    fn fill_from_tqsl_only_fills_empty_fields() {
        let mut form = StationForm {
            name: "Home".into(),
            grid: "FN31".into(),
            cq_zone: " ".into(),
            ..Default::default()
        };
        let filled = form.fill_from_tqsl(&tqsl_loc("Home"));
        // call, county, state, CQ, ITU — not grid (typed), not IOTA (TQSL has none).
        assert_eq!(filled, 5);
        assert_eq!(form.grid, "FN31");
        assert_eq!(form.station_call, "W1ABC");
        assert_eq!(form.county, "Hartford");
        assert_eq!(form.state, "CT");
        assert_eq!(form.cq_zone, "5");
        assert_eq!(form.itu_zone, "8");
        assert_eq!(form.iota, "");
        assert_eq!(
            form.fill_from_tqsl(&tqsl_loc("Home")),
            0,
            "a second fill changes nothing"
        );
    }

    #[test]
    fn preselects_an_exact_name_match_only_when_nothing_is_stored() {
        let tqsl = read(&["Home", "Field Day"]);
        assert_eq!(preselect_tqsl("Home", "", &tqsl).as_deref(), Some("Home"));
        assert_eq!(preselect_tqsl("home", "", &tqsl), None, "exact match only");
        assert_eq!(preselect_tqsl("Home", "Field Day", &tqsl), None);
        assert_eq!(preselect_tqsl("Home", "", &unavailable()), None);
    }

    #[test]
    fn editor_proposes_but_never_stores_without_save() {
        let loc = location("Home");
        let editor = StationEditor::new(Some(loc.clone()), read(&["Home"]));
        assert_eq!(editor.form.tqsl_name, "Home");
        assert!(editor.preselected);
        // The proposal is in the form only; the location itself is untouched.
        assert_eq!(editor.editing.as_ref().unwrap().lotw_station_location, None);
        let saved = editor
            .form
            .to_location(editor.editing.as_ref(), Utc::now())
            .unwrap();
        assert_eq!(saved.lotw_station_location.as_deref(), Some("Home"));
    }

    #[test]
    fn renaming_follows_the_proposal_until_the_operator_picks() {
        let mut editor = StationEditor::new(None, read(&["Home", "Field Day"]));
        assert_eq!(editor.form.tqsl_name, "");
        editor.update(StationEditorMsg::Changed(Field::Name, "Field Day".into()));
        assert_eq!(editor.form.tqsl_name, "Field Day");
        editor.update(StationEditorMsg::Changed(Field::Name, "Field Day 2".into()));
        assert_eq!(
            editor.form.tqsl_name, "",
            "a proposal that no longer matches goes away"
        );

        editor.update(StationEditorMsg::TqslPicked(TqslChoice::None));
        editor.update(StationEditorMsg::Changed(Field::Name, "Home".into()));
        assert_eq!(editor.form.tqsl_name, "", "an explicit (none) sticks");
    }

    #[test]
    fn renaming_keeps_a_stored_mapping() {
        let mut loc = location("Home");
        loc.lotw_station_location = Some("Field Day".into());
        let mut editor = StationEditor::new(Some(loc), read(&["Home", "Field Day"]));
        editor.update(StationEditorMsg::Changed(Field::Name, "Home".into()));
        assert_eq!(editor.form.tqsl_name, "Field Day");
    }

    #[test]
    fn a_stored_name_missing_from_tqsl_is_offered_flagged() {
        let mut loc = location("Home");
        loc.lotw_station_location = Some("Old QTH".into());
        let editor = StationEditor::new(Some(loc), read(&["Home"]));
        let (options, selected) = editor.choices();
        assert_eq!(selected, TqslChoice::Missing("Old QTH".into()));
        assert_eq!(selected.to_string(), "Old QTH (not found in TQSL)");
        assert_eq!(
            options,
            [
                TqslChoice::None,
                TqslChoice::Name("Home".into()),
                TqslChoice::Missing("Old QTH".into())
            ]
        );
        assert!(editor.selected_tqsl().is_none());
    }

    #[test]
    fn mismatch_warnings_compare_call_and_grid_loosely() {
        let t = tqsl_loc("Home");
        let form = |call: &str, grid: &str| StationForm {
            station_call: call.into(),
            grid: grid.into(),
            ..Default::default()
        };
        assert!(tqsl_mismatches(&form("w1abc", "fn31PR"), &t).is_empty());
        assert!(
            tqsl_mismatches(&form("", "FN31"), &t).is_empty(),
            "a coarser grid agrees"
        );
        assert!(tqsl_mismatches(&form("", ""), &t).is_empty());
        let w = tqsl_mismatches(&form("N9UNX", "EM69"), &t);
        assert_eq!(w.len(), 2);
        assert!(w[0].contains("N9UNX") && w[1].contains("EM69"));
    }
}
