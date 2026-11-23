//! UI preference persistence.
//!
//! The position, size, open/closed state and stacking order of every
//! Operating panel, plus the chosen theme and text size, are written to
//! `~/.local/share/slogger/ui-layout.json` whenever the operator changes
//! one of them, and read back at boot.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::panels::Panel;

/// Bumped when the file format changes incompatibly. Version 1 was the
/// tiled split tree used before panels became free-floating.
const VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
struct LayoutFile {
    version: u32,
    /// The Operating tab's panels, in back-to-front stacking order.
    panels: Vec<Panel>,
    /// The QSL tab's panels; absent in files written before they floated.
    #[serde(default)]
    qsl_panels: Vec<Panel>,
    /// The Logbook tab's panels; absent likewise.
    #[serde(default)]
    logbook_panels: Vec<Panel>,
    /// UI scale factor; absent in files written before it existed.
    #[serde(default)]
    font_scale: Option<f32>,
    /// Theme name as shown in the picker.
    #[serde(default)]
    theme: Option<String>,
    /// Whether POTA spots are switched on. Absent until the operator
    /// first presses the POTA button; `[pota] enabled` decides until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pota: Option<bool>,
}

/// Everything the layout file remembers.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedUi {
    pub panels: Vec<Panel>,
    pub qsl_panels: Vec<Panel>,
    pub logbook_panels: Vec<Panel>,
    pub font_scale: Option<f32>,
    pub theme: Option<String>,
    pub pota: Option<bool>,
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
pub(super) fn load_layout() -> Option<SavedUi> {
    let path = layout_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    parse_layout(&contents).or_else(|| {
        tracing::info!(path = %path.display(), "saved ui layout is unreadable or from an older version; using default layout");
        None
    })
}

fn parse_layout(contents: &str) -> Option<SavedUi> {
    let file: LayoutFile = serde_json::from_str(contents).ok()?;
    (file.version == VERSION).then_some(SavedUi {
        panels: file.panels,
        qsl_panels: file.qsl_panels,
        logbook_panels: file.logbook_panels,
        font_scale: file.font_scale,
        theme: file.theme,
        pota: file.pota,
    })
}

/// Best-effort save. Writes pretty JSON for hand-editability. Errors are
/// logged but not surfaced to the operator — layout persistence is a
/// nice-to-have, not a correctness requirement.
pub(super) async fn save_layout(ui: SavedUi) {
    let Some(path) = layout_path() else {
        return;
    };
    let file = LayoutFile {
        version: VERSION,
        panels: ui.panels,
        qsl_panels: ui.qsl_panels,
        logbook_panels: ui.logbook_panels,
        font_scale: ui.font_scale,
        theme: ui.theme,
        pota: ui.pota,
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
    use crate::app::panels::{PanelKind, Panels};

    #[test]
    fn current_format_parses() {
        let file = LayoutFile {
            version: VERSION,
            panels: Panels::new(PanelKind::OPERATING).to_saved(),
            qsl_panels: Panels::new(PanelKind::QSL).to_saved(),
            logbook_panels: Panels::new(PanelKind::LOGBOOK).to_saved(),
            font_scale: Some(1.2),
            theme: Some("Nord".into()),
            pota: Some(true),
        };
        let json = serde_json::to_string_pretty(&file).unwrap();
        assert_eq!(
            parse_layout(&json),
            Some(SavedUi {
                panels: Panels::new(PanelKind::OPERATING).to_saved(),
                qsl_panels: Panels::new(PanelKind::QSL).to_saved(),
                logbook_panels: Panels::new(PanelKind::LOGBOOK).to_saved(),
                font_scale: Some(1.2),
                theme: Some("Nord".into()),
                pota: Some(true),
            })
        );
    }

    #[test]
    fn a_file_without_theme_or_scale_still_loads() {
        let saved = parse_layout(r#"{"version":2,"panels":[]}"#).unwrap();
        assert_eq!(saved.font_scale, None);
        assert_eq!(saved.theme, None);
        assert_eq!(saved.pota, None);
    }

    /// A file written before the CondX panel existed still loads, and the
    /// new panel joins it closed rather than the layout being discarded.
    #[test]
    fn a_file_from_before_condx_loads_with_condx_closed() {
        let before_condx = r#"{
          "version": 2,
          "panels": [
            { "kind": "Spots",   "x": 0.0, "y": 0.0,  "w": 0.6, "h": 0.68, "open": true },
            { "kind": "Entry",   "x": 0.6, "y": 0.0,  "w": 0.4, "h": 0.3,  "open": true },
            { "kind": "Keyer",   "x": 0.6, "y": 0.3,  "w": 0.4, "h": 0.2,  "open": false },
            { "kind": "Station", "x": 0.6, "y": 0.5,  "w": 0.4, "h": 0.18, "open": true },
            { "kind": "Recent",  "x": 0.0, "y": 0.68, "w": 1.0, "h": 0.32, "open": true }
          ],
          "font_scale": 1.1,
          "theme": "Nord"
        }"#;
        let saved = parse_layout(before_condx).expect("pre-CondX layout should load");
        assert_eq!(saved.font_scale, Some(1.1));
        // Written before the other tabs' panels floated: those start at
        // their defaults.
        assert!(saved.qsl_panels.is_empty() && saved.logbook_panels.is_empty());
        let qsl = Panels::from_saved(PanelKind::QSL, saved.qsl_panels);
        assert_eq!(qsl.to_saved(), Panels::new(PanelKind::QSL).to_saved());
        let mut panels = Panels::from_saved(PanelKind::OPERATING, saved.panels);
        assert!(!panels.is_open(PanelKind::Keyer), "saved choices are kept");
        assert!(!panels.is_open(PanelKind::CondX));
        panels.toggle(PanelKind::CondX);
        assert!(panels.is_open(PanelKind::CondX));
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
