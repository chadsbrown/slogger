//! Sessions drawer. Surfaces the operating-session model (the
//! architectural centerpiece per the plans) — list recent sessions, show
//! which is active, and let the operator explicitly end the current one
//! before starting another at the next boot.

use iced::widget::{button, column, row, scrollable, text};
use iced::{Element, Length};
use radio_core::OperatingSession;

use crate::app::message::Message;
use crate::app::state::App;

#[derive(Debug, Default)]
pub(crate) struct SessionsDrawerState {
    /// Last fetched session list, populated when the drawer opens or on
    /// explicit refresh. Bounded to the most recent 50 by default.
    pub sessions: Vec<OperatingSession>,
    pub loading: bool,
    pub last_status: Option<String>,
}

impl App {
    pub(in crate::app) fn view_sessions_drawer(&self) -> Element<'_, Message> {
        let state = &self.sessions_drawer;

        let refresh_btn = button(text(if state.loading {
            "Loading…"
        } else {
            "Refresh"
        }))
        .on_press(Message::SessionsRefreshPressed);

        let active_label: Element<'_, Message> = match self.active_session {
            Some(id) => text(format!("Active session: {}", id)).size(11).into(),
            None => text("No active session").size(11).into(),
        };

        let end_btn: Element<'_, Message> = if self.active_session.is_some() {
            button(text("End active session"))
                .on_press(Message::SessionsEndActivePressed)
                .style(button::danger)
                .into()
        } else {
            column![].into()
        };

        let mut list = column![row![
            text("Started").width(Length::Fixed(160.0)),
            text("Name").width(Length::Fixed(160.0)),
            text("Status").width(Length::Fill),
        ]
        .spacing(4)]
        .spacing(2);
        for s in &state.sessions {
            let is_active = self.active_session == Some(s.id);
            let started = s.started_at.format("%Y-%m-%d %H:%M").to_string();
            let status = if is_active {
                "● active".to_string()
            } else if let Some(ended) = s.ended_at {
                let dur = ended.signed_duration_since(s.started_at);
                let mins = dur.num_minutes().max(0);
                if mins >= 60 {
                    format!("{}h {}m", mins / 60, mins % 60)
                } else {
                    format!("{}m", mins)
                }
            } else {
                "(orphaned)".to_string()
            };
            let name = s.name.clone().unwrap_or_else(|| "(unnamed)".into());
            list = list.push(
                row![
                    text(started).width(Length::Fixed(160.0)).size(12),
                    text(name).width(Length::Fixed(160.0)).size(12),
                    text(status).width(Length::Fill).size(12),
                ]
                .spacing(4),
            );
        }
        if state.sessions.is_empty() && !state.loading {
            list = list.push(text("(no sessions yet — press Refresh)").size(12));
        }

        let status_line: Element<'_, Message> = match &state.last_status {
            Some(s) => text(s.as_str()).size(12).into(),
            None => column![].into(),
        };

        column![
            row![refresh_btn, end_btn].spacing(8),
            active_label,
            scrollable(list).height(Length::Fill),
            status_line,
        ]
        .spacing(8)
        .into()
    }
}
