//! Awards Detail drawer. Pick an award (DXCC / WAS / WPX / IOTA /
//! Marathon), see the per-unit worked/confirmed grid, optionally filter
//! DXCC by band, and target an unworked unit so the live Spots pane
//! highlights matching spots.

use iced::widget::{button, column, pick_list, row, scrollable, text};
use iced::{Element, Length};
use logbook_domain::AwardUnit;

use crate::app::constants::BANDS;
use crate::app::drawers::logbook::BandFilter;
use crate::app::message::Message;
use crate::app::state::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwardKind {
    Dxcc,
    Was,
    Wpx,
    Iota,
    Marathon,
}

impl std::fmt::Display for AwardKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AwardKind::Dxcc => "DXCC",
            AwardKind::Was => "WAS",
            AwardKind::Wpx => "WPX",
            AwardKind::Iota => "IOTA",
            AwardKind::Marathon => "Marathon",
        })
    }
}

pub(crate) const AWARD_KINDS: &[AwardKind] = &[
    AwardKind::Dxcc,
    AwardKind::Was,
    AwardKind::Wpx,
    AwardKind::Iota,
    AwardKind::Marathon,
];

/// Per-drawer state. Lives on App as `awards_drawer`.
#[derive(Debug, Default)]
pub(crate) struct AwardsDrawerState {
    pub selected_kind: AwardKindOpt,
    /// Optional per-band filter; only meaningful for DXCC today (the
    /// other awards expose only totals). Defaults to "Any band".
    pub band_filter: BandFilter,
    /// If set, the live Spots pane border-highlights spots whose
    /// resolved DXCC entity matches this key. Cross-reference is only
    /// implemented for DXCC for now — other awards would need extra
    /// resolver data.
    pub target_unit: Option<TargetUnit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AwardKindOpt(pub AwardKind);

impl Default for AwardKindOpt {
    fn default() -> Self {
        Self(AwardKind::Dxcc)
    }
}

impl std::fmt::Display for AwardKindOpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// A "watch this unit" pointer. The spots panel reads this and adds a
/// highlight border to any spot whose resolved entity matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetUnit {
    pub kind: AwardKind,
    pub key: String,
}

impl App {
    pub(in crate::app) fn view_awards_drawer(&self) -> Element<'_, Message> {
        let state = &self.awards_drawer;
        let snap = &self.awards;

        // Total / confirmed for the selected award. Read from the
        // pre-computed snapshot so this drawer doesn't re-run any
        // backend queries.
        let (worked_total, confirmed_total, units, by_band_supported) = match state.selected_kind.0 {
            AwardKind::Dxcc => (
                snap.dxcc.worked,
                snap.dxcc.confirmed,
                snap.dxcc.units.clone(),
                true,
            ),
            AwardKind::Was => (
                snap.was.worked,
                snap.was.confirmed,
                snap.was.units.clone(),
                false,
            ),
            AwardKind::Wpx => (
                snap.wpx.worked,
                snap.wpx.confirmed,
                snap.wpx.units.clone(),
                false,
            ),
            AwardKind::Iota => (
                snap.iota.worked,
                snap.iota.confirmed,
                snap.iota.units.clone(),
                false,
            ),
            AwardKind::Marathon => (
                snap.marathon.entities.worked,
                snap.marathon.entities.confirmed,
                snap.marathon.entities.units.clone(),
                false,
            ),
        };

        // For DXCC, optionally narrow units to a specific band by
        // pulling that band's AwardProgress out of `dxcc_by_band`.
        let (units, scope_label): (Vec<AwardUnit>, String) = match (
            state.selected_kind.0,
            state.band_filter.0,
        ) {
            (AwardKind::Dxcc, Some(b)) => match snap.dxcc_by_band.get(&b) {
                Some(prog) => (
                    prog.units.clone(),
                    format!(
                        " · {} band: {}/{}",
                        b.as_adif(),
                        prog.worked,
                        prog.confirmed
                    ),
                ),
                None => (Vec::new(), format!(" · {} band: 0/0", b.as_adif())),
            },
            _ => (units, String::new()),
        };

        let header = row![
            text("Award:").width(Length::Fixed(70.0)),
            pick_list(
                AWARD_KINDS,
                Some(state.selected_kind.0),
                Message::AwardsKindChanged,
            )
            .width(Length::Fixed(140.0)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let band_filter_row: Element<'_, Message> = if by_band_supported {
            let mut opts = Vec::with_capacity(BANDS.len() + 1);
            opts.push(BandFilter(None));
            opts.extend(BANDS.iter().map(|b| BandFilter(Some(*b))));
            row![
                text("Band:").width(Length::Fixed(70.0)),
                pick_list(opts, Some(state.band_filter), Message::AwardsBandFilterChanged)
                    .width(Length::Fixed(120.0)),
            ]
            .spacing(8)
            .into()
        } else {
            text("(per-band drill-down for non-DXCC awards lands in Phase 3.5)")
                .size(11)
                .into()
        };

        let totals_line = text(format!(
            "{}: {} worked, {} confirmed{}",
            state.selected_kind.0, worked_total, confirmed_total, scope_label
        ))
        .size(13);

        // Cross-reference status. Hint about which entity is currently
        // being watched in the spots pane.
        let target_line: Element<'_, Message> = match &state.target_unit {
            Some(t) if t.kind == AwardKind::Dxcc => row![
                text(format!("Watching DXCC {} in Spots", t.key)).size(12),
                button(text("Clear").size(12))
                    .on_press(Message::AwardsClearTarget)
                    .style(button::text),
            ]
            .spacing(8)
            .into(),
            Some(_) => text("(spot cross-reference is DXCC-only for now)")
                .size(11)
                .into(),
            None => text("Click a unit below to highlight matching spots.")
                .size(11)
                .into(),
        };

        let mut list = column![row![
            text("Key").width(Length::Fixed(80.0)),
            text("Worked").width(Length::Fixed(70.0)),
            text("Confirmed").width(Length::Fill),
        ]
        .spacing(4)]
        .spacing(2);
        let kind = state.selected_kind.0;
        for unit in &units {
            let key = unit.key.clone();
            let is_target = matches!(
                &state.target_unit,
                Some(t) if t.kind == kind && t.key == key
            );
            let label = format!(
                "{:<6}  {:<5}  {}",
                unit.key,
                unit.worked_count,
                if unit.confirmed { "yes" } else { "no" }
            );
            let target_unit = TargetUnit {
                kind,
                key: key.clone(),
            };
            let row_btn = if is_target {
                button(text(label).size(12)).on_press(Message::AwardsClearTarget)
            } else {
                button(text(label).size(12))
                    .on_press(Message::AwardsSetTarget(target_unit))
                    .style(button::text)
            };
            list = list.push(row_btn);
        }
        if units.is_empty() {
            list = list.push(text("(no entries)").size(12));
        }

        column![
            header,
            band_filter_row,
            totals_line,
            target_line,
            scrollable(list).height(Length::Fill),
        ]
        .spacing(8)
        .into()
    }
}
