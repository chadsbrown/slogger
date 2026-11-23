//! Visual language shared by every view: the application theme, the
//! monospace font used for tabular data, and the small set of container /
//! button / text styles the panes are built from.
//!
//! Views should reach for these helpers rather than hard-coding colours so
//! the whole app can be re-skinned from this one file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

use iced::theme::Palette;
use iced::theme::palette::{Extended, Pair};
use iced::widget::{Row, Text, button, container, text};
use iced::{Border, Color, Element, Font, Length, Theme};

use super::message::Message;

/// Font for anything columnar or numeric: callsigns, frequencies, tables.
pub(super) const MONO: Font = Font::MONOSPACE;
/// Callsigns and frequencies: the things an operator hunts for.
pub(super) const MONO_BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..Font::MONOSPACE
};

/// Body size for table rows and dense readouts.
pub(super) const ROW_TEXT: f32 = 13.0;
/// Size for field labels, column headers and hints.
pub(super) const LABEL_TEXT: f32 = 11.0;

mod community;
mod spec;

pub(super) use spec::{EDGE_MAX, Role, Spec, from_hsl, to_hex, to_hsl};

/// Everything a theme defines. iced's own [`Palette`] has six of these;
/// the rest are the surface shades and the extra hues that give a scheme
/// its character.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Scheme {
    pub(super) background: Color,
    /// One step off the background: title bars, hovered rows, the status
    /// strip.
    pub(super) surface: Color,
    /// Two steps off: borders and the active tab.
    pub(super) line: Color,
    /// De-emphasised text.
    pub(super) muted: Color,
    pub(super) text: Color,
    pub(super) accent: Color,
    pub(super) green: Color,
    pub(super) yellow: Color,
    pub(super) red: Color,
    pub(super) cyan: Color,
    pub(super) magenta: Color,
    pub(super) orange: Color,
}

impl Scheme {
    /// In the order of the fields above.
    const fn from_hex(c: [u32; 12]) -> Self {
        Scheme {
            background: rgb(c[0]),
            surface: rgb(c[1]),
            line: rgb(c[2]),
            muted: rgb(c[3]),
            text: rgb(c[4]),
            accent: rgb(c[5]),
            green: rgb(c[6]),
            yellow: rgb(c[7]),
            red: rgb(c[8]),
            cyan: rgb(c[9]),
            magenta: rgb(c[10]),
            orange: rgb(c[11]),
        }
    }

    /// Fills in what a six-colour palette leaves out by blending the
    /// colours it does have.
    fn from_palette(p: Palette) -> Self {
        Scheme {
            background: p.background,
            surface: mix(p.background, p.text, 0.07),
            line: mix(p.background, p.text, 0.2),
            muted: mix(p.background, p.text, 0.6),
            text: p.text,
            accent: p.primary,
            green: p.success,
            yellow: p.warning,
            red: p.danger,
            cyan: mix(p.primary, p.success, 0.5),
            magenta: mix(p.primary, p.danger, 0.5),
            orange: mix(p.warning, p.danger, 0.5),
        }
    }

    /// The status colours double as text (a needed spot, a poor band), so
    /// they are nudged until they can be read on the background.
    fn palette(&self) -> Palette {
        let on = |c| legible(c, self.background, 3.0);
        Palette {
            background: self.background,
            text: self.text,
            primary: on(self.accent),
            success: on(self.green),
            warning: on(self.yellow),
            danger: on(self.red),
        }
    }

    /// The colours table text is drawn in, each made readable on this
    /// scheme's background.
    fn hues(&self) -> Hues {
        let on = |c| legible(c, self.background, 4.5);
        Hues {
            muted: on(self.muted),
            call: on(self.orange),
            freq: on(self.cyan),
            band: on(self.magenta),
            mode: on(self.accent),
            select: mix(self.background, self.accent, 0.25),
            select_text: None,
            hover: self.surface,
            control: self.surface,
            edge: 1.0,
            status: StatusLook::Cell,
            zebra: false,
            bar: self.accent,
            bar_text: ink_on(self.accent),
            rig: self.accent,
            rig_text: ink_on(self.accent),
            pane: self.surface,
            chrome: mix(self.background, Color::BLACK, if dark(self.background) { 0.3 } else { 0.22 }),
            button: self.line,
            button_text: self.text,
            good: self.green,
            warn: self.yellow,
            bad: self.red,
        }
    }

    /// Builds the iced theme: iced derives its shades from the palette,
    /// then the scheme's own surfaces replace the blended ones.
    fn theme(&self, name: impl Into<String>) -> Theme {
        self.theme_with(name, self.hues())
    }

    /// [`Scheme::theme`], with table text in `hues` rather than the
    /// scheme's usual choice.
    fn theme_with(&self, name: impl Into<String>, hues: Hues) -> Theme {
        let name = name.into();
        if let Ok(mut registry) = HUES.write() {
            registry.insert(name.clone(), hues);
        }
        // A highlighted row that inverts (white on a navy bar, say) is
        // drawn in a twin of the theme whose text is all one colour.
        if let Some(ink) = hues.select_text {
            let twin_name = format!("{name} (selected row)");
            let twin_hues = Hues {
                muted: ink,
                call: ink,
                freq: ink,
                band: ink,
                mode: ink,
                ..hues
            };
            let twin = Scheme { text: ink, muted: ink, background: hues.select, ..*self };
            if let (Ok(mut registry), Ok(mut twins)) = (HUES.write(), SELECTED.write()) {
                registry.insert(twin_name.clone(), twin_hues);
                twins.insert(name.clone(), twin.build(twin_name, twin_hues));
            }
        }
        self.build(name, hues)
    }

    fn build(&self, name: String, hues: Hues) -> Theme {
        let scheme = *self;
        Theme::custom_with_fn(name, self.palette(), move |palette| {
            let mut ext = Extended::generate(palette);
            // Drop-downs and scrollbar rails take the weak shade.
            ext.background.weak = Pair::new(hues.control, scheme.text);
            ext.background.strong = Pair::new(scheme.line, scheme.text);
            ext
        })
    }
}

/// Text colours for the kinds of data the tables show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Hues {
    pub(super) muted: Color,
    pub(super) call: Color,
    pub(super) freq: Color,
    pub(super) band: Color,
    pub(super) mode: Color,
    /// Behind a highlighted table row. The data colours above have to
    /// stay readable on it.
    pub(super) select: Color,
    /// Set when a highlighted row inverts: every cell on it is drawn in
    /// this one colour.
    pub(super) select_text: Option<Color>,
    /// Behind a row under the pointer.
    pub(super) hover: Color,
    /// The face of a drop-down.
    pub(super) control: Color,
    /// Border width around panes, tables and buttons.
    pub(super) edge: f32,
    /// How a status (a needed spot, a band rating) is shown.
    pub(super) status: StatusLook,
    /// Whether every other table row is shaded.
    pub(super) zebra: bool,
    /// A title bar or the active tab, and the text on it.
    pub(super) bar: Color,
    pub(super) bar_text: Color,
    /// The active radio in the Station pane, when there is more than
    /// one, and everything written on it.
    pub(super) rig: Color,
    pub(super) rig_text: Color,
    /// The window frame: what shows around and between the panes.
    pub(super) chrome: Color,
    /// The body of a pane or panel. Tables and entry fields inside it sit
    /// on the theme's background, a third tone.
    pub(super) pane: Color,
    /// A push button's face, and the text on it.
    pub(super) button: Color,
    pub(super) button_text: Color,
    /// Fills for status cells: something wanted, a caution, a problem.
    pub(super) good: Color,
    pub(super) warn: Color,
    pub(super) bad: Color,
}

/// How a theme shows a status. Every shipped theme fills a cell; a
/// theme of the user's own can choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLook {
    /// A cell filled with the status colour.
    Cell,
    /// Bold text in the status colour.
    Text,
    /// The whole table row tinted with the status colour.
    Row,
}

/// The hues of every scheme-built theme, by theme name. An iced `Theme`
/// has no room for them, and the style functions only get the theme.
static HUES: LazyLock<RwLock<HashMap<String, Hues>>> = LazyLock::new(Default::default);

/// For each theme whose highlighted rows invert, the twin theme those
/// rows are drawn in, by theme name.
static SELECTED: LazyLock<RwLock<HashMap<String, Theme>>> = LazyLock::new(Default::default);

/// The theme a highlighted row's cells are drawn in, if `theme` inverts
/// them.
fn selected_row_theme(theme: &Theme) -> Option<Theme> {
    SELECTED.read().ok()?.get(&theme.to_string()).cloned()
}

/// The data hues that go with a theme. Themes not built from a [`Scheme`]
/// (iced's built-ins) get hues blended from their palette.
pub(super) fn hues(theme: &Theme) -> Hues {
    HUES.read()
        .ok()
        .and_then(|hues| hues.get(&theme.to_string()).copied())
        .unwrap_or_else(|| Scheme::from_palette(theme.palette()).hues())
}

const fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// `a` moved `amount` of the way to `b`.
fn mix(a: Color, b: Color, amount: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * amount,
        a.g + (b.g - a.g) * amount,
        a.b + (b.b - a.b) * amount,
    )
}

/// WCAG relative luminance: 0 for black, 1 for white.
fn luminance(c: Color) -> f32 {
    let linear = |v: f32| {
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

/// WCAG contrast ratio between two colours, from 1 (identical) to 21.
fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Whether light text belongs on `background`.
fn dark(background: Color) -> bool {
    luminance(background) < 0.18
}

/// Black or white, whichever reads better on `on`.
pub(super) fn ink_on(on: Color) -> Color {
    if contrast(Color::BLACK, on) >= contrast(Color::WHITE, on) {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

/// `color`, lightened on a dark background (or darkened on a light one)
/// just far enough to reach the `min` contrast ratio against it.
fn legible(color: Color, background: Color, min: f32) -> Color {
    let toward = if dark(background) { Color::WHITE } else { Color::BLACK };
    let mut out = color;
    for step in 1..=20 {
        if contrast(out, background) >= min {
            break;
        }
        out = mix(color, toward, step as f32 * 0.05);
    }
    out
}

/// slogger's original theme: a low-glare dark palette suited to a shack
/// at night.
const SLOGGER: Scheme = Scheme::from_hex([
    0x15181d, 0x1f242b, 0x39404a, 0x8a929d, 0xd5dae1, 0x4f9fe8,
    0x58c27d, 0xe8a33d, 0xe5604f, 0x4fc3c9, 0xb48ae0, 0xe88a4a,
]);

pub(super) fn app_theme() -> Theme {
    SLOGGER.theme("slogger")
}

/// A theme designed for slogger, with every colour chosen by hand.
struct Look {
    name: &'static str,
    /// The window frame around the panes.
    chrome: u32,
    /// The body of a pane.
    pane: u32,
    /// Where data sits inside a pane: tables and entry fields.
    field: u32,
    /// A hovered row.
    surface: u32,
    /// Borders.
    line: u32,
    text: u32,
    muted: u32,
    /// Primary buttons and links.
    accent: u32,
    call: u32,
    freq: u32,
    green: u32,
    amber: u32,
    red: u32,
    magenta: u32,
    /// The bar behind a highlighted row; text, callsigns and frequencies
    /// all have to read on it.
    select: u32,
    /// Set when that bar inverts the row: the one colour of its text.
    select_text: Option<u32>,
    /// Title bars and the active tab, and the text on them.
    bar: u32,
    bar_text: u32,
    /// Push buttons.
    button: u32,
    /// Border width.
    edge: f32,
}

/// The designed themes: six families (Light, Warm, Cool, Grey, Dark,
/// Blue), each at three strengths of contrast (Soft, Standard, High) so
/// one can be found for the light in the room, then two for a dark shack.
/// Colour is used the way traditional logging programs use it: three
/// grounds (the window frame, the panes on it, and the tables and fields
/// in them), solid bars and status cells, and plain text for everything
/// that is not a callsign, a frequency or a status. The family tables are
/// generated; contrast is checked by the tests below.
const LOOKS: &[Look] = &[
    Look {
        name: "Light Soft",
        chrome: 0xdbdbdb, pane: 0xededed, field: 0xf8f8f8, surface: 0xe3e3ee, line: 0xa4a4a4,
        text: 0x333333, muted: 0x636363, accent: 0x4747a4, call: 0x0000c0, freq: 0x005a5a,
        green: 0x007000, amber: 0xa85000, red: 0xc00000, magenta: 0x800080, select: 0x4747a4,
        bar: 0x4747a4, bar_text: 0xffffff, button: 0xf8f8f8,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Light Standard",
        chrome: 0xababab, pane: 0xd7d7d7, field: 0xfcfcfc, surface: 0xdedeed, line: 0x636363,
        text: 0x0b0b0b, muted: 0x424242, accent: 0x000080, call: 0x0000c0, freq: 0x005a5a,
        green: 0x007000, amber: 0xa85000, red: 0xc00000, magenta: 0x800080, select: 0x000080,
        bar: 0x000080, bar_text: 0xffffff, button: 0xebebeb,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Light High",
        chrome: 0x5d5d5d, pane: 0xbababa, field: 0xffffff, surface: 0xe0e0ea, line: 0x060606,
        text: 0x000000, muted: 0x1b1b1b, accent: 0x000053, call: 0x0000c0, freq: 0x005a5a,
        green: 0x007000, amber: 0xa85000, red: 0xc00000, magenta: 0x800080, select: 0x000053,
        bar: 0x000053, bar_text: 0xffffff, button: 0xd2d2d2,
        select_text: Some(0xffffff), edge: 2.0,
    },
    Look {
        name: "Warm Soft",
        chrome: 0xe5d9c7, pane: 0xf7ebd8, field: 0xfef8ee, surface: 0xf3e5da, line: 0xaea391,
        text: 0x37322b, muted: 0x67635a, accent: 0x9f5747, call: 0x0000c0, freq: 0x005a5a,
        green: 0x007000, amber: 0xa85000, red: 0xc00000, magenta: 0x800080, select: 0x9f5747,
        bar: 0x9f5747, bar_text: 0xffffff, button: 0xfcf7ef,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Warm Standard",
        chrome: 0xb4a997, pane: 0xe1d6c3, field: 0xfefbf7, surface: 0xeedfd9, line: 0x6c6251,
        text: 0x0e0b05, muted: 0x46423a, accent: 0x7a1500, call: 0x0000c0, freq: 0x005a5a,
        green: 0x007000, amber: 0xa85000, red: 0xc00000, magenta: 0x800080, select: 0x7a1500,
        bar: 0x7a1500, bar_text: 0xffffff, button: 0xf0eae1,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Warm High",
        chrome: 0x665c4c, pane: 0xc4b9a7, field: 0xffffff, surface: 0xeae2e0, line: 0x0a0500,
        text: 0x000000, muted: 0x1e1a13, accent: 0x4f0e00, call: 0x0000c0, freq: 0x005a5a,
        green: 0x007000, amber: 0xa85000, red: 0xc00000, magenta: 0x800080, select: 0x4f0e00,
        bar: 0x4f0e00, bar_text: 0xffffff, button: 0xd9d2c6,
        select_text: Some(0xffffff), edge: 2.0,
    },
    Look {
        name: "Cool Soft",
        chrome: 0xcfdde9, pane: 0xe1effb, field: 0xf5f9fd, surface: 0xe0eaf4, line: 0x9aa6b2,
        text: 0x2e3438, muted: 0x5e6469, accent: 0x477baf, call: 0x0000c0, freq: 0x005a5a,
        green: 0x007000, amber: 0xa85000, red: 0xc00000, magenta: 0x800080, select: 0x477baf,
        bar: 0x477baf, bar_text: 0x000000, button: 0xf3f9fd,
        select_text: Some(0x000000), edge: 1.0,
    },
    Look {
        name: "Cool Standard",
        chrome: 0xa0adb8, pane: 0xccd9e5, field: 0xfafcfe, surface: 0xdce6f1, line: 0x59656f,
        text: 0x080c10, muted: 0x3e4348, accent: 0x004890, call: 0x0000c0, freq: 0x005a5a,
        green: 0x007000, amber: 0xa85000, red: 0xc00000, magenta: 0x800080, select: 0x004890,
        bar: 0x004890, bar_text: 0xffffff, button: 0xe6ecf2,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Cool High",
        chrome: 0x545f69, pane: 0xafbcc8, field: 0xffffff, surface: 0xe0e6ec, line: 0x01060d,
        text: 0x000000, muted: 0x161b1f, accent: 0x002f5e, call: 0x0000c0, freq: 0x005a5a,
        green: 0x007000, amber: 0xa85000, red: 0xc00000, magenta: 0x800080, select: 0x002f5e,
        bar: 0x002f5e, bar_text: 0xffffff, button: 0xcbd3db,
        select_text: Some(0xffffff), edge: 2.0,
    },
    Look {
        name: "Grey Soft",
        chrome: 0x8c8c8c, pane: 0xa8a8a8, field: 0xc7c7c7, surface: 0xb8b8c3, line: 0x636363,
        text: 0x090909, muted: 0x333333, accent: 0x4747a4, call: 0x0000c0, freq: 0x004848,
        green: 0x006400, amber: 0x8c4000, red: 0xc00000, magenta: 0x800080, select: 0x4747a4,
        bar: 0x4747a4, bar_text: 0xffffff, button: 0xdcdcdc,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Grey Standard",
        chrome: 0x636363, pane: 0x989898, field: 0xd7d7d7, surface: 0xbdbdcd, line: 0x3d3d3d,
        text: 0x000000, muted: 0x242424, accent: 0x000080, call: 0x0000c0, freq: 0x004848,
        green: 0x006400, amber: 0x8c4000, red: 0xc00000, magenta: 0x800080, select: 0x000080,
        bar: 0x000080, bar_text: 0xffffff, button: 0xcccccc,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Grey High",
        chrome: 0x383838, pane: 0x868686, field: 0xf5f5f5, surface: 0xd8d8e2, line: 0x030303,
        text: 0x000000, muted: 0x121212, accent: 0x000053, call: 0x0000c0, freq: 0x004848,
        green: 0x006400, amber: 0x8c4000, red: 0xc00000, magenta: 0x800080, select: 0x000053,
        bar: 0x000053, bar_text: 0xffffff, button: 0xb0b0b0,
        select_text: Some(0xffffff), edge: 2.0,
    },
    Look {
        name: "Dark Soft",
        chrome: 0x3e4043, pane: 0x595b5e, field: 0x44464a, surface: 0x55575a, line: 0x787a7e,
        text: 0xeaebed, muted: 0xccced0, accent: 0x2f7cff, call: 0xffe000, freq: 0x40e0e0,
        green: 0x20d040, amber: 0xff9500, red: 0xff6a6a, magenta: 0xff60ff, select: 0x0048b8,
        bar: 0x002e80, bar_text: 0xffffff, button: 0x66686b,
        select_text: None, edge: 1.0,
    },
    Look {
        name: "Dark Standard",
        chrome: 0x1e1f22, pane: 0x4b4d50, field: 0x2f3033, surface: 0x424345, line: 0x8a8c90,
        text: 0xf4f5f7, muted: 0xccced0, accent: 0x2f7cff, call: 0xffe000, freq: 0x40e0e0,
        green: 0x20d040, amber: 0xff9500, red: 0xff6a6a, magenta: 0xff60ff, select: 0x0048b8,
        bar: 0x002e80, bar_text: 0xffffff, button: 0x595b5e,
        select_text: None, edge: 1.0,
    },
    Look {
        name: "Dark High",
        chrome: 0x0a0b0e, pane: 0x434548, field: 0x17181b, surface: 0x2c2d30, line: 0xcfd1d5,
        text: 0xffffff, muted: 0xdddee0, accent: 0x2f7cff, call: 0xffe000, freq: 0x40e0e0,
        green: 0x20d040, amber: 0xff9500, red: 0xff6a6a, magenta: 0xff60ff, select: 0x0048b8,
        bar: 0x001a58, bar_text: 0xffffff, button: 0x525457,
        select_text: None, edge: 2.0,
    },
    Look {
        name: "Blue Soft",
        chrome: 0x1b2c51, pane: 0x2d4a88, field: 0x1f376e, surface: 0x33497b, line: 0x7786a5,
        text: 0xe7effd, muted: 0xc2cbdb, accent: 0x00a8a8, call: 0xffff55, freq: 0x55ffff,
        green: 0x55ff55, amber: 0xffaa00, red: 0xff6060, magenta: 0xff55ff, select: 0xa800a8,
        bar: 0x00a8a8, bar_text: 0x000000, button: 0x3e5892,
        select_text: None, edge: 1.0,
    },
    Look {
        name: "Blue Standard",
        chrome: 0x02103e, pane: 0x062da1, field: 0x01166e, surface: 0x182b7b, line: 0x71809f,
        text: 0xffffff, muted: 0xc9d1e2, accent: 0x00a8a8, call: 0xffff55, freq: 0x55ffff,
        green: 0x55ff55, amber: 0xffaa00, red: 0xff6060, magenta: 0xff55ff, select: 0xa800a8,
        bar: 0x00a8a8, bar_text: 0x000000, button: 0x1a3ea9,
        select_text: None, edge: 1.0,
    },
    Look {
        name: "Blue High",
        chrome: 0x000316, pane: 0x0029c1, field: 0x00014c, surface: 0x17185c, line: 0x8f9fbe,
        text: 0xffffff, muted: 0xd6deef, accent: 0x00a8a8, call: 0xffff55, freq: 0x55ffff,
        green: 0x55ff55, amber: 0xffaa00, red: 0xff6060, magenta: 0xff55ff, select: 0xa800a8,
        bar: 0x00a8a8, bar_text: 0x000000, button: 0x143ac6,
        select_text: None, edge: 2.0,
    },
    // A dark shack: near-black, with everything dimmed.
    Look {
        name: "Night",
        chrome: 0x000000, pane: 0x161616, field: 0x060606, surface: 0x121212, line: 0x303030,
        text: 0xa8a8a8, muted: 0x808080, accent: 0x3a68b8,
        call: 0xc8a000, freq: 0x00a0a8,
        green: 0x18a030, amber: 0xc07000, red: 0xd03030, magenta: 0xa050a0,
        select: 0x14284c, select_text: None, bar: 0x2c3a50, bar_text: 0xa8a8a8, button: 0x202020, edge: 1.0,
    },
    // Red only, to keep the eyes adapted to the dark.
    Look {
        name: "Night Red",
        chrome: 0x000000, pane: 0x340000, field: 0x060000, surface: 0x180000, line: 0x600000,
        text: 0xff5a4a, muted: 0xec3434, accent: 0xc00000,
        call: 0xff8a3c, freq: 0xff5a4a,
        green: 0xff9a00, amber: 0xff6a00, red: 0xff0000, magenta: 0xd03060,
        select: 0x480000, select_text: None, bar: 0x780000, bar_text: 0xffc0b0, button: 0x500000, edge: 1.0,
    },
];

impl Look {
    fn scheme(&self) -> Scheme {
        Scheme::from_hex([
            self.field, self.surface, self.line, self.muted, self.text, self.accent,
            self.green, self.amber, self.red, self.freq, self.magenta, self.call,
        ])
    }

    /// The hues exactly as chosen: nothing is nudged, and bands and modes
    /// stay in the text colour.
    fn hues(&self) -> Hues {
        Hues {
            muted: rgb(self.muted),
            call: rgb(self.call),
            freq: rgb(self.freq),
            band: rgb(self.text),
            mode: rgb(self.text),
            select: rgb(self.select),
            select_text: self.select_text.map(rgb),
            hover: rgb(self.surface),
            control: rgb(self.button),
            edge: self.edge,
            status: StatusLook::Cell,
            zebra: false,
            bar: rgb(self.bar),
            bar_text: rgb(self.bar_text),
            rig: rgb(self.bar),
            rig_text: rgb(self.bar_text),
            chrome: rgb(self.chrome),
            pane: rgb(self.pane),
            button: rgb(self.button),
            button_text: rgb(self.text),
            good: rgb(self.green),
            warn: rgb(self.amber),
            bad: rgb(self.red),
        }
    }
}

fn designed() -> impl Iterator<Item = Theme> {
    LOOKS.iter().map(|look| look.scheme().theme_with(look.name, look.hues()))
}

/// Themes built on the Material Design 2 colour system. A primary colour
/// marks structure (title bars, the active tab, buttons) and its dark
/// variant the window frame; a secondary colour is kept for the one thing
/// selected. Grounds stay neutral, callsigns take the primary's dark
/// variant and all other data is plain, so status colours (never the
/// primary or secondary) are the only other colour on screen. The dark
/// ones follow its dark-theme rules: grey grounds that lighten as they
/// rise, bars in a ground tone, and light, unsaturated accents.
const DUOS: &[Look] = &[
    Look {
        name: "Indigo & Pink",
        chrome: 0x303f9f, pane: 0xd5d9ee, field: 0xffffff, surface: 0xe8eaf6, line: 0x7986cb,
        text: 0x212121, muted: 0x555a66, accent: 0x3f51b5, call: 0x1a237e, freq: 0x212121,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0xff4081,
        bar: 0x3f51b5, bar_text: 0xffffff, button: 0xf5f5f5,
        select_text: Some(0x000000), edge: 1.0,
    },
    Look {
        name: "Cyan & Violet",
        chrome: 0x006064, pane: 0xcde4e7, field: 0xffffff, surface: 0xe0f7fa, line: 0x6ea3a8,
        text: 0x212121, muted: 0x4c5b5d, accent: 0x00838f, call: 0x006064, freq: 0x212121,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0x7c4dff,
        bar: 0x00838f, bar_text: 0xffffff, button: 0xf5f5f5,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Brown & Blue",
        chrome: 0x4e342e, pane: 0xe0d5cf, field: 0xfffbf5, surface: 0xefebe9, line: 0xa1887f,
        text: 0x212121, muted: 0x5d544f, accent: 0x6d4c41, call: 0x4e342e, freq: 0x212121,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0x0277bd,
        bar: 0x6d4c41, bar_text: 0xffffff, button: 0xfaf6f2,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Slate & Cyan",
        chrome: 0x263238, pane: 0xcfd8dc, field: 0xfafafa, surface: 0xeceff1, line: 0x78909c,
        text: 0x212121, muted: 0x4a555a, accent: 0x455a64, call: 0x263238, freq: 0x212121,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0x00b8d4,
        bar: 0x455a64, bar_text: 0xffffff, button: 0xeceff1,
        select_text: Some(0x000000), edge: 1.0,
    },
    // Bars blend into the page, so structure recedes and content leads;
    // the primary shows only in headings, callsigns and the selection.
    Look {
        name: "Paper & Blue",
        chrome: 0xbdbdbd, pane: 0xeeeeee, field: 0xffffff, surface: 0xe3f2fd, line: 0x9e9e9e,
        text: 0x212121, muted: 0x5a5a5a, accent: 0x1976d2, call: 0x0d47a1, freq: 0x212121,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0x1976d2,
        bar: 0xcfcfcf, bar_text: 0x0d47a1, button: 0xffffff,
        select_text: Some(0xffffff), edge: 1.0,
    },
    Look {
        name: "Blue & Amber",
        chrome: 0x0d47a1, pane: 0xd3e0ee, field: 0xffffff, surface: 0xe3f2fd, line: 0x7fa6cf,
        text: 0x212121, muted: 0x4f5a66, accent: 0x1976d2, call: 0x0d47a1, freq: 0x212121,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0xffc107,
        bar: 0x1976d2, bar_text: 0xffffff, button: 0xf5f5f5,
        select_text: Some(0x000000), edge: 1.0,
    },
    Look {
        name: "Purple & Teal",
        chrome: 0x4527a0, pane: 0xddd6ec, field: 0xffffff, surface: 0xede7f6, line: 0x9575cd,
        text: 0x212121, muted: 0x5a5566, accent: 0x673ab7, call: 0x311b92, freq: 0x212121,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0x00bfa5,
        bar: 0x673ab7, bar_text: 0xffffff, button: 0xf5f5f5,
        select_text: Some(0x000000), edge: 1.0,
    },
    // Black, white and one yellow bar, for a bright room.
    Look {
        name: "Ink & Yellow",
        chrome: 0x212121, pane: 0xdcdcdc, field: 0xffffff, surface: 0xeeeeee, line: 0x616161,
        text: 0x000000, muted: 0x424242, accent: 0x424242, call: 0x000000, freq: 0x000000,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0xffd600,
        bar: 0x424242, bar_text: 0xffffff, button: 0xf5f5f5,
        select_text: Some(0x000000), edge: 1.0,
    },
    // Warm, with bars that blend into the page.
    Look {
        name: "Sand & Indigo",
        chrome: 0xc9bfae, pane: 0xefe8da, field: 0xfffdf7, surface: 0xe8eaf6, line: 0xa89f8f,
        text: 0x212121, muted: 0x5c574e, accent: 0x3949ab, call: 0x283593, freq: 0x212121,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0x3949ab,
        bar: 0xd0c5b1, bar_text: 0x283593, button: 0xfffdf7,
        select_text: Some(0xffffff), edge: 1.0,
    },
    // Mid-tone: neither a light theme nor a dark one.
    Look {
        name: "Fog & Blue",
        chrome: 0x546e7a, pane: 0xb0bec5, field: 0xeceff1, surface: 0xdce3e6, line: 0x607d8b,
        text: 0x000000, muted: 0x37474f, accent: 0x37474f, call: 0x263238, freq: 0x000000,
        green: 0x2e7d32, amber: 0xe65100, red: 0xb00020, magenta: 0x7b1fa2, select: 0x1565c0,
        bar: 0x37474f, bar_text: 0xffffff, button: 0xeceff1,
        select_text: Some(0xffffff), edge: 1.0,
    },
    // Dark themes on tinted grounds that are not near black.
    Look {
        name: "Midnight & Rose",
        chrome: 0x1b2233, pane: 0x283044, field: 0x364058, surface: 0x445069, line: 0x6b7896,
        text: 0xffffff, muted: 0xcdd3e0, accent: 0x9fa8da, call: 0xc5cae9, freq: 0xffffff,
        green: 0xa5d6a7, amber: 0xffb74d, red: 0xff8a80, magenta: 0xce93d8, select: 0xf48fb1,
        bar: 0x445069, bar_text: 0xffffff, button: 0x445069,
        select_text: Some(0x000000), edge: 1.0,
    },
    Look {
        name: "Plum & Amber",
        chrome: 0x2a1f33, pane: 0x3a2c47, field: 0x4a3a5a, surface: 0x594869, line: 0x85749a,
        text: 0xffffff, muted: 0xd6cde0, accent: 0xb39ddb, call: 0xd1c4e9, freq: 0xffffff,
        green: 0xa5d6a7, amber: 0xffb74d, red: 0xff8a80, magenta: 0xce93d8, select: 0xffd54f,
        bar: 0x594869, bar_text: 0xffffff, button: 0x594869,
        select_text: Some(0x000000), edge: 1.0,
    },
    Look {
        name: "Cocoa & Aqua",
        chrome: 0x2b211e, pane: 0x3e302b, field: 0x4e3f39, surface: 0x5d4c45, line: 0x8d7a72,
        text: 0xffffff, muted: 0xd7ccc8, accent: 0xbcaaa4, call: 0xefdcd5, freq: 0xffffff,
        green: 0xa5d6a7, amber: 0xffb74d, red: 0xff8a80, magenta: 0xce93d8, select: 0x80deea,
        bar: 0x5d4c45, bar_text: 0xffffff, button: 0x5d4c45,
        select_text: Some(0x000000), edge: 1.0,
    },
    // A dark theme that is not very dark: blue-grey grounds.
    Look {
        name: "Dusk & Sky",
        chrome: 0x263238, pane: 0x37474f, field: 0x455a64, surface: 0x4f6772, line: 0x78909c,
        text: 0xffffff, muted: 0xcfd8dc, accent: 0x81d4fa, call: 0xb3e5fc, freq: 0xffffff,
        green: 0xa5d6a7, amber: 0xffb74d, red: 0xff8a80, magenta: 0xce93d8, select: 0x81d4fa,
        bar: 0x546e7a, bar_text: 0xffffff, button: 0x546e7a,
        select_text: Some(0x000000), edge: 1.0,
    },
    // The Material baseline dark theme: #121212 lightened by elevation.
    Look {
        name: "Graphite & Violet",
        chrome: 0x121212, pane: 0x1e1e1e, field: 0x2e2e2e, surface: 0x3a3a3a, line: 0x4a4a4a,
        text: 0xe4e4e4, muted: 0xababab, accent: 0xbb86fc, call: 0xbb86fc, freq: 0xe4e4e4,
        green: 0x81c784, amber: 0xffb74d, red: 0xcf6679, magenta: 0xf48fb1, select: 0x03dac6,
        bar: 0x383838, bar_text: 0xe4e4e4, button: 0x383838,
        select_text: Some(0x000000), edge: 1.0,
    },
];

/// The theme used until one is chosen, and when the saved choice no
/// longer exists.
pub(super) fn default_theme() -> Theme {
    duos().find(|theme| theme.to_string() == DEFAULT_THEME).unwrap_or_else(app_theme)
}

const DEFAULT_THEME: &str = "Fog & Blue";

fn duos() -> impl Iterator<Item = Theme> {
    DUOS.iter().map(|look| look.scheme().theme_with(look.name, look.hues()))
}

/// Schemes collected from elsewhere, in alphabetical order: the themes
/// distributed with Omarchy and a selection from the Tinted Theming
/// Base16 collection (both MIT licensed), less the ones iced already
/// provides. Colours are in [`Scheme`] field order.
const BUNDLED: &[(&str, [u32; 12])] = &[
    ("Atelier Dune", [0x20201d, 0x292824, 0x6e6b5e, 0x7d7a68, 0xa6a28c, 0x6684e1,
        0x60ac39, 0xae9513, 0xd73737, 0x1fad83, 0xb854d4, 0xb65611]),
    ("Atelier Forest", [0x1b1918, 0x2c2421, 0x68615e, 0x766e6b, 0xa8a19f, 0x407ee7,
        0x7b9726, 0xc38418, 0xf22c40, 0x3d97b8, 0x6666ea, 0xdf5320]),
    ("Ayu Dark", [0x0b0e14, 0x131721, 0x202229, 0x3e4b59, 0xe6e1cf, 0x59c2ff,
        0xaad94c, 0xffb454, 0xf07178, 0x95e6cb, 0xd2a6ff, 0xff8f40]),
    ("Ayu Light", [0xf8f9fa, 0xedeff1, 0xd2d4d8, 0xa0a6ac, 0x5c6166, 0x399ee6,
        0x6cbf49, 0xf2ae49, 0xf07171, 0x4cbf99, 0xa37acc, 0xfa8d3e]),
    ("Ayu Mirage", [0x1f2430, 0x242936, 0x323844, 0x4a5059, 0xcccac2, 0x73d0ff,
        0xd5ff80, 0xffd173, 0xf28779, 0x95e6cb, 0xd4bfff, 0xffad66]),
    ("Eighties", [0x2d2d2d, 0x393939, 0x515151, 0x747369, 0xd3d0c8, 0x6699cc,
        0x99cc99, 0xffcc66, 0xf2777a, 0x66cccc, 0xcc99cc, 0xf99157]),
    ("Espresso", [0x2d2d2d, 0x393939, 0x515151, 0x777777, 0xcccccc, 0x6c99bb,
        0xa5c261, 0xffc66d, 0xd25252, 0xbed6ff, 0xd197d9, 0xf9a959]),
    ("Ethereal", [0x060b1e, 0x131a3a, 0x252e56, 0x6d7db6, 0xffcead, 0x7d82d9,
        0x92a593, 0xe9bb4f, 0xed5b5a, 0xa3bfd1, 0xc89dc1, 0xeb8b54]),
    ("Everforest", [0x2d353b, 0x343f44, 0x3d484d, 0x475258, 0xd3c6aa, 0x7fbbb3,
        0xa7c080, 0xdbbc7f, 0xe67e80, 0x83c092, 0xd699b6, 0xe09d7f]),
    ("Flexoki Dark", [0x100f0f, 0x1c1b1a, 0x282726, 0x575653, 0xcecdc3, 0x4385be,
        0x879a39, 0xd0a215, 0xd14d41, 0x3aa99f, 0x8b7ec8, 0xda702c]),
    ("Flexoki Light", [0xfffcf0, 0xe6e4d9, 0xcecdc3, 0xb7b5ac, 0x100f0f, 0x205ea6,
        0x879a39, 0xd0a215, 0xd14d41, 0x3aa99f, 0xce5d97, 0xd0772b]),
    ("GitHub Dark", [0x0d1117, 0x151b23, 0x2f3742, 0x656c76, 0xd1d7e0, 0x58a6ff,
        0x3fb950, 0xd29922, 0xff7b72, 0x39c5cf, 0xbe8fff, 0xffa657]),
    ("GitHub Light", [0xffffff, 0xf6f8fa, 0xd1d9e0, 0x818b98, 0x454c54, 0x0969da,
        0x116329, 0x9a6700, 0xcf222e, 0x1b7c83, 0x8250df, 0x953800]),
    ("Gruvbox Material", [0x282828, 0x3c3836, 0x504945, 0x665c54, 0xd4be98, 0x7daea3,
        0xa9b665, 0xd8a657, 0xea6962, 0x89b482, 0xd3869b, 0xe1875c]),
    ("Hackerman", [0x0b0c16, 0x151828, 0x1f253a, 0x2d3450, 0xddf7ff, 0x82fb9c,
        0x4fe88f, 0x50f7d4, 0x50f872, 0x7cf8f7, 0x86a7df, 0x50f7a3]),
    ("Jade Horse", [0x142620, 0x203b31, 0x2b5147, 0x64887b, 0xdfede5, 0x8bcbb1,
        0x8bcbb1, 0xc5cc91, 0xd88d91, 0x86c7c5, 0xb7a1c8, 0xd2ab8d]),
    ("Jellybeans", [0x121212, 0x929292, 0xbdbdbd, 0xc5c5c5, 0xd5d5d5, 0xb1d8f6,
        0xbddeab, 0xffdca0, 0xffa1a1, 0x1ab2a8, 0xfbdaff, 0xffba7b]),
    ("Last Horizon", [0x0c0b0c, 0x0c0b0c, 0x584e51, 0x584e51, 0xfafcfb, 0xb59790,
        0x87a9b0, 0x6b5e73, 0xc38b7b, 0xa5a0b6, 0xc4d8e2, 0x977477]),
    ("Lumon", [0x16242d, 0x1b2d40, 0x243d56, 0x304860, 0xd6e2ee, 0x8bc9eb,
        0x5e95bc, 0x6fa4c9, 0x4d86b0, 0xb4e4f6, 0x8bc9eb, 0x8bc9eb]),
    ("Lupine", [0xfafafa, 0xf5f5f5, 0xd0d0d0, 0x9e9e9e, 0x212121, 0x3264eb,
        0x4a2fd0, 0x026fde, 0xc900c4, 0x0c67de, 0x8a4ad7, 0x026fde]),
    ("Material Darker", [0x212121, 0x303030, 0x353535, 0x4a4a4a, 0xeeffff, 0x82aaff,
        0xc3e88d, 0xffcb6b, 0xf07178, 0x89ddff, 0xc792ea, 0xf78c6c]),
    ("Material Palenight", [0x292d3e, 0x444267, 0x32374d, 0x676e95, 0x959dcb, 0x82aaff,
        0xc3e88d, 0xffcb6b, 0xf07178, 0x89ddff, 0xc792ea, 0xf78c6c]),
    ("Matte Black", [0x121212, 0x1e1e1e, 0x2a2a2a, 0x333333, 0xbebebe, 0xe68e0d,
        0xffc107, 0xb91c1c, 0xd35f5f, 0xbebebe, 0xd35f5f, 0xc63d3d]),
    ("Miasma", [0x222222, 0x2c2c2c, 0x383838, 0x666666, 0xc2c2b0, 0x78824b,
        0x5f875f, 0xb36d43, 0x685742, 0xc9a554, 0xbb7744, 0x8d6242]),
    ("Mocha", [0x3b3228, 0x534636, 0x645240, 0x7e705a, 0xd0c8c6, 0x8ab3b5,
        0xbeb55b, 0xf4bc87, 0xcb6077, 0x7bbda4, 0xa89bb9, 0xd28b71]),
    ("Monokai", [0x272822, 0x383830, 0x49483e, 0x75715e, 0xf8f8f2, 0x66d9ef,
        0xa6e22e, 0xf4bf75, 0xf92672, 0xa1efe4, 0xae81ff, 0xfd971f]),
    ("Moonlight", [0x212337, 0x403c64, 0x596399, 0x748cd6, 0xa3ace1, 0x40ffff,
        0x2df4c0, 0xffc777, 0xff5370, 0x04d1f9, 0xb994f1, 0xf67f81]),
    ("Nightfox", [0x192330, 0x212e3f, 0x29394f, 0x575860, 0xcdcecf, 0x719cd6,
        0x81b29a, 0xdbc074, 0xc94f6d, 0x63cdcf, 0x9d79d6, 0xf4a261]),
    ("Ocean", [0x2b303b, 0x343d46, 0x4f5b66, 0x65737e, 0xc0c5ce, 0x8fa1b3,
        0xa3be8c, 0xebcb8b, 0xbf616a, 0x96b5b4, 0xb48ead, 0xd08770]),
    ("One Dark", [0x282c34, 0x353b45, 0x3e4451, 0x545862, 0xabb2bf, 0x61afef,
        0x98c379, 0xe5c07b, 0xe06c75, 0x56b6c2, 0xc678dd, 0xd19a66]),
    ("One Light", [0xfafafa, 0xf0f0f1, 0xe5e5e6, 0xa0a1a7, 0x383a42, 0x4078f2,
        0x50a14f, 0xc18401, 0xca1243, 0x0184bc, 0xa626a4, 0xd75f00]),
    ("Osaka Jade", [0x111c18, 0x23372b, 0x32473b, 0x53685b, 0xc1c497, 0x509475,
        0x549e6a, 0x459451, 0xff5345, 0x2dd5b7, 0xd2689c, 0xa2734b]),
    ("Outrun Dark", [0x00002a, 0x20204a, 0x30305a, 0x50507a, 0xd0d0fa, 0x66b0ff,
        0x59f176, 0xf3e877, 0xff4242, 0x0ef0f0, 0xf10596, 0xfc8d28]),
    ("PaperColor Dark", [0x1c1c1c, 0x363636, 0x424242, 0x585858, 0x9e9e9e, 0x5fafd7,
        0x5faf5f, 0xffaf00, 0xff5faf, 0x00afaf, 0xaf87d7, 0xd7af5f]),
    ("PaperColor Light", [0xeeeeee, 0xc4c4c4, 0x9e9e9e, 0x858585, 0x5e5e5e, 0x005f87,
        0x008700, 0xd75f00, 0xd70000, 0x0087af, 0x8700af, 0xd75f00]),
    ("Paraiso", [0x2f1e2e, 0x41323f, 0x4f424c, 0x776e71, 0xa39e9b, 0x06b6ef,
        0x48b685, 0xfec418, 0xef6155, 0x5bc4bf, 0x815ba4, 0xf99b15]),
    ("PhD", [0x061229, 0x2a3448, 0x4d5666, 0x717885, 0xb8bbc2, 0x5299bf,
        0x99bf52, 0xfbd461, 0xd07346, 0x72b9bf, 0x9989cc, 0xf0a000]),
    ("Retro 82", [0x05182e, 0x0a2540, 0x134e5a, 0x2a6b78, 0xf6dcac, 0xfaa968,
        0x028391, 0xe97b3c, 0xf85525, 0x8cbfb8, 0x3f8f8a, 0xfaa968]),
    ("Ristretto", [0x2c2525, 0x3d2f2a, 0x403e41, 0x72696a, 0xe6d9db, 0xf38d70,
        0xadda78, 0xf9cc6c, 0xfd6883, 0x85dacc, 0xa8a9eb, 0xfb9a77]),
    ("Rose Pine", [0x191724, 0x1f1d2e, 0x26233a, 0x6e6a86, 0xe0def4, 0xc4a7e7,
        0x31748f, 0xebbcba, 0xeb6f92, 0x9ccfd8, 0xf6c177, 0xf6c177]),
    ("Rose Pine Dawn", [0xfaf4ed, 0xf2e9e1, 0xdfdad9, 0xcecacd, 0x575279, 0x56949f,
        0x286983, 0xea9d34, 0xb4637a, 0xd7827e, 0x907aa9, 0xcf8057]),
    ("Rose Pine Moon", [0x232136, 0x2a273f, 0x393552, 0x6e6a86, 0xe0def4, 0xc4a7e7,
        0x3e8fb0, 0xea9a97, 0xeb6f92, 0x9ccfd8, 0xf6c177, 0xf6c177]),
    ("Selenized Dark", [0x103c48, 0x184956, 0x2d5b69, 0x72898f, 0xadbcbc, 0x4695f7,
        0x75b938, 0xdbb32d, 0xfa5750, 0x41c7b9, 0xaf88eb, 0xed8649]),
    ("Selenized Light", [0xfbf3db, 0xece3cc, 0xd5cdb6, 0x909995, 0x53676d, 0x006dce,
        0x428b00, 0xa78300, 0xcc1729, 0x00978a, 0x825dc0, 0xbc5819]),
    ("Shades of Purple", [0x1e1e3f, 0x43d426, 0xf1d000, 0x808080, 0xc7c7c7, 0x6943ff,
        0x3ad900, 0xffe700, 0xd90429, 0x00c5c7, 0xff2c70, 0xf92a1c]),
    ("Snazzy", [0x282a36, 0x34353e, 0x43454f, 0x78787e, 0xe2e4e5, 0x57c7ff,
        0x5af78e, 0xf3f99d, 0xff5c57, 0x9aedfe, 0xff6ac1, 0xff9f43]),
    ("Solitude", [0x101315, 0x101315, 0x343d41, 0x4b4e55, 0xcacccc, 0x798186,
        0x9fa5a9, 0xd9dbdc, 0x565d60, 0x707070, 0xaeaeae, 0x989c9e]),
    ("Spaceduck", [0x16172d, 0x1b1c36, 0x30365f, 0x686f9a, 0xecf0c1, 0x7a5ccc,
        0x5ccc96, 0xf2ce00, 0xe33400, 0x00a3cc, 0xb3a1e6, 0xe39400]),
    ("Starship", [0x1c1c1e, 0x2c2c2e, 0x3a3a3c, 0x8e8e93, 0xf5f5f7, 0xff9f0a,
        0xff9f0a, 0xffc27a, 0xff5f4a, 0xa6dcef, 0xff8fa3, 0xff6a3d]),
    ("Synth Midnight", [0x050608, 0x1a1b1c, 0x28292a, 0x474849, 0xc1c3c4, 0x03aeff,
        0x06ea61, 0xc9d364, 0xb53b50, 0x42fff9, 0xea5ce2, 0xea770d]),
    ("Tender", [0x282828, 0x383838, 0x484848, 0x4c4c4c, 0xeeeeee, 0xb3deef,
        0xc9d05c, 0xffc24b, 0xf43753, 0x73cef4, 0xd3b987, 0xdc9656]),
    ("Tomorrow", [0xffffff, 0xe0e0e0, 0xc5c8c6, 0xb4b7b4, 0x373b41, 0x4271ae,
        0x718c00, 0xeab700, 0xc82829, 0x3e999f, 0x8959a8, 0xf5871f]),
    ("Tomorrow Night", [0x1d1f21, 0x282a2e, 0x373b41, 0x969896, 0xc5c8c6, 0x81a2be,
        0xb5bd68, 0xf0c674, 0xcc6666, 0x8abeb7, 0xb294bb, 0xde935f]),
    ("Twilight", [0x1e1e1e, 0x323537, 0x464b50, 0x5f5a60, 0xa7a7a7, 0x7587a6,
        0x8f9d6a, 0xf9ee98, 0xcf6a4c, 0xafc4db, 0x9b859d, 0xcda869]),
    ("Vantablack", [0x000000, 0x1a1a1a, 0x1a1a1a, 0x7a7a7a, 0xffffff, 0x8d8d8d,
        0xb6b6b6, 0xcecece, 0xa4a4a4, 0xb0b0b0, 0x9b9b9b, 0xb9b9b9]),
    ("Vesper", [0x101010, 0x232323, 0x222222, 0x333333, 0xb7b7b7, 0x8eaaaa,
        0x5f8787, 0xffc799, 0xde6e6e, 0x60a592, 0xd69094, 0xdab083]),
    ("White", [0xffffff, 0xc0c0c0, 0xc0c0c0, 0x808080, 0x000000, 0x6e6e6e,
        0x3a3a3a, 0x4a4a4a, 0x2a2a2a, 0x3e3e3e, 0x2e2e2e, 0x3a3a3a]),
    ("Woodland", [0x231e18, 0x302b25, 0x48413a, 0x9d8b70, 0xcabcb1, 0x88a4d3,
        0xb7ba53, 0xe0ac16, 0xd35c5c, 0x6eb958, 0xbb90e2, 0xca7f32]),
    ("Zenburn", [0x383838, 0x404040, 0x606060, 0x6f6f6f, 0xdcdccc, 0x7cb8bb,
        0x5f7f5f, 0xe0cf9f, 0xdca3a3, 0x93e0e3, 0xdc8cc3, 0xdfaf8f]),
];

fn schemes(table: &'static [(&'static str, [u32; 12])]) -> impl Iterator<Item = Theme> {
    table
        .iter()
        .map(|&(name, colors)| Scheme::from_hex(colors).theme(name))
}

/// Where a picker theme comes from. The picker groups its tiles by this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThemeSource {
    /// Designed for slogger.
    Slogger,
    /// A theme or palette file in `~/.config/slogger/themes/`.
    Custom,
    /// A scheme from elsewhere, shipped with slogger.
    Bundled,
    /// A palette from the Omarchy community theme list, shipped with
    /// slogger.
    Community,
    /// Built on a primary and a secondary colour.
    Duo,
    /// One of iced's built-in themes.
    BuiltIn,
}

impl ThemeSource {
    /// Picker sections, in the order they are shown.
    pub(super) const ALL: [ThemeSource; 6] = [
        ThemeSource::Duo,
        ThemeSource::Slogger,
        ThemeSource::Custom,
        ThemeSource::Bundled,
        ThemeSource::Community,
        ThemeSource::BuiltIn,
    ];

    /// Section heading in the picker.
    pub(super) fn title(self) -> &'static str {
        match self {
            ThemeSource::Duo => "ONE COLOUR FOR STRUCTURE, ONE FOR THE SELECTION",
            ThemeSource::Slogger => "SLOGGER",
            ThemeSource::Custom => "YOUR THEMES",
            ThemeSource::Bundled => "COLLECTION",
            ThemeSource::Community => "OMARCHY COMMUNITY",
            ThemeSource::BuiltIn => "CLASSICS",
        }
    }
}

/// One theme the picker offers.
#[derive(Debug, Clone)]
pub(super) struct ThemeEntry {
    pub(super) source: ThemeSource,
    pub(super) theme: Theme,
    /// The full theme file this came from, if it is one the editor can
    /// change in place.
    pub(super) file: Option<PathBuf>,
}

/// Every theme the picker offers: the ones designed for slogger, palette
/// files from `~/.config/slogger/themes/`, the bundled collection, then
/// iced's built-ins. Reads the themes directory, so call it when the list
/// is needed afresh rather than on every frame.
#[cfg(test)]
pub(super) fn all_themes() -> Vec<ThemeEntry> {
    load_themes().0
}

/// [`all_themes`], with a note for each theme file (or entry in one)
/// that could not be used.
pub(super) fn load_themes() -> (Vec<ThemeEntry>, Vec<String>) {
    load_from(themes_dir().as_deref())
}

/// Where the user's theme files live.
pub(super) fn themes_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("slogger").join("themes"))
}

/// The theme the editor panel itself is drawn in, so it stays readable
/// whatever is being done to the theme under edit.
pub(super) fn editor_theme() -> Theme {
    duos().find(|theme| theme.to_string() == "Paper & Blue").unwrap_or_else(app_theme)
}

#[cfg(test)]
fn themes_from(custom: Option<&Path>) -> Vec<ThemeEntry> {
    load_from(custom).0
}

fn load_from(custom: Option<&Path>) -> (Vec<ThemeEntry>, Vec<String>) {
    let entry = |source| move |theme| ThemeEntry { source, theme, file: None };
    let mut problems = vec![];
    // Families first, three strengths to a picker row; the original
    // theme closes the last row.
    let mut entries: Vec<ThemeEntry> = designed()
        .chain(std::iter::once(app_theme()))
        .map(entry(ThemeSource::Slogger))
        .chain(duos().map(entry(ThemeSource::Duo)))
        .collect();
    let bundled: Vec<ThemeEntry> = schemes(BUNDLED)
        .map(entry(ThemeSource::Bundled))
        .chain(schemes(community::COMMUNITY).map(entry(ThemeSource::Community)))
        .collect();
    let built_in = Theme::ALL.iter().cloned().map(entry(ThemeSource::BuiltIn));

    // Names identify themes in the layout file, so each must be unique.
    // The fixed themes claim theirs first so a saved choice keeps meaning
    // the same theme whatever files appear later.
    let mut names: HashSet<String> = entries
        .iter()
        .chain(&bundled)
        .map(|e| e.theme.to_string())
        .chain(Theme::ALL.iter().map(|t| t.to_string()))
        .collect();

    for (stem, path) in custom.map(theme_files).unwrap_or_default() {
        let shown = path.file_name().and_then(|f| f.to_str()).unwrap_or(&stem).to_owned();
        let mut problem = |what: String| {
            tracing::warn!(path = %path.display(), "theme file: {what}");
            problems.push(format!("theme file {shown}: {what}"));
        };
        let (name, parsed) = match read_theme_file(&path, &stem) {
            Ok((name, parsed, notes)) => {
                notes.into_iter().for_each(&mut problem);
                (name, parsed)
            }
            Err(why) => {
                problem(format!("skipped, {why}"));
                continue;
            }
        };
        let name = if names.insert(name.clone()) {
            name
        } else {
            let tagged = format!("{name} (custom)");
            if !names.insert(tagged.clone()) {
                problem(format!("skipped, there is already a theme named {name}"));
                continue;
            }
            tagged
        };
        let (theme, file) = match parsed {
            Parsed::Palette(scheme) => (scheme.theme(name), None),
            Parsed::Full(spec) => (spec.theme_named(name), Some(path.clone())),
        };
        entries.push(ThemeEntry { source: ThemeSource::Custom, theme, file });
    }
    entries.extend(bundled);
    entries.extend(built_in);
    (entries, problems)
}

enum Parsed {
    /// A handful of colours; slogger decides where they go.
    Palette(Scheme),
    /// Every colour and option.
    Full(Box<Spec>),
}

/// Reads one theme file: its name, what it holds, and a note for each
/// entry that could not be used. `Err` says why the file is no theme.
fn read_theme_file(path: &Path, stem: &str) -> Result<(String, Parsed, Vec<String>), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("could not be read ({e})"))?;
    if is_yaml(path) {
        let (name, scheme) = parse_base16(&src, stem)
            .ok_or("it has no base00 and base05 colours")?;
        return Ok((name, Parsed::Palette(scheme), vec![]));
    }
    let table: toml::Table = src
        .parse()
        .map_err(|e: toml::de::Error| format!("it is not valid TOML ({})", e.message()))?;
    if spec::is_spec(&table) {
        let (spec, notes) = Spec::from_table(&table, stem)?;
        return Ok((spec.name.clone(), Parsed::Full(Box::new(spec)), notes));
    }
    let (name, scheme) = parse_palette(&table, stem)
        .ok_or("it has no background and foreground colours")?;
    Ok((name, Parsed::Palette(scheme), vec![]))
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

/// Builds a scheme from whichever of its colours a file supplies.
/// `get` looks a colour up by [`Scheme`] field name. The background and
/// text are required; the accent and status colours fall back to
/// slogger's own, and the rest are blended from those.
fn scheme_from(get: impl Fn(&str) -> Option<Color>) -> Option<Scheme> {
    let fallback = SLOGGER.palette();
    let base = Scheme::from_palette(Palette {
        background: get("background")?,
        text: get("text")?,
        primary: get("accent").unwrap_or(fallback.primary),
        success: get("green").unwrap_or(fallback.success),
        warning: get("yellow").unwrap_or(fallback.warning),
        danger: get("red").unwrap_or(fallback.danger),
    });
    Some(Scheme {
        surface: get("surface").unwrap_or(base.surface),
        line: get("line").unwrap_or(base.line),
        muted: get("muted").unwrap_or(base.muted),
        cyan: get("cyan").unwrap_or(base.cyan),
        magenta: get("magenta").unwrap_or(base.magenta),
        orange: get("orange").unwrap_or(base.orange),
        ..base
    })
}

/// Reads a Base16 scheme: `base00` (background) and `base05` (text) are
/// required. Handles both the current layout (`name:` plus colours nested
/// under `palette:`) and the older flat one (`scheme:` plus top-level
/// colours), which is all the YAML these files use — so the lines are
/// read directly.
fn parse_base16(src: &str, stem: &str) -> Option<(String, Scheme)> {
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
    let scheme = scheme_from(|field| {
        let slot = match field {
            "background" => "base00",
            "surface" => "base01",
            "line" => "base02",
            "muted" => "base03",
            "text" => "base05",
            "red" => "base08",
            "orange" => "base09",
            "yellow" => "base0a",
            "green" => "base0b",
            "cyan" => "base0c",
            "accent" => "base0d",
            "magenta" => "base0e",
            _ => return None,
        };
        colors.iter().find(|(k, _)| k == slot).map(|(_, c)| *c)
    })?;
    let name = name
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| title_case(stem));
    Some((name, scheme))
}

/// Reads a palette from a TOML file of `key = "#rrggbb"` lines (the
/// layout Omarchy's `colors.toml` uses): `background` and `foreground`
/// are required; `accent` (else `blue`), `green`, `yellow`, `red`,
/// `cyan`, `magenta`, `orange`, `muted`, `lighter_background` and
/// `selection` are used when present. An optional `name` overrides the
/// one derived from the file's stem.
fn parse_palette(table: &toml::Table, stem: &str) -> Option<(String, Scheme)> {
    let get = |key: &str| table.get(key).and_then(|v| v.as_str()).and_then(parse_hex);
    let scheme = scheme_from(|field| match field {
        "text" => get("foreground"),
        "accent" => get("accent").or_else(|| get("blue")),
        "surface" => get("lighter_background"),
        "line" => get("selection"),
        other => get(other),
    })?;
    let name = table
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| title_case(stem));
    Some((name, scheme))
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
        color: Some(hues(theme).muted),
    }
}

/// A callsign.
pub(super) fn call(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(hues(theme).call),
    }
}

/// A frequency.
pub(super) fn freq(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(hues(theme).freq),
    }
}

/// A band.
pub(super) fn band(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(hues(theme).band),
    }
}

/// A mode.
pub(super) fn mode(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(hues(theme).mode),
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
            text_color: Some(hues(theme).muted),
            ..container::Style::default()
        })
        .into()
}

/// A clickable table row. `highlighted` rows get a persistent background
/// (the QSO being edited, a spot matching the award target).
pub(super) fn table_row<'a>(
    theme: &Theme,
    cells: Row<'a, Message>,
    on_press: Message,
    highlighted: bool,
) -> Element<'a, Message> {
    table_row_at(theme, cells, on_press, highlighted, 0, None)
}

/// [`table_row`] for row number `index` (alternate rows are shaded in
/// themes that stripe their tables), with the status the row stands for
/// (themes that colour whole rows by status use it).
pub(super) fn table_row_at<'a>(
    theme: &Theme,
    cells: Row<'a, Message>,
    on_press: Message,
    highlighted: bool,
    index: usize,
    tone: Option<Tone>,
) -> Element<'a, Message> {
    let cells = cells.spacing(CELL_SPACING);
    let cells: Element<'a, Message> = match selected_row_theme(theme) {
        // Unstyled text takes its colour from above the row, so the twin
        // has to supply its own.
        Some(twin) if highlighted => iced::widget::themer(Some(twin), cells)
            .text_color(|twin: &Theme| twin.palette().text)
            .into(),
        _ => cells.into(),
    };
    button(cells)
        .padding(ROW_PADDING)
        .width(Length::Fill)
        .on_press(on_press)
        .style(move |theme: &Theme, status| {
            let hues = hues(theme);
            let ground = theme.palette().background;
            let background = if highlighted {
                Some(hues.select.into())
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                Some(hues.hover.into())
            } else if let (StatusLook::Row, Some(tone)) = (hues.status, tone) {
                Some(mix(ground, tone.fill(theme), 0.3).into())
            } else if hues.zebra && index % 2 == 1 {
                Some(mix(ground, theme.palette().text, 0.06).into())
            } else {
                None
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
        text_color: Some(theme.palette().text),
        background: Some(hues(theme).pane.into()),
        border: Border {
            color: ext.background.strong.color,
            width: hues(theme).edge,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    }
}

/// Pane title bar: set off from the body so the drag handle reads.
pub(super) fn title_bar_style(theme: &Theme) -> container::Style {
    let hues = hues(theme);
    container::Style {
        background: Some(hues.bar.into()),
        text_color: Some(hues.bar_text),
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
        text_color: Some(theme.palette().text),
        background: Some(hues(theme).pane.into()),
        border: Border {
            color: ext.background.strong.color,
            width: hues(theme).edge,
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

/// The window frame, and the canvas the floating panels sit on: a tone
/// of its own, so the panes holding data stand out from it.
pub(super) fn canvas_style(theme: &Theme) -> container::Style {
    let hues = hues(theme);
    container::Style {
        background: Some(hues.chrome.into()),
        text_color: Some(ink_toward(theme.palette().text, hues.chrome)),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The bottom status bar: a pane of its own on the window frame.
pub(super) fn strip_style(theme: &Theme) -> container::Style {
    let ext = theme.extended_palette();
    container::Style {
        text_color: Some(theme.palette().text),
        background: Some(hues(theme).pane.into()),
        border: Border {
            color: ext.background.strong.color,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    }
}

/// The ground tables sit on inside a pane: the theme's background, edged
/// so it reads as set into the pane.
pub(super) fn field_style(theme: &Theme) -> container::Style {
    let ext = theme.extended_palette();
    container::Style {
        background: Some(ext.background.base.color.into()),
        text_color: Some(theme.palette().text),
        border: Border {
            color: ext.background.strong.color,
            width: hues(theme).edge,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    }
}

/// Sets a table (or any block of data) into its pane on [`field_style`].
pub(super) fn grid<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(2)
        .width(Length::Fill)
        .style(field_style)
        .into()
}

/// What a filled status cell says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    /// Something wanted: a needed spot, an open band.
    Good,
    Warn,
    Bad,
}

impl Tone {
    pub(super) fn fill(self, theme: &Theme) -> Color {
        let hues = hues(theme);
        match self {
            Tone::Good => hues.good,
            Tone::Warn => hues.warn,
            Tone::Bad => hues.bad,
        }
    }
}

/// A table cell filled with a status colour, the same size as a
/// [`cell`] of that width so the columns still line up.
pub(super) fn filled_cell<'a>(
    s: impl text::IntoFragment<'a>,
    width: f32,
    tone: Tone,
) -> Element<'a, Message> {
    container(
        text(s)
            .size(ROW_TEXT)
            .font(MONO_BOLD)
            .wrapping(text::Wrapping::None),
    )
    .width(Length::Fixed(width))
    .align_x(iced::alignment::Horizontal::Center)
    .clip(true)
    .style(move |theme: &Theme| {
        let fill = tone.fill(theme);
        match hues(theme).status {
            StatusLook::Cell => container::Style {
                background: Some(fill.into()),
                text_color: Some(ink_on(fill)),
                ..container::Style::default()
            },
            // Coloured text, made readable on the table's ground.
            StatusLook::Text => container::Style {
                text_color: Some(legible(fill, theme.palette().background, 4.5)),
                ..container::Style::default()
            },
            // Where a whole row carries the colour the caller draws a
            // plain cell instead, so anything arriving here is a status
            // outside a row (a band rating) and is filled as usual.
            StatusLook::Row => container::Style {
                background: Some(fill.into()),
                text_color: Some(ink_on(fill)),
                ..container::Style::default()
            },
        }
    })
    .into()
}

/// A [`badge`] in a status colour.
pub(super) fn tone_badge<'a>(s: impl text::IntoFragment<'a>, tone: Tone) -> Element<'a, Message> {
    container(text(s).size(12).font(MONO_BOLD))
        .padding([1, 6])
        .style(move |theme: &Theme| {
            let fill = tone.fill(theme);
            container::Style {
                background: Some(fill.into()),
                text_color: Some(ink_on(fill)),
                border: Border {
                    radius: 3.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// A table cell naming what kind of thing a row is ("POTA"), the same
/// size as a [`cell`] of that width. Filled with the theme's primary
/// colour rather than a status colour: it names, it doesn't judge.
pub(super) fn tag_cell<'a>(s: impl text::IntoFragment<'a>, width: f32) -> Element<'a, Message> {
    container(
        text(s)
            .size(ROW_TEXT)
            .font(MONO_BOLD)
            .wrapping(text::Wrapping::None),
    )
    .width(Length::Fixed(width))
    .align_x(iced::alignment::Horizontal::Center)
    .clip(true)
    .style(move |theme: &Theme| {
        let fill = theme.palette().primary;
        container::Style {
            background: Some(fill.into()),
            text_color: Some(ink_on(fill)),
            ..container::Style::default()
        }
    })
    .into()
}

/// An ordinary push button: a solid face with a border, not a wash of
/// the background.
pub(super) fn push_button(theme: &Theme, status: button::Status) -> button::Style {
    let hues = hues(theme);
    let edge = theme.extended_palette().background.strong.color;
    let toward = ink_on(hues.button);
    let (face, text_color) = match status {
        button::Status::Active => (hues.button, hues.button_text),
        button::Status::Hovered => (mix(hues.button, toward, 0.12), hues.button_text),
        button::Status::Pressed => (mix(hues.button, toward, 0.22), hues.button_text),
        button::Status::Disabled => (hues.button, mix(hues.button, hues.button_text, 0.55)),
    };
    button::Style {
        background: Some(face.into()),
        text_color,
        border: Border {
            color: mix(edge, toward, 0.25),
            width: hues.edge,
            radius: 3.0.into(),
        },
        ..button::Style::default()
    }
}

/// `color` if it can be read on `ground`, otherwise black or white.
fn ink_toward(color: Color, ground: Color) -> Color {
    if contrast(color, ground) >= 4.5 { color } else { ink_on(ground) }
}

/// A text-only button sitting on a title bar.
pub(super) fn bar_button(theme: &Theme, status: button::Status) -> button::Style {
    let color = hues(theme).bar_text;
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        text_color: if hovered { color } else { Color { a: 0.75, ..color } },
        ..button::Style::default()
    }
}

/// A tab or toggle sitting on a pane: solid when active, plain text
/// otherwise.
pub(super) fn tab_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme: &Theme, status| {
        let hues = hues(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let text = theme.palette().text;
        button::Style {
            background: if active {
                Some(hues.bar.into())
            } else if hovered {
                Some(hues.button.into())
            } else {
                None
            },
            text_color: if active {
                hues.bar_text
            } else if hovered {
                hues.button_text
            } else {
                Color { a: 0.8, ..text }
            },
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

/// One radio's block in the Station pane, where it is a button that
/// makes that radio the active one.
pub(super) fn rig_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme: &Theme, status| {
        let hues = hues(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if active {
                Some(hues.rig.into())
            } else if hovered {
                Some(hues.hover.into())
            } else {
                None
            },
            text_color: if active { hues.rig_text } else { theme.palette().text },
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

/// A view tab in the top bar, which sits on the window frame.
pub(super) fn frame_tab_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme: &Theme, status| {
        let hues = hues(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let ink = ink_toward(theme.palette().text, hues.chrome);
        button::Style {
            background: if active {
                Some(hues.bar.into())
            } else if hovered {
                Some(Color { a: 0.15, ..ink }.into())
            } else {
                None
            },
            text_color: if active {
                hues.bar_text
            } else {
                Color { a: if hovered { 1.0 } else { 0.8 }, ..ink }
            },
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

/// A toggle on the window frame for something that is open or shut (a
/// panel): open is a raised button, shut is plain text.
pub(super) fn frame_toggle_style(open: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme: &Theme, status| {
        if open {
            return push_button(theme, status);
        }
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let ink = ink_toward(theme.palette().text, hues(theme).chrome);
        button::Style {
            background: hovered.then(|| Color { a: 0.15, ..ink }.into()),
            text_color: Color { a: if hovered { 1.0 } else { 0.7 }, ..ink },
            border: Border {
                color: Color { a: 0.35, ..ink },
                width: 1.0,
                radius: 3.0.into(),
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

    fn palette_of(src: &str, stem: &str) -> Option<(String, Scheme)> {
        parse_palette(&src.parse().ok()?, stem)
    }

    const JADE: &str = r##"
mode = "dark"
accent = "#509475"
background = "#111c18"
foreground = "#C1C497"
selection = "#32473B"
red = "#FF5345"
yellow = "#459451"
green = "#549e6a"
cyan = "#2DD5B7"
"##;

    #[test]
    fn a_palette_file_maps_onto_the_theme_colours() {
        let (name, scheme) = palette_of(JADE, "osaka-jade").unwrap();
        assert_eq!(name, "Osaka Jade");
        assert_eq!(scheme.background, rgb(0x111c18));
        assert_eq!(scheme.text, rgb(0xc1c497));
        assert_eq!(scheme.accent, rgb(0x509475));
        assert_eq!(scheme.red, rgb(0xff5345));
        assert_eq!(scheme.line, rgb(0x32473b));
        assert_eq!(scheme.cyan, rgb(0x2dd5b7));
    }

    #[test]
    fn a_palette_needs_a_background_and_a_foreground() {
        assert!(palette_of("background = \"#000000\"", "x").is_none());
        assert!(palette_of("not toml at all [", "x").is_none());
        let (name, scheme) =
            palette_of("name = \"Mine\"\nbackground = \"000000\"\nforeground = \"#ffffff\"", "x")
                .unwrap();
        assert_eq!(name, "Mine");
        assert_eq!(scheme.accent, SLOGGER.palette().primary);
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
  base01: "#353b45"
  base05: "#abb2bf"
  base08: "#e06c75"
  base0A: "#e5c07b"
  base0B: "#98c379"
  base0D: "#61afef"
"##;
        let (name, scheme) = parse_base16(current, "onedark").unwrap();
        assert_eq!(name, "OneDark");
        assert_eq!(scheme.background, rgb(0x282c34));
        assert_eq!(scheme.surface, rgb(0x353b45));
        assert_eq!(scheme.text, rgb(0xabb2bf));
        assert_eq!(scheme.accent, rgb(0x61afef));
        assert_eq!(scheme.green, rgb(0x98c379));
        assert_eq!(scheme.yellow, rgb(0xe5c07b));
        assert_eq!(scheme.red, rgb(0xe06c75));

        let flat = "scheme: 'Old Style'\nbase00: 181818 # background\nbase05: \"d8d8d8\"\n";
        let (name, scheme) = parse_base16(flat, "old").unwrap();
        assert_eq!(name, "Old Style");
        assert_eq!(scheme.background, rgb(0x181818));
        assert_eq!(scheme.accent, SLOGGER.palette().primary);

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
        assert_eq!(shipped[0], "Light Soft");
        assert_eq!(shipped.last().map(String::as_str), Some("slogger"));
        assert_eq!(shipped.len(), 1 + LOOKS.len());
        assert_eq!(names(ThemeSource::Bundled).len(), BUNDLED.len());
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
    fn the_default_theme_is_one_the_picker_offers() {
        let default = default_theme().to_string();
        assert_eq!(default, DEFAULT_THEME);
        assert!(theme_by_name(&themes_from(None), &default).is_some());
    }

    #[test]
    fn nothing_but_the_fixed_themes_without_a_themes_directory() {
        let entries = themes_from(None);
        assert_eq!(
            entries.len(),
            1 + LOOKS.len()
                + DUOS.len()
                + BUNDLED.len()
                + community::COMMUNITY.len()
                + Theme::ALL.len()
        );
    }

    #[test]
    fn designed_themes_can_be_read_wherever_they_put_text() {
        for (theme, look) in designed().chain(duos()).zip(LOOKS.iter().chain(DUOS)) {
            let name = look.name;
            let h = hues(&theme);
            let field = rgb(look.field);
            let text = rgb(look.text);
            // Nothing is nudged: the theme shows the colours as chosen.
            assert_eq!((h.call, h.freq, h.muted), (rgb(look.call), rgb(look.freq), rgb(look.muted)));
            assert_eq!((h.band, h.mode), (text, text), "{name}");
            let p = theme.palette();
            assert_eq!((p.background, p.text), (field, text), "{name}");

            let reads = |what: &str, ink: Color, ground: Color, min: f32| {
                let ratio = contrast(ink, ground);
                assert!(ratio >= min, "{name}: {what} is only {ratio:.1}:1");
            };
            reads("text", text, field, 6.0);
            reads("bar text", h.bar_text, h.bar, 4.5);
            reads("button text", h.button_text, h.button, 4.5);
            reads("frame text", ink_toward(text, h.chrome), h.chrome, 4.5);
            reads("text on a pane", text, h.pane, 5.5);
            reads("muted text on a pane", h.muted, h.pane, 4.0);
            reads("bar on its pane", h.bar, h.pane, 1.3);
            for (what, ink) in [("text", text), ("call", h.call), ("freq", h.freq)] {
                let on_bar = h.select_text.unwrap_or(ink);
                reads(&format!("{what} on a selected row"), on_bar, h.select, 4.5);
                reads(&format!("{what} on a hovered row"), ink, rgb(look.surface), 4.0);
            }
            for (what, fill) in [("good", h.good), ("warn", h.warn), ("bad", h.bad)] {
                reads(&format!("{what} cell"), ink_on(fill), fill, 4.5);
                reads(&format!("{what} status text"), fill, field, 3.0);
            }
            // The frame and the bars are tones of their own.
            assert!(
                h.chrome != h.pane && h.pane != field && h.chrome != field,
                "{name}: frame, pane and data need a tone each"
            );
            assert!(contrast(h.pane, field) >= 1.1, "{name}: fields are lost in the pane");
        }
    }

    #[test]
    fn every_shipped_theme_has_readable_table_text() {
        for entry in themes_from(None) {
            let theme = &entry.theme;
            let background = theme.palette().background;
            let h = hues(theme);
            for c in [h.muted, h.call, h.freq, h.band, h.mode] {
                assert!(contrast(c, background) >= 4.4, "{theme}");
            }
        }
    }

    #[test]
    fn legible_leaves_readable_colours_alone_and_fixes_the_rest() {
        let dark = rgb(0x202020);
        assert_eq!(legible(rgb(0xffffff), dark, 4.5), rgb(0xffffff));
        let fixed = legible(rgb(0x303030), dark, 4.5);
        assert!(contrast(fixed, dark) >= 4.5);
        let light = rgb(0xe0e0e0);
        assert!(contrast(legible(rgb(0xd0d0d0), light, 4.5), light) >= 4.5);
    }

    #[test]
    fn rig_freq_groups_mhz_khz_and_tens_of_hz() {
        assert_eq!(format_rig_freq(14_074_000), "14.074.00");
        assert_eq!(format_rig_freq(7_005_250), "7.005.25");
        assert_eq!(format_rig_freq(144_200_000), "144.200.00");
    }
}
