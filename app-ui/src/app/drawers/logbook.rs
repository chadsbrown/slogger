//! Logbook Manager drawer. Search builder + result list + bulk actions.
//!
//! Drives `LogbookService::search_qsos` / `count_matching` /
//! `bulk_*_by_search` from the backend. The drawer reads its own
//! sub-state (`LogbookSearchState`) on `App`; mutation goes through
//! `Message::Logbook*` variants routed through update.rs.

use std::collections::HashSet;

use iced::widget::{
    button, checkbox, column, pick_list, row, rule, scrollable, text, text_input,
};
use iced::{Element, Length};
use chrono::{DateTime, Utc};
use logbook_domain::{QsoDetail, QsoSummary};
use radio_core::{Band, Mode, PropagationMode, QsoId, StationLocation};

use crate::app::constants::{BANDS, modes};
use crate::app::message::Message;
use crate::app::state::App;
use crate::app::theme;

/// Tri-state filter for the `lotw_confirmed` field on `QsoSearch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LotwFilter {
    #[default]
    Any,
    Confirmed,
    Unconfirmed,
}

impl LotwFilter {
    pub(crate) fn as_search_filter(self) -> Option<bool> {
        match self {
            LotwFilter::Any => None,
            LotwFilter::Confirmed => Some(true),
            LotwFilter::Unconfirmed => Some(false),
        }
    }
}

impl std::fmt::Display for LotwFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LotwFilter::Any => "Any",
            LotwFilter::Confirmed => "Confirmed",
            LotwFilter::Unconfirmed => "Unconfirmed",
        })
    }
}

const LOTW_FILTER_OPTIONS: &[LotwFilter] = &[
    LotwFilter::Any,
    LotwFilter::Confirmed,
    LotwFilter::Unconfirmed,
];

/// Option wrapper for Band so the pick_list can offer "Any" as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BandFilter(pub Option<Band>);

impl std::fmt::Display for BandFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(b) => f.write_str(b.as_adif()),
            None => f.write_str("Any"),
        }
    }
}

/// Option wrapper for Mode (same shape as BandFilter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeFilter(pub Option<Mode>);

impl std::fmt::Display for ModeFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(m) => f.write_str(m.as_adif()),
            None => f.write_str("Any"),
        }
    }
}

fn band_filter_options() -> Vec<BandFilter> {
    let mut opts = Vec::with_capacity(BANDS.len() + 1);
    opts.push(BandFilter(None));
    opts.extend(BANDS.iter().map(|b| BandFilter(Some(*b))));
    opts
}

fn mode_filter_options() -> Vec<ModeFilter> {
    let mut opts = Vec::with_capacity(modes().len() + 1);
    opts.push(ModeFilter(None));
    opts.extend(modes().into_iter().map(|m| ModeFilter(Some(m))));
    opts
}

/// Per-drawer state. Lives on `App` as `logbook_search`.
#[derive(Debug, Default)]
pub(crate) struct LogbookSearchState {
    pub call_prefix: String,
    pub exact_call: String,
    pub band: BandFilter,
    pub mode: ModeFilter,
    pub lotw_filter: LotwFilter,
    /// Last search results (lazy: only populated after Search pressed).
    pub results: Vec<QsoSummary>,
    /// Total matching count from `count_matching` — usually equals
    /// results.len() but caps at the search limit.
    pub total_count: Option<usize>,
    pub searching: bool,
    /// Per-row selection for bulk actions.
    pub selected: HashSet<QsoId>,
    /// Increments on each Search press; running bulk operations stamp
    /// their result with this generation so a stale result that arrives
    /// after the operator re-searched doesn't overwrite the new state.
    pub generation: u64,
    /// Status / outcome of the most recent bulk action, e.g.
    /// "deleted 12 QSOs".
    pub last_action: Option<String>,
}

impl Default for BandFilter {
    fn default() -> Self {
        Self(None)
    }
}

impl Default for ModeFilter {
    fn default() -> Self {
        Self(None)
    }
}

impl App {
    /// Search filter form — call_prefix, exact_call, band, mode, LotW
    /// confirmed tri-state, plus the Search button and result count.
    pub(in crate::app) fn view_logbook_search_pane(&self) -> Element<'_, Message> {
        let state = &self.logbook_search;
        column![
            row![
                text("Call prefix:").width(Length::Fixed(110.0)),
                text_input("JA*", &state.call_prefix)
                    .on_input(Message::LogbookCallPrefixChanged)
                    .on_submit(Message::LogbookSearchPressed)
                    .width(Length::Fixed(160.0)),
                text("Exact:").width(Length::Fixed(60.0)),
                text_input("JA1XYZ", &state.exact_call)
                    .on_input(Message::LogbookExactCallChanged)
                    .on_submit(Message::LogbookSearchPressed)
                    .width(Length::Fixed(140.0)),
            ]
            .spacing(8),
            row![
                text("Band:").width(Length::Fixed(110.0)),
                pick_list(
                    band_filter_options(),
                    Some(state.band),
                    Message::LogbookBandChanged,
                )
                .width(Length::Fixed(120.0)),
                text("Mode:").width(Length::Fixed(60.0)),
                pick_list(
                    mode_filter_options(),
                    Some(state.mode.clone()),
                    Message::LogbookModeChanged,
                )
                .width(Length::Fixed(120.0)),
            ]
            .spacing(8),
            row![
                text("LotW:").width(Length::Fixed(110.0)),
                pick_list(
                    LOTW_FILTER_OPTIONS,
                    Some(state.lotw_filter),
                    Message::LogbookLotwFilterChanged,
                )
                .width(Length::Fixed(140.0)),
            ]
            .spacing(8),
            row![
                button(text(if state.searching {
                    "Searching…"
                } else {
                    "Search"
                }))
                .on_press(Message::LogbookSearchPressed),
                {
                    let count_label = match state.total_count {
                        Some(n) if n > state.results.len() => {
                            format!("{} matches (showing {})", n, state.results.len())
                        }
                        Some(n) => format!("{n} matches"),
                        None => String::new(),
                    };
                    text(count_label)
                }
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(6)
        .into()
    }

    /// Bulk-actions toolbar + result list. Selection state lives on
    /// App, so this is purely render.
    pub(in crate::app) fn view_logbook_grid_pane(&self) -> Element<'_, Message> {
        let state = &self.logbook_search;

        let bulk_actions: Element<'_, Message> = if state.selected.is_empty() {
            column![].into()
        } else {
            row![
                text(format!("{} selected:", state.selected.len())),
                button(text("Soft delete"))
                    .on_press(Message::LogbookBulkDelete)
                    .style(button::danger),
                button(text("Mark uploaded → LotW"))
                    .on_press(Message::LogbookBulkMarkUploaded("lotw".into())),
                button(text("eQSL"))
                    .on_press(Message::LogbookBulkMarkUploaded("eqsl".into())),
                button(text("Club Log"))
                    .on_press(Message::LogbookBulkMarkUploaded("clublog".into())),
                button(text("QRZ"))
                    .on_press(Message::LogbookBulkMarkUploaded("qrz".into())),
                button(text("HRDLog"))
                    .on_press(Message::LogbookBulkMarkUploaded("hrdlog".into())),
                button(text("Clear")).on_press(Message::LogbookClearSelection),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into()
        };

        let action_status: Element<'_, Message> = match &state.last_action {
            Some(s) => text(s.as_str()).size(12).into(),
            None => column![].into(),
        };

        let mut list = column![row![
            text("").width(Length::Fixed(20.0)),
            text("Time UTC").width(Length::Fixed(160.0)),
            text("Call").width(Length::Fixed(100.0)),
            text("Band").width(Length::Fixed(50.0)),
            text("Mode").width(Length::Fixed(50.0)),
            text("DXCC").width(Length::Fill),
        ]
        .spacing(4)]
        .spacing(2);
        for q in &state.results {
            let id = q.id;
            let is_selected = state.selected.contains(&id);
            let summary_label = format!(
                "{:<19}  {:<10}  {:<5}  {:<5}  {}",
                q.qso_begin.format("%Y-%m-%d %H:%M:%S"),
                q.call.as_str(),
                q.band.map(|b| b.as_adif().to_string()).unwrap_or_default(),
                q.mode
                    .as_ref()
                    .map(|m| m.as_adif().to_string())
                    .unwrap_or_default(),
                q.dxcc_prefix.as_deref().unwrap_or(""),
            );
            list = list.push(
                row![
                    checkbox(is_selected).on_toggle(move |_| Message::LogbookRowToggle(id)),
                    button(text(summary_label).size(12))
                        .on_press(Message::QsoSelected(id))
                        .style(button::text),
                ]
                .spacing(4),
            );
        }

        column![bulk_actions, action_status, scrollable(list).height(Length::Fill)]
            .spacing(8)
            .into()
    }

    /// Detail pane. The top is the entry form bound to the selected QSO
    /// (the editable fields; saving goes through `update_qso`). Below it,
    /// read-only, is everything else stored about the QSO.
    pub(in crate::app) fn view_logbook_detail_pane(&self) -> Element<'_, Message> {
        if self.editing_qso.is_none() {
            return text("Select a QSO from the Results pane to view detail.")
                .size(12)
                .into();
        }
        let Some(detail) = &self.qso_detail else {
            // Still loading; the form fills in when the record arrives.
            return self.entry_view();
        };
        let mut rest = column![].spacing(12);
        for (title, rows) in detail_sections(detail, &self.station_locations) {
            if rows.is_empty() {
                continue;
            }
            let mut section = column![theme::label(title)].spacing(2);
            for (name, value) in rows {
                section = section.push(
                    row![
                        text(name)
                            .size(theme::ROW_TEXT)
                            .style(theme::muted)
                            .width(Length::Fixed(120.0)),
                        text(value).size(theme::ROW_TEXT).font(theme::MONO),
                    ]
                    .spacing(8),
                );
            }
            rest = rest.push(section);
        }
        column![
            self.entry_view(),
            rule::horizontal(1),
            scrollable(rest).height(Length::Fill).width(Length::Fill),
        ]
        .spacing(8)
        .into()
    }

    /// Tools pane — ADIF import path + Import button. Future: Export
    /// selected results to ADIF (operates on the Grid pane's checked
    /// rows via `export_adif`).
    pub(in crate::app) fn view_logbook_tools_pane(&self) -> Element<'_, Message> {
        let import_btn = {
            let btn = button(text(if self.importing { "Importing…" } else { "Import" }));
            if self.importing {
                btn
            } else {
                btn.on_press(Message::ImportPressed)
            }
        };
        column![
            text("ADIF import").size(13),
            row![
                text("File:").width(Length::Fixed(50.0)),
                text_input("/path/to/log.adi", &self.adif_path)
                    .on_input(Message::AdifPathChanged)
                    .width(Length::Fill),
                import_btn,
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            text("(ADIF export from selected results: coming next)")
                .size(11),
        ]
        .spacing(8)
        .into()
    }
}

// ---- QSO detail ----

type DetailSection = (&'static str, Vec<(String, String)>);

fn timestamp(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// Display name for a QSL service key as stored in the database.
fn service_name(key: &str) -> String {
    match key {
        "lotw" => "LoTW".into(),
        "eqsl" => "eQSL".into(),
        "clublog" => "Club Log".into(),
        "qrz" => "QRZ".into(),
        "hrdlog" => "HRDLog".into(),
        other => other.to_string(),
    }
}

/// Everything stored about a QSO beyond what the entry form edits,
/// grouped for display. Only fields that have a value are listed, so a
/// section can come back empty (the view skips those).
fn detail_sections(detail: &QsoDetail, locations: &[StationLocation]) -> Vec<DetailSection> {
    /// Adds a row when the value is present.
    fn push(rows: &mut Vec<(String, String)>, name: &str, value: Option<String>) {
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            rows.push((name.to_string(), v));
        }
    }
    let q = &detail.qso;

    let mut time = Vec::new();
    push(&mut time, "Start", Some(timestamp(q.qso_begin)));
    push(&mut time, "End", q.qso_end.map(timestamp));
    push(&mut time, "Submode", q.submode.clone());

    let mut dx = Vec::new();
    push(
        &mut dx,
        "DXCC",
        match (q.dxcc_id, q.dxcc_prefix.as_deref()) {
            (Some(id), Some(prefix)) => Some(format!("{prefix} (#{id})")),
            (Some(id), None) => Some(format!("#{id}")),
            (None, Some(prefix)) => Some(prefix.to_string()),
            (None, None) => None,
        },
    );
    push(&mut dx, "Continent", q.continent.clone());
    push(&mut dx, "CQ zone", q.cq_zone.map(|z| z.to_string()));
    push(&mut dx, "ITU zone", q.itu_zone.map(|z| z.to_string()));
    push(&mut dx, "Grid", q.grid.clone());
    push(&mut dx, "State", q.state.clone());
    push(&mut dx, "County", q.county.clone());
    push(&mut dx, "Province", q.province.clone());
    push(&mut dx, "IOTA", q.iota.clone());

    let mut path = Vec::new();
    push(
        &mut path,
        "Propagation",
        q.propagation_mode.as_ref().map(|p| match p {
            PropagationMode::Terrestrial => "Terrestrial".to_string(),
            PropagationMode::Satellite => "Satellite".to_string(),
            PropagationMode::Eme => "EME".to_string(),
            PropagationMode::MeteorScatter => "Meteor scatter".to_string(),
            PropagationMode::Aurora => "Aurora".to_string(),
            PropagationMode::AircraftScatter => "Aircraft scatter".to_string(),
            PropagationMode::Other(s) => s.clone(),
        }),
    );
    push(&mut path, "Satellite", q.sat_name.clone());
    push(&mut path, "Sat mode", q.sat_mode.clone());
    push(&mut path, "TX power", q.tx_power_w.map(|w| format!("{w} W")));
    push(&mut path, "RX power", q.rx_power_w.map(|w| format!("{w} W")));
    push(&mut path, "Distance", q.distance_km.map(|d| format!("{d:.0} km")));
    push(&mut path, "Bearing", q.bearing_deg.map(|b| format!("{b:.0}°")));

    let mut station = Vec::new();
    push(
        &mut station,
        "Station call",
        q.station_callsign.as_ref().map(|c| c.as_str().to_string()),
    );
    push(
        &mut station,
        "Owner call",
        q.owner_callsign.as_ref().map(|c| c.as_str().to_string()),
    );
    push(
        &mut station,
        "Location",
        q.station_location_id.map(|id| {
            locations
                .iter()
                .find(|l| l.id == id)
                .map(|l| l.name.clone())
                .unwrap_or_else(|| id.to_string())
        }),
    );
    push(&mut station, "Operator", q.operator_id.map(|id| id.to_string()));

    let qsl = detail
        .service_states
        .iter()
        .map(|s| {
            let mut parts = vec![match s.uploaded_at {
                Some(at) => format!("{} {}", s.upload_state, at.format("%Y-%m-%d")),
                None => s.upload_state.clone(),
            }];
            match (s.confirmation_state.as_str(), s.confirmed_at) {
                ("unknown", _) => {}
                (state, Some(at)) => parts.push(format!("{state} {}", at.format("%Y-%m-%d"))),
                (state, None) => parts.push(state.to_string()),
            }
            if let Some(err) = s.last_error.as_deref().filter(|e| !e.is_empty()) {
                parts.push(format!("error: {err}"));
            }
            (service_name(&s.service), parts.join("  ·  "))
        })
        .collect();

    let other = detail
        .exchange_fields
        .iter()
        .map(|f| {
            let value = match f.normalized_value.as_deref() {
                Some(n) if n != f.raw_value => format!("{}  ({n})", f.raw_value),
                _ => f.raw_value.clone(),
            };
            (f.name.clone(), value)
        })
        .collect();

    let mut record = Vec::new();
    push(&mut record, "Created", Some(timestamp(q.created_at)));
    push(&mut record, "Updated", Some(timestamp(q.updated_at)));
    push(&mut record, "ID", Some(q.id.to_string()));

    vec![
        ("TIME", time),
        ("DX STATION", dx),
        ("PATH", path),
        ("MY STATION", station),
        ("QSL", qsl),
        ("OTHER FIELDS", other),
        ("RECORD", record),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use logbook_domain::QsoServiceState;
    use radio_core::{Callsign, FieldSource, Qso, QsoExchangeField};

    fn section<'a>(sections: &'a [DetailSection], title: &str) -> &'a [(String, String)] {
        &sections.iter().find(|(t, _)| *t == title).unwrap().1
    }

    #[test]
    fn detail_lists_what_is_set_and_omits_what_is_not() {
        let begin = Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap();
        let qso = Qso {
            id: QsoId::new(),
            call: Callsign::parse("DL1XYZ").unwrap(),
            qso_begin: begin,
            qso_end: None,
            band: None,
            freq_hz: None,
            mode: None,
            submode: None,
            rst_sent: None,
            rst_rcvd: None,
            operator_id: None,
            station_location_id: None,
            station_callsign: None,
            owner_callsign: None,
            dxcc_id: Some(230),
            dxcc_prefix: Some("DL".into()),
            continent: None,
            cq_zone: Some(14),
            itu_zone: None,
            grid: None,
            state: None,
            county: None,
            province: None,
            iota: None,
            tx_power_w: None,
            rx_power_w: None,
            propagation_mode: None,
            sat_name: None,
            sat_mode: None,
            distance_km: None,
            bearing_deg: None,
            created_at: begin,
            updated_at: begin,
        };
        let detail = QsoDetail {
            qso,
            exchange_fields: vec![QsoExchangeField {
                name: "NAME".into(),
                raw_value: "Hans".into(),
                normalized_value: None,
                source: FieldSource::ImportedAdif,
            }],
            service_states: vec![QsoServiceState {
                service: "lotw".into(),
                upload_state: "verified".into(),
                confirmation_state: "confirmed".into(),
                uploaded_at: None,
                confirmed_at: Some(Utc.with_ymd_and_hms(2026, 5, 9, 0, 0, 0).unwrap()),
                last_error: None,
            }],
        };

        let sections = detail_sections(&detail, &[]);
        assert_eq!(
            section(&sections, "DX STATION"),
            [
                ("DXCC".to_string(), "DL (#230)".to_string()),
                ("CQ zone".to_string(), "14".to_string()),
            ]
        );
        assert!(section(&sections, "PATH").is_empty());
        assert_eq!(
            section(&sections, "QSL"),
            [("LoTW".to_string(), "verified  ·  confirmed 2026-05-09".to_string())]
        );
        assert_eq!(
            section(&sections, "OTHER FIELDS"),
            [("NAME".to_string(), "Hans".to_string())]
        );
        assert_eq!(section(&sections, "TIME")[0].1, "2026-05-08 18:30:00 UTC");
    }
}
