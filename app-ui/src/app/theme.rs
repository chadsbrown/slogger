//! Visual language shared by every view: the application theme, the
//! monospace font used for tabular data, and the small set of container /
//! button / text styles the panes are built from.
//!
//! Views should reach for these helpers rather than hard-coding colours so
//! the whole app can be re-skinned from this one file.

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

/// Every theme the picker offers: slogger's own first, then iced's
/// built-ins.
pub(super) fn all_themes() -> Vec<Theme> {
    std::iter::once(app_theme())
        .chain(Theme::ALL.iter().cloned())
        .collect()
}

/// Looks a theme up by the name the picker shows (and the layout file
/// stores).
pub(super) fn theme_by_name(name: &str) -> Option<Theme> {
    all_themes().into_iter().find(|t| t.to_string() == name)
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
    use super::format_rig_freq;

    #[test]
    fn rig_freq_groups_mhz_khz_and_tens_of_hz() {
        assert_eq!(format_rig_freq(14_074_000), "14.074.00");
        assert_eq!(format_rig_freq(7_005_250), "7.005.25");
        assert_eq!(format_rig_freq(144_200_000), "144.200.00");
    }
}
