//! Top-level views. Each view is a self-contained workflow with its own
//! set of floating panels (see `floating`), selected with the tab buttons
//! in the bottom bar (see `crate::app::view`) or Ctrl+1..3:
//!
//! - **Operating** — live: spots, entry, station state.
//! - **QSL** — confirmation workflow: uploads, fetches, pending QSOs.
//! - **Logbook** — querying: search, awards drill-down, sessions, tools.
//!
//! Settings is intentionally not a view — operators edit
//! `~/.config/slogger/config.toml` directly per the TOML-as-truth model.

pub(super) mod floating;
pub(super) mod logbook;
pub(super) mod operating;
pub(super) mod qsl;

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
