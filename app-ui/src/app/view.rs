//! Application shell: the selected view's body over the bottom bar, which
//! holds the view tabs, the Panels menu and the status message. Per-view content lives in `views/`.

use iced::widget::{
    Space, button, column, container, mouse_area, pick_list, row, stack, text, tooltip,
};
use iced::{Element, Length};

use super::message::Message;
use super::state::App;
use super::theme;
use super::types::StationOption;
use super::views::ViewKind;

/// Space between the window edge and its contents.
const WINDOW_PADDING: f32 = 8.0;
/// Roughly how tall the bottom bar is: what the Panels menu clears.
const BAR_HEIGHT: f32 = 32.0;

impl App {
    pub fn theme(&self) -> iced::Theme {
        self.theme.clone()
    }

    /// Whole-UI scale factor, driven by the A− / A+ buttons.
    pub fn scale_factor(&self) -> f32 {
        self.font_scale
    }

    pub fn view(&self) -> Element<'_, Message> {
        if let Some(err) = &self.boot_error {
            return container(text(format!("startup failed: {err}")))
                .padding(20)
                .into();
        }
        if self.service.is_none() {
            return container(text("opening database…")).padding(20).into();
        }

        let view_body: Element<'_, Message> = match self.current_view {
            ViewKind::Operating => super::views::operating::view(self),
            ViewKind::Qsl => super::views::qsl::view(self),
            ViewKind::Logbook => super::views::logbook::view(self),
        };

        let body = column![view_body, self.status_bar()].spacing(6);
        let main: Element<'_, Message> = container(body)
            .padding(WINDOW_PADDING)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::canvas_style)
            .into();
        // The Panels menu rises from its button at the left of the bottom
        // bar. A click anywhere else puts it away.
        let main: Element<'_, Message> = if self.panels_menu_open {
            let panels = match self.current_view {
                ViewKind::Operating => &self.panels,
                ViewKind::Qsl => &self.qsl_panels,
                ViewKind::Logbook => &self.logbook_panels,
            };
            let away = mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
                .on_press(Message::PanelsMenuClosed);
            let menu = container(self.panels_menu(panels))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced::alignment::Vertical::Bottom)
                .padding(iced::Padding {
                    left: WINDOW_PADDING,
                    bottom: WINDOW_PADDING + BAR_HEIGHT + 2.0,
                    ..iced::Padding::ZERO
                });
            stack![main, away, menu].into()
        } else {
            main
        };
        // The theme editor docks beside the UI rather than covering it,
        // so the theme being edited can be seen in use.
        let main: Element<'_, Message> = match &self.theme_editor {
            Some(editor) => container(row![main, super::editor::view(editor)])
                .style(theme::canvas_style)
                .into(),
            None => main,
        };
        match &self.modal {
            Some(modal) => super::modals::overlay(main, self, modal),
            None => main,
        }
    }

    /// The station location in use, with Edit and New. It lives in a QSL
    /// panel because it is rarely changed. Until the first location
    /// exists the bottom bar also offers to create one.
    pub(super) fn station_location_pane(&self) -> Element<'_, Message> {
        if self.station_locations.is_empty() {
            return column![
                theme::label("No station location yet. QSOs are logged as made from one."),
                button(text("Create station location").size(13))
                    .padding([2, 10])
                    .on_press(Message::NewStationPressed)
                    .style(button::primary),
            ]
            .spacing(8)
            .into();
        }
        let station_options: Vec<StationOption> =
            self.station_locations.iter().map(StationOption::from).collect();
        let active_option: Option<StationOption> =
            self.active_location.as_ref().map(StationOption::from);
        let small_button = |caption: &'static str, msg: Option<Message>| {
            button(text(caption).size(13))
                .padding([2, 8])
                .on_press_maybe(msg)
                .style(theme::tab_style(false))
        };
        column![
            theme::label("QSOS ARE LOGGED FROM"),
            pick_list(station_options, active_option, Message::StationLocationSelected)
                .placeholder("(none)")
                .text_size(13)
                .padding([2, 8])
                .width(Length::Fill),
            row![
                small_button(
                    "Edit",
                    self.active_location.as_ref().map(|_| Message::EditStationPressed),
                ),
                small_button("New", Some(Message::NewStationPressed)),
            ]
            .spacing(4),
        ]
        .spacing(6)
        .into()
    }

    /// The bottom bar, the only fixed strip in the window. Left: the
    /// Panels menu button, the view tabs and the status message. Centre:
    /// the UTC clock. Right: text size and theme. Until the first station
    /// location exists there is also one prominent "Create station
    /// location" button, so first-run setup is obvious.
    fn status_bar(&self) -> Element<'_, Message> {
        let small_button = |caption: &'static str, msg: Message| {
            button(text(caption).size(13))
                .padding([2, 8])
                .on_press(msg)
                .style(theme::tab_style(false))
        };

        let mut left = row![
            button(text("Panels ▴").size(13))
                .padding([2, 8])
                .on_press(Message::PanelsMenuToggled)
                .style(theme::tab_style(self.panels_menu_open)),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);
        for view in ViewKind::ALL {
            left = left.push(
                button(
                    row![
                        text(view.label()).size(13),
                        // Plain text, so it takes the tab's own colour.
                        text(format!("^{}", view.shortcut_digit())).size(theme::LABEL_TEXT),
                    ]
                    .spacing(4)
                    .align_y(iced::Alignment::Center),
                )
                .padding([2, 8])
                .on_press(Message::ViewChanged(*view))
                .style(theme::tab_style(*view == self.current_view)),
            );
        }
        if self.station_locations.is_empty() {
            left = left.push(
                button(text("Create station location").size(13))
                    .padding([2, 10])
                    .on_press(Message::NewStationPressed)
                    .style(button::primary),
            );
        }
        left = left.push(Space::new().width(6));
        // One line, clipped when it runs into the clock; hovering shows
        // the whole message.
        // Informational messages and notices expire; the clock's tick
        // redraws the bar, so they go without anything else happening.
        let shown = self.status.as_ref().filter(|s| s.shown_at(std::time::Instant::now()));
        let message = shown.map_or("", |s| s.text.as_str());
        let tone = shown.and_then(|s| s.tone());
        let coloured = move |theme: &iced::Theme| match tone {
            Some(tone) => theme::tone_text(tone, theme),
            None => text::Style::default(),
        };
        let clipped = container(
            text(message).size(13).wrapping(text::Wrapping::None).style(coloured),
        )
        .width(Length::Fill)
        .clip(true);
        let status_text: Element<'_, Message> = if message.is_empty() {
            clipped.into()
        } else {
            tooltip(
                clipped,
                container(text(message).size(13))
                    .padding(8)
                    .max_width(700)
                    .style(theme::panel_style),
                tooltip::Position::Top,
            )
            .into()
        };
        let left = left.push(
            status_text,
        );

        let clock = row![
            text(chrono::Utc::now().format("%H:%M:%S").to_string())
                .size(14)
                .font(theme::MONO),
            theme::label("UTC"),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        let right = row![
            small_button("A−", Message::FontScaleDown),
            small_button("A+", Message::FontScaleUp),
            theme::label("THEME"),
            button(text(self.theme.to_string()).size(13))
                .padding([2, 8])
                .on_press_maybe(
                    self.theme_editor
                        .is_none()
                        .then_some(Message::OpenModal(super::modals::Modal::ThemePicker)),
                )
                .style(theme::tab_style(false)),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        // The message gets all the width the controls don't need; the
        // clock sits with the controls on the right.
        let content = row![container(left).width(Length::Fill), clock, right]
            .spacing(16)
            .align_y(iced::Alignment::Center);

        container(content)
            .padding([4, 8])
            .width(Length::Fill)
            .style(theme::strip_style)
            .into()
    }
}
