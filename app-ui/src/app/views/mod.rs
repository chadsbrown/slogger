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
use super::theme;

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

impl ViewKind {
    pub(super) const ALL: &[ViewKind] = &[ViewKind::Operating, ViewKind::Qsl, ViewKind::Logbook];

    /// Digit that, with Ctrl held, switches to this view.
    pub(super) fn shortcut_digit(self) -> &'static str {
        match self {
            ViewKind::Operating => "1",
            ViewKind::Qsl => "2",
            ViewKind::Logbook => "3",
        }
    }
}

/// Render the top bar: view tabs on the left, UTC clock on the right.
pub(super) fn tab_bar(current: ViewKind) -> Element<'static, Message> {
    let mut bar = row![].spacing(4).align_y(iced::Alignment::Center);
    for v in ViewKind::ALL {
        bar = bar.push(
            button(
                row![
                    text(v.label()).size(14),
                    theme::label(format!("^{}", v.shortcut_digit())),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            )
            .padding([4, 12])
            .on_press(Message::ViewChanged(*v))
            .style(theme::tab_style(*v == current)),
        );
    }
    container(bar).padding([0, 4]).width(Length::Fill).into()
}
