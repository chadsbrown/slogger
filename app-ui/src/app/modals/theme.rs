//! Theme picker: a grid of tiles, each drawn in its own theme's colours
//! so it previews what choosing it would look like. Picking a tile
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
    let name_color = palette.background.base.text;
    let muted = Color { a: 0.5, ..name_color };

    let name_row = row![
        text(theme.to_string())
            .size(style::ROW_TEXT)
            .color(name_color)
            .width(Length::Fill),
        text(if is_current { "✓" } else { "" })
            .size(style::ROW_TEXT)
            .color(palette.primary.base.color),
    ]
    .align_y(iced::Alignment::Center);

    let swatches = row![
        swatch(palette.primary.base.color),
        swatch(palette.success.base.color),
        swatch(palette.warning.base.color),
        swatch(palette.danger.base.color),
        Space::new().width(Length::Fill),
        // A stand-in for a line of muted text.
        container(Space::new().width(28).height(4)).style(move |_: &Theme| container::Style {
            background: Some(muted.into()),
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        }),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    let inner = container(column![name_row, swatches].spacing(10))
        .padding([12, 14])
        .width(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(background.into()),
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
