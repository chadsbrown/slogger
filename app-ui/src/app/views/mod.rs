//! Top-level views. Each view is a self-contained workflow with its own
//! pane layout, selected via the tab bar between the persistent header
//! strip and the main canvas:
//!
//! - **Operating** — live: spots, entry, station state.
//! - **QSL** — confirmation workflow: uploads, fetches, pending QSOs.
//! - **Logbook** — querying: search, awards drill-down, sessions, tools.
//!
//! Settings is intentionally not a view — operators edit
//! `~/.config/slogger/config.toml` directly per the TOML-as-truth model.

pub(super) mod logbook;
pub(super) mod operating;
pub(super) mod qsl;

use iced::widget::{button, container, row, text};
use iced::{Element, Length};

use super::message::Message;

/// Which view is currently selected. Persistent across the App's
/// lifetime; not saved to disk (we want a fresh app to land in
/// Operating, the most-used view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Operating,
    Qsl,
    Logbook,
}

impl Default for ViewKind {
    fn default() -> Self {
        ViewKind::Operating
    }
}

impl ViewKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            ViewKind::Operating => "Operating",
            ViewKind::Qsl => "QSL",
            ViewKind::Logbook => "Logbook",
        }
    }
}

const ALL_VIEWS: &[ViewKind] = &[ViewKind::Operating, ViewKind::Qsl, ViewKind::Logbook];

/// Render the tab bar between the header strip and the main canvas. The
/// active tab uses the default button style; inactive tabs use the text
/// style so the bar reads as labels rather than chunky buttons.
pub(super) fn tab_bar(current: ViewKind) -> Element<'static, Message> {
    let mut bar = row![].spacing(4);
    for v in ALL_VIEWS {
        let is_active = *v == current;
        let btn = button(text(v.label())).on_press(Message::ViewChanged(*v));
        let styled = if is_active {
            btn
        } else {
            btn.style(button::text)
        };
        bar = bar.push(styled);
    }
    container(bar).padding(4).width(Length::Fill).into()
}
