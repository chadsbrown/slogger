//! Drawing for floating panels, shared by every tab. The geometry and
//! stacking live in [`crate::app::panels`]; this turns one tab's
//! [`Panels`] into its panel bar and canvas.

use iced::widget::{Space, Stack, button, column, container, mouse_area, pin, responsive, row, stack, text};
use iced::{Element, Length, Point, Size, mouse};

use crate::app::message::Message;
use crate::app::panels::{DragMode, PanelKind, Panels};
use crate::app::state::App;
use crate::app::theme::{self, LABEL_TEXT, label, muted};

/// A tab made of floating panels: its panel bar above its canvas.
pub(in crate::app) fn view<'a>(app: &'a App, panels: &'a Panels) -> Element<'a, Message> {
    // `responsive` hands us the canvas size, which the panels need to turn
    // their fractional geometry into pixels.
    let canvas = responsive(move |size| app.panel_canvas(panels, size));
    column![app.panel_bar(panels), canvas]
        .spacing(6)
        .into()
}

/// Gap left around each panel so neighbours don't share a border.
const PANEL_INSET: f32 = 3.0;

impl App {
    /// One toggle per panel (lit when open) plus "Reset layout". This is
    /// where a closed panel is brought back.
    fn panel_bar<'a>(&'a self, panels: &'a Panels) -> Element<'a, Message> {
        // Plain text: this bar sits on the window frame, not on a pane.
        let mut bar = row![text("PANELS").size(LABEL_TEXT)]
            .spacing(4)
            .align_y(iced::Alignment::Center);
        for kind in panels.kinds() {
            bar = bar.push(
                button(text(kind.title()).size(12))
                    .padding([2, 10])
                    .on_press(Message::PanelToggled(*kind))
                    .style(theme::frame_toggle_style(panels.is_open(*kind))),
            );
        }
        bar = bar.push(Space::new().width(Length::Fill)).push(
            button(text("Reset layout").size(12))
                .padding([2, 10])
                .on_press(Message::PanelsReset)
                .style(theme::frame_toggle_style(false)),
        );
        container(bar.wrap()).padding([0, 4]).width(Length::Fill).into()
    }

    /// A tab's canvas: every open panel pinned at its own position,
    /// stacked back-to-front.
    fn panel_canvas<'a>(&'a self, panels: &'a Panels, canvas: Size) -> Element<'a, Message> {
        let mut layers = Stack::new().width(Length::Fill).height(Length::Fill).clip(true);
        let backdrop: Element<'_, Message> = if panels.open().next().is_none() {
            container(label("No panels open — choose one from the PANELS bar above."))
                .center(Length::Fill)
                .style(theme::canvas_style)
                .into()
        } else {
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::canvas_style)
                .into()
        };
        layers = layers.push(backdrop);
        for panel in panels.open() {
            let (position, size) = panel.rect.to_pixels(canvas);
            let size = Size::new(
                (size.width - 2.0 * PANEL_INSET).max(1.0),
                (size.height - 2.0 * PANEL_INSET).max(1.0),
            );
            layers = layers.push(
                pin(self.panel_frame(panel.kind, size, canvas))
                    .position(Point::new(position.x + PANEL_INSET, position.y + PANEL_INSET)),
            );
        }
        layers.into()
    }

    /// A panel's chrome: draggable title bar with a close button, the
    /// body, and a resize grip in the bottom-right corner.
    fn panel_frame(&self, kind: PanelKind, size: Size, canvas: Size) -> Element<'_, Message> {
        let title_bar = mouse_area(
            container(
                row![
                    text(kind.title().to_uppercase()).size(LABEL_TEXT),
                    Space::new().width(Length::Fill),
                    button(text("×").size(14))
                        .padding([0, 6])
                        .on_press(Message::PanelToggled(kind))
                        .style(theme::bar_button),
                ]
                .align_y(iced::Alignment::Center),
            )
            .padding([2, 8])
            .width(Length::Fill)
            .style(theme::title_bar_style),
        )
        .on_press(Message::PanelDragStarted {
            kind,
            mode: DragMode::Move,
            canvas,
        })
        .interaction(mouse::Interaction::Grab);

        let grip = mouse_area(container(text("◢").size(11).style(muted)).padding([0, 2]))
            .on_press(Message::PanelDragStarted {
                kind,
                mode: DragMode::Resize,
                canvas,
            })
            .interaction(mouse::Interaction::ResizingDiagonallyDown);

        let body = stack![
            container(self.panel_body(kind))
                .padding(8)
                .width(Length::Fill)
                .height(Length::Fill),
            container(grip)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right)
                .align_y(iced::alignment::Vertical::Bottom),
        ];

        // The outer mouse_area does two jobs. A click on any inert part of
        // the panel raises it. And because it claims the pointer over the
        // whole panel, a panel underneath never sees hovers or clicks
        // through this one.
        mouse_area(
            container(column![title_bar, body])
                .width(Length::Fixed(size.width))
                .height(Length::Fixed(size.height))
                .clip(true)
                .style(theme::panel_style),
        )
        .on_press(Message::PanelPressed(kind))
        .interaction(mouse::Interaction::Idle)
        .into()
    }

    /// The content of each panel. New panels get an arm here.
    fn panel_body(&self, kind: PanelKind) -> Element<'_, Message> {
        match kind {
            PanelKind::Spots => self.spots_pane(),
            PanelKind::Entry => self.entry_view(),
            PanelKind::Station => self.station_pane(),
            PanelKind::Keyer => self.keyer_pane(),
            PanelKind::Recent => self.recent_pane(),
            PanelKind::CondX => self.condx_pane(),
            PanelKind::Greyline => self.greyline_pane(),
            PanelKind::QslServices => super::qsl::view_services_pane(self),
            PanelKind::QslPending => super::qsl::view_pending_pane(self),
            PanelKind::QslConfirmations => super::qsl::view_confirmations_pane(self),
            PanelKind::LogSearch => self.view_logbook_search_pane(),
            PanelKind::LogResults => self.view_logbook_grid_pane(),
            PanelKind::LogDetail => self.view_logbook_detail_pane(),
            PanelKind::LogAwards => self.view_awards_drawer(),
            PanelKind::LogTools => self.view_logbook_tools_pane(),
        }
    }
}
