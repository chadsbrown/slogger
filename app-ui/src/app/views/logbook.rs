//! Logbook view — the "general querying" canvas. Five floating panels
//! (Search / Results / Detail / Awards / Tools), drawn by
//! [`super::floating`]; their bodies live in `drawers`.

use iced::Element;

use crate::app::message::Message;
use crate::app::state::App;

pub(in crate::app) fn view(app: &App) -> Element<'_, Message> {
    super::floating::view(app, &app.logbook_panels)
}
