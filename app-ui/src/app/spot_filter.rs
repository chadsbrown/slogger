//! Choosing the spot filter: the files on offer and where they live.

use std::path::{Path, PathBuf};

/// One entry in the Spots panel's filter list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterChoice {
    /// What the list shows: the file name without `.json`.
    pub label: String,
    /// The filter file; `None` is "No filter".
    pub path: Option<PathBuf>,
}

impl FilterChoice {
    pub fn none() -> Self {
        Self { label: "No filter".to_string(), path: None }
    }

    pub fn for_path(path: &Path) -> Self {
        let label = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Self { label, path: Some(path.to_path_buf()) }
    }
}

impl std::fmt::Display for FilterChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Where filter files are looked for: `[dxcluster] filter_dir`, or a
/// `spot-filters` folder beside config.toml.
pub fn directory(configured: Option<&Path>) -> Option<PathBuf> {
    match configured {
        Some(dir) => Some(dir.to_path_buf()),
        None => Some(app_config::Config::default_path().ok()?.parent()?.join("spot-filters")),
    }
}

/// "No filter", then every `.json` file in the folder by name, then the
/// filter in force if it lives somewhere else (as `filter_file` in
/// config.toml may), so the list always contains the current choice.
pub fn choices(dir: Option<&Path>, current: Option<&Path>) -> Vec<FilterChoice> {
    let mut files: Vec<PathBuf> = dir
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json")))
        .collect();
    files.sort_by_key(|path| path.file_name().map(|n| n.to_string_lossy().to_lowercase()));

    let mut out = vec![FilterChoice::none()];
    out.extend(files.iter().map(|path| FilterChoice::for_path(path)));
    if let Some(current) = current
        && !files.iter().any(|path| path == current)
    {
        out.push(FilterChoice::for_path(current));
    }
    out
}

/// How the choice is written in the layout file: the path, or an empty
/// string for "No filter" (absent means no choice was ever made).
pub fn to_saved(choice: Option<&Path>) -> String {
    choice.map(|path| path.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn from_saved(saved: &str) -> Option<PathBuf> {
    (!saved.is_empty()).then(|| PathBuf::from(saved))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(choices: &[FilterChoice]) -> Vec<&str> {
        choices.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn the_list_is_no_filter_then_the_folders_json_files_by_name() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["contest.json", "Everyday.JSON", "notes.txt", "dx-evening.json"] {
            std::fs::write(dir.path().join(name), "{}").unwrap();
        }
        std::fs::create_dir(dir.path().join("old.json")).unwrap();

        let list = choices(Some(dir.path()), None);
        assert_eq!(labels(&list), ["No filter", "contest", "dx-evening", "Everyday"]);
        assert_eq!(list[0].path, None);
        assert_eq!(list[1].path.as_deref(), Some(dir.path().join("contest.json").as_path()));

        // The current filter is listed once, wherever it lives.
        let inside = dir.path().join("contest.json");
        assert_eq!(choices(Some(dir.path()), Some(&inside)).len(), 4);
        let elsewhere = Path::new("/somewhere/else/mine.json");
        let list = choices(Some(dir.path()), Some(elsewhere));
        assert_eq!(labels(&list), ["No filter", "contest", "dx-evening", "Everyday", "mine"]);

        // No folder yet: still a usable list.
        let missing = dir.path().join("nope");
        assert_eq!(labels(&choices(Some(&missing), Some(elsewhere))), ["No filter", "mine"]);
    }

    #[test]
    fn the_saved_choice_tells_no_filter_from_never_chosen() {
        assert_eq!(to_saved(None), "");
        assert_eq!(from_saved(""), None);
        let path = Path::new("/a/b/cw.json");
        assert_eq!(from_saved(&to_saved(Some(path))).as_deref(), Some(path));
        assert_eq!(directory(Some(Path::new("/x"))).as_deref(), Some(Path::new("/x")));
        assert!(directory(None).is_none_or(|d| d.ends_with("slogger/spot-filters")));
    }
}
