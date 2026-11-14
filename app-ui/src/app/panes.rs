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
    Keyer,
    Recent,
}

impl PaneKind {
    pub(super) const ALL: &[PaneKind] = &[
        PaneKind::Spots,
        PaneKind::Entry,
        PaneKind::Station,
        PaneKind::Keyer,
        PaneKind::Recent,
    ];
}

impl PaneKind {
    pub(super) fn title(self) -> &'static str {
        match self {
            PaneKind::Spots => "Spots",
            PaneKind::Entry => "Entry",
            PaneKind::Station => "Station",
            PaneKind::Keyer => "CW Keyer",
            PaneKind::Recent => "Recent",
        }
    }
}

/// Default pane layout the operator sees on first launch (or when no saved
/// layout is restored). Geometry:
/// ```text
/// ┌─────────────────────────┬─────────────────┐
/// │                         │     Entry       │
/// │                         ├─────────────────┤
/// │         Spots           │    CW Keyer     │
/// │                         ├─────────────────┤
/// │                         │    Station      │
/// ├─────────────────────────┴─────────────────┤
/// │                Recent                     │
/// └───────────────────────────────────────────┘
/// ```
/// Top row takes 70% of vertical space; Spots takes 60% of the top row's
/// width; the right column is split Entry 40% / CW Keyer 30% / Station 30%.
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
                ratio: 0.40,
                a: Box::new(Configuration::Pane(PaneKind::Entry)),
                b: Box::new(Configuration::Split {
                    axis: Axis::Horizontal,
                    ratio: 0.50,
                    a: Box::new(Configuration::Pane(PaneKind::Keyer)),
                    b: Box::new(Configuration::Pane(PaneKind::Station)),
                }),
            }),
        }),
        b: Box::new(Configuration::Pane(PaneKind::Recent)),
    }
}
