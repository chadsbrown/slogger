//! Visual language shared by every view: the application theme, the
//! monospace font used for tabular data, and the small set of container /
//! button / text styles the panes are built from.
//!
//! Views should reach for these helpers rather than hard-coding colours so
//! the whole app can be re-skinned from this one file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use iced::theme::Palette;
use iced::widget::{Row, Text, button, container, text};
use iced::{Border, Color, Element, Font, Length, Theme, color};

use super::message::Message;

/// Font for anything columnar or numeric: callsigns, frequencies, tables.
pub(super) const MONO: Font = Font::MONOSPACE;

/// Body size for table rows and dense readouts.
pub(super) const ROW_TEXT: f32 = 13.0;
/// Size for field labels, column headers and hints.
pub(super) const LABEL_TEXT: f32 = 11.0;

/// The slogger theme: a low-glare dark palette suited to a shack at night.
pub(super) fn app_theme() -> Theme {
    Theme::custom(
        "slogger",
        Palette {
            background: color!(0x15181d),
            text: color!(0xd5dae1),
            primary: color!(0x4f9fe8),
            success: color!(0x58c27d),
            warning: color!(0xe8a33d),
            danger: color!(0xe5604f),
        },
    )
}

/// Extra palettes shipped with slogger, as `(name, [background, text,
/// primary, success, warning, danger])`, in alphabetical order. The
/// colours come from the themes distributed with Omarchy and from the
/// Tinted Theming Base16 scheme collection (both MIT licensed), less the
/// ones iced already provides.
const BUNDLED: &[(&str, [u32; 6])] = &[
    ("Atelier Dune", [0x20201d, 0xa6a28c, 0x6684e1, 0x60ac39, 0xae9513, 0xd73737]),
    ("Atelier Forest", [0x1b1918, 0xa8a19f, 0x407ee7, 0x7b9726, 0xc38418, 0xf22c40]),
    ("Ayu Dark", [0x0b0e14, 0xe6e1cf, 0x59c2ff, 0xaad94c, 0xffb454, 0xf07178]),
    ("Ayu Light", [0xf8f9fa, 0x5c6166, 0x399ee6, 0x6cbf49, 0xf2ae49, 0xf07171]),
    ("Ayu Mirage", [0x1f2430, 0xcccac2, 0x73d0ff, 0xd5ff80, 0xffd173, 0xf28779]),
    ("Eighties", [0x2d2d2d, 0xd3d0c8, 0x6699cc, 0x99cc99, 0xffcc66, 0xf2777a]),
    ("Espresso", [0x2d2d2d, 0xcccccc, 0x6c99bb, 0xa5c261, 0xffc66d, 0xd25252]),
    ("Ethereal", [0x060b1e, 0xffcead, 0x7d82d9, 0x92a593, 0xe9bb4f, 0xed5b5a]),
    ("Everforest", [0x2d353b, 0xd3c6aa, 0x7fbbb3, 0xa7c080, 0xdbbc7f, 0xe67e80]),
    ("Flexoki Dark", [0x100f0f, 0xcecdc3, 0x4385be, 0x879a39, 0xd0a215, 0xd14d41]),
    ("Flexoki Light", [0xfffcf0, 0x100f0f, 0x205ea6, 0x879a39, 0xd0a215, 0xd14d41]),
    ("GitHub Dark", [0x0d1117, 0xd1d7e0, 0x58a6ff, 0x3fb950, 0xd29922, 0xff7b72]),
    ("GitHub Light", [0xffffff, 0x454c54, 0x0969da, 0x116329, 0x9a6700, 0xcf222e]),
    ("Gruvbox Material", [0x282828, 0xd4be98, 0x7daea3, 0xa9b665, 0xd8a657, 0xea6962]),
    ("Hackerman", [0x0b0c16, 0xddf7ff, 0x82fb9c, 0x4fe88f, 0x50f7d4, 0x50f872]),
    ("Jade Horse", [0x142620, 0xdfede5, 0x8bcbb1, 0x8bcbb1, 0xc5cc91, 0xd88d91]),
    ("Jellybeans", [0x121212, 0xd5d5d5, 0xb1d8f6, 0xbddeab, 0xffdca0, 0xffa1a1]),
    ("Last Horizon", [0x0c0b0c, 0xfafcfb, 0xb59790, 0x87a9b0, 0x6b5e73, 0xc38b7b]),
    ("Lumon", [0x16242d, 0xd6e2ee, 0x8bc9eb, 0x5e95bc, 0x6fa4c9, 0x4d86b0]),
    ("Lupine", [0xfafafa, 0x212121, 0x3264eb, 0x4a2fd0, 0x026fde, 0xc900c4]),
    ("Material Darker", [0x212121, 0xeeffff, 0x82aaff, 0xc3e88d, 0xffcb6b, 0xf07178]),
    ("Material Palenight", [0x292d3e, 0x959dcb, 0x82aaff, 0xc3e88d, 0xffcb6b, 0xf07178]),
    ("Matte Black", [0x121212, 0xbebebe, 0xe68e0d, 0xffc107, 0xb91c1c, 0xd35f5f]),
    ("Miasma", [0x222222, 0xc2c2b0, 0x78824b, 0x5f875f, 0xb36d43, 0x685742]),
    ("Mocha", [0x3b3228, 0xd0c8c6, 0x8ab3b5, 0xbeb55b, 0xf4bc87, 0xcb6077]),
    ("Monokai", [0x272822, 0xf8f8f2, 0x66d9ef, 0xa6e22e, 0xf4bf75, 0xf92672]),
    ("Moonlight", [0x212337, 0xa3ace1, 0x40ffff, 0x2df4c0, 0xffc777, 0xff5370]),
    ("Nightfox", [0x192330, 0xcdcecf, 0x719cd6, 0x81b29a, 0xdbc074, 0xc94f6d]),
    ("Ocean", [0x2b303b, 0xc0c5ce, 0x8fa1b3, 0xa3be8c, 0xebcb8b, 0xbf616a]),
    ("One Dark", [0x282c34, 0xabb2bf, 0x61afef, 0x98c379, 0xe5c07b, 0xe06c75]),
    ("One Light", [0xfafafa, 0x383a42, 0x4078f2, 0x50a14f, 0xc18401, 0xca1243]),
    ("Osaka Jade", [0x111c18, 0xc1c497, 0x509475, 0x549e6a, 0x459451, 0xff5345]),
    ("Outrun Dark", [0x00002a, 0xd0d0fa, 0x66b0ff, 0x59f176, 0xf3e877, 0xff4242]),
    ("PaperColor Dark", [0x1c1c1c, 0x9e9e9e, 0x5fafd7, 0x5faf5f, 0xffaf00, 0xff5faf]),
    ("PaperColor Light", [0xeeeeee, 0x5e5e5e, 0x005f87, 0x008700, 0xd75f00, 0xd70000]),
    ("Paraiso", [0x2f1e2e, 0xa39e9b, 0x06b6ef, 0x48b685, 0xfec418, 0xef6155]),
    ("PhD", [0x061229, 0xb8bbc2, 0x5299bf, 0x99bf52, 0xfbd461, 0xd07346]),
    ("Retro 82", [0x05182e, 0xf6dcac, 0xfaa968, 0x028391, 0xe97b3c, 0xf85525]),
    ("Ristretto", [0x2c2525, 0xe6d9db, 0xf38d70, 0xadda78, 0xf9cc6c, 0xfd6883]),
    ("Rose Pine", [0x191724, 0xe0def4, 0xc4a7e7, 0x31748f, 0xebbcba, 0xeb6f92]),
    ("Rose Pine Dawn", [0xfaf4ed, 0x575279, 0x56949f, 0x286983, 0xea9d34, 0xb4637a]),
    ("Rose Pine Moon", [0x232136, 0xe0def4, 0xc4a7e7, 0x3e8fb0, 0xea9a97, 0xeb6f92]),
    ("Selenized Dark", [0x103c48, 0xadbcbc, 0x4695f7, 0x75b938, 0xdbb32d, 0xfa5750]),
    ("Selenized Light", [0xfbf3db, 0x53676d, 0x006dce, 0x428b00, 0xa78300, 0xcc1729]),
    ("Shades of Purple", [0x1e1e3f, 0xc7c7c7, 0x6943ff, 0x3ad900, 0xffe700, 0xd90429]),
    ("Snazzy", [0x282a36, 0xe2e4e5, 0x57c7ff, 0x5af78e, 0xf3f99d, 0xff5c57]),
    ("Solitude", [0x101315, 0xcacccc, 0x798186, 0x9fa5a9, 0xd9dbdc, 0x565d60]),
    ("Spaceduck", [0x16172d, 0xecf0c1, 0x7a5ccc, 0x5ccc96, 0xf2ce00, 0xe33400]),
    ("Starship", [0x1c1c1e, 0xf5f5f7, 0xff9f0a, 0xff9f0a, 0xffc27a, 0xff5f4a]),
    ("Synth Midnight", [0x050608, 0xc1c3c4, 0x03aeff, 0x06ea61, 0xc9d364, 0xb53b50]),
    ("Tender", [0x282828, 0xeeeeee, 0xb3deef, 0xc9d05c, 0xffc24b, 0xf43753]),
    ("Tomorrow", [0xffffff, 0x373b41, 0x4271ae, 0x718c00, 0xeab700, 0xc82829]),
    ("Tomorrow Night", [0x1d1f21, 0xc5c8c6, 0x81a2be, 0xb5bd68, 0xf0c674, 0xcc6666]),
    ("Twilight", [0x1e1e1e, 0xa7a7a7, 0x7587a6, 0x8f9d6a, 0xf9ee98, 0xcf6a4c]),
    ("Vantablack", [0x000000, 0xffffff, 0x8d8d8d, 0xb6b6b6, 0xcecece, 0xa4a4a4]),
    ("Vesper", [0x101010, 0xb7b7b7, 0x8eaaaa, 0x5f8787, 0xffc799, 0xde6e6e]),
    ("White", [0xffffff, 0x000000, 0x6e6e6e, 0x3a3a3a, 0x4a4a4a, 0x2a2a2a]),
    ("Woodland", [0x231e18, 0xcabcb1, 0x88a4d3, 0xb7ba53, 0xe0ac16, 0xd35c5c]),
    ("Zenburn", [0x383838, 0xdcdccc, 0x7cb8bb, 0x5f7f5f, 0xe0cf9f, 0xdca3a3]),
];

fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// slogger's own theme followed by the bundled ones.
fn bundled_themes() -> impl Iterator<Item = Theme> {
    std::iter::once(app_theme()).chain(BUNDLED.iter().map(|&(name, c)| {
        Theme::custom(
            name,
            Palette {
                background: rgb(c[0]),
                text: rgb(c[1]),
                primary: rgb(c[2]),
                success: rgb(c[3]),
                warning: rgb(c[4]),
                danger: rgb(c[5]),
            },
        )
    }))
}

/// Where a picker theme comes from. The picker groups its tiles by this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThemeSource {
    /// Shipped with slogger: its own palette and the bundled ones.
    Slogger,
    /// A palette file in `~/.config/slogger/themes/`.
    Custom,
    /// One of iced's built-in themes.
    BuiltIn,
}

impl ThemeSource {
    /// Picker sections, in the order they are shown.
    pub(super) const ALL: [ThemeSource; 3] = [
        ThemeSource::Slogger,
        ThemeSource::Custom,
        ThemeSource::BuiltIn,
    ];

    /// Section heading in the picker.
    pub(super) fn title(self) -> &'static str {
        match self {
            ThemeSource::Slogger => "SLOGGER",
            ThemeSource::Custom => "YOUR THEMES",
            ThemeSource::BuiltIn => "CLASSICS",
        }
    }
}

/// One theme the picker offers.
#[derive(Debug, Clone)]
pub(super) struct ThemeEntry {
    pub(super) source: ThemeSource,
    pub(super) theme: Theme,
}

/// Every theme the picker offers: the ones shipped with slogger, palette
/// files from `~/.config/slogger/themes/`, then iced's built-ins. Reads
/// the themes directory, so call it when the list is needed afresh rather
/// than on every frame.
pub(super) fn all_themes() -> Vec<ThemeEntry> {
    let custom = dirs::config_dir().map(|d| d.join("slogger").join("themes"));
    themes_from(custom.as_deref())
}

fn themes_from(custom: Option<&Path>) -> Vec<ThemeEntry> {
    // Names identify themes in the layout file, so each must be unique.
    // The fixed themes claim theirs first so a saved choice keeps meaning
    // the same theme whatever files appear later.
    let mut names: HashSet<String> = bundled_themes()
        .chain(Theme::ALL.iter().cloned())
        .map(|t| t.to_string())
        .collect();

    let mut entries: Vec<ThemeEntry> = bundled_themes()
        .map(|theme| ThemeEntry {
            source: ThemeSource::Slogger,
            theme,
        })
        .collect();
    for (stem, path) in custom.map(theme_files).unwrap_or_default() {
        let parse = if is_yaml(&path) { parse_base16 } else { parse_palette };
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|src| parse(&src, &stem));
        let Some((name, palette)) = parsed else {
            tracing::warn!(path = %path.display(), "skipping unreadable theme file");
            continue;
        };
        let name = if names.insert(name.clone()) {
            name
        } else {
            let tagged = format!("{name} (custom)");
            if !names.insert(tagged.clone()) {
                continue;
            }
            tagged
        };
        entries.push(ThemeEntry {
            source: ThemeSource::Custom,
            theme: Theme::custom(name, palette),
        });
    }
    entries.extend(Theme::ALL.iter().cloned().map(|theme| ThemeEntry {
        source: ThemeSource::BuiltIn,
        theme,
    }));
    entries
}

/// The palette files in the themes directory, as `(stem, path)` sorted by
/// stem: `<stem>.toml`, `<stem>/colors.toml`, or a Base16 scheme as
/// `<stem>.yaml` / `<stem>.yml`.
fn theme_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut files: Vec<(String, PathBuf)> = read
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                let colors = path.join("colors.toml");
                let stem = path.file_name()?.to_str()?.to_owned();
                colors.is_file().then_some((stem, colors))
            } else if is_toml(&path) || is_yaml(&path) {
                Some((path.file_stem()?.to_str()?.to_owned(), path))
            } else {
                None
            }
        })
        .collect();
    files.sort();
    files
}

fn is_toml(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "toml")
}

fn is_yaml(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "yaml" || e == "yml")
}

/// Reads a Base16 scheme: `base00` (background) and `base05` (text) are
/// required; `base0D` (blue), `base0B` (green), `base0A` (yellow) and
/// `base08` (red) fall back to slogger's own colours. Handles both the
/// current layout (`name:` plus colours nested under `palette:`) and the
/// older flat one (`scheme:` plus top-level colours), which is all the
/// YAML these files use — so the lines are read directly.
fn parse_base16(src: &str, stem: &str) -> Option<(String, Palette)> {
    let mut name = None;
    let mut colors: Vec<(String, Color)> = vec![];
    for line in src.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        // A quoted value ends at its closing quote; a bare one at a comment.
        let value = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or(""),
            _ => value.split(" #").next().unwrap_or("").trim(),
        };
        if key == "name" || key == "scheme" {
            name.get_or_insert_with(|| value.to_owned());
        } else if let Some(color) = key.starts_with("base").then(|| parse_hex(value)).flatten() {
            colors.push((key, color));
        }
    }
    let get = |key: &str| colors.iter().find(|(k, _)| k == key).map(|(_, c)| *c);
    let fallback = app_theme().palette();
    Some((
        name.filter(|n| !n.is_empty())
            .unwrap_or_else(|| title_case(stem)),
        Palette {
            background: get("base00")?,
            text: get("base05")?,
            primary: get("base0d").unwrap_or(fallback.primary),
            success: get("base0b").unwrap_or(fallback.success),
            warning: get("base0a").unwrap_or(fallback.warning),
            danger: get("base08").unwrap_or(fallback.danger),
        },
    ))
}

/// Reads a palette from a TOML file of `key = "#rrggbb"` lines (the
/// layout Omarchy's `colors.toml` uses): `background` and `foreground`
/// are required; `accent` (else `blue`), `green`, `yellow`
/// and `red` fall back to slogger's own colours. An optional `name`
/// overrides the one derived from the file's stem.
fn parse_palette(src: &str, stem: &str) -> Option<(String, Palette)> {
    let table: toml::Table = src.parse().ok()?;
    let get = |key: &str| table.get(key).and_then(|v| v.as_str()).and_then(parse_hex);
    let fallback = app_theme().palette();
    let name = table
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| title_case(stem));
    Some((
        name,
        Palette {
            background: get("background")?,
            text: get("foreground")?,
            primary: get("accent").or_else(|| get("blue")).unwrap_or(fallback.primary),
            success: get("green").unwrap_or(fallback.success),
            warning: get("yellow").unwrap_or(fallback.warning),
            danger: get("red").unwrap_or(fallback.danger),
        },
    ))
}

/// `#rrggbb` (the `#` is optional) to a colour.
fn parse_hex(s: &str) -> Option<Color> {
    let hex = s.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Color::from_rgb8(channel(0)?, channel(2)?, channel(4)?))
}

/// `tokyo-night` to `Tokyo Night`.
fn title_case(stem: &str) -> String {
    stem.split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().map(|c| c.to_uppercase().to_string());
            first.unwrap_or_default() + chars.as_str()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Looks a theme up by the name the picker shows (and the layout file
/// stores).
pub(super) fn theme_by_name(themes: &[ThemeEntry], name: &str) -> Option<Theme> {
    themes
        .iter()
        .find(|entry| entry.theme.to_string() == name)
        .map(|entry| entry.theme.clone())
}

pub(super) const FONT_SCALE_MIN: f32 = 0.6;
pub(super) const FONT_SCALE_MAX: f32 = 2.0;
pub(super) const FONT_SCALE_STEP: f32 = 0.1;

// ---- text styles ----

/// De-emphasised text: labels, headers, already-worked rows.
pub(super) fn muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(Color {
            a: 0.55,
            ..theme.palette().text
        }),
    }
}

/// A small muted label (field captions, column headers, hints).
pub(super) fn label<'a>(s: impl text::IntoFragment<'a>) -> Text<'a> {
    text(s).size(LABEL_TEXT).style(muted)
}

// ---- tables ----

/// One fixed-width monospace table cell. Never wraps, so every row keeps
/// the same height regardless of content.
pub(super) fn cell<'a>(s: impl text::IntoFragment<'a>, width: f32) -> Text<'a> {
    text(s)
        .size(ROW_TEXT)
        .font(MONO)
        .width(Length::Fixed(width))
        .wrapping(text::Wrapping::None)
}

/// A right-aligned [`cell`], for numeric columns.
pub(super) fn cell_right<'a>(s: impl text::IntoFragment<'a>, width: f32) -> Text<'a> {
    cell(s, width).align_x(text::Alignment::Right)
}

/// The last, stretchy column of a table. Clipped rather than wrapped.
pub(super) fn cell_fill<'a>(s: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    container(
        text(s)
            .size(ROW_TEXT)
            .font(MONO)
            .wrapping(text::Wrapping::None),
    )
    .width(Length::Fill)
    .clip(true)
    .into()
}

const ROW_PADDING: [u16; 2] = [2, 6];
const CELL_SPACING: f32 = 10.0;

/// A table header: same geometry as [`table_row`] so columns line up.
pub(super) fn table_header<'a>(cells: Row<'a, Message>) -> Element<'a, Message> {
    container(cells.spacing(CELL_SPACING))
        .padding(ROW_PADDING)
        .width(Length::Fill)
        .style(|theme: &Theme| container::Style {
            text_color: Some(Color {
                a: 0.55,
                ..theme.palette().text
            }),
            ..container::Style::default()
        })
        .into()
}

/// A clickable table row. `highlighted` rows get a persistent background
/// (the QSO being edited, a spot matching the award target).
pub(super) fn table_row<'a>(
    cells: Row<'a, Message>,
    on_press: Message,
    highlighted: bool,
) -> Element<'a, Message> {
    button(cells.spacing(CELL_SPACING))
        .padding(ROW_PADDING)
        .width(Length::Fill)
        .on_press(on_press)
        .style(move |theme: &Theme, status| {
            let ext = theme.extended_palette();
            let background = if highlighted {
                Some(ext.primary.weak.color.scale_alpha(0.35).into())
            } else {
                match status {
                    button::Status::Hovered | button::Status::Pressed => {
                        Some(ext.background.weak.color.into())
                    }
                    _ => None,
                }
            };
            button::Style {
                background,
                text_color: theme.palette().text,
                border: Border {
                    radius: 3.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// A table row with nothing to click: same geometry as [`table_header`]
/// and [`table_row`] so the columns line up.
pub(super) fn table_line<'a>(cells: Row<'a, Message>) -> Element<'a, Message> {
    container(cells.spacing(CELL_SPACING))
        .padding(ROW_PADDING)
        .width(Length::Fill)
        .into()
}

// ---- containers ----

/// Pane body: a hairline border so the operator can see where panes end.
pub(super) fn pane_content_style(theme: &Theme) -> container::Style {
    let ext = theme.extended_palette();
    container::Style {
        background: Some(ext.background.base.color.into()),
        border: Border {
            color: ext.background.strong.color,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    }
}

/// Pane title bar: one step lighter than the body so the drag handle reads.
pub(super) fn title_bar_style(theme: &Theme) -> container::Style {
    let ext = theme.extended_palette();
    container::Style {
        background: Some(ext.background.weak.color.into()),
        text_color: Some(Color {
            a: 0.7,
            ..theme.palette().text
        }),
        border: Border {
            radius: iced::border::top(4.0),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// A floating panel: opaque (panels can overlap) with a border strong
/// enough to separate it from whatever it sits on.
pub(super) fn panel_style(theme: &Theme) -> container::Style {
    let ext = theme.extended_palette();
    container::Style {
        background: Some(ext.background.base.color.into()),
        border: Border {
            color: ext.background.strong.color,
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: iced::Shadow {
            color: Color { a: 0.35, ..Color::BLACK },
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..container::Style::default()
    }
}

/// The canvas the floating panels sit on: slightly darker than a panel.
pub(super) fn canvas_style(theme: &Theme) -> container::Style {
    let base = theme.extended_palette().background.base.color;
    container::Style {
        background: Some(
            Color {
                r: base.r * 0.75,
                g: base.g * 0.75,
                b: base.b * 0.75,
                a: 1.0,
            }
            .into(),
        ),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Quiet strip used for the bottom status bar.
pub(super) fn strip_style(theme: &Theme) -> container::Style {
    let ext = theme.extended_palette();
    container::Style {
        background: Some(ext.background.weak.color.into()),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Filled pill for attention-grabbing badges (DUPE, NEW).
pub(super) fn badge<'a>(s: impl text::IntoFragment<'a>, color: Color) -> Element<'a, Message> {
    container(text(s).size(12).font(MONO).color(Color::BLACK))
        .padding([1, 6])
        .style(move |_theme: &Theme| container::Style {
            background: Some(color.into()),
            border: Border {
                radius: 3.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// A tab in the top view switcher.
pub(super) fn tab_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme: &Theme, status| {
        let ext = theme.extended_palette();
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if active {
                Some(ext.background.strong.color.into())
            } else if hovered {
                Some(ext.background.weak.color.into())
            } else {
                None
            },
            text_color: if active {
                theme.palette().text
            } else {
                Color {
                    a: 0.6,
                    ..theme.palette().text
                }
            },
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

/// Formats a frequency the way a rig display does: `14.074.00`
/// (MHz . kHz . tens-of-Hz).
pub(super) fn format_rig_freq(hz: u64) -> String {
    format!(
        "{}.{:03}.{:02}",
        hz / 1_000_000,
        (hz / 1_000) % 1_000,
        (hz / 10) % 100
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const JADE: &str = r##"
mode = "dark"
accent = "#509475"
background = "#111c18"
foreground = "#C1C497"
red = "#FF5345"
yellow = "#459451"
green = "#549e6a"
"##;

    #[test]
    fn a_palette_file_maps_onto_the_theme_colours() {
        let (name, palette) = parse_palette(JADE, "osaka-jade").unwrap();
        assert_eq!(name, "Osaka Jade");
        assert_eq!(palette.background, Color::from_rgb8(0x11, 0x1c, 0x18));
        assert_eq!(palette.text, Color::from_rgb8(0xc1, 0xc4, 0x97));
        assert_eq!(palette.primary, Color::from_rgb8(0x50, 0x94, 0x75));
        assert_eq!(palette.danger, Color::from_rgb8(0xff, 0x53, 0x45));
    }

    #[test]
    fn a_palette_needs_a_background_and_a_foreground() {
        assert!(parse_palette("background = \"#000000\"", "x").is_none());
        assert!(parse_palette("not toml at all [", "x").is_none());
        let (name, palette) =
            parse_palette("name = \"Mine\"\nbackground = \"000000\"\nforeground = \"#ffffff\"", "x")
                .unwrap();
        assert_eq!(name, "Mine");
        assert_eq!(palette.primary, app_theme().palette().primary);
    }

    #[test]
    fn a_base16_scheme_reads_in_either_layout() {
        let current = r##"
system: "base16"
name: "OneDark"
author: "Someone (http://example.com)"
variant: "dark"
palette:
  base00: "#282c34"
  base05: "#abb2bf"
  base08: "#e06c75"
  base0A: "#e5c07b"
  base0B: "#98c379"
  base0D: "#61afef"
"##;
        let (name, palette) = parse_base16(current, "onedark").unwrap();
        assert_eq!(name, "OneDark");
        assert_eq!(palette.background, Color::from_rgb8(0x28, 0x2c, 0x34));
        assert_eq!(palette.text, Color::from_rgb8(0xab, 0xb2, 0xbf));
        assert_eq!(palette.primary, Color::from_rgb8(0x61, 0xaf, 0xef));
        assert_eq!(palette.success, Color::from_rgb8(0x98, 0xc3, 0x79));
        assert_eq!(palette.warning, Color::from_rgb8(0xe5, 0xc0, 0x7b));
        assert_eq!(palette.danger, Color::from_rgb8(0xe0, 0x6c, 0x75));

        let flat = "scheme: 'Old Style'\nbase00: 181818 # background\nbase05: \"d8d8d8\"\n";
        let (name, palette) = parse_base16(flat, "old").unwrap();
        assert_eq!(name, "Old Style");
        assert_eq!(palette.background, Color::from_rgb8(0x18, 0x18, 0x18));
        assert_eq!(palette.primary, app_theme().palette().primary);

        assert!(parse_base16("base00: \"#181818\"", "x").is_none());
        let (name, _) = parse_base16("base00: 000000\nbase05: ffffff", "my-scheme").unwrap();
        assert_eq!(name, "My Scheme");
    }

    #[test]
    fn theme_names_are_unique_and_custom_files_join_the_list() {
        let dir = std::env::temp_dir().join(format!("slogger-themes-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("in-a-folder")).unwrap();
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        std::fs::write(dir.join("shack_red.toml"), JADE).unwrap();
        std::fs::write(dir.join("nord.toml"), JADE).unwrap();
        std::fs::write(dir.join("broken.toml"), "nope").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a theme").unwrap();
        std::fs::write(
            dir.join("tube.yaml"),
            "name: \"Tube\"\npalette:\n  base00: \"#231f20\"\n  base05: \"#d9d8d8\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("in-a-folder/colors.toml"), JADE).unwrap();

        let entries = themes_from(Some(&dir));
        std::fs::remove_dir_all(&dir).unwrap();

        let names = |source| -> Vec<String> {
            entries
                .iter()
                .filter(|e| e.source == source)
                .map(|e| e.theme.to_string())
                .collect()
        };
        let shipped = names(ThemeSource::Slogger);
        assert_eq!(shipped[0], "slogger");
        assert_eq!(shipped.len(), 1 + BUNDLED.len());
        // Iced already has a "Nord".
        assert_eq!(
            names(ThemeSource::Custom),
            ["In A Folder", "Nord (custom)", "Shack Red", "Tube"]
        );
        assert_eq!(names(ThemeSource::BuiltIn).len(), Theme::ALL.len());

        let unique: HashSet<String> = entries.iter().map(|e| e.theme.to_string()).collect();
        assert_eq!(unique.len(), entries.len());
        assert!(theme_by_name(&entries, "Osaka Jade").is_some());
        assert!(theme_by_name(&entries, "Nope").is_none());
    }

    #[test]
    fn nothing_but_the_fixed_themes_without_a_themes_directory() {
        let entries = themes_from(None);
        assert_eq!(entries.len(), 1 + BUNDLED.len() + Theme::ALL.len());
    }

    #[test]
    fn rig_freq_groups_mhz_khz_and_tens_of_hz() {
        assert_eq!(format_rig_freq(14_074_000), "14.074.00");
        assert_eq!(format_rig_freq(7_005_250), "7.005.25");
        assert_eq!(format_rig_freq(144_200_000), "144.200.00");
    }
}
