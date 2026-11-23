//! Theme picker: a grid of tiles, each drawn in its own theme's colours
//! (with a sample table row) so it previews what choosing it would look
//! like. Picking a tile
//! applies the theme immediately and leaves the modal open, so several
//! can be tried in a row. Tiles are grouped by where the theme comes from.

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Border, Color, Element, Length, Theme};

use super::super::message::Message;
use super::super::theme::{self as style, ThemeEntry, ThemeSource};

const PER_ROW: usize = 3;
const GAP: f32 = 10.0;

pub(super) fn view<'a>(current: &'a Theme, themes: &'a [ThemeEntry]) -> Element<'a, Message> {
    let current_name = current.to_string();

    let mut rows = column![].spacing(GAP).width(Length::Fill);
    for source in ThemeSource::ALL {
        let group: Vec<&Theme> = themes
            .iter()
            .filter(|entry| entry.source == source)
            .map(|entry| &entry.theme)
            .collect();
        if group.is_empty() {
            continue;
        }
        rows = rows.push(style::label(source.title()));
        for chunk in group.chunks(PER_ROW) {
            let mut tiles = row![].spacing(GAP);
            for theme in chunk {
                tiles = tiles.push(tile(theme, theme.to_string() == current_name));
            }
            // Pad a short final row so its tiles keep the same width.
            for _ in chunk.len()..PER_ROW {
                tiles = tiles.push(Space::new().width(Length::Fill));
            }
            rows = rows.push(tiles);
        }
    }

    let body = scrollable(container(rows).padding(iced::Padding {
        right: 14.0,
        ..iced::Padding::ZERO
    }))
    .width(Length::Fill)
    .height(Length::Fill);

    let footer = row![
        style::label("CURRENT"),
        text(current_name).size(style::ROW_TEXT),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    super::frame("Theme", body.into(), Some(footer.into()), 680.0, Some(560.0))
}

fn tile(theme: &Theme, is_current: bool) -> Element<'static, Message> {
    let palette = theme.extended_palette();
    let background = palette.background.base.color;
    let hues = style::hues(theme);
    let (bar, name_color, pane) = (hues.bar, hues.bar_text, hues.pane);

    let name_row = row![
        text(theme.to_string())
            .size(style::ROW_TEXT)
            .color(name_color)
            .width(Length::Fill),
        text(if is_current { "✓" } else { "" })
            .size(style::ROW_TEXT)
            .color(name_color),
    ]
    .align_y(iced::Alignment::Center);
    // The name sits on the theme's title bar.
    let title = container(name_row)
        .padding([5, 14])
        .width(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(bar.into()),
            border: Border {
                radius: iced::border::top(5.0),
                ..Border::default()
            },
            ..container::Style::default()
        });

    // A table row as this theme would draw it.
    let sample = |s: &'static str, color: Color| {
        text(s)
            .size(style::LABEL_TEXT)
            .font(style::MONO)
            .color(color)
            .wrapping(text::Wrapping::None)
    };
    let sample_row = row![
        sample("18:42", hues.muted),
        sample("W1AW", hues.call),
        sample("14074.0", hues.freq),
        sample("20M", hues.band),
        sample("FT8", hues.mode),
    ]
    .spacing(7);
    // The row sits on the theme's data ground, set into the pane.
    let line = palette.background.strong.color;
    let sample_row = container(sample_row)
        .padding([3, 6])
        .width(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(background.into()),
            border: Border {
                color: line,
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        });

    let swatches = row![
        // Status fills, then the row highlight and the window frame.
        swatch(hues.good),
        swatch(hues.warn),
        swatch(hues.bad),
        Space::new().width(Length::Fill),
        swatch(hues.select),
        swatch(hues.chrome),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    let body = container(column![sample_row, swatches].spacing(8)).padding([9, 14]);
    let inner = container(column![title, body])
        .width(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(pane.into()),
            border: Border {
                radius: 5.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });

    // The button's own background shows as a 2px ring around the tile:
    // accent for the current theme, brighter on hover.
    button(inner)
        .on_press(Message::ThemeSelected(theme.clone()))
        .padding(2)
        .width(Length::Fill)
        .style(move |app_theme: &Theme, status| {
            let ext = app_theme.extended_palette();
            let ring = if is_current {
                ext.primary.base.color
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                ext.background.strong.text
            } else {
                ext.background.strong.color
            };
            button::Style {
                background: Some(ring.into()),
                border: Border {
                    radius: 6.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        })
        .into()
}

fn swatch(color: Color) -> Element<'static, Message> {
    container(Space::new().width(14).height(14))
        .style(move |_: &Theme| container::Style {
            background: Some(color.into()),
            border: Border {
                color: Color { a: 0.2, ..Color::BLACK },
                width: 1.0,
                radius: 7.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}
