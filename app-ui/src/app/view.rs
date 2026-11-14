//! Application shell: the tab bar, the selected view's body, and the
//! persistent bottom status bar. Per-view content lives in `views/`.

use iced::widget::{Space, button, column, container, pick_list, row, text, text_input};
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
        theme::app_theme()
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
        container(body).padding(8).into()
    }

    /// Persistent bottom status bar: the station selector plus the global
    /// status message line. On first launch (no station_locations yet) it
    /// also surfaces the inline create-station form so setup is reachable.
    fn status_bar(&self) -> Element<'_, Message> {
        let station_options: Vec<StationOption> = self
            .station_locations
            .iter()
            .map(StationOption::from)
            .collect();
        let active_option: Option<StationOption> =
            self.active_location.as_ref().map(StationOption::from);

        let selector_row = row![
            theme::label("STATION"),
            pick_list(
                station_options,
                active_option,
                Message::StationLocationSelected,
            )
            .placeholder("(none)")
            .text_size(13)
            .padding([2, 8])
            .width(Length::Fixed(220.0)),
            Space::new().width(Length::Fill),
            text(self.status.as_deref().unwrap_or(""))
                .size(13)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        // Compact by default: just the selector row. The inline
        // create-station form is appended only on first launch (no
        // station_locations yet) so setup stays reachable; once the first
        // location exists it folds away and the bar collapses to one line.
        let mut content = column![selector_row].spacing(6);
        if self.station_locations.is_empty() {
            let create_btn = {
                let btn = button(text(if self.creating_location {
                    "Creating…"
                } else {
                    "Create"
                }));
                if self.creating_location {
                    btn
                } else {
                    btn.on_press(Message::CreateLocationPressed)
                }
            };
            content = content.push(
                row![
                    theme::label("NEW STATION"),
                    text_input("Name", &self.new_location_name)
                        .on_input(Message::NewLocationNameChanged)
                        .width(Length::Fixed(140.0)),
                    text_input("Call", &self.new_location_call)
                        .on_input(Message::NewLocationCallsignChanged)
                        .width(Length::Fixed(110.0)),
                    text_input("Grid", &self.new_location_grid)
                        .on_input(Message::NewLocationGridChanged)
                        .width(Length::Fixed(110.0)),
                    create_btn,
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }

        container(content)
            .padding([4, 8])
            .width(Length::Fill)
            .style(theme::strip_style)
            .into()
    }
}
