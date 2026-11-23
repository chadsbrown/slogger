//! QSL view — the confirmation workflow canvas. Three panes:
//!
//! - **Service status** — one row per upload service with pending count,
//!   last upload/fetch timestamps, last error inline.
//! - **Pending QSOs** — list of QSOs pending upload, filterable by
//!   service, with bulk mark-uploaded / soft-delete toolbar.
//! - **Confirmations** — the most recent confirmations from every
//!   service, newest first; a row opens its QSO in the Logbook tab.

use std::collections::HashSet;

use iced::widget::pane_grid::{Axis, Configuration, Content, PaneGrid, TitleBar};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, rule, scrollable, text,
};
use iced::{Element, Length};
use logbook_domain::RecentConfirmation;
use serde::{Deserialize, Serialize};
use station_resolver::Resolver;

use crate::app::helpers::service_name;
use crate::app::message::Message;
use crate::app::state::App;
use crate::app::theme::{self, cell, cell_fill, label, muted, table_header, table_row};
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
            QslService::Lotw => "LoTW",
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
    /// The Confirmations pane's list. `None` until it is first loaded,
    /// which happens when the QSL tab is opened; after that every
    /// `refresh_task` reloads it.
    pub confirmations: Option<Vec<ConfirmationRow>>,
    /// Why the last load of `confirmations` failed, if it did.
    pub confirmations_error: Option<String>,
}

/// How many confirmations the Confirmations pane lists.
pub(crate) const RECENT_CONFIRMATIONS_LIMIT: u32 = 200;

/// One row of the Confirmations pane: the confirmation plus the entity
/// name to show for it, worked out once when the list is loaded rather
/// than on every frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmationRow {
    pub confirmation: RecentConfirmation,
    pub entity: String,
}

pub(crate) fn confirmation_rows(
    confirmations: Vec<RecentConfirmation>,
    resolver: Option<&dyn Resolver>,
) -> Vec<ConfirmationRow> {
    confirmations
        .into_iter()
        .map(|c| ConfirmationRow {
            entity: entity_label(&c, resolver),
            confirmation: c,
        })
        .collect()
}

/// The entity column for a confirmation. The country name comes from the
/// callsign resolver, as on the Operating tab, but only when it doesn't
/// contradict the entity stored on the QSO: that one was right for the
/// QSO's date, while the resolver knows today's prefixes. Otherwise the
/// stored prefix, then the entity number.
fn entity_label(c: &RecentConfirmation, resolver: Option<&dyn Resolver>) -> String {
    let resolved = resolver.and_then(|r| r.resolve(&c.call));
    if let Some(res) = resolved {
        let contradicts = matches!((res.dxcc_id, c.dxcc_id), (Some(a), Some(b)) if a != b);
        if let Some(country) = res.country.filter(|_| !contradicts) {
            return country;
        }
    }
    match (&c.dxcc_prefix, c.dxcc_id) {
        (Some(prefix), _) => prefix.clone(),
        (None, Some(id)) => format!("#{id}"),
        (None, None) => String::new(),
    }
}

/// The line above the table. A full list is the newest `limit`, not
/// every confirmation in the log, and says so.
fn confirmations_heading(count: usize, limit: u32) -> String {
    if count >= limit as usize {
        format!("Most recent {limit} confirmations")
    } else if count == 1 {
        "1 confirmation".into()
    } else {
        format!("{count} confirmations")
    }
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
    // Update reads only what changed at LoTW since the last sync; this
    // reads everything again.
    let resync_btn = button(text("Full LoTW resync"))
        .style(theme::push_button)
        .on_press_maybe((!app.syncing).then_some(Message::LotwFullResyncPressed));

    let mut out = column![col, row![update_btn, resync_btn].spacing(8)].spacing(8);
    if let Some(result) = &app.last_lotw_resync {
        // Wraps, unlike the status bar, so nothing is cut off.
        out = out.push(text(format!("Last full resync — {result}")).size(12));
    }
    out.into()
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
                    parts.push(format!("up {}", crate::app::lotw_upload::format_lotw_upload(u)));
                } else if let Some(e) = &s.upload_error {
                    parts.push(format!("up error: {e}"));
                } else if let Some(r) = &s.upload_skipped_reason {
                    parts.push(format!("up skipped: {r}"));
                }
                if let Some(f) = &s.fetch {
                    parts.push(format!(
                        "sync {}",
                        crate::app::services::format_lotw_fetch(f)
                    ));
                } else if let Some(e) = &s.fetch_error {
                    parts.push(format!("sync error: {e}"));
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
                let mut parts = Vec::new();
                if let Some(f) = &s.fetch {
                    parts.push(format!(
                        "{} new conf / {} verified",
                        f.new_confirmations, f.uploads_verified
                    ));
                } else if let Some(e) = &s.fetch_error {
                    parts.push(format!("sync error: {e} (nothing uploaded)"));
                }
                if s.uploaded + s.duplicates + s.rejected > 0 {
                    parts.push(format!(
                        "up {} ok / {} already there / {} rejected",
                        s.uploaded, s.duplicates, s.rejected
                    ));
                }
                if s.held_back > 0 {
                    parts.push(format!("{} for another logbook", s.held_back));
                }
                if let Some(e) = &s.upload_error {
                    parts.push(format!("error: {e}"));
                }
                if parts.is_empty() {
                    "nothing to do".into()
                } else {
                    parts.join(" · ")
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
            None => button(text(mark_label)).style(theme::push_button),
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
        theme::grid(scrollable(list).height(Length::Fill)),
    ]
    .spacing(8)
    .into()
}

// Column widths for the Confirmations table, in the monospace cell font.
const W_CONFIRMED: f32 = 84.0;
const W_SERVICE: f32 = 70.0;
const W_CALL: f32 = 96.0;
const W_QSO_TIME: f32 = 128.0;
const W_BAND: f32 = 40.0;
const W_MODE: f32 = 44.0;

fn view_confirmations_pane(app: &App) -> Element<'_, Message> {
    let state = &app.qsl_view;
    let mut out = column![].spacing(4);
    if let Some(e) = &state.confirmations_error {
        out = out.push(label(format!("Couldn't load confirmations: {e}")));
    }
    let Some(rows) = &state.confirmations else {
        if state.confirmations_error.is_none() {
            out = out.push(label("Loading…"));
        }
        return out.into();
    };
    if rows.is_empty() {
        return out.push(label("No confirmations yet.")).into();
    }

    let header = table_header(row![
        cell("CONFIRMED", W_CONFIRMED),
        cell("SERVICE", W_SERVICE),
        cell("CALL", W_CALL),
        cell("QSO UTC", W_QSO_TIME),
        cell("BAND", W_BAND),
        cell("MODE", W_MODE),
        cell_fill("ENTITY"),
    ]);
    let mut list = column![].spacing(0);
    for r in rows {
        let c = &r.confirmation;
        let cells = row![
            cell(
                c.confirmed_at
                    .map(|t| t.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "—".into()),
                W_CONFIRMED
            ),
            cell(service_name(&c.service), W_SERVICE),
            cell(c.call.as_str(), W_CALL)
                .font(theme::MONO_BOLD)
                .style(theme::call),
            cell(c.qso_begin.format("%Y-%m-%d %H:%M").to_string(), W_QSO_TIME).style(muted),
            cell(c.band.map(|b| b.as_adif()).unwrap_or(""), W_BAND).style(theme::band),
            cell(
                c.mode.as_ref().map(|m| m.as_adif().to_string()).unwrap_or_default(),
                W_MODE
            )
            .style(theme::mode),
            cell_fill(r.entity.as_str()),
        ];
        list = list.push(table_row(
            &app.theme,
            cells,
            Message::ConfirmationClicked(c.qso_id),
            false,
        ));
    }

    out.push(label(confirmations_heading(rows.len(), RECENT_CONFIRMATIONS_LIMIT)))
        .push(theme::grid(column![
            header,
            rule::horizontal(1),
            scrollable(list).height(Length::Fill)
        ]))
        .into()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use radio_core::{Band, Callsign, Mode};
    use station_resolver::Resolution;

    use super::*;

    #[derive(Debug)]
    struct FixedResolver(Resolution);

    impl Resolver for FixedResolver {
        fn resolve(&self, _call: &Callsign) -> Option<Resolution> {
            Some(self.0.clone())
        }
    }

    fn resolves_to(dxcc_id: Option<u16>, country: Option<&str>) -> FixedResolver {
        FixedResolver(Resolution {
            dxcc_id,
            country: country.map(String::from),
            ..Resolution::empty()
        })
    }

    fn confirmation(dxcc_id: Option<u16>, dxcc_prefix: Option<&str>) -> RecentConfirmation {
        RecentConfirmation {
            confirmed_at: Some(Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap()),
            service: "lotw".into(),
            qso_id: QsoId::new(),
            call: Callsign::parse("DL1ABC").unwrap(),
            qso_begin: Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap(),
            band: Some(Band::M20),
            mode: Some(Mode::CW),
            dxcc_id,
            dxcc_prefix: dxcc_prefix.map(String::from),
            continent: None,
            state: None,
        }
    }

    #[test]
    fn entity_is_the_resolved_country_unless_it_contradicts_the_qso() {
        let c = confirmation(Some(230), Some("DL"));
        let germany = resolves_to(Some(230), Some("Germany"));
        assert_eq!(entity_label(&c, Some(&germany)), "Germany");
        // The resolver doesn't always know the entity number.
        let unnumbered = resolves_to(None, Some("Germany"));
        assert_eq!(entity_label(&c, Some(&unnumbered)), "Germany");
        // The QSO's stored entity wins over a different one today.
        let elsewhere = resolves_to(Some(281), Some("Spain"));
        assert_eq!(entity_label(&c, Some(&elsewhere)), "DL");
    }

    #[test]
    fn entity_falls_back_to_prefix_then_number_then_blank() {
        let nameless = resolves_to(Some(230), None);
        assert_eq!(entity_label(&confirmation(Some(230), Some("DL")), Some(&nameless)), "DL");
        assert_eq!(entity_label(&confirmation(Some(230), Some("DL")), None), "DL");
        assert_eq!(entity_label(&confirmation(Some(230), None), None), "#230");
        assert_eq!(entity_label(&confirmation(None, None), None), "");
    }

    #[test]
    fn rows_keep_order_and_carry_their_entity() {
        let first = confirmation(Some(230), Some("DL"));
        let second = confirmation(None, None);
        let rows = confirmation_rows(vec![first.clone(), second.clone()], None);
        assert_eq!(rows.len(), 2);
        assert_eq!((&rows[0].confirmation, rows[0].entity.as_str()), (&first, "DL"));
        assert_eq!((&rows[1].confirmation, rows[1].entity.as_str()), (&second, ""));
    }

    #[test]
    fn heading_says_when_the_list_is_capped() {
        assert_eq!(confirmations_heading(200, 200), "Most recent 200 confirmations");
        assert_eq!(confirmations_heading(37, 200), "37 confirmations");
        assert_eq!(confirmations_heading(1, 200), "1 confirmation");
    }
}
