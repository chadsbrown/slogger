//! Modal dialogs: one at a time, drawn centred over a dimmed copy of the
//! whole window.
//!
//! To add a modal:
//! 1. add a variant to [`Modal`] (carrying whatever state it needs);
//! 2. write its `view` in a submodule, wrapping the content in [`frame`];
//! 3. add an arm for it in [`view`];
//! 4. open it with `Message::OpenModal(Modal::…)`, or with a message of
//!    its own when building its state needs the app (the station editor).
//!
//! Closing is handled here for every modal: the × button, Esc, and (for
//! modals that hold nothing to lose) a click on the backdrop.

pub(super) mod station;
pub(super) mod theme;

use iced::widget::{Space, button, column, container, mouse_area, row, rule, stack, text};
use iced::{Border, Color, Element, Length, Theme};

use super::message::Message;
use super::state::App;

/// The modal currently open, if any (`App::modal`).
#[derive(Debug, Clone)]
pub enum Modal {
    ThemePicker,
    StationEditor(station::StationEditor),
}

impl Modal {
    /// Whether a click outside the modal dismisses it. True for pickers,
    /// where nothing is lost; false for forms, where a stray click would
    /// throw away what the operator typed (× and Esc still close them).
    fn closes_on_backdrop_click(&self) -> bool {
        match self {
            Modal::ThemePicker => true,
            Modal::StationEditor(_) => false,
        }
    }
}

/// Lays `modal` over `main`: a dimming backdrop that closes the modal
/// when clicked and keeps clicks from reaching the UI underneath, then
/// the modal itself.
pub(super) fn overlay<'a>(
    main: Element<'a, Message>,
    app: &'a App,
    modal: &'a Modal,
) -> Element<'a, Message> {
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(Color { a: 0.55, ..Color::BLACK }.into()),
                ..container::Style::default()
            }),
    )
    .on_press(if modal.closes_on_backdrop_click() {
        Message::CloseModal
    } else {
        Message::ModalClicked
    });
    stack![main, backdrop, view(app, modal)].into()
}

fn view<'a>(app: &'a App, modal: &'a Modal) -> Element<'a, Message> {
    match modal {
        Modal::ThemePicker => theme::view(&app.theme),
        Modal::StationEditor(editor) => station::view(editor),
    }
}

/// The shared modal shell: title and close button, the body, and an
/// optional footer, centred in the window. With `height` set the body
/// fills it (use a scrollable); otherwise the modal shrinks to fit.
pub(super) fn frame<'a>(
    title: &'a str,
    body: Element<'a, Message>,
    footer: Option<Element<'a, Message>>,
    width: f32,
    height: Option<f32>,
) -> Element<'a, Message> {
    let header = row![
        text(title).size(16),
        Space::new().width(Length::Fill),
        button(text("×").size(16))
            .padding([0, 8])
            .on_press(Message::CloseModal)
            .style(super::theme::tab_style(false)),
    ]
    .align_y(iced::Alignment::Center);

    let mut col = column![
        container(header).padding([10, 16]),
        rule::horizontal(1),
        container(body)
            .padding(16)
            .width(Length::Fill)
            .height(if height.is_some() {
                Length::Fill
            } else {
                Length::Shrink
            }),
    ];
    if let Some(footer) = footer {
        col = col
            .push(rule::horizontal(1))
            .push(container(footer).padding([10, 16]).width(Length::Fill));
    }

    let framed = container(col)
        .width(Length::Fixed(width))
        .height(height.map(Length::Fixed).unwrap_or(Length::Shrink))
        .style(|theme: &Theme| {
            let ext = theme.extended_palette();
            container::Style {
                background: Some(ext.background.base.color.into()),
                text_color: Some(theme.palette().text),
                border: Border {
                    color: ext.background.strong.color,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: iced::Shadow {
                    color: Color { a: 0.5, ..Color::BLACK },
                    offset: iced::Vector::new(0.0, 6.0),
                    blur_radius: 24.0,
                },
                ..container::Style::default()
            }
        });

    // The frame swallows clicks so only the bare backdrop closes the modal.
    container(mouse_area(framed).on_press(Message::ModalClicked))
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}
