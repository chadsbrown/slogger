//! Operating view — the live-at-the-radio workflow: spots, entry,
//! station state, recent QSOs, and a passive awards readout. This is the
//! default view when the app launches and the one the operator spends the
//! most time in.
//!
//! Keyboard: Enter logs, Esc wipes (or aborts CW), Tab moves between
//! fields, F1–F3 fire keyer macros. See `subscriptions::key_event`.

use chrono::{DateTime, Utc};
use iced::widget::pane_grid::{Content, PaneGrid, TitleBar};
use iced::widget::{Id, Space, button, column, container, pick_list, row, rule, scrollable, text, text_input};
use iced::{Color, Element, Length, Theme};
use logbook_domain::AwardProgress;
use radio_core::Callsign;

use crate::app::constants::{BANDS, SPOT_MAX_AGE_SECS, modes};
use crate::app::drawers::awards::AwardKind;
use crate::app::focus::{ENTRY_CALL_ID, ENTRY_FREQ_ID, ENTRY_RST_RCVD_ID, ENTRY_RST_SENT_ID};
use crate::app::message::Message;
use crate::app::panes::PaneKind;
use crate::app::spots::{SpotStatus, describe_spot, status_label};
use crate::app::state::App;
use crate::app::theme::{
    self, LABEL_TEXT, MONO, ROW_TEXT, badge, cell, cell_fill, cell_right, label, muted,
    table_header, table_row,
};

pub(in crate::app) fn view(app: &App) -> Element<'_, Message> {
    let grid = PaneGrid::new(&app.panes, |_pane_id, kind, _maximized| {
        let title_bar = TitleBar::new(text(kind.title().to_uppercase()).size(LABEL_TEXT))
            .padding([3, 8])
            .style(theme::title_bar_style);
        let body: Element<'_, Message> = match kind {
            PaneKind::Spots => app.spots_pane(),
            PaneKind::Entry => app.entry_view(),
            PaneKind::Station => app.station_pane(),
            PaneKind::Recent => app.recent_pane(),
        };
        Content::new(container(body).padding(8))
            .title_bar(title_bar)
            .style(theme::pane_content_style)
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .min_size(140.0)
    .spacing(6)
    .on_click(Message::PaneClicked)
    .on_drag(Message::PaneDragged)
    .on_resize(8, Message::PaneResized);

    column![grid, app.awards_strip()].spacing(6).into()
}

// Column widths, shared between each table's header and its rows.
const W_STATUS: f32 = 30.0;
const W_AGE: f32 = 34.0;
const W_CALL: f32 = 96.0;
const W_FREQ: f32 = 76.0;
const W_BAND: f32 = 40.0;
const W_MODE: f32 = 44.0;
const W_ENTITY: f32 = 150.0;
const W_DATE: f32 = 84.0;
const W_TIME: f32 = 44.0;
const W_PREFIX: f32 = 50.0;
const W_CONT: f32 = 36.0;

/// A captioned form field: small label above the input.
fn field<'a>(caption: &'static str, input: Element<'a, Message>) -> Element<'a, Message> {
    column![label(caption), input].spacing(2).into()
}

/// kHz with one decimal — the convention DX clusters and logs use.
fn khz(hz: i64) -> String {
    format!("{:.1}", hz as f64 / 1_000.0)
}

impl App {
    // ---- Entry ----

    /// The QSO entry form. Also embedded in the Logbook view's Detail pane
    /// when editing a QSO there.
    pub(in crate::app) fn entry_view(&self) -> Element<'_, Message> {
        let call = text_input("CALL", &self.call_input)
            .id(Id::new(ENTRY_CALL_ID))
            .on_input(Message::CallChanged)
            .on_submit(Message::LogPressed)
            .font(MONO)
            .size(24)
            .padding([6, 10])
            .width(Length::Fixed(210.0));

        let call_row = row![call, self.call_info()]
            .spacing(12)
            .align_y(iced::Alignment::Center);

        let fields = row![
            field(
                "FREQ MHz",
                text_input("14.250", &self.freq_input)
                    .id(Id::new(ENTRY_FREQ_ID))
                    .on_input(Message::FreqChanged)
                    .on_submit(Message::LogPressed)
                    .font(MONO)
                    .width(Length::Fixed(100.0))
                    .into(),
            ),
            field(
                "BAND",
                pick_list(BANDS, self.band, Message::BandChanged)
                    .width(Length::Fixed(84.0))
                    .into(),
            ),
            field(
                "MODE",
                pick_list(modes(), self.mode.clone(), Message::ModeChanged)
                    .width(Length::Fixed(84.0))
                    .into(),
            ),
            field(
                "SENT",
                text_input("59", &self.rst_sent)
                    .id(Id::new(ENTRY_RST_SENT_ID))
                    .on_input(Message::RstSentChanged)
                    .on_submit(Message::LogPressed)
                    .font(MONO)
                    .width(Length::Fixed(56.0))
                    .into(),
            ),
            field(
                "RCVD",
                text_input("59", &self.rst_rcvd)
                    .id(Id::new(ENTRY_RST_RCVD_ID))
                    .on_input(Message::RstRcvdChanged)
                    .on_submit(Message::LogPressed)
                    .font(MONO)
                    .width(Length::Fixed(56.0))
                    .into(),
            ),
        ]
        .spacing(8)
        .wrap();

        column![
            call_row,
            fields,
            self.entry_buttons(),
            label("Enter log  ·  Esc clear  ·  Tab next field"),
        ]
        .spacing(10)
        .into()
    }

    /// What we know about the callsign being typed: a dupe warning, the
    /// resolved entity, and whether that entity is needed on this band.
    fn call_info(&self) -> Element<'_, Message> {
        let mut lines = column![].spacing(4);

        if self.editing_qso.is_some() {
            lines = lines.push(text("Editing logged QSO").size(12).style(text::warning));
        }

        let resolution = Callsign::parse(self.call_input.trim())
            .ok()
            .and_then(|call| self.resolver.as_deref()?.resolve(&call));
        if let Some(res) = resolution {
            let mut parts: Vec<String> = Vec::new();
            parts.extend(res.country.clone());
            parts.extend(res.continent.clone());
            parts.extend(res.cq_zone.map(|z| format!("CQ {z}")));
            if !parts.is_empty() {
                lines = lines.push(text(parts.join("  ·  ")).size(14));
            }
            if let (Some(id), Some(band)) = (res.dxcc_id, self.band) {
                let worked = self
                    .worked_by_band
                    .get(&band)
                    .is_some_and(|set| set.contains(&id));
                lines = lines.push(if worked {
                    Element::from(
                        text(format!("entity worked on {}", band.as_adif()))
                            .size(12)
                            .style(muted),
                    )
                } else {
                    Element::from(
                        text(format!("NEW entity on {}", band.as_adif()))
                            .size(12)
                            .style(text::success),
                    )
                });
            }
        }

        if let Some(dupe) = &self.dupe_match {
            let band = dupe
                .band
                .map(|b| b.as_adif().to_string())
                .unwrap_or_else(|| "?".into());
            lines = lines.push(
                row![
                    badge("DUPE", theme::app_theme().palette().warning),
                    text(format!("worked {} on {band}", dupe.qso_begin.format("%Y-%m-%d")))
                        .size(12)
                        .style(text::warning),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            );
        }

        lines.into()
    }

    fn entry_buttons(&self) -> Element<'_, Message> {
        let mut buttons = match self.editing_qso {
            None => row![button(text("Log QSO")).on_press(Message::LogPressed)],
            Some(_) => row![
                button(text("Update QSO")).on_press(Message::LogPressed),
                button(text("Delete"))
                    .on_press(Message::DeletePressed)
                    .style(button::danger),
                button(text("Cancel"))
                    .on_press(Message::CancelEditPressed)
                    .style(button::secondary),
            ],
        }
        .spacing(8)
        .align_y(iced::Alignment::Center);
        if self.active_rig_snapshot().is_some() {
            buttons = buttons.push(
                button(text("← From rig").size(12))
                    .on_press(Message::UseRigPressed)
                    .style(button::secondary),
            );
        }
        if self.active_rig_handle().is_some() {
            buttons = buttons.push(
                button(text("→ To rig").size(12))
                    .on_press(Message::SendToRigPressed)
                    .style(button::secondary),
            );
        }
        buttons.into()
    }

    // ---- Spots ----

    fn spots_pane(&self) -> Element<'_, Message> {
        if !self.spots_active {
            return column![
                text("DX cluster not configured").size(14),
                label("Add a [dxcluster] section to config.toml to see live spots here."),
            ]
            .spacing(4)
            .into();
        }

        let resolver = self.resolver.as_deref();
        let now = Utc::now();
        // Only DXCC targeting is supported today — other award kinds need
        // extra resolver data.
        let target_dxcc_id: Option<u16> = self
            .awards_drawer
            .target_unit
            .as_ref()
            .filter(|t| t.kind == AwardKind::Dxcc)
            .and_then(|t| t.key.parse::<u16>().ok());

        // Drop spots older than the age cutoff so the pane reflects current
        // activity rather than a long backlog.
        let recent: Vec<_> = self
            .spots
            .iter()
            .filter(|s| {
                now.signed_duration_since(s.spotted_at).num_seconds() <= SPOT_MAX_AGE_SECS as i64
            })
            .map(|s| (s, describe_spot(s, resolver, &self.worked_by_band)))
            .collect();
        let total = recent.len();
        let needed = recent
            .iter()
            .filter(|(_, info)| info.status == SpotStatus::NeededBand)
            .count();

        let mut summary = format!("{total} spots  ·  {needed} needed");
        if let Some(s) = &self.spots_status {
            summary.push_str(&format!("  ·  {s}"));
        }
        let toolbar = row![
            label(summary).width(Length::Fill),
            button(
                text(if self.spots_needed_only {
                    "Showing needed only"
                } else {
                    "Showing all"
                })
                .size(12)
            )
            .padding([2, 8])
            .on_press(Message::ToggleSpotsNeededOnly)
            .style(if self.spots_needed_only {
                button::primary
            } else {
                button::secondary
            }),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let header = table_header(row![
            cell("", W_STATUS),
            cell_right("AGE", W_AGE),
            cell("CALL", W_CALL),
            cell_right("kHz", W_FREQ),
            cell("BAND", W_BAND),
            cell("MODE", W_MODE),
            cell("ENTITY", W_ENTITY),
            cell_fill("COMMENT"),
        ]);

        let mut rows = column![].spacing(0);
        for (spot, info) in recent
            .into_iter()
            .filter(|(_, info)| !self.spots_needed_only || info.status == SpotStatus::NeededBand)
        {
            let needed = info.status == SpotStatus::NeededBand;
            let worked = info.status == SpotStatus::Worked;
            // Needed spots pop; already-worked ones recede.
            let tone = move |theme: &Theme| -> text::Style {
                if worked { muted(theme) } else { text::Style::default() }
            };
            let status = cell(status_label(info.status), W_STATUS).style(move |theme: &Theme| {
                if needed { text::success(theme) } else { muted(theme) }
            });
            let cells = row![
                status,
                cell_right(age_label(now.signed_duration_since(spot.spotted_at)), W_AGE)
                    .style(muted),
                cell(spot.call.as_str(), W_CALL).style(tone),
                cell_right(khz(spot.freq_hz as i64), W_FREQ).style(tone),
                cell(info.band.map(|b| b.as_adif()).unwrap_or(""), W_BAND).style(tone),
                cell(spot.mode.as_deref().unwrap_or(""), W_MODE).style(tone),
                cell(info.country.clone().unwrap_or_default(), W_ENTITY).style(tone),
                cell_fill(spot.comment.as_deref().unwrap_or("")),
            ];
            let is_target = target_dxcc_id.is_some() && info.dxcc_id == target_dxcc_id;
            rows = rows.push(table_row(cells, Message::SpotClicked(spot.clone()), is_target));
        }

        column![
            toolbar,
            header,
            rule::horizontal(1),
            scrollable(rows).height(Length::Fill),
        ]
        .spacing(4)
        .into()
    }

    // ---- Recent QSOs ----

    fn recent_pane(&self) -> Element<'_, Message> {
        if self.recent.is_empty() {
            return label("No QSOs logged yet.").into();
        }
        let header = table_header(row![
            cell("DATE", W_DATE),
            cell("UTC", W_TIME),
            cell("CALL", W_CALL),
            cell_right("kHz", W_FREQ),
            cell("BAND", W_BAND),
            cell("MODE", W_MODE),
            cell("PFX", W_PREFIX),
            cell("CONT", W_CONT),
            cell_fill("CQ"),
        ]);
        let mut rows = column![].spacing(0);
        for q in &self.recent {
            let cells = row![
                cell(q.qso_begin.format("%Y-%m-%d").to_string(), W_DATE).style(muted),
                cell(q.qso_begin.format("%H:%M").to_string(), W_TIME),
                cell(q.call.as_str(), W_CALL),
                cell_right(q.freq_hz.map(khz).unwrap_or_default(), W_FREQ),
                cell(q.band.map(|b| b.as_adif()).unwrap_or(""), W_BAND),
                cell(
                    q.mode.as_ref().map(|m| m.as_adif().to_string()).unwrap_or_default(),
                    W_MODE
                ),
                cell(q.dxcc_prefix.as_deref().unwrap_or(""), W_PREFIX),
                cell(q.continent.as_deref().unwrap_or(""), W_CONT),
                cell_fill(q.cq_zone.map(|z| z.to_string()).unwrap_or_default()),
            ];
            rows = rows.push(table_row(
                cells,
                Message::QsoSelected(q.id),
                self.editing_qso == Some(q.id),
            ));
        }
        column![
            header,
            rule::horizontal(1),
            scrollable(rows).height(Length::Fill),
        ]
        .spacing(4)
        .into()
    }

    // ---- Station ----

    fn station_pane(&self) -> Element<'_, Message> {
        let palette = theme::app_theme().palette();
        let off = Color { a: 0.3, ..palette.text };
        let mut col = column![].spacing(8);

        // Rigs — one block per rig. With more than one, each block is a
        // button that makes that rig active (and moves SO2R TX with it).
        if self.rigs.is_empty() {
            col = col.push(status_line(off, "Rig", "not configured"));
        }
        for (idx, e) in self.rigs.iter().enumerate() {
            let active = idx == self.active_rig;
            let (freq, detail) = match e.snapshot.as_ref() {
                Some(s) => (
                    s.freq_hz
                        .map(theme::format_rig_freq)
                        .unwrap_or_else(|| "--.---.--".into()),
                    format!("{}{}", s.mode.as_deref().unwrap_or(""), stale_label(s.at)),
                ),
                None => ("--.---.--".into(), e.status.clone()),
            };
            let block = column![
                label(format!(
                    "{}{}{}",
                    if active && self.rigs.len() > 1 { "▶ " } else { "" },
                    e.label.to_uppercase(),
                    match e.config.model.as_deref() {
                        Some(model) if !model.eq_ignore_ascii_case(&e.label) =>
                            format!("  ·  {model}"),
                        _ => String::new(),
                    }
                )),
                row![
                    text(freq).font(MONO).size(22),
                    text(detail).size(ROW_TEXT).style(muted),
                ]
                .spacing(10)
                .align_y(iced::Alignment::End),
            ]
            .spacing(1);
            col = col.push(if self.rigs.len() > 1 {
                Element::from(
                    button(block)
                        .padding([2, 6])
                        .on_press(Message::ActiveRigChanged(idx))
                        .style(theme::tab_style(active)),
                )
            } else {
                Element::from(block)
            });
        }

        // Keyer. The dot goes green while CW is going out — the most
        // reliable TX signal slogger has today (rig PTT isn't surfaced).
        let keying = self.keyer_snapshot.as_ref().is_some_and(|s| s.keying);
        let (keyer_color, keyer_text) = if self.keyer_active {
            let wpm = self.keyer_snapshot.as_ref().map(|s| s.wpm).unwrap_or(0);
            (
                if keying { palette.success } else { palette.primary },
                format!("{wpm} WPM{}", if keying { "  ·  sending" } else { "" }),
            )
        } else {
            (off, self.keyer_status.clone().unwrap_or_else(|| "off".into()))
        };
        col = col.push(status_line(keyer_color, "Keyer", keyer_text));

        if self.keyer_active {
            let mut macros = row![].spacing(4);
            for n in 1..=3u8 {
                if let Some((name, cw)) = self.keyer_macro(n) {
                    macros = macros.push(
                        button(text(format!("F{n} {name}")).size(12))
                            .padding([2, 8])
                            .on_press(Message::KeyerSendMacro(cw))
                            .style(button::secondary),
                    );
                }
            }
            col = col.push(macros);
        }

        let (so2r_color, so2r_text) = if self.so2r_active {
            let s = self.so2r_snapshot.as_ref();
            (
                palette.primary,
                format!(
                    "TX R{}  ·  RX {}",
                    s.map(|s| s.tx_radio).unwrap_or(0),
                    s.map(|s| s.rx_mode.clone()).unwrap_or_default()
                ),
            )
        } else {
            (off, self.so2r_status.clone().unwrap_or_else(|| "off".into()))
        };
        col = col.push(status_line(so2r_color, "SO2R", so2r_text));

        let (wsjtx_color, wsjtx_text) = match (self.wsjtx_active, self.wsjtx_bind_addr.as_deref()) {
            (true, Some(addr)) => (
                palette.primary,
                format!("{addr}  ·  {} imported", self.wsjtx_imported),
            ),
            _ => (off, "off".to_string()),
        };
        col = col.push(status_line(wsjtx_color, "WSJT-X", wsjtx_text));

        scrollable(col).into()
    }

    // ---- Awards ----

    /// Passive one-line awards readout. The drill-down and spot
    /// cross-reference target live in the Logbook view's Awards pane.
    fn awards_strip(&self) -> Element<'_, Message> {
        let a = &self.awards;
        let mut chips = row![
            label(format!("{} QSOs   worked/confirmed", a.total_qsos)),
            award_chip("DXCC".into(), &a.dxcc),
            award_chip("WAS".into(), &a.was),
            award_chip("WPX".into(), &a.wpx),
            award_chip("IOTA".into(), &a.iota),
            award_chip(format!("Marathon {}", a.marathon.year), &a.marathon.entities),
        ]
        .spacing(14)
        .align_y(iced::Alignment::Center);
        if !a.dxcc_by_band.is_empty() {
            chips = chips.push(Space::new().width(Length::Fixed(6.0)));
            chips = chips.push(label("DXCC by band"));
            for (band, progress) in &a.dxcc_by_band {
                chips = chips.push(award_chip(band.as_adif().to_string(), progress));
            }
        }
        container(chips.wrap())
            .padding([4, 8])
            .width(Length::Fill)
            .style(theme::strip_style)
            .into()
    }
}

/// `● Name  detail` — one subsystem's state in the Station pane.
fn status_line<'a>(
    dot: Color,
    name: &'static str,
    detail: impl text::IntoFragment<'a>,
) -> Element<'a, Message> {
    row![
        text("●").size(11).color(dot),
        text(name).size(ROW_TEXT).width(Length::Fixed(52.0)),
        text(detail).size(ROW_TEXT).style(muted),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

/// `DXCC 112/87` — worked in full strength, confirmed muted.
fn award_chip<'a>(name: String, progress: &AwardProgress) -> Element<'a, Message> {
    row![
        label(name),
        text(progress.worked.to_string()).font(MONO).size(ROW_TEXT),
        text(format!("/{}", progress.confirmed))
            .font(MONO)
            .size(ROW_TEXT)
            .style(muted),
    ]
    .spacing(4)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Compact relative age for spot rows: "now" / "12s" / "5m" / "2h".
fn age_label(d: chrono::Duration) -> String {
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

/// "  ·  stale 2m" when the rig hasn't reported in a while; empty when
/// fresh. Threshold is 30s — most modern rigs send events on every dial
/// click, so 30s of silence usually means the connection has died.
fn stale_label(at: DateTime<Utc>) -> String {
    let secs = Utc::now().signed_duration_since(at).num_seconds();
    if secs < 30 {
        String::new()
    } else {
        format!("  ·  stale {}", age_label(chrono::Duration::seconds(secs)))
    }
}
