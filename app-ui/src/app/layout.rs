//! Operating-panel layout persistence.
//!
//! The position, size, open/closed state and stacking order of every
//! Operating panel is written to `~/.local/share/slogger/ui-layout.json`
//! whenever the operator finishes moving, resizing, opening or closing a
//! panel, and read back at boot.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::panels::Panel;

/// Bumped when the file format changes incompatibly. Version 1 was the
/// tiled split tree used before panels became free-floating.
const VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
struct LayoutFile {
    version: u32,
    /// Back-to-front stacking order.
    panels: Vec<Panel>,
}

/// Resolves the on-disk path for the saved layout. Returns None if the
/// platform doesn't provide a data dir (extremely rare). The file lives
/// alongside the SQLite database in the slogger data directory.
pub(super) fn layout_path() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("slogger").join("ui-layout.json"))
}

/// Best-effort load. Returns None on any problem (file missing, written
/// by an older build, hand-edited into nonsense) so the caller falls back
/// to the default arrangement — a bad layout file must never stop the
/// app from starting.
pub(super) fn load_layout() -> Option<Vec<Panel>> {
    let path = layout_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    parse_layout(&contents).or_else(|| {
        tracing::info!(path = %path.display(), "saved ui layout is unreadable or from an older version; using default layout");
        None
    })
}

fn parse_layout(contents: &str) -> Option<Vec<Panel>> {
    let file: LayoutFile = serde_json::from_str(contents).ok()?;
    (file.version == VERSION).then_some(file.panels)
}

/// Best-effort save. Writes pretty JSON for hand-editability. Errors are
/// logged but not surfaced to the operator — layout persistence is a
/// nice-to-have, not a correctness requirement.
pub(super) async fn save_layout(panels: Vec<Panel>) {
    let Some(path) = layout_path() else {
        return;
    };
    let file = LayoutFile {
        version: VERSION,
        panels,
    };
    let json = match serde_json::to_string_pretty(&file) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::panels::Panels;

    #[test]
    fn current_format_parses() {
        let file = LayoutFile {
            version: VERSION,
            panels: Panels::default().to_saved(),
        };
        let json = serde_json::to_string_pretty(&file).unwrap();
        assert_eq!(parse_layout(&json), Some(Panels::default().to_saved()));
    }

    #[test]
    fn old_tiled_layout_is_rejected_rather_than_crashing() {
        let v1 = r#"{"kind":"Split","axis":"Horizontal","ratio":0.7,
                    "a":{"kind":"Pane","0":"Spots"},"b":{"kind":"Pane","0":"Recent"}}"#;
        assert_eq!(parse_layout(v1), None);
        assert_eq!(parse_layout("not json"), None);
        assert_eq!(parse_layout(r#"{"version":99,"panels":[]}"#), None);
    }
}
