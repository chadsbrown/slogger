//! Logbook view — the "general querying" canvas. Hosts six panes
//! (Search / Grid / Detail / Awards / Sessions / Tools) in one
//! resizable layout; replaces the previous Logbook / Awards / Sessions
//! drawer overlays.

use iced::widget::pane_grid::{Axis, Configuration, Content, PaneGrid, TitleBar};
use iced::widget::{container, text};
use iced::{Element, Length};
use serde::{Deserialize, Serialize};

use crate::app::message::Message;
use crate::app::state::App;

/// Identifier for each pane in the Logbook view. Carried by
/// `pane_grid::State<LogbookPaneKind>` on App.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogbookPaneKind {
    Search,
    Grid,
    Detail,
    Awards,
    Sessions,
    Tools,
}

impl LogbookPaneKind {
    fn title(self) -> &'static str {
        match self {
            LogbookPaneKind::Search => "Search",
            LogbookPaneKind::Grid => "Results",
            LogbookPaneKind::Detail => "Detail",
            LogbookPaneKind::Awards => "Awards",
            LogbookPaneKind::Sessions => "Sessions",
            LogbookPaneKind::Tools => "Tools",
        }
    }
}

/// Default Logbook canvas split.
///
/// ```text
/// ┌──────────────────────────┬──────────────────┐
/// │ Search       (top-left)  │ Awards (top-r)   │
/// ├──────────────────────────┼──────────────────┤
/// │ Grid    (center-left)    │ Sessions (mid-r) │
/// ├──────────────────────────┼──────────────────┤
/// │ Detail  (bottom-left)    │ Tools  (bot-r)   │
/// └──────────────────────────┴──────────────────┘
/// ```
pub fn default_logbook_configuration() -> Configuration<LogbookPaneKind> {
    Configuration::Split {
        axis: Axis::Vertical,
        ratio: 0.65,
        a: Box::new(Configuration::Split {
            axis: Axis::Horizontal,
            ratio: 0.25,
            a: Box::new(Configuration::Pane(LogbookPaneKind::Search)),
            b: Box::new(Configuration::Split {
                axis: Axis::Horizontal,
                ratio: 0.55,
                a: Box::new(Configuration::Pane(LogbookPaneKind::Grid)),
                b: Box::new(Configuration::Pane(LogbookPaneKind::Detail)),
            }),
        }),
        b: Box::new(Configuration::Split {
            axis: Axis::Horizontal,
            ratio: 0.45,
            a: Box::new(Configuration::Pane(LogbookPaneKind::Awards)),
            b: Box::new(Configuration::Split {
                axis: Axis::Horizontal,
                ratio: 0.55,
                a: Box::new(Configuration::Pane(LogbookPaneKind::Sessions)),
                b: Box::new(Configuration::Pane(LogbookPaneKind::Tools)),
            }),
        }),
    }
}

pub(in crate::app) fn view(app: &App) -> Element<'_, Message> {
    let grid = PaneGrid::new(&app.logbook_panes, |_pane_id, kind, _maximized| {
        let title_bar = TitleBar::new(text(kind.title()).size(13))
            .padding(4)
            .style(crate::app::view::title_bar_style);
        let body: Element<'_, Message> = match kind {
            LogbookPaneKind::Search => app.view_logbook_search_pane(),
            LogbookPaneKind::Grid => app.view_logbook_grid_pane(),
            LogbookPaneKind::Detail => app.view_logbook_detail_pane(),
            LogbookPaneKind::Awards => app.view_awards_drawer(),
            LogbookPaneKind::Sessions => app.view_sessions_drawer(),
            LogbookPaneKind::Tools => app.view_logbook_tools_pane(),
        };
        Content::new(container(body).padding(8))
            .title_bar(title_bar)
            .style(crate::app::view::pane_content_style)
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .min_size(140.0)
    .spacing(6)
    .on_click(Message::PaneClicked)
    .on_drag(Message::PaneDragged)
    .on_resize(8, Message::PaneResized);

    container(grid).padding(4).into()
}
