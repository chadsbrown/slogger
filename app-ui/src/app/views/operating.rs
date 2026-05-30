//! Operating view — the live-at-the-radio workflow: spots, entry,
//! station state, recent QSOs. This is the default view when the app
//! launches and the one the operator spends the most time in during
//! actual operating sessions.
//!
//! For now this is a thin wrapper that delegates to the existing
//! `App::view_operating_canvas` method in `view.rs`. Subsequent phases
//! will pull the pane content directly into this module.

use iced::Element;

use crate::app::message::Message;
use crate::app::state::App;

pub(in crate::app) fn view(app: &App) -> Element<'_, Message> {
    app.view_operating_canvas()
}
