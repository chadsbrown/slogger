//! QSL view — the confirmation workflow canvas. Three panes:
//!
//! - **Service status** — one row per upload service with pending count,
//!   last upload/fetch timestamps, last error inline.
//! - **Pending QSOs** — list of QSOs pending upload, filterable by
//!   service, with bulk mark-uploaded / soft-delete toolbar.
//! - **Confirmations** — recent inbound confirmations (placeholder for
//!   now; will hold the LotW QSL_RCVD / eQSL inbox surface).

use std::collections::HashSet;

use iced::widget::pane_grid::{Axis, Configuration, Content, PaneGrid, TitleBar};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, text,
};
use iced::{Element, Length};
use serde::{Deserialize, Serialize};

use crate::app::message::Message;
use crate::app::state::App;
use radio_core::{Qso, QsoId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QslPaneKind {
    Services,
    Pending,
    Confirmations,
}

impl QslPaneKind {
    fn title(self) -> &'static str {
        match self {
            QslPaneKind::Services => "Services",
            QslPaneKind::Pending => "Pending uploads",
            QslPaneKind::Confirmations => "Recent confirmations",
        }
    }
}

/// Default QSL canvas split. Service status on top, pending uploads in
/// the middle (largest pane), confirmations at the bottom.
pub fn default_qsl_configuration() -> Configuration<QslPaneKind> {
    Configuration::Split {
        axis: Axis::Horizontal,
        ratio: 0.30,
        a: Box::new(Configuration::Pane(QslPaneKind::Services)),
        b: Box::new(Configuration::Split {
            axis: Axis::Horizontal,
            ratio: 0.55,
            a: Box::new(Configuration::Pane(QslPaneKind::Pending)),
            b: Box::new(Configuration::Pane(QslPaneKind::Confirmations)),
        }),
    }
}

/// Identifies one of the 5 confirmation services. Used as the filter
/// for the Pending pane and the key for per-service action triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QslService {
    Lotw,
    Eqsl,
    ClubLog,
    Qrz,
    Hrdlog,
}

impl QslService {
    pub fn key(self) -> &'static str {
        match self {
            QslService::Lotw => "lotw",
            QslService::Eqsl => "eqsl",
            QslService::ClubLog => "clublog",
            QslService::Qrz => "qrz",
            QslService::Hrdlog => "hrdlog",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            QslService::Lotw => "LotW",
            QslService::Eqsl => "eQSL",
            QslService::ClubLog => "Club Log",
            QslService::Qrz => "QRZ",
            QslService::Hrdlog => "HRDLog",
        }
    }
}

pub const ALL_SERVICES: &[QslService] = &[
    QslService::Lotw,
    QslService::Eqsl,
    QslService::ClubLog,
    QslService::Qrz,
    QslService::Hrdlog,
];

/// Wrapper for the pending-pane filter pick_list. None = "Any service".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QslServiceFilter(pub Option<QslService>);

impl Default for QslServiceFilter {
    fn default() -> Self {
        Self(Some(QslService::Lotw))
    }
}

impl std::fmt::Display for QslServiceFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(s) => f.write_str(s.label()),
            None => f.write_str("Any"),
        }
    }
}

fn service_filter_options() -> Vec<QslServiceFilter> {
    let mut opts = Vec::with_capacity(ALL_SERVICES.len() + 1);
    opts.push(QslServiceFilter(None));
    opts.extend(ALL_SERVICES.iter().map(|s| QslServiceFilter(Some(*s))));
    opts
}

#[derive(Debug, Default)]
pub(crate) struct QslViewState {
    pub service_filter: QslServiceFilter,
    /// Last-loaded pending QSOs for whichever service filter is active.
    /// Refreshed on demand when the operator presses Refresh or after a
    /// bulk action / services update.
    pub pending: Vec<Qso>,
    pub loading_pending: bool,
    pub selected: HashSet<QsoId>,
    pub last_action: Option<String>,
}

pub(in crate::app) fn view(app: &App) -> Element<'_, Message> {
    let grid = PaneGrid::new(&app.qsl_panes, |_pane_id, kind, _maximized| {
        let title_bar = TitleBar::new(text(kind.title()).size(13))
            .padding(4)
            .style(crate::app::view::title_bar_style);
        let body: Element<'_, Message> = match kind {
            QslPaneKind::Services => view_services_pane(app),
            QslPaneKind::Pending => view_pending_pane(app),
            QslPaneKind::Confirmations => view_confirmations_pane(app),
        };
        Content::new(container(body).padding(8))
            .title_bar(title_bar)
            .style(crate::app::view::pane_content_style)
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .min_size(120.0)
    .spacing(6)
    .on_click(Message::PaneClicked)
    .on_drag(Message::PaneDragged)
    .on_resize(8, Message::PaneResized);

    container(grid).padding(4).into()
}

fn view_services_pane(app: &App) -> Element<'_, Message> {
    let mut col = column![
        row![
            text("Service").width(Length::Fixed(90.0)),
            text("Pending").width(Length::Fixed(80.0)),
            text("Last result").width(Length::Fill),
        ]
        .spacing(4),
    ]
    .spacing(2);

    for svc in ALL_SERVICES {
        let pending = pending_count_for(app, *svc);
        let last = describe_last_outcome(app, *svc);
        col = col.push(
            row![
                text(svc.label()).width(Length::Fixed(90.0)).size(13),
                text(pending.to_string())
                    .width(Length::Fixed(80.0))
                    .size(13),
                text(last).width(Length::Fill).size(12),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
        );
    }

    let update_btn = {
        let btn = button(text(if app.syncing {
            "Updating…"
        } else {
            "Update all services"
        }));
        if app.syncing {
            btn
        } else {
            btn.on_press(Message::ServicesUpdatePressed)
        }
    };

    column![col, update_btn].spacing(8).into()
}

fn pending_count_for(app: &App, svc: QslService) -> usize {
    match svc {
        QslService::Lotw => app.pending_lotw,
        QslService::Eqsl => app.pending_eqsl,
        QslService::ClubLog => app.pending_clublog,
        QslService::Qrz => app.pending_qrz,
        QslService::Hrdlog => app.pending_hrdlog,
    }
}

/// Pulls a human-readable one-liner about the most recent
/// `MultiUpdateSummary` for the given service. Empty when no update has
/// run yet this session.
fn describe_last_outcome(app: &App, svc: QslService) -> String {
    let Some(summary) = &app.last_services_update else {
        return "—".into();
    };
    match svc {
        QslService::Lotw => match &summary.lotw {
            None => "not configured".into(),
            Some(s) => {
                let mut parts = Vec::new();
                if let Some(u) = &s.upload {
                    parts.push(format!("up {}: {}", u.uploaded, u.note));
                } else if let Some(e) = &s.upload_error {
                    parts.push(format!("up error: {e}"));
                }
                if let Some(f) = &s.fetch {
                    parts.push(format!(
                        "fetch {} verified / {} confirmed",
                        f.verified, f.confirmed
                    ));
                } else if let Some(e) = &s.fetch_error {
                    parts.push(format!("fetch error: {e}"));
                }
                if parts.is_empty() {
                    "nothing to do".into()
                } else {
                    parts.join(" · ")
                }
            }
        },
        QslService::Eqsl => match &summary.eqsl {
            None => "not configured".into(),
            Some(s) => {
                let mut parts = Vec::new();
                if let Some(u) = &s.upload {
                    parts.push(format!("up {}: {}", u.uploaded, u.note));
                } else if let Some(e) = &s.upload_error {
                    parts.push(format!("up error: {e}"));
                }
                if s.fetched > 0 || s.confirmed > 0 {
                    parts.push(format!(
                        "fetch {} confirmed / {} unmatched",
                        s.confirmed, s.unmatched
                    ));
                } else if let Some(e) = &s.fetch_error {
                    parts.push(format!("fetch error: {e}"));
                }
                if parts.is_empty() {
                    "nothing to do".into()
                } else {
                    parts.join(" · ")
                }
            }
        },
        QslService::ClubLog => match &summary.clublog {
            None => "not configured".into(),
            Some(s) => {
                if let Some(u) = &s.upload {
                    format!("up {}: {}", u.uploaded, u.note)
                } else if let Some(e) = &s.upload_error {
                    format!("up error: {e}")
                } else {
                    "nothing to do".into()
                }
            }
        },
        QslService::Qrz => match &summary.qrz {
            None => "not configured".into(),
            Some(s) => {
                if let Some(e) = &s.upload_error {
                    format!("up error: {e}")
                } else if s.uploaded == 0 && s.rejected == 0 {
                    "nothing to do".into()
                } else {
                    format!("up {} ok / {} rejected", s.uploaded, s.rejected)
                }
            }
        },
        QslService::Hrdlog => match &summary.hrdlog {
            None => "not configured".into(),
            Some(s) => {
                if let Some(u) = &s.upload {
                    format!("up {}: {}", u.uploaded, u.note)
                } else if let Some(e) = &s.upload_error {
                    format!("up error: {e}")
                } else {
                    "nothing to do".into()
                }
            }
        },
    }
}

fn view_pending_pane(app: &App) -> Element<'_, Message> {
    let state = &app.qsl_view;

    let filter_row = row![
        text("Service:").width(Length::Fixed(80.0)),
        pick_list(
            service_filter_options(),
            Some(state.service_filter),
            Message::QslServiceFilterChanged,
        )
        .width(Length::Fixed(140.0)),
        button(text(if state.loading_pending {
            "Refreshing…"
        } else {
            "Refresh"
        }))
        .on_press(Message::QslPendingRefreshPressed),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let bulk_actions: Element<'_, Message> = if state.selected.is_empty() {
        column![].into()
    } else {
        let mark_label = state
            .service_filter
            .0
            .map(|s| format!("Mark uploaded → {}", s.label()))
            .unwrap_or_else(|| "Mark uploaded …".into());
        let mark_btn = match state.service_filter.0 {
            Some(s) => button(text(mark_label))
                .on_press(Message::QslBulkMarkUploaded(s.key().into())),
            None => button(text(mark_label)).style(button::secondary),
        };
        row![
            text(format!("{} selected:", state.selected.len())),
            mark_btn,
            button(text("Soft delete"))
                .on_press(Message::QslBulkDelete)
                .style(button::danger),
            button(text("Clear")).on_press(Message::QslClearSelection),
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
    for q in &state.pending {
        let id = q.id;
        let is_selected = state.selected.contains(&id);
        let label = format!(
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
                checkbox(is_selected)
                    .on_toggle(move |_| Message::QslPendingRowToggle(id)),
                text(label).size(12),
            ]
            .spacing(4),
        );
    }
    if state.pending.is_empty() && !state.loading_pending {
        list = list.push(
            text(match state.service_filter.0 {
                Some(s) => format!("No QSOs pending upload to {}.", s.label()),
                None => "No QSOs pending upload.".into(),
            })
            .size(12),
        );
    }

    column![
        filter_row,
        bulk_actions,
        action_status,
        scrollable(list).height(Length::Fill),
    ]
    .spacing(8)
    .into()
}

fn view_confirmations_pane(_app: &App) -> Element<'_, Message> {
    // First cut: just a hint pointing the operator at the bottom of the
    // service-status pane for fetch counts. A proper "recent
    // confirmations" listing wants a repo query for QSOs with
    // `confirmed_at` in the last N days — added later.
    column![
        text("Recent confirmations").size(13),
        text(
            "Per-QSO confirmation list lands here once the repo exposes \
             `confirmed_since`. For now the Service status pane above \
             shows how many confirmations the last fetch picked up."
        )
        .size(11),
    ]
    .spacing(6)
    .into()
}
