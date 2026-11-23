//! A theme with every colour and option spelled out: what the theme
//! editor works on and what a full theme file holds.
//!
//! A [`Spec`] can be taken from any theme (a copy with no tie to where it
//! came from), turned back into a theme, and read from or written to a
//! TOML file of `role = "#rrggbb"` lines in the themes directory.

use std::path::{Path, PathBuf};

use iced::{Color, Theme};

use super::{Hues, Scheme, StatusLook, contrast, hues, parse_hex, scheme_from, title_case};

/// One colour a theme defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Frame,
    Pane,
    Field,
    Hover,
    Border,
    Text,
    Muted,
    Bar,
    BarText,
    Button,
    ButtonText,
    Control,
    Accent,
    Selection,
    SelectionText,
    Call,
    Freq,
    Band,
    Mode,
    Good,
    Warn,
    Bad,
}

impl Role {
    pub(in crate::app) const COUNT: usize = 22;

    /// Every role, in file and editor order.
    pub(in crate::app) const ALL: [Role; Role::COUNT] = [
        Role::Frame,
        Role::Pane,
        Role::Field,
        Role::Hover,
        Role::Border,
        Role::Text,
        Role::Muted,
        Role::Bar,
        Role::BarText,
        Role::Button,
        Role::ButtonText,
        Role::Control,
        Role::Accent,
        Role::Selection,
        Role::SelectionText,
        Role::Call,
        Role::Freq,
        Role::Band,
        Role::Mode,
        Role::Good,
        Role::Warn,
        Role::Bad,
    ];

    /// The key in a theme file.
    pub(in crate::app) fn key(self) -> &'static str {
        match self {
            Role::Frame => "frame",
            Role::Pane => "pane",
            Role::Field => "field",
            Role::Hover => "hover",
            Role::Border => "border",
            Role::Text => "text",
            Role::Muted => "muted",
            Role::Bar => "bar",
            Role::BarText => "bar_text",
            Role::Button => "button",
            Role::ButtonText => "button_text",
            Role::Control => "control",
            Role::Accent => "accent",
            Role::Selection => "selection",
            Role::SelectionText => "selection_text",
            Role::Call => "callsign",
            Role::Freq => "frequency",
            Role::Band => "band",
            Role::Mode => "mode",
            Role::Good => "good",
            Role::Warn => "warn",
            Role::Bad => "bad",
        }
    }

    /// What the editor calls it.
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Role::Frame => "Window frame",
            Role::Pane => "Panes",
            Role::Field => "Tables and fields",
            Role::Hover => "Row under the pointer",
            Role::Border => "Borders",
            Role::Text => "Text",
            Role::Muted => "Dim text",
            Role::Bar => "Title bars, active tab",
            Role::BarText => "Title bar text",
            Role::Button => "Buttons",
            Role::ButtonText => "Button text",
            Role::Control => "Drop-downs",
            Role::Accent => "Main button, links",
            Role::Selection => "Selected row",
            Role::SelectionText => "Selected row text",
            Role::Call => "Callsigns",
            Role::Freq => "Frequencies",
            Role::Band => "Bands",
            Role::Mode => "Modes",
            Role::Good => "Good / needed",
            Role::Warn => "Fair / caution",
            Role::Bad => "Poor / problem",
        }
    }

    /// The heading the editor lists it under.
    pub(in crate::app) fn group(self) -> &'static str {
        match self {
            Role::Frame | Role::Pane | Role::Field | Role::Hover | Role::Border => "GROUNDS",
            Role::Text | Role::Muted => "TEXT",
            Role::Bar
            | Role::BarText
            | Role::Button
            | Role::ButtonText
            | Role::Control
            | Role::Accent => "STRUCTURE",
            Role::Selection | Role::SelectionText => "SELECTION",
            Role::Call | Role::Freq | Role::Band | Role::Mode => "DATA",
            Role::Good | Role::Warn | Role::Bad => "STATUS",
        }
    }
}

impl std::fmt::Display for StatusLook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StatusLook::Cell => "Filled cell",
            StatusLook::Text => "Coloured text",
            StatusLook::Row => "Whole row",
        })
    }
}

impl StatusLook {
    pub(in crate::app) const ALL: [StatusLook; 3] =
        [StatusLook::Cell, StatusLook::Text, StatusLook::Row];

    fn key(self) -> &'static str {
        match self {
            StatusLook::Cell => "cell",
            StatusLook::Text => "text",
            StatusLook::Row => "row",
        }
    }
}

/// Widest border a theme may ask for.
pub(in crate::app) const EDGE_MAX: f32 = 3.0;

/// A whole theme, nothing derived.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct Spec {
    pub(in crate::app) name: String,
    /// Indexed by [`Role`].
    pub(in crate::app) colors: [Color; Role::COUNT],
    /// Whether a selected row is drawn in [`Role::SelectionText`] alone,
    /// rather than each cell keeping its own colour.
    pub(in crate::app) one_ink: bool,
    /// Border width.
    pub(in crate::app) edge: f32,
    pub(in crate::app) status: StatusLook,
    /// Whether alternate table rows are shaded.
    pub(in crate::app) zebra: bool,
}

impl Spec {
    pub(in crate::app) fn get(&self, role: Role) -> Color {
        self.colors[role as usize]
    }

    pub(in crate::app) fn set(&mut self, role: Role, color: Color) {
        self.colors[role as usize] = color;
    }

    /// Everything `theme` shows, as a free-standing copy.
    pub(in crate::app) fn from_theme(theme: &Theme) -> Self {
        let palette = theme.palette();
        Spec::from_parts(
            theme.to_string(),
            palette.background,
            palette.text,
            palette.primary,
            theme.extended_palette().background.strong.color,
            &hues(theme),
        )
    }

    fn from_parts(
        name: String,
        field: Color,
        text: Color,
        accent: Color,
        border: Color,
        h: &Hues,
    ) -> Self {
        let mut spec = Spec {
            name,
            colors: [Color::BLACK; Role::COUNT],
            one_ink: h.select_text.is_some(),
            edge: h.edge,
            status: h.status,
            zebra: h.zebra,
        };
        for role in Role::ALL {
            let color = match role {
                Role::Frame => h.chrome,
                Role::Pane => h.pane,
                Role::Field => field,
                Role::Hover => h.hover,
                Role::Border => border,
                Role::Text => text,
                Role::Muted => h.muted,
                Role::Bar => h.bar,
                Role::BarText => h.bar_text,
                Role::Button => h.button,
                Role::ButtonText => h.button_text,
                Role::Control => h.control,
                Role::Accent => accent,
                Role::Selection => h.select,
                Role::SelectionText => h.select_text.unwrap_or(text),
                Role::Call => h.call,
                Role::Freq => h.freq,
                Role::Band => h.band,
                Role::Mode => h.mode,
                Role::Good => h.good,
                Role::Warn => h.warn,
                Role::Bad => h.bad,
            };
            spec.set(role, color);
        }
        spec
    }

    /// The theme this describes. Every colour is used as given, except
    /// that where the accent and status colours are drawn as text they
    /// are still nudged until they can be read, as in every theme.
    pub(in crate::app) fn theme_named(&self, name: String) -> Theme {
        let c = |role| self.get(role);
        let scheme = Scheme {
            background: c(Role::Field),
            surface: c(Role::Hover),
            line: c(Role::Border),
            muted: c(Role::Muted),
            text: c(Role::Text),
            accent: c(Role::Accent),
            green: c(Role::Good),
            yellow: c(Role::Warn),
            red: c(Role::Bad),
            cyan: c(Role::Freq),
            magenta: c(Role::Band),
            orange: c(Role::Call),
        };
        let hues = Hues {
            muted: c(Role::Muted),
            call: c(Role::Call),
            freq: c(Role::Freq),
            band: c(Role::Band),
            mode: c(Role::Mode),
            select: c(Role::Selection),
            select_text: self.one_ink.then(|| c(Role::SelectionText)),
            hover: c(Role::Hover),
            control: c(Role::Control),
            edge: self.edge,
            status: self.status,
            zebra: self.zebra,
            bar: c(Role::Bar),
            bar_text: c(Role::BarText),
            chrome: c(Role::Frame),
            pane: c(Role::Pane),
            button: c(Role::Button),
            button_text: c(Role::ButtonText),
            good: c(Role::Good),
            warn: c(Role::Warn),
            bad: c(Role::Bad),
        };
        scheme.theme_with(name, hues)
    }

    /// Places where this theme would be hard to read, in plain words.
    /// Empty when there are none.
    pub(in crate::app) fn warnings(&self) -> Vec<String> {
        let mut out = vec![];
        let mut reads = |ink: Role, ground: Role, min: f32| {
            let ratio = contrast(self.get(ink), self.get(ground));
            if ratio < min {
                out.push(format!(
                    "{} on {}: contrast {ratio:.1}, aim for {min:.1}",
                    ink.label(),
                    ground.label().to_lowercase()
                ));
            }
        };
        reads(Role::Text, Role::Field, 4.5);
        reads(Role::Text, Role::Pane, 4.5);
        reads(Role::Text, Role::Control, 4.5);
        reads(Role::Muted, Role::Field, 3.0);
        reads(Role::Muted, Role::Pane, 3.0);
        reads(Role::BarText, Role::Bar, 4.5);
        reads(Role::ButtonText, Role::Button, 4.5);
        for data in [Role::Call, Role::Freq, Role::Band, Role::Mode] {
            reads(data, Role::Field, 4.5);
        }
        for ink in [Role::Text, Role::Call] {
            reads(ink, Role::Hover, 3.0);
        }
        if self.one_ink {
            reads(Role::SelectionText, Role::Selection, 4.5);
        } else {
            for ink in [Role::Text, Role::Call, Role::Freq, Role::Band, Role::Mode] {
                reads(ink, Role::Selection, 3.0);
            }
        }
        if self.status == StatusLook::Text {
            for status in [Role::Good, Role::Warn, Role::Bad] {
                reads(status, Role::Field, 3.0);
            }
        }
        for (a, b) in [(Role::Frame, Role::Pane), (Role::Pane, Role::Field)] {
            if contrast(self.get(a), self.get(b)) < 1.05 {
                out.push(format!(
                    "{} and {} are nearly the same colour",
                    a.label(),
                    b.label().to_lowercase()
                ));
            }
        }
        out
    }

    /// The theme file for this spec.
    pub(in crate::app) fn to_toml(&self) -> String {
        let mut out = String::from("# A slogger theme. Colours are \"#rrggbb\".\n");
        out.push_str(&format!("name = {}\n", toml::Value::String(self.name.clone())));
        let mut group = "";
        for role in Role::ALL {
            if role == Role::SelectionText && !self.one_ink {
                out.push_str("# selection_text: unset, so each cell keeps its own colour.\n");
                continue;
            }
            if role.group() != group {
                group = role.group();
                out.push_str(&format!("\n# {}\n", title_case(&group.to_lowercase())));
            }
            out.push_str(&format!(
                "{} = \"{}\"  # {}\n",
                role.key(),
                to_hex(self.get(role)),
                role.label()
            ));
        }
        out.push_str("\n# Options\n");
        out.push_str(&format!("border_width = {:.1}\n", self.edge));
        out.push_str(&format!(
            "status_style = \"{}\"  # cell, text or row\n",
            self.status.key()
        ));
        out.push_str(&format!("striped_rows = {}\n", self.zebra));
        out
    }

    /// Reads a theme file's table. `field` and `text` are required; any
    /// other role left out is derived from the ones given. Returns the
    /// spec with a note for each entry that could not be used, or the
    /// reason there is no spec at all.
    pub(in crate::app) fn from_table(
        table: &toml::Table,
        stem: &str,
    ) -> Result<(Spec, Vec<String>), String> {
        let mut notes = vec![];
        let color = |role: Role| -> Option<Result<Color, String>> {
            let value = table.get(role.key())?;
            Some(
                value
                    .as_str()
                    .and_then(parse_hex)
                    .ok_or_else(|| format!("{} = {value} is not a colour", role.key())),
            )
        };
        let required = |role: Role| match color(role) {
            Some(Ok(c)) => Ok(c),
            Some(Err(e)) => Err(e),
            None => Err(format!("{} is missing", role.key())),
        };
        let field = required(Role::Field)?;
        let text = required(Role::Text)?;
        let given = |role: Role| color(role).and_then(Result::ok);

        // What the file leaves out is blended from what it gives, the
        // way a plain palette file is filled in.
        let scheme = scheme_from(|key| match key {
            "background" => Some(field),
            "text" => Some(text),
            "accent" => given(Role::Accent),
            "green" => given(Role::Good),
            "yellow" => given(Role::Warn),
            "red" => given(Role::Bad),
            "surface" => given(Role::Hover),
            "line" => given(Role::Border),
            "muted" => given(Role::Muted),
            "cyan" => given(Role::Freq),
            "magenta" => given(Role::Band),
            "orange" => given(Role::Call),
            _ => None,
        })
        .ok_or("no usable colours")?;
        let name = match table.get("name") {
            Some(toml::Value::String(name)) if !name.trim().is_empty() => name.trim().to_owned(),
            Some(other) => {
                notes.push(format!("name = {other} is not a name"));
                title_case(stem)
            }
            None => title_case(stem),
        };
        let mut spec = Spec::from_parts(
            name,
            field,
            text,
            scheme.accent,
            scheme.line,
            &scheme.hues(),
        );
        for role in Role::ALL {
            match color(role) {
                Some(Ok(c)) => spec.set(role, c),
                Some(Err(e)) => notes.push(e),
                None => {}
            }
        }
        spec.one_ink = given(Role::SelectionText).is_some();

        if let Some(value) = table.get("border_width") {
            let width = value
                .as_float()
                .or_else(|| value.as_integer().map(|i| i as f64));
            match width {
                Some(w) if (0.0..=EDGE_MAX as f64).contains(&w) => spec.edge = w as f32,
                _ => notes.push(format!("border_width = {value} is not between 0 and {EDGE_MAX}")),
            }
        }
        if let Some(value) = table.get("status_style") {
            let look = value
                .as_str()
                .and_then(|s| StatusLook::ALL.into_iter().find(|look| look.key() == s));
            match look {
                Some(look) => spec.status = look,
                None => notes.push(format!("status_style = {value} is not cell, text or row")),
            }
        }
        if let Some(value) = table.get("striped_rows") {
            match value.as_bool() {
                Some(on) => spec.zebra = on,
                None => notes.push(format!("striped_rows = {value} is not true or false")),
            }
        }
        for key in table.keys() {
            let known = Role::ALL.iter().any(|role| role.key() == key)
                || ["name", "border_width", "status_style", "striped_rows"].contains(&key.as_str());
            if !known {
                notes.push(format!("{key} is not something a theme sets"));
            }
        }
        Ok((spec, notes))
    }

    /// Writes the theme file: over `existing` when editing one, otherwise
    /// to a new file in `dir` named after the theme.
    pub(in crate::app) fn save(&self, dir: &Path, existing: Option<&Path>) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = match existing {
            Some(path) => path.to_owned(),
            None => {
                let slug = slug(&self.name);
                let mut path = dir.join(format!("{slug}.toml"));
                let mut n = 2;
                while path.exists() || dir.join(&slug).with_extension("yaml").exists() {
                    path = dir.join(format!("{slug}-{n}.toml"));
                    n += 1;
                    if n > 99 {
                        break;
                    }
                }
                path
            }
        };
        std::fs::write(&path, self.to_toml())?;
        Ok(path)
    }
}

/// Whether a TOML table is a full theme file rather than a plain palette.
pub(super) fn is_spec(table: &toml::Table) -> bool {
    table.contains_key(Role::Field.key())
}

/// `My Theme!` to `my-theme`.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() { "theme".to_owned() } else { out.to_owned() }
}

/// `#rrggbb`.
pub(in crate::app) fn to_hex(c: Color) -> String {
    let [r, g, b, _] = c.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Hue in degrees, saturation and lightness from 0 to 1.
pub(in crate::app) fn to_hsl(c: Color) -> [f32; 3] {
    let (max, min) = (c.r.max(c.g).max(c.b), c.r.min(c.g).min(c.b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-5 {
        return [0.0, 0.0, l];
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-5);
    let h = if max == c.r {
        ((c.g - c.b) / d).rem_euclid(6.0)
    } else if max == c.g {
        (c.b - c.r) / d + 2.0
    } else {
        (c.r - c.g) / d + 4.0
    };
    [h * 60.0, s.min(1.0), l]
}

pub(in crate::app) fn from_hsl([h, s, l]: [f32; 3]) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color::from_rgb(
        (r + m).clamp(0.0, 1.0),
        (g + m).clamp(0.0, 1.0),
        (b + m).clamp(0.0, 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::super::{default_theme, themes_from};
    use super::*;

    #[test]
    fn a_copy_of_any_theme_looks_the_same_and_survives_its_file() {
        for entry in themes_from(None) {
            let spec = Spec::from_theme(&entry.theme);
            let copy = spec.theme_named(format!("{} copy", entry.theme));
            assert_eq!(hues(&copy), hues(&entry.theme), "{}", entry.theme);
            // iced's own themes can have status colours too faint to read
            // as text; a copy has those nudged like any slogger theme.
            if entry.source != super::super::ThemeSource::BuiltIn {
                assert_eq!(copy.palette(), entry.theme.palette(), "{}", entry.theme);
            }

            let table: toml::Table = spec.to_toml().parse().expect("valid TOML");
            assert!(is_spec(&table));
            let (read, notes) = Spec::from_table(&table, "stem").expect("readable");
            assert!(notes.is_empty(), "{notes:?}");
            assert_eq!(read.name, spec.name);
            assert_eq!((read.one_ink, read.edge, read.status, read.zebra),
                (spec.one_ink, spec.edge, spec.status, spec.zebra));
            for role in Role::ALL {
                if role == Role::SelectionText && !spec.one_ink {
                    continue;
                }
                assert_eq!(to_hex(read.get(role)), to_hex(spec.get(role)), "{role:?}");
            }
        }
    }

    #[test]
    fn a_file_needs_only_a_field_and_text_and_says_what_it_could_not_use() {
        let table: toml::Table = r##"
            field = "#ffffff"
            text = "#101010"
            pane = "#dddddd"
            bar = "navy"
            status_style = "blob"
            sparkle = true
        "##
        .parse()
        .unwrap();
        let (spec, notes) = Spec::from_table(&table, "my-theme").unwrap();
        assert_eq!(spec.name, "My Theme");
        assert_eq!(to_hex(spec.get(Role::Pane)), "#dddddd");
        assert!(!spec.one_ink);
        assert_eq!(notes.len(), 3, "{notes:?}");

        let no_text: toml::Table = "field = \"#ffffff\"".parse().unwrap();
        assert_eq!(Spec::from_table(&no_text, "x").unwrap_err(), "text is missing");
    }

    #[test]
    fn the_shipped_default_has_no_warnings_and_an_unreadable_theme_does() {
        let mut spec = Spec::from_theme(&default_theme());
        assert_eq!(spec.warnings(), Vec::<String>::new());
        spec.set(Role::Text, spec.get(Role::Field));
        assert!(spec.warnings().iter().any(|w| w.starts_with("Text on tables")));
    }

    #[test]
    fn a_saved_theme_is_offered_as_the_users_own_and_bad_files_are_reported() {
        let dir = std::env::temp_dir().join(format!("slogger-spec-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut spec = Spec::from_theme(&default_theme());
        spec.name = "My Fog".into();
        spec.set(Role::Pane, super::super::rgb(0x123456));
        spec.zebra = true;
        let path = spec.save(&dir, None).unwrap();
        assert_eq!(path, dir.join("my-fog.toml"));
        // Saving again over the same file keeps the one file.
        assert_eq!(spec.save(&dir, Some(&path)).unwrap(), path);
        std::fs::write(dir.join("broken.toml"), "field = \"#fff\"\ntext = \"#000000\"").unwrap();

        let (entries, problems) = super::super::load_from(Some(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
        let mine = entries.iter().find(|e| e.theme.to_string() == "My Fog").expect("listed");
        assert_eq!(mine.file.as_deref(), Some(path.as_path()));
        let h = hues(&mine.theme);
        assert_eq!((to_hex(h.pane).as_str(), h.zebra), ("#123456", true));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("theme file broken.toml: skipped"), "{problems:?}");
    }

    #[test]
    fn hsl_round_trips() {
        for hex in [0x000000, 0xffffff, 0x3f51b5, 0xe65100, 0x2e7d32, 0x808080, 0xff4081] {
            let c = super::super::rgb(hex);
            assert_eq!(to_hex(from_hsl(to_hsl(c))), to_hex(c));
        }
    }

    #[test]
    fn slugs_are_safe_file_names() {
        assert_eq!(slug("Fog & Blue copy"), "fog-blue-copy");
        assert_eq!(slug("../../etc"), "etc");
        assert_eq!(slug("!!"), "theme");
    }
}
