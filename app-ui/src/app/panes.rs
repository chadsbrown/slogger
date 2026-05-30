use iced::widget::pane_grid::{Axis, Configuration};
use serde::{Deserialize, Serialize};

/// Identifier for the slogger panes. `pane_grid::State<PaneKind>` carries one
/// of these per pane; the `Pane` ID iced hands us in messages is the opaque
/// per-instance handle, distinct from our content identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PaneKind {
    Spots,
    Entry,
    Station,
    Recent,
}

impl PaneKind {
    pub(super) fn title(self) -> &'static str {
        match self {
            PaneKind::Spots => "Spots",
            PaneKind::Entry => "Entry",
            PaneKind::Station => "Station",
            PaneKind::Recent => "Recent",
        }
    }
}

/// Default pane layout the operator sees on first launch (or when no saved
/// layout is restored). Geometry:
/// ```text
/// ┌─────────────────────────┬─────────────────┐
/// │                         │     Entry       │
/// │         Spots           ├─────────────────┤
/// │                         │    Station      │
/// ├─────────────────────────┴─────────────────┤
/// │                Recent                     │
/// └───────────────────────────────────────────┘
/// ```
/// Top row takes 70% of vertical space; Spots takes 60% of the top row's
/// width; Entry takes 55% of the right column's height.
pub(super) fn default_pane_configuration() -> Configuration<PaneKind> {
    Configuration::Split {
        axis: Axis::Horizontal,
        ratio: 0.70,
        a: Box::new(Configuration::Split {
            axis: Axis::Vertical,
            ratio: 0.60,
            a: Box::new(Configuration::Pane(PaneKind::Spots)),
            b: Box::new(Configuration::Split {
                axis: Axis::Horizontal,
                ratio: 0.55,
                a: Box::new(Configuration::Pane(PaneKind::Entry)),
                b: Box::new(Configuration::Pane(PaneKind::Station)),
            }),
        }),
        b: Box::new(Configuration::Pane(PaneKind::Recent)),
    }
}
