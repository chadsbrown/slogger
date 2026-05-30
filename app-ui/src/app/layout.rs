//! Pane layout persistence.
//!
//! iced's `pane_grid::Node` is the internal layout tree (binary splits
//! plus opaque `Pane` IDs). It isn't directly serializable, and its leaf
//! `Pane` IDs are runtime-only, so we mirror the structure into a
//! serializable `LayoutTree` keyed on our own `PaneKind`. On boot the
//! tree (if present at `~/.local/share/slogger/ui-layout.json`) is
//! converted back into `pane_grid::Configuration<PaneKind>`. On every
//! `PaneResized` / `PaneDragged` event the new layout is written
//! asynchronously; a generation counter drops in-flight writes that
//! have been superseded by a newer resize.

use std::path::PathBuf;

use iced::widget::pane_grid::{self, Axis, Configuration, Node};
use serde::{Deserialize, Serialize};

use super::panes::PaneKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum LayoutTree {
    Split {
        axis: AxisRepr,
        ratio: f32,
        a: Box<LayoutTree>,
        b: Box<LayoutTree>,
    },
    Pane(PaneKind),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AxisRepr {
    Horizontal,
    Vertical,
}

impl From<Axis> for AxisRepr {
    fn from(a: Axis) -> Self {
        match a {
            Axis::Horizontal => AxisRepr::Horizontal,
            Axis::Vertical => AxisRepr::Vertical,
        }
    }
}

impl From<AxisRepr> for Axis {
    fn from(a: AxisRepr) -> Self {
        match a {
            AxisRepr::Horizontal => Axis::Horizontal,
            AxisRepr::Vertical => Axis::Vertical,
        }
    }
}

/// Walks the iced internal layout tree and translates each opaque
/// `pane_grid::Pane` to its corresponding `PaneKind` via the state's
/// `get()` lookup. The tree shape is preserved verbatim — splits stay
/// splits with the same axis and ratio.
pub(super) fn tree_from_state(state: &pane_grid::State<PaneKind>) -> LayoutTree {
    walk(state.layout(), state)
}

fn walk(node: &Node, state: &pane_grid::State<PaneKind>) -> LayoutTree {
    match node {
        Node::Split {
            axis, ratio, a, b, ..
        } => LayoutTree::Split {
            axis: (*axis).into(),
            ratio: *ratio,
            a: Box::new(walk(a, state)),
            b: Box::new(walk(b, state)),
        },
        Node::Pane(pane) => {
            // A pane present in the layout tree but missing from the state
            // map shouldn't happen, but if it ever does we fall back to
            // Spots — better to have a usable layout than a panic.
            let kind = state.get(*pane).copied().unwrap_or(PaneKind::Spots);
            LayoutTree::Pane(kind)
        }
    }
}

/// Builds an iced `Configuration` from a saved tree. Used at boot when
/// `ui-layout.json` is present; otherwise the caller falls back to
/// `default_pane_configuration()`.
pub(super) fn configuration_from_tree(tree: &LayoutTree) -> Configuration<PaneKind> {
    match tree {
        LayoutTree::Split { axis, ratio, a, b } => Configuration::Split {
            axis: (*axis).into(),
            ratio: *ratio,
            a: Box::new(configuration_from_tree(a)),
            b: Box::new(configuration_from_tree(b)),
        },
        LayoutTree::Pane(kind) => Configuration::Pane(*kind),
    }
}

/// Resolves the on-disk path for the saved layout. Returns None if the
/// platform doesn't provide a data dir (extremely rare). The file lives
/// alongside the SQLite database in the slogger data directory.
pub(super) fn layout_path() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("slogger").join("ui-layout.json"))
}

/// Best-effort load. Returns None on any error (file missing, parse
/// failure, etc.) so the caller falls back to the default layout. The
/// file lives in a directory the operator could hand-edit; we don't
/// crash if they corrupt it.
pub(super) fn load_layout() -> Option<LayoutTree> {
    let path = layout_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str(&contents) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "ui-layout.json unparseable; using default layout");
            None
        }
    }
}

/// Best-effort save. Writes pretty JSON for hand-editability. Errors are
/// logged but not surfaced to the operator — layout persistence is a
/// nice-to-have, not a correctness requirement.
pub(super) async fn save_layout(tree: LayoutTree) {
    let Some(path) = layout_path() else {
        return;
    };
    let json = match serde_json::to_string_pretty(&tree) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize ui layout");
            return;
        }
    };
    if let Some(parent) = path.parent()
        && let Err(e) = tokio::fs::create_dir_all(parent).await
    {
        tracing::warn!(path = %parent.display(), error = %e, "failed to ensure ui layout dir");
        return;
    }
    if let Err(e) = tokio::fs::write(&path, json).await {
        tracing::warn!(path = %path.display(), error = %e, "failed to write ui layout");
    }
}
