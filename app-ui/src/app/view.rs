//! Application shell: the tab bar, the selected view's body, and the
//! persistent bottom status bar. Per-view content lives in `views/`.

use iced::widget::{button, column, container, pick_list, row, text, tooltip};
use iced::{Element, Length};

use super::message::Message;
use super::state::App;
use super::theme;
use super::types::StationOption;

// Shared pane chrome, re-exported so the QSL / Logbook views keep their
// existing `crate::app::view::…` paths.
pub(super) use super::theme::{pane_content_style, title_bar_style};

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

        let tabs = super::views::tab_bar(self.current_view);
        let view_body: Element<'_, Message> = match self.current_view {
            super::views::ViewKind::Operating => super::views::operating::view(self),
            super::views::ViewKind::Qsl => super::views::qsl::view(self),
            super::views::ViewKind::Logbook => super::views::logbook::view(self),
        };

        let body = column![tabs, view_body, self.status_bar()].spacing(6);
        let main: Element<'_, Message> = container(body)
            .padding(8)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::canvas_style)
            .into();
        match &self.modal {
            Some(modal) => super::modals::overlay(main, self, modal),
            None => main,
        }
    }

    /// Persistent bottom status bar. Left: the station selector with its
    /// Edit / New buttons and the global status message. Centre: the UTC
    /// clock. Right: text size and theme. Until the first station location
    /// exists the selector is replaced by one prominent "Create station
    /// location" button, so first-run setup is obvious.
    fn status_bar(&self) -> Element<'_, Message> {
        let small_button = |caption: &'static str, msg: Message| {
            button(text(caption).size(13))
                .padding([2, 8])
                .on_press(msg)
                .style(theme::tab_style(false))
        };

        let mut left = row![theme::label("STATION")]
            .spacing(10)
            .align_y(iced::Alignment::Center);
        if self.station_locations.is_empty() {
            left = left.push(
                button(text("Create station location").size(13))
                    .padding([2, 10])
                    .on_press(Message::NewStationPressed)
                    .style(button::primary),
            );
        } else {
            let station_options: Vec<StationOption> = self
                .station_locations
                .iter()
                .map(StationOption::from)
                .collect();
            let active_option: Option<StationOption> =
                self.active_location.as_ref().map(StationOption::from);
            let edit = button(text("Edit").size(13))
                .padding([2, 8])
                .on_press_maybe(
                    self.active_location
                        .as_ref()
                        .map(|_| Message::EditStationPressed),
                )
                .style(theme::tab_style(false));
            left = left.push(
                row![
                    pick_list(
                        station_options,
                        active_option,
                        Message::StationLocationSelected,
                    )
                    .placeholder("(none)")
                    .text_size(13)
                    .padding([2, 8])
                    .width(Length::Fixed(220.0)),
                    edit,
                    small_button("New", Message::NewStationPressed),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
            );
        }
        // One line, clipped when it runs into the clock; hovering shows
        // the whole message.
        let message = self.status.as_deref().unwrap_or("");
        let clipped = container(text(message).size(13).wrapping(text::Wrapping::None))
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
            text(chrono::Utc::now().format("%Y-%m-%d  %H:%M:%S").to_string())
                .size(14)
                .font(theme::MONO),
            theme::label("UTC"),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        let right = row![
            theme::label(format!("TEXT {:.0}%", self.font_scale * 100.0)),
            small_button("A−", Message::FontScaleDown),
            small_button("A+", Message::FontScaleUp),
            theme::label("THEME"),
            button(text(self.theme.to_string()).size(13))
                .padding([2, 8])
                .on_press(Message::OpenModal(super::modals::Modal::ThemePicker))
                .style(theme::tab_style(false)),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        // Left and right get equal shares so the clock sits at the true
        // centre of the window whatever their contents.
        let content = row![
            container(left).width(Length::FillPortion(1)),
            clock,
            container(right)
                .width(Length::FillPortion(1))
                .align_x(iced::alignment::Horizontal::Right),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        container(content)
            .padding([4, 8])
            .width(Length::Fill)
            .style(theme::strip_style)
            .into()
    }
}
