//! Operating view — the live-at-the-radio workflow. Its content lives in
//! free-floating panels (see `panels.rs`) that the operator arranges,
//! opens and closes to suit what they're doing, above a passive awards
//! readout. This is the
//! default view when the app launches and the one the operator spends the
//! most time in.
//!
//! Keyboard: Enter logs, Esc wipes (or aborts CW), Tab moves between
//! fields, F1–F12 send the keyer pane's messages. See `subscriptions::key_event`.

use chrono::{DateTime, Utc};
use iced::widget::{
    Id, Space, button, column, pick_list, row, rule, scrollable, text, text_input,
};
use iced::{Color, Element, Length, Theme};
use radio_core::Callsign;

use crate::app::condx::{
    RatingTone, group_by_band, number_or_dash, rating_tone, text_or_dash, updated_label,
};
use crate::app::constants::{BANDS, SPOT_MAX_AGE_SECS, modes};
use crate::app::drawers::awards::AwardKind;
use crate::app::focus::{
    ENTRY_CALL_ID, ENTRY_FREQ_ID, ENTRY_RST_RCVD_ID, ENTRY_RST_SENT_ID, KEYER_TEXT_ID,
};
use crate::app::message::Message;
use crate::app::pota;
use crate::app::spots::{
    SpotColumn, SpotInfo, SpotStatus, describe_spot, sort_spots, status_label,
};
use spot_feed::Spot;
use crate::app::state::App;
use crate::app::theme::{
    self, MONO, ROW_TEXT, Tone, cell, cell_fill, cell_right, filled_cell,
    label, muted, table_header, table_line,
};

pub(in crate::app) fn view(app: &App) -> Element<'_, Message> {
    super::floating::view(app, &app.panels)
}

// Column widths, shared between each table's header and its rows.
const W_STATUS: f32 = 30.0;
const W_AGE: f32 = 34.0;
/// The POTA marker column, present only while POTA spots are on.
const W_POTA: f32 = 40.0;
const W_CALL: f32 = 96.0;
const W_FREQ: f32 = 76.0;
const W_BAND: f32 = 40.0;
const W_MODE: f32 = 44.0;
const W_ENTITY: f32 = 150.0;
const W_DATE: f32 = 84.0;
const W_TIME: f32 = 44.0;
const W_PREFIX: f32 = 50.0;
const W_CONT: f32 = 36.0;
const W_BAND_GROUP: f32 = 70.0;
const W_RATING: f32 = 56.0;

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

    /// What we know about the callsign being typed: an earlier contact, the
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
                if !worked {
                    lines = lines.push(theme::tone_badge(
                        format!("NEW entity on {}", band.as_adif()),
                        Tone::Good,
                    ));
                }
            }
        }

        if let Some(dupe) = &self.dupe_match {
            // Working a station again is normal outside contests (POTA
            // activators, friends), so this is information, not a warning.
            lines = lines.push(
                text(format!("last worked {}", dupe.qso_begin.format("%Y-%m-%d")))
                    .size(12)
                    .style(muted),
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
                    .style(theme::push_button),
            ],
        }
        .spacing(8)
        .align_y(iced::Alignment::Center);
        if self.active_rig_snapshot().is_some() {
            buttons = buttons.push(
                button(text("← From rig").size(12))
                    .on_press(Message::UseRigPressed)
                    .style(theme::push_button),
            );
        }
        if self.active_rig_handle().is_some() {
            buttons = buttons.push(
                button(text("→ To rig").size(12))
                    .on_press(Message::SendToRigPressed)
                    .style(theme::push_button),
            );
        }
        buttons.into()
    }

    // ---- Spots ----

    pub(in crate::app) fn spots_pane(&self) -> Element<'_, Message> {
        // Switches POTA spots on and off; lit while on.
        let pota_button = button(
            text(if self.pota_on { "POTA spots on" } else { "POTA spots off" }).size(12),
        )
        .padding([2, 8])
        .on_press(Message::TogglePota)
        .style(if self.pota_on { button::primary } else { theme::push_button });

        if !self.spots_active && !self.pota_on {
            return column![
                text("DX cluster not configured").size(14),
                label("Add a [dxcluster] section to config.toml to see live spots here."),
                row![label("Parks on the Air spots need no setup:"), pota_button]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
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
        let fresh = |at: DateTime<Utc>| {
            now.signed_duration_since(at).num_seconds() <= SPOT_MAX_AGE_SECS as i64
        };
        let cluster: Vec<&Spot> = self.spots.iter().filter(|s| fresh(s.spotted_at)).collect();
        // POTA activators. One the cluster also has on the same band is
        // that cluster row with its park added; the rest get rows of
        // their own, with no worked/needed status.
        let activations: Vec<&pota::Activation> =
            self.pota.iter().filter(|a| fresh(a.spot.spotted_at)).collect();
        let pota_only: Vec<Spot> = pota::unmatched(&activations, &cluster);
        let park = |s: &Spot| pota::activation_for(s, &activations).map(|a| a.park_label());
        let mut recent: Vec<(&Spot, SpotInfo)> = cluster
            .iter()
            .map(|s| {
                let info = describe_spot(s, resolver, &self.worked_by_band);
                (*s, SpotInfo { pota: park(s), ..info })
            })
            .chain(pota_only.iter().map(|s| {
                let info = describe_spot(s, resolver, &self.worked_by_band);
                (s, SpotInfo { pota: park(s), status: SpotStatus::Unknown, ..info })
            }))
            .collect();
        sort_spots(&mut recent, self.spots_sort);
        let total = recent.len();
        let needed = recent
            .iter()
            .filter(|(_, info)| info.status == SpotStatus::NeededBand)
            .count();

        let mut summary = format!("{total} spots  ·  {needed} needed");
        if self.pota_on {
            let parks = recent.iter().filter(|(_, info)| info.pota.is_some()).count();
            summary.push_str(&format!("  ·  {parks} POTA"));
        }
        if let Some(s) = &self.spots_status {
            summary.push_str(&format!("  ·  {s}"));
        }
        // Whether cluster spots are being filtered, and by which file:
        // an unfiltered feed is otherwise indistinguishable from a
        // filter that isn't doing what was meant.
        if self.spots_active {
            let filter = self
                .config
                .as_ref()
                .and_then(|c| c.dxcluster.filter_file.as_ref())
                .and_then(|p| p.file_name())
                .map(|name| name.to_string_lossy().into_owned());
            summary.push_str(&match filter {
                Some(name) => format!("  ·  filter: {name}"),
                None => "  ·  no filter".to_string(),
            });
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
                theme::push_button
            }),
            pota_button,
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        // Each header is a button: click to sort ascending, again for
        // descending, a third time to return to the default order.
        let sort = self.spots_sort;
        let head = |name: &'static str, column: SpotColumn, width: Length, right: bool| {
            let arrow = match sort {
                Some(s) if s.column == column && s.ascending => "▲",
                Some(s) if s.column == column => "▼",
                _ => "",
            };
            let caption = text(format!("{name}{arrow}"))
                .size(ROW_TEXT)
                .font(MONO)
                .wrapping(text::Wrapping::None)
                .width(Length::Fill)
                .align_x(if right {
                    text::Alignment::Right
                } else {
                    text::Alignment::Left
                });
            let active = arrow != "";
            button(caption)
                .padding(0)
                .width(width)
                .on_press(Message::SpotsSortClicked(column))
                .style(move |theme: &Theme, status| {
                    let lit = active
                        || matches!(status, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: None,
                        text_color: Color {
                            a: if lit { 1.0 } else { 0.55 },
                            ..theme.palette().text
                        },
                        ..button::Style::default()
                    }
                })
        };
        let fixed = Length::Fixed;
        let mut header = row![head("ST", SpotColumn::Status, fixed(W_STATUS), false)];
        // The POTA marker column exists only while POTA spots are on.
        if self.pota_on {
            header = header.push(head("POTA", SpotColumn::Pota, fixed(W_POTA), false));
        }
        let header = table_header(
            header
                .push(head("AGE", SpotColumn::Age, fixed(W_AGE), true))
                .push(head("CALL", SpotColumn::Call, fixed(W_CALL), false))
                .push(head("kHz", SpotColumn::Freq, fixed(W_FREQ), true))
                .push(head("BAND", SpotColumn::Band, fixed(W_BAND), false))
                .push(head("MODE", SpotColumn::Mode, fixed(W_MODE), false))
                .push(head("ENTITY", SpotColumn::Entity, fixed(W_ENTITY), false))
                .push(head("COMMENT", SpotColumn::Comment, Length::Fill, false)),
        );

        let mut rows = column![].spacing(0);
        for (index, (spot, info)) in recent
            .into_iter()
            .filter(|(_, info)| !self.spots_needed_only || info.status == SpotStatus::NeededBand)
            .enumerate()
        {
            let needed = info.status == SpotStatus::NeededBand;
            let worked = info.status == SpotStatus::Worked;
            // Needed spots pop; already-worked ones recede, losing their
            // column colours along with their brightness.
            let tint = move |live: fn(&Theme) -> text::Style| {
                move |theme: &Theme| if worked { muted(theme) } else { live(theme) }
            };
            let tone = tint(|_| text::Style::default());
            let row_carries_status = theme::hues(&self.theme).status == theme::StatusLook::Row;
            let status: Element<'_, Message> = if needed && row_carries_status {
                cell(status_label(info.status), W_STATUS).font(theme::MONO_BOLD).into()
            } else if needed {
                filled_cell(status_label(info.status), W_STATUS, Tone::Good)
            } else {
                cell(status_label(info.status), W_STATUS).style(muted).into()
            };
            // A POTA activator's park goes ahead of whatever the spot
            // itself said. Owned: a POTA-only row is built here, not
            // held in the app.
            let said = spot.comment.clone().unwrap_or_default();
            let comment = cell_fill(match &info.pota {
                Some(park) if said.is_empty() => park.clone(),
                Some(park) => format!("{park}  ·  {said}"),
                None => said,
            });
            let mut cells = row![status];
            if self.pota_on {
                cells = cells.push(match &info.pota {
                    Some(_) => theme::tag_cell("POTA", W_POTA),
                    None => cell("", W_POTA).into(),
                });
            }
            let cells = cells
                .push(
                    cell_right(age_label(now.signed_duration_since(spot.spotted_at)), W_AGE)
                        .style(muted),
                )
                .push(
                    cell(spot.call.clone(), W_CALL)
                        .font(theme::MONO_BOLD)
                        .style(tint(theme::call)),
                )
                .push(cell_right(khz(spot.freq_hz as i64), W_FREQ).style(tint(theme::freq)))
                .push(
                    cell(info.band.map(|b| b.as_adif()).unwrap_or(""), W_BAND)
                        .style(tint(theme::band)),
                )
                .push(cell(spot.mode.clone().unwrap_or_default(), W_MODE).style(tint(theme::mode)))
                .push(cell(info.country.clone().unwrap_or_default(), W_ENTITY).style(tone))
                .push(comment);
            let is_target = target_dxcc_id.is_some() && info.dxcc_id == target_dxcc_id;
            rows = rows.push(theme::table_row_at(
                &self.theme,
                cells,
                Message::SpotClicked(spot.clone()),
                is_target,
                index,
                needed.then_some(Tone::Good),
            ));
        }

        column![
            toolbar,
            theme::grid(
                column![header, rule::horizontal(1), scrollable(rows).height(Length::Fill)]
                    .spacing(4)
            ),
        ]
        .spacing(4)
        .into()
    }

    // ---- Recent QSOs ----

    pub(in crate::app) fn recent_pane(&self) -> Element<'_, Message> {
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
        for (index, q) in self.recent.iter().enumerate() {
            let cells = row![
                cell(q.qso_begin.format("%Y-%m-%d").to_string(), W_DATE).style(muted),
                cell(q.qso_begin.format("%H:%M").to_string(), W_TIME),
                cell(q.call.as_str(), W_CALL)
                    .font(theme::MONO_BOLD)
                    .style(theme::call),
                cell_right(q.freq_hz.map(khz).unwrap_or_default(), W_FREQ).style(theme::freq),
                cell(q.band.map(|b| b.as_adif()).unwrap_or(""), W_BAND).style(theme::band),
                cell(
                    q.mode.as_ref().map(|m| m.as_adif().to_string()).unwrap_or_default(),
                    W_MODE
                )
                .style(theme::mode),
                cell(q.dxcc_prefix.as_deref().unwrap_or(""), W_PREFIX),
                cell(q.continent.as_deref().unwrap_or(""), W_CONT),
                cell_fill(q.cq_zone.map(|z| z.to_string()).unwrap_or_default()),
            ];
            rows = rows.push(theme::table_row_at(
                &self.theme,
                cells,
                Message::QsoSelected(q.id),
                self.editing_qso == Some(q.id),
                index,
                None,
            ));
        }
        theme::grid(
            column![header, rule::horizontal(1), scrollable(rows).height(Length::Fill)].spacing(4),
        )
    }

    // ---- Station ----

    pub(in crate::app) fn station_pane(&self) -> Element<'_, Message> {
        let palette = self.theme.palette();
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
            // On the active radio's block everything is one colour,
            // chosen to be read on it.
            let on_active = (active && self.rigs.len() > 1)
                .then(|| theme::hues(&self.theme).rig_text);
            let ink = move |fallback: fn(&iced::Theme) -> text::Style| {
                move |theme: &iced::Theme| match on_active {
                    Some(color) => text::Style { color: Some(color) },
                    None => fallback(theme),
                }
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
                ))
                .style(ink(muted)),
                row![
                    text(freq).font(theme::MONO_BOLD).size(22).style(ink(theme::freq)),
                    text(detail).size(ROW_TEXT).style(ink(muted)),
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
                        .style(theme::rig_style(active)),
                )
            } else {
                Element::from(block)
            });
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
        if self.so2r_active {
            // Headphone routing: one radio per ear, or the active radio
            // in both.
            let current = self.so2r_rx_mode();
            let choice = |name: &'static str, mode: &'static str| {
                button(text(name).size(ROW_TEXT))
                    .padding([2, 10])
                    .on_press(Message::So2rRxModeSelected(mode))
                    .style(theme::tab_style(current == mode))
            };
            col = col.push(
                row![
                    Space::new().width(Length::Fixed(69.0)),
                    label("AUDIO"),
                    choice("Stereo", "stereo"),
                    choice("Mono", "mono"),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            );
        }

        let (wsjtx_color, wsjtx_text) = match (self.wsjtx_active, self.wsjtx_bind_addr.as_deref()) {
            (true, Some(addr)) => (
                palette.primary,
                format!("{addr}  ·  {} imported", self.wsjtx_imported),
            ),
            _ => (off, "off".to_string()),
        };
        col = col.push(status_line(wsjtx_color, "WSJT-X", wsjtx_text));

        scrollable(col.width(Length::Fill)).into()
    }

    // ---- CW keyer ----

    /// Free-text CW box, speed / abort controls, and the configurable
    /// message buttons (`[[keyer.macro]]`, bound to F1..F12 in order).
    /// Shown even without a keyer so the operator can see what's set up;
    /// the controls are simply inert until one connects.
    pub(in crate::app) fn keyer_pane(&self) -> Element<'_, Message> {
        let palette = self.theme.palette();
        let connected = self.keyer_active && self.keyer_handle.is_some();
        let keying = self.keyer_snapshot.as_ref().is_some_and(|s| s.keying);
        let wpm = self.keyer_snapshot.as_ref().map(|s| s.wpm);

        // Status + speed + abort. The dot goes green while CW is going
        // out — the most reliable TX signal slogger has today.
        let small = |caption: &'static str, msg: Option<Message>| {
            button(text(caption).size(12))
                .padding([2, 8])
                .on_press_maybe(msg)
                .style(theme::push_button)
        };
        let status: Element<'_, Message> = if connected {
            let dot = if keying { palette.success } else { palette.primary };
            row![
                text("●").size(11).color(dot),
                text(if keying { "Sending" } else { "Ready" }).size(ROW_TEXT),
                Space::new().width(Length::Fill),
                small("−", wpm.map(|w| Message::KeyerSetWpm(w.saturating_sub(1)))),
                text(wpm.map(|w| format!("{w} WPM")).unwrap_or_else(|| "-- WPM".into()))
                    .font(MONO)
                    .size(ROW_TEXT),
                small("+", wpm.map(|w| Message::KeyerSetWpm(w.saturating_add(1)))),
                button(text("Abort").size(12))
                    .padding([2, 8])
                    .on_press_maybe(keying.then_some(Message::KeyerAbortPressed))
                    .style(button::danger),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into()
        } else {
            row![
                text("●").size(11).color(Color { a: 0.3, ..palette.text }),
                text(
                    self.keyer_status
                        .clone()
                        .unwrap_or_else(|| "Keyer not configured — see [keyer] in config.toml".into())
                )
                .size(ROW_TEXT)
                .style(muted),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into()
        };

        let send_box = text_input("Type CW and press Enter", &self.keyer_text)
            .id(Id::new(KEYER_TEXT_ID))
            .on_input(Message::KeyerTextChanged)
            .on_submit(Message::KeyerTextSubmitted)
            .font(MONO)
            .padding([5, 8]);

        // One button per message. A message whose placeholders can't be
        // filled yet (e.g. {CALL} with an empty entry form) is disabled.
        let mut buttons = row![].spacing(4);
        for (i, m) in self.keyer_macros().into_iter().enumerate() {
            let caption = if i < 12 {
                format!("F{} {}", i + 1, m.label)
            } else {
                m.label.clone()
            };
            let on_press = self
                .expand_macro(&m)
                .ok()
                .filter(|_| connected)
                .map(Message::KeyerSendMacro);
            buttons = buttons.push(
                button(text(caption).size(12))
                    .padding([3, 8])
                    .on_press_maybe(on_press)
                    .style(theme::push_button),
            );
        }

        scrollable(column![status, send_box, buttons.wrap()].spacing(8)).into()
    }
}

impl App {
    // ---- CondX ----

    /// Solar and propagation conditions from hamqsl.com: six headline
    /// figures in a 3×2 grid, N0NBH's day / night rating for each band
    /// group, and when hamqsl last updated them.
    pub(in crate::app) fn condx_pane(&self) -> Element<'_, Message> {
        if !self.condx_active {
            return column![
                text("CondX not enabled").size(14),
                label("Set [condx] enabled = true in config.toml to see solar and band conditions here."),
            ]
            .spacing(4)
            .into();
        }
        let Some(snap) = &self.condx else {
            return label("Waiting for the first update…").into();
        };

        let stats = column![
            row![
                condx_stat("SFI", number_or_dash(snap.sfi)),
                condx_stat("SUNSPOTS", number_or_dash(snap.sunspots)),
                condx_stat(
                    "A / K",
                    format!("{} / {}", number_or_dash(snap.a_index), number_or_dash(snap.k_index)),
                ),
            ]
            .spacing(8),
            row![
                condx_stat("X-RAY", text_or_dash(&snap.xray)),
                condx_stat("GEOMAG", text_or_dash(&snap.geomag_field)),
                condx_stat("S/N", text_or_dash(&snap.signal_noise)),
            ]
            .spacing(8),
        ]
        .spacing(8);

        let mut col = column![stats].spacing(10);

        let bands = group_by_band(&snap.conditions);
        if !bands.is_empty() {
            let mut table = column![
                table_header(row![
                    cell("BAND", W_BAND_GROUP),
                    cell("DAY", W_RATING),
                    cell("NIGHT", W_RATING),
                ]),
                rule::horizontal(1),
            ];
            for b in bands {
                table = table.push(table_line(row![
                    cell(b.band, W_BAND_GROUP),
                    rating_cell(b.day),
                    rating_cell(b.night),
                ]));
            }
            col = col.push(theme::grid(table));
        }

        if let Some(updated) = updated_label(&snap.updated) {
            col = col.push(label(updated));
        }

        scrollable(col).into()
    }
}

/// One of the CondX headline figures: caption above value, sharing its
/// row equally with its neighbours.
fn condx_stat<'a>(caption: &'static str, value: String) -> Element<'a, Message> {
    column![label(caption), text(value).font(MONO).size(15)]
        .spacing(1)
        .width(Length::FillPortion(1))
        .into()
}

/// A band rating as a filled cell: good, fair or poor each in its status
/// colour; anything else (or missing) is muted text.
fn rating_cell<'a>(rating: Option<String>) -> Element<'a, Message> {
    let tone = match rating.as_deref().and_then(rating_tone) {
        Some(RatingTone::Good) => Tone::Good,
        Some(RatingTone::Fair) => Tone::Warn,
        Some(RatingTone::Poor) => Tone::Bad,
        None => return cell(rating.unwrap_or_else(|| "—".into()), W_RATING).style(muted).into(),
    };
    filled_cell(rating.unwrap_or_default(), W_RATING, tone)
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
