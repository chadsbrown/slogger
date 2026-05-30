use iced::widget::pane_grid::{Content, PaneGrid, TitleBar};
use iced::widget::{Id, Space, button, column, container, pick_list, row, scrollable, text, text_input};
use iced::{Element, Length};
use radio_core::Callsign;
use spot_feed::Spot;

use super::constants::{BANDS, SPOT_MAX_AGE_SECS, modes};
use super::drawers::awards::AwardKind;
use super::focus::ENTRY_CALL_ID;
use super::message::Message;
use super::panes::PaneKind;
use super::spots::{SpotStatus, annotate_spot, status_label};
use super::state::App;
use super::types::StationOption;

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        if let Some(err) = &self.boot_error {
            return container(text(format!("startup failed: {err}")))
                .padding(20)
                .into();
        }
        if self.service.is_none() {
            return container(text("opening database…")).padding(20).into();
        }

        // Persistent header + view tab bar across all three views; the
        // view body itself comes from the per-view module.
        let header = self.header_view();
        let tabs = super::views::tab_bar(self.current_view);
        let view_body: Element<'_, Message> = match self.current_view {
            super::views::ViewKind::Operating => super::views::operating::view(self),
            super::views::ViewKind::Qsl => super::views::qsl::view(self),
            super::views::ViewKind::Logbook => super::views::logbook::view(self),
        };

        let body = column![header, tabs, view_body].spacing(6);
        container(body).padding(10).into()
    }

    /// Renders the Operating canvas — the existing pane_grid layout
    /// (Spots / Entry / Station / Recent) plus the awards strip and
    /// status toast. Called from `views::operating::view`; will be
    /// inlined back into that module in a later phase once all the
    /// per-pane render helpers are moved across.
    pub(super) fn view_operating_canvas(&self) -> Element<'_, Message> {
        let awards = self.awards_view();
        let status = match &self.status {
            Some(s) => text(s.as_str()),
            None => text(""),
        };

        let grid = PaneGrid::new(&self.panes, |_pane_id, kind, _maximized| {
            let title_bar = TitleBar::new(text(kind.title()).size(13))
                .padding(4)
                .style(title_bar_style);
            let body: Element<'_, Message> = match kind {
                PaneKind::Spots => self.spots_panel(),
                PaneKind::Entry => self.entry_view(),
                PaneKind::Station => self.station_view(),
                PaneKind::Recent => self.recent_view(),
            };
            Content::new(container(body).padding(8))
                .title_bar(title_bar)
                .style(pane_content_style)
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .min_size(140.0)
        .spacing(6)
        .on_click(Message::PaneClicked)
        .on_drag(Message::PaneDragged)
        .on_resize(8, Message::PaneResized);

        column![grid, awards, status].spacing(8).into()
    }

    fn header_view(&self) -> Element<'_, Message> {
        let station_options: Vec<StationOption> = self
            .station_locations
            .iter()
            .map(StationOption::from)
            .collect();
        let active_option: Option<StationOption> =
            self.active_location.as_ref().map(StationOption::from);

        // Pending sync pills — five small text segments. When a category
        // has zero pending, we still show it so the layout doesn't shift.
        let pending_pills = text(format!(
            "L{} · E{} · C{} · Q{} · H{}",
            self.pending_lotw,
            self.pending_eqsl,
            self.pending_clublog,
            self.pending_qrz,
            self.pending_hrdlog,
        ));

        let update_btn = {
            let btn = button(text(if self.syncing {
                "Updating…"
            } else {
                "Update services"
            }));
            if self.syncing {
                btn
            } else {
                btn.on_press(Message::ServicesUpdatePressed)
            }
        };

        // Session label is now a passive readout. Session management
        // (switch / end) lives in the Logbook view's Sessions pane;
        // operators reach it via the view tabs.
        let session_text = match &self.active_location {
            Some(loc) => format!("Session · {}", loc.name),
            None => "Session · (no station)".into(),
        };
        let session_label: Element<'_, Message> = text(session_text).into();

        let top_row = row![
            session_label,
            text("Station:"),
            pick_list(
                station_options,
                active_option,
                Message::StationLocationSelected,
            )
            .placeholder("(none)")
            .width(Length::Fixed(220.0)),
            Space::new().width(Length::Fill),
            text("Pending:"),
            pending_pills,
            update_btn,
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        // ADIF import controls moved into the Logbook view's Tools pane.

        // Inline create-station form — visible whenever station_locations
        // is empty so first-launch operators have an obvious path to set up
        // a station. Once the first location exists, the form folds away.
        let create_row: Element<'_, Message> = if self.station_locations.is_empty() {
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
            row![
                text("New station:"),
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
            .align_y(iced::Alignment::Center)
            .into()
        } else {
            text("").into()
        };

        container(column![top_row, create_row].spacing(6))
            .padding(8)
            .style(header_style)
            .into()
    }

    pub(in crate::app) fn entry_view(&self) -> Element<'_, Message> {
        let dupe_badge: Element<'_, Message> = match &self.dupe_match {
            Some(dupe) => {
                let date = dupe.qso_begin.format("%Y-%m-%d");
                let band = dupe
                    .band
                    .map(|b| b.as_adif().to_string())
                    .unwrap_or_else(|| "?".into());
                container(
                    text(format!("DUP B4: {date} ({band})"))
                        .size(12)
                        .color(iced::Color::from_rgb(0.85, 0.45, 0.10)),
                )
                .padding(2)
                .into()
            }
            None => text("").into(),
        };
        let form = column![
            row![
                text("Call:").width(Length::Fixed(80.0)),
                text_input("W1AW", &self.call_input)
                    .id(Id::new(ENTRY_CALL_ID))
                    .on_input(Message::CallChanged)
                    .on_submit(Message::LogPressed)
                    .width(Length::Fixed(180.0)),
                dupe_badge,
            ]
            .spacing(10),
            row![
                text("Band:").width(Length::Fixed(80.0)),
                pick_list(BANDS, self.band, Message::BandChanged).width(Length::Fixed(120.0)),
                text("Mode:").width(Length::Fixed(60.0)),
                pick_list(modes(), self.mode.clone(), Message::ModeChanged)
                    .width(Length::Fixed(120.0)),
            ]
            .spacing(10),
            row![
                text("Freq MHz:").width(Length::Fixed(80.0)),
                text_input("14.250", &self.freq_input)
                    .id(Id::new(super::focus::ENTRY_FREQ_ID))
                    .on_input(Message::FreqChanged)
                    .on_submit(Message::LogPressed)
                    .width(Length::Fixed(120.0)),
            ]
            .spacing(10),
            row![
                text("RST snt:").width(Length::Fixed(80.0)),
                text_input("59", &self.rst_sent)
                    .id(Id::new(super::focus::ENTRY_RST_SENT_ID))
                    .on_input(Message::RstSentChanged)
                    .on_submit(Message::LogPressed)
                    .width(Length::Fixed(80.0)),
                text("RST rcv:").width(Length::Fixed(80.0)),
                text_input("59", &self.rst_rcvd)
                    .id(Id::new(super::focus::ENTRY_RST_RCVD_ID))
                    .on_input(Message::RstRcvdChanged)
                    .on_submit(Message::LogPressed)
                    .width(Length::Fixed(80.0)),
            ]
            .spacing(10),
            self.entry_buttons(),
        ]
        .spacing(8);
        form.into()
    }

    fn station_view(&self) -> Element<'_, Message> {
        let mut col = column![].spacing(6);

        // Rigs — one row per rig. Active rig marked with [*]. For
        // multi-rig setups, each label doubles as a "set this active"
        // button.
        if self.rigs.is_empty() {
            col = col.push(text("Rig off (no [[rig]] in config)").size(13));
        } else {
            for (idx, e) in self.rigs.iter().enumerate() {
                let marker = if idx == self.active_rig { "▶" } else { "  " };
                let model = e
                    .config
                    .model
                    .as_deref()
                    .unwrap_or(e.label.as_str());
                let body = match e.snapshot.as_ref() {
                    Some(s) => {
                        let freq = s
                            .freq_hz
                            .map(|hz| format!("{:.5}", hz as f64 / 1_000_000.0))
                            .unwrap_or_else(|| "?".into());
                        let mode = s.mode.as_deref().unwrap_or("?");
                        let stale = stale_label(s.at);
                        format!("{marker} {} ({}) {} {}{}", e.label, model, freq, mode, stale)
                    }
                    None => format!("{marker} {} ({}) — {}", e.label, model, e.status),
                };
                col = if self.rigs.len() > 1 {
                    col.push(
                        button(text(body).size(13))
                            .on_press(Message::ActiveRigChanged(idx))
                            .style(button::text),
                    )
                } else {
                    col.push(text(body).size(13))
                };
            }
        }

        // Keyer + TX/RX indicator. The dot is derived from keyer.keying
        // primarily (the most reliable TX signal slogger gets today —
        // rig PTT isn't surfaced). If SO2R is configured but no keyer,
        // we fall back to gray since we can't tell TX state without
        // either a keyer or a PTT event from the rig.
        let keying = self
            .keyer_snapshot
            .as_ref()
            .map(|s| s.keying)
            .unwrap_or(false);
        let keyer_label = if self.keyer_active {
            let s = self.keyer_snapshot.as_ref();
            let wpm = s.map(|s| s.wpm).unwrap_or(0);
            format!("Keyer · {wpm} WPM")
        } else if let Some(s) = &self.keyer_status {
            s.clone()
        } else {
            "Keyer off".into()
        };
        let indicator_color = if keying {
            iced::Color::from_rgb(0.20, 0.75, 0.30) // green = TX
        } else if self.keyer_active {
            iced::Color::from_rgb(0.55, 0.55, 0.55) // gray = idle/RX
        } else {
            iced::Color::from_rgba(0.55, 0.55, 0.55, 0.4) // muted gray = no keyer
        };
        col = col.push(
            iced::widget::row![
                text("●").size(14).color(indicator_color),
                text(keyer_label).size(13),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
        );

        // Keyer macros — only when the keyer is actually connected. CQ
        // splices in the operator's own callsign so the macro stays
        // useful as the operator moves stations.
        if self.keyer_active {
            let my_call = self
                .active_location
                .as_ref()
                .and_then(|loc| loc.station_callsign.as_ref().map(|c| c.as_str().to_string()))
                .or_else(|| {
                    self.config
                        .as_ref()
                        .and_then(|c| c.station.default_callsign.clone())
                })
                .unwrap_or_else(|| "<CALL>".into());
            let his_call = self.call_input.trim().to_string();
            let his_call_for_button = his_call.clone();
            let cq_text = format!("CQ CQ DE {0} {0} K", my_call);
            let his_call_text = if his_call.is_empty() {
                String::new()
            } else {
                format!("{} DE {}", his_call_for_button, my_call)
            };
            let mut macros = iced::widget::row![
                button(text("CQ").size(12))
                    .on_press(Message::KeyerSendMacro(cq_text)),
                button(text("TU 73").size(12))
                    .on_press(Message::KeyerSendMacro("TU 73".into())),
            ]
            .spacing(4);
            if !his_call.is_empty() {
                macros = macros.push(
                    button(text(format!("→ {}", his_call_for_button)).size(12))
                        .on_press(Message::KeyerSendMacro(his_call_text)),
                );
            }
            col = col.push(macros);
        }

        // SO2R switch.
        let so2r_label = if self.so2r_active {
            let s = self.so2r_snapshot.as_ref();
            let tx = s.map(|s| s.tx_radio).unwrap_or(0);
            let rx_mode = s.map(|s| s.rx_mode.clone()).unwrap_or_default();
            format!("SO2R · TX:R{tx} · RX:{rx_mode}")
        } else if let Some(s) = &self.so2r_status {
            s.clone()
        } else {
            "SO2R off".into()
        };
        col = col.push(text(so2r_label).size(13));

        // WSJT-X.
        let wsjtx_label = match (self.wsjtx_active, self.wsjtx_bind_addr.as_deref()) {
            (true, Some(addr)) => format!(
                "WSJT-X {addr} · auto-imported {}",
                self.wsjtx_imported
            ),
            _ => "WSJT-X off".into(),
        };
        col = col.push(text(wsjtx_label).size(13));

        col.into()
    }

    fn recent_view(&self) -> Element<'_, Message> {
        let header = row![
            text("Time UTC").width(Length::Fixed(180.0)),
            text("Call").width(Length::Fixed(110.0)),
            text("Band").width(Length::Fixed(60.0)),
            text("Mode").width(Length::Fixed(60.0)),
            text("Freq").width(Length::Fixed(110.0)),
            text("DXCC").width(Length::Fixed(60.0)),
            text("Cont").width(Length::Fixed(50.0)),
            text("CQ").width(Length::Fill),
        ];
        let mut list = column![header].spacing(2);
        for q in &self.recent {
            let row_label = format!(
                "{:<19} {:<10} {:<6} {:<6} {:<10} {:<6} {:<5} {}",
                q.qso_begin.format("%Y-%m-%d %H:%M:%S"),
                q.call.as_str(),
                q.band.map(|b| b.as_adif().to_string()).unwrap_or_default(),
                q.mode
                    .as_ref()
                    .map(|m| m.as_adif().to_string())
                    .unwrap_or_default(),
                q.freq_hz
                    .map(|hz| format!("{:.5}", hz as f64 / 1_000_000.0))
                    .unwrap_or_default(),
                q.dxcc_prefix.as_deref().unwrap_or(""),
                q.continent.as_deref().unwrap_or(""),
                q.cq_zone.map(|z| z.to_string()).unwrap_or_default(),
            );
            let editing_this = self.editing_qso == Some(q.id);
            let mut row_btn = button(text(row_label)).on_press(Message::QsoSelected(q.id));
            if !editing_this {
                row_btn = row_btn.style(button::text);
            }
            list = list.push(row_btn);
        }
        scrollable(list).height(Length::Fill).into()
    }

    fn awards_view(&self) -> Element<'_, Message> {
        let a = &self.awards;
        let summary = format!(
            "Awards ({} qso) · DXCC {}/{} · WAS {}/{} · WPX {}/{} · IOTA {}/{} · Marathon{} {}",
            a.total_qsos,
            a.dxcc.worked,
            a.dxcc.confirmed,
            a.was.worked,
            a.was.confirmed,
            a.wpx.worked,
            a.wpx.confirmed,
            a.iota.worked,
            a.iota.confirmed,
            a.marathon.year,
            a.marathon.entities.worked,
        );
        let dxcc_by_band: Option<String> = if a.dxcc_by_band.is_empty() {
            None
        } else {
            let parts: Vec<String> = a
                .dxcc_by_band
                .iter()
                .map(|(band, prog)| {
                    format!("{}={}/{}", band.as_adif(), prog.worked, prog.confirmed)
                })
                .collect();
            Some(format!("DXCC by band: {}", parts.join(", ")))
        };
        let inner: Element<'_, Message> = match dxcc_by_band {
            Some(line) => column![text(summary).size(13), text(line).size(12)]
                .spacing(2)
                .into(),
            None => text(summary).size(13).into(),
        };
        // Passive readout in Operating — the drill-down + spot
        // cross-reference target lives in the Logbook view's Awards
        // pane.
        container(inner).padding(6).width(Length::Fill).into()
    }

    pub(super) fn entry_buttons(&self) -> Element<'_, Message> {
        let mut buttons = match self.editing_qso {
            None => row![button(text("Log QSO")).on_press(Message::LogPressed)],
            Some(_) => row![
                button(text("Update QSO")).on_press(Message::LogPressed),
                button(text("Delete")).on_press(Message::DeletePressed),
                button(text("Cancel edit")).on_press(Message::CancelEditPressed),
            ],
        }
        .spacing(8);
        if self.active_rig_snapshot().is_some() {
            buttons = buttons.push(
                button(text("Use rig").size(12))
                    .on_press(Message::UseRigPressed)
                    .style(button::text),
            );
        }
        if self.active_rig_handle().is_some() {
            buttons = buttons.push(
                button(text("Send to rig").size(12))
                    .on_press(Message::SendToRigPressed)
                    .style(button::text),
            );
        }
        buttons.into()
    }

    pub(super) fn spots_panel(&self) -> Element<'_, Message> {
        if !self.spots_active {
            return text("Spots: dxcluster not configured").into();
        }

        let resolver = self.resolver.as_deref();
        let now = chrono::Utc::now();
        // Resolve the awards-drawer's targeted DXCC entity (if any) once
        // up front so we can compare each spot's resolved entity against
        // it without re-parsing the key per row. Only DXCC targeting is
        // supported today — other award kinds need extra resolver data.
        let target_dxcc_id: Option<u16> = self
            .awards_drawer
            .target_unit
            .as_ref()
            .filter(|t| t.kind == AwardKind::Dxcc)
            .and_then(|t| t.key.parse::<u16>().ok());
        // Annotate every spot with its DXCC need state, then drop spots
        // older than the age cutoff so the panel reflects current
        // operating activity rather than a long backlog.
        let annotated: Vec<(Spot, SpotStatus)> = self
            .spots
            .iter()
            .filter(|s| {
                now.signed_duration_since(s.spotted_at).num_seconds()
                    <= SPOT_MAX_AGE_SECS as i64
            })
            .map(|s| {
                let status = match resolver {
                    Some(r) => annotate_spot(s, r, &self.worked_by_band),
                    None => SpotStatus::Unknown,
                };
                (s.clone(), status)
            })
            .collect();
        let total_recent = annotated.len();
        let needed_count = annotated
            .iter()
            .filter(|(_, st)| matches!(st, SpotStatus::NeededBand))
            .count();
        let visible: Vec<&(Spot, SpotStatus)> = if self.spots_needed_only {
            annotated
                .iter()
                .filter(|(_, s)| matches!(s, SpotStatus::NeededBand))
                .collect()
        } else {
            annotated.iter().collect()
        };
        let shown = visible.len();

        let mut header_text = format!(
            "{shown} shown of {total_recent} · {needed_count} needed"
        );
        if let Some(s) = &self.spots_status {
            header_text.push_str(&format!(" — {s}"));
        }
        let toggle_label = if self.spots_needed_only {
            "Show all"
        } else {
            "Needed only"
        };
        let header_row = row![
            text(header_text).size(12).width(Length::Fill),
            button(text(toggle_label).size(12))
                .on_press(Message::ToggleSpotsNeededOnly)
                .style(button::text),
        ]
        .spacing(8);
        let mut col = column![header_row].spacing(2);
        for (spot, status) in visible.iter() {
            let mhz = spot.freq_hz as f64 / 1_000_000.0;
            let mode_label = spot.mode.as_deref().unwrap_or("");
            let comment = spot.comment.as_deref().unwrap_or("");
            let age = relative_age_label(now.signed_duration_since(spot.spotted_at));
            let label = format!(
                "{:<3} {:>5} {:<8} {:>9.4} {:<4}  {}",
                status_label(*status),
                age,
                spot.call,
                mhz,
                mode_label,
                comment
            );
            let is_target = match (target_dxcc_id, resolver) {
                (Some(target), Some(r)) => spot_dxcc_id(spot, r) == Some(target),
                _ => false,
            };
            let btn = button(text(label).size(12))
                .on_press(Message::SpotClicked(spot.clone()))
                .style(button::text);
            let row_el: Element<'_, Message> = if is_target {
                container(btn).padding(2).style(target_row_style).into()
            } else {
                btn.into()
            };
            col = col.push(row_el);
        }
        scrollable(col).height(Length::Fill).into()
    }
}

/// Look up a spot's DXCC entity id via the resolver. Returns None if the
/// callsign is invalid, the resolver doesn't know it, or the resolver
/// returned no dxcc_id. Used by the awards-cross-reference highlight in
/// the spots panel.
fn spot_dxcc_id(spot: &spot_feed::Spot, resolver: &dyn station_resolver::Resolver) -> Option<u16> {
    let call = Callsign::parse(&spot.call).ok()?;
    resolver.resolve(&call)?.dxcc_id
}

/// Border style for a spot row that matches the operator's currently
/// targeted award unit. Uses the primary palette pair so the highlight
/// stands out without being garish.
fn target_row_style(theme: &iced::Theme) -> iced::widget::container::Style {
    let pair = theme.extended_palette().primary.weak;
    iced::widget::container::Style {
        background: Some(pair.color.into()),
        text_color: Some(pair.text),
        border: iced::Border {
            color: theme.extended_palette().primary.strong.color,
            width: 1.5,
            radius: 4.0.into(),
        },
        ..iced::widget::container::Style::default()
    }
}

fn header_style(theme: &iced::Theme) -> iced::widget::container::Style {
    let pair = theme.extended_palette().primary.weak;
    iced::widget::container::Style {
        background: Some(pair.color.into()),
        text_color: Some(pair.text),
        border: iced::Border {
            color: theme.extended_palette().primary.strong.color,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced::widget::container::Style::default()
    }
}

/// Pane content background — slight border so the operator can see where
/// each pane ends; reuses the standard "weak background" palette pair.
pub(super) fn pane_content_style(theme: &iced::Theme) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: None,
        border: iced::Border {
            color: theme.extended_palette().background.strong.color,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced::widget::container::Style::default()
    }
}

/// Pane title bar background — slightly stronger than the content area so
/// the operator's eye finds the drag handle.
pub(super) fn title_bar_style(theme: &iced::Theme) -> iced::widget::container::Style {
    let pair = theme.extended_palette().background.strong;
    iced::widget::container::Style {
        background: Some(pair.color.into()),
        text_color: Some(pair.text),
        border: iced::Border {
            color: theme.extended_palette().background.strong.color,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced::widget::container::Style::default()
    }
}

/// Compact relative-age label for spot rows. "now" / "12s" / "5m" / "2h".
/// Designed to fit a 5-char column width.
fn relative_age_label(d: chrono::Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 5 {
        "now".into()
    } else if secs < 60 {
        format!("{secs}s")
    } else if secs < 60 * 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

/// Format a "(stale: N min ago)" suffix when the rig hasn't reported in a
/// while. Empty string when fresh. Threshold is 30s — chosen because most
/// modern rigs send events on every dial click; 30s of silence usually
/// means the connection has died, not that the operator stopped tuning.
fn stale_label(at: chrono::DateTime<chrono::Utc>) -> String {
    let elapsed = chrono::Utc::now().signed_duration_since(at);
    if elapsed.num_seconds() < 30 {
        return String::new();
    }
    let secs = elapsed.num_seconds();
    if secs < 60 {
        format!("  (stale: {secs}s ago)")
    } else if secs < 3600 {
        format!("  (stale: {} min ago)", secs / 60)
    } else {
        format!("  (stale: {:.1}h ago)", secs as f64 / 3600.0)
    }
}
