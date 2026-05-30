//! Logbook Manager drawer. Search builder + result list + bulk actions.
//!
//! Drives `LogbookService::search_qsos` / `count_matching` /
//! `bulk_*_by_search` from the backend. The drawer reads its own
//! sub-state (`LogbookSearchState`) on `App`; mutation goes through
//! `Message::Logbook*` variants routed through update.rs.

use std::collections::HashSet;

use iced::widget::{button, checkbox, column, pick_list, row, scrollable, text, text_input};
use iced::{Element, Length};
use logbook_domain::QsoSummary;
use radio_core::{Band, Mode, QsoId};

use crate::app::constants::{BANDS, modes};
use crate::app::message::Message;
use crate::app::state::App;

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

    /// Detail pane — when a result row is selected (editing_qso is Some),
    /// renders the same entry form widgets bound to the same state. Edits
    /// flow through `update_qso` exactly as if the form were in
    /// Operating.
    pub(in crate::app) fn view_logbook_detail_pane(&self) -> Element<'_, Message> {
        if self.editing_qso.is_none() {
            return text("Select a QSO from the Results pane to view detail.")
                .size(12)
                .into();
        }
        self.entry_view()
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
