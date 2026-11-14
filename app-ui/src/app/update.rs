use std::time::Duration;

use app_config::{KeyerMacro, MacroContext};
use chrono::Utc;
use iced::Task;
use iced::widget::pane_grid;
use logbook_domain::{CreateQsoCommand, QsoSearch};
use radio_core::{Band, Callsign, Mode, QsoId, StationLocation, StationLocationId};
use rig_control::{RigHandle, RigSnapshot};
use spot_feed::{Spot, SpotEvent};
use wsjtx_bridge::WsjtxMessage;

use super::boot::{create_qso, import_adif_file, insert_location, refresh};
use super::constants::SPOT_HISTORY_LIMIT;
use super::helpers::{
    default_freq_for_band, option_from_str, parse_mhz_to_hz, station_call_from_config,
};
use super::focus::focus_call;
use super::layout::{SavedUi, save_layout};
use super::theme;
use super::message::{DupeMatch, LogbookSearchResult, Message};
use super::panels::Panels;
use super::services::{format_multi_summary, run_services_update};
use super::spots::build_worked_by_band;
use super::state::{App, EntryDraft};

impl App {
    pub fn update(&mut self, message: Message) -> Task<Message> {
        // A modal owns the keyboard: the UI underneath must not send CW,
        // move focus or switch tabs while it is covered. (Mouse clicks
        // are already stopped by the modal's backdrop.)
        if self.modal.is_some()
            && matches!(
                message,
                Message::FunctionKey(_)
                    | Message::FocusNext
                    | Message::FocusPrevious
                    | Message::ViewChanged(_)
            )
        {
            return Task::none();
        }
        match message {
            Message::Booted(Ok(bundle)) => {
                self.service = Some(bundle.service.clone());
                self.repo = Some(bundle.repo.clone());
                self.station_repo = Some(bundle.station_repo.clone());
                self.resolver = Some(bundle.resolver);
                self.station_locations = bundle.station_locations;
                self.active_location = bundle.active_location;
                self.spots_active = bundle.spots_active;
                self.wsjtx_active = bundle.wsjtx_active;
                self.wsjtx_bind_addr = bundle.wsjtx_bind_addr;
                self.rigs = bundle.rigs;
                self.active_rig = 0;
                self.keyer_active = bundle.keyer_active;
                self.keyer_status = bundle.keyer_status;
                self.keyer_handle = bundle.keyer_handle;
                self.so2r_active = bundle.so2r_active;
                self.so2r_status = bundle.so2r_status;
                self.so2r_handle = bundle.so2r_handle;
                self.config = Some(bundle.config);
                if let Some(saved) = bundle.panel_layout {
                    self.panels = Panels::from_saved(saved.panels);
                    if let Some(scale) = saved.font_scale {
                        self.font_scale = scale.clamp(theme::FONT_SCALE_MIN, theme::FONT_SCALE_MAX);
                    }
                    if let Some(t) = saved.theme.as_deref().and_then(theme::theme_by_name) {
                        self.theme = t;
                    }
                }
                // Land with the cursor in the Call field: the operator can
                // start typing the moment the window appears.
                Task::batch([
                    Task::perform(refresh(bundle.repo, bundle.service), Message::Refreshed),
                    focus_call(),
                ])
            }
            Message::Booted(Err(e)) => {
                self.boot_error = Some(e);
                Task::none()
            }
            Message::CallChanged(s) => {
                // Callsigns are always upper-case; normalising here means
                // the operator never has to think about Caps Lock.
                self.call_input = s.to_ascii_uppercase();
                self.schedule_dupe_check()
            }
            Message::BandChanged(b) => {
                self.band = Some(b);
                if let Some(hz) = default_freq_for_band(b) {
                    self.freq_input = format!("{:.4}", hz as f64 / 1_000_000.0);
                }
                self.schedule_dupe_check()
            }
            Message::ModeChanged(m) => {
                self.mode = Some(m);
                Task::none()
            }
            Message::FreqChanged(s) => {
                self.freq_input = s;
                Task::none()
            }
            Message::RstSentChanged(s) => {
                self.rst_sent = s;
                Task::none()
            }
            Message::RstRcvdChanged(s) => {
                self.rst_rcvd = s;
                Task::none()
            }
            Message::LogPressed => self.submit_qso(),
            Message::QsoSelected(id) => {
                let Some(svc) = self.service.clone() else {
                    return Task::none();
                };
                if self.editing_qso.is_none() {
                    self.entry_draft = Some(EntryDraft {
                        call_input: self.call_input.clone(),
                        band: self.band,
                        mode: self.mode.clone(),
                        freq_input: self.freq_input.clone(),
                        rst_sent: self.rst_sent.clone(),
                        rst_rcvd: self.rst_rcvd.clone(),
                    });
                }
                self.editing_qso = Some(id);
                self.qso_detail = None;
                Task::perform(
                    async move {
                        match svc.get_qso_detail(&id).await {
                            Ok(Some(q)) => Ok(q),
                            Ok(None) => Err("qso not found".into()),
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::QsoLoadedForEdit,
                )
            }
            Message::QsoLoadedForEdit(Ok(detail)) => {
                let q = detail.qso.clone();
                self.qso_detail = Some(detail);
                self.call_input = q.call.as_str().to_string();
                self.band = q.band;
                self.mode = q.mode;
                self.freq_input = q
                    .freq_hz
                    .map(|hz| format!("{:.5}", hz as f64 / 1_000_000.0))
                    .unwrap_or_default();
                self.rst_sent = q.rst_sent.unwrap_or_default();
                self.rst_rcvd = q.rst_rcvd.unwrap_or_default();
                Task::none()
            }
            Message::QsoLoadedForEdit(Err(e)) => {
                self.status = Some(format!("load qso error: {e}"));
                self.end_edit()
            }
            Message::CancelEditPressed => Task::batch([self.end_edit(), focus_call()]),
            Message::DeletePressed => {
                let Some(svc) = self.service.clone() else {
                    return Task::none();
                };
                let Some(id) = self.editing_qso else {
                    return Task::none();
                };
                Task::perform(
                    async move { svc.delete_qso(&id).await.map_err(|e| e.to_string()) },
                    Message::QsoDeleted,
                )
            }
            Message::QsoDeleted(Ok(())) => {
                self.status = Some("qso deleted".into());
                Task::batch([self.end_edit(), self.refresh_task()])
            }
            Message::QsoDeleted(Err(e)) => {
                self.status = Some(format!("delete error: {e}"));
                Task::none()
            }
            Message::QsoUpdated(Ok(())) => {
                self.status = Some("qso updated".into());
                Task::batch([self.end_edit(), self.refresh_task(), focus_call()])
            }
            Message::QsoUpdated(Err(e)) => {
                self.status = Some(format!("update error: {e}"));
                Task::none()
            }
            Message::WsjtxMessage(WsjtxMessage::LoggedAdif { id, adif }) => {
                let Some(svc) = self.service.clone() else {
                    return Task::none();
                };
                tracing::info!(instance = %id, "wsjtx logged a qso; importing");
                Task::perform(
                    async move {
                        let outcome = match logbook_domain::parse_adif(&adif) {
                            Ok(o) => o,
                            Err(e) => return Err(e.to_string()),
                        };
                        let report = svc.import_qsos(outcome.commands).await;
                        Ok(report.created)
                    },
                    Message::WsjtxImportFinished,
                )
            }
            Message::WsjtxMessage(_) => Task::none(),
            Message::WsjtxImportFinished(Ok(count)) => {
                self.wsjtx_imported += count;
                self.status = Some(format!(
                    "wsjtx: imported {count} qso (total this run: {})",
                    self.wsjtx_imported
                ));
                self.refresh_task()
            }
            Message::WsjtxImportFinished(Err(e)) => {
                self.status = Some(format!("wsjtx import error: {e}"));
                Task::none()
            }
            Message::RigSnapshot(tagged) => {
                let Some(entry) = self.rigs.get_mut(tagged.rig_index) else {
                    return Task::none();
                };
                let previous = entry.snapshot.replace(tagged.snapshot);
                if tagged.rig_index == self.active_rig {
                    self.follow_rig(previous.as_ref())
                } else {
                    Task::none()
                }
            }
            Message::ActiveRigChanged(idx) => {
                if idx >= self.rigs.len() {
                    return Task::none();
                }
                self.active_rig = idx;
                // The form now describes a different radio: resync it fully.
                let follow = self.follow_rig(None);
                // Active-radio abstraction: when SO2R is configured, the
                // same click that switches the entry-form / rig-command
                // target also moves the OTRSP TX line. Otherwise an
                // operator who clicked "R2" in the Station pane would
                // tune Radio 2 but transmit on Radio 1 — silent footgun.
                // The headphones follow too, so in mono the operator hears
                // the radio they just switched to.
                if let Some(handle) = self.so2r_handle.clone() {
                    let radio = (idx as u8) + 1;
                    let mode = self.so2r_rx_mode();
                    let so2r = Task::perform(
                        async move {
                            handle
                                .set_tx_radio(radio)
                                .await
                                .map_err(|e| format!("SO2R set_tx error: {e}"))?;
                            handle
                                .set_rx_audio(radio, mode)
                                .await
                                .map_err(|e| format!("SO2R set_rx error: {e}"))?;
                            Ok(format!("SO2R TX/RX → R{radio}"))
                        },
                        Message::SendToRigFinished,
                    );
                    return Task::batch([follow, so2r]);
                }
                follow
            }
            Message::So2rRxModeSelected(mode) => {
                let Some(handle) = self.so2r_handle.clone() else {
                    return Task::none();
                };
                let radio = (self.active_rig as u8) + 1;
                Task::perform(
                    async move {
                        handle
                            .set_rx_audio(radio, mode)
                            .await
                            .map(|()| format!("SO2R RX {mode}"))
                            .map_err(|e| format!("SO2R set_rx error: {e}"))
                    },
                    Message::SendToRigFinished,
                )
            }
            Message::UseRigPressed => self.use_rig_now(),
            Message::SendToRigPressed => self.send_to_rig(),
            Message::SendToRigFinished(Ok(note)) => {
                self.status = Some(format!("rig: {note}"));
                Task::none()
            }
            Message::SendToRigFinished(Err(e)) => {
                self.status = Some(format!("rig set error: {e}"));
                Task::none()
            }
            Message::KeyerSnapshot(snap) => {
                self.keyer_snapshot = Some(snap);
                Task::none()
            }
            Message::So2rSnapshotMsg(snap) => {
                self.so2r_snapshot = Some(snap);
                Task::none()
            }
            Message::QsoCreated(Ok(_id)) => {
                self.status = Some(format!(
                    "logged {}",
                    self.call_input.trim().to_ascii_uppercase()
                ));
                self.call_input.clear();
                self.dupe_match = None;
                // Refocus the Call input so the operator can immediately
                // type the next callsign without reaching for the mouse.
                Task::batch([self.refresh_task(), focus_call()])
            }
            Message::QsoCreated(Err(e)) => {
                self.status = Some(format!("log error: {e}"));
                Task::none()
            }
            Message::Refreshed(Ok(snap)) => {
                self.recent = snap.recent;
                self.pending_lotw = snap.pending_lotw;
                self.pending_eqsl = snap.pending_eqsl;
                self.pending_clublog = snap.pending_clublog;
                self.pending_qrz = snap.pending_qrz;
                self.pending_hrdlog = snap.pending_hrdlog;
                self.worked_by_band = build_worked_by_band(&snap.awards);
                self.awards = snap.awards;
                Task::none()
            }
            Message::Refreshed(Err(e)) => {
                self.status = Some(format!("refresh error: {e}"));
                Task::none()
            }
            Message::AdifPathChanged(s) => {
                self.adif_path = s;
                Task::none()
            }
            Message::ImportPressed => {
                let (Some(svc), Some(_)) = (self.service.clone(), self.repo.clone()) else {
                    return Task::none();
                };
                let path = self.adif_path.trim().to_string();
                if path.is_empty() {
                    self.status = Some("ADIF path is empty".into());
                    return Task::none();
                }
                self.importing = true;
                self.status = Some(format!("importing {path}…"));
                Task::perform(import_adif_file(svc, path), Message::ImportFinished)
            }
            Message::ImportFinished(Ok(summary)) => {
                self.importing = false;
                let mut msg = format!(
                    "import: created {} / skipped {} / parse-errors {}",
                    summary.created, summary.skipped, summary.parse_errors
                );
                if let Some(first) = summary.first_errors.first() {
                    msg.push_str(&format!(" — first: {first}"));
                }
                self.status = Some(msg);
                self.refresh_task()
            }
            Message::ImportFinished(Err(e)) => {
                self.importing = false;
                self.status = Some(format!("import error: {e}"));
                Task::none()
            }
            Message::ServicesUpdatePressed => self.start_services_update(),
            Message::ServicesUpdateFinished(summary) => {
                self.syncing = false;
                self.status = Some(format_multi_summary(&summary));
                // Persist the summary so the QSL view's Service status
                // pane can render per-service detail (last error,
                // counts, etc.) without losing it the next time
                // self.status is overwritten.
                self.last_services_update = Some(summary);
                self.refresh_task()
            }
            Message::StationLocationSelected(opt) => self.select_station_location(opt.id),
            Message::NewLocationNameChanged(s) => {
                self.new_location_name = s;
                Task::none()
            }
            Message::NewLocationCallsignChanged(s) => {
                self.new_location_call = s;
                Task::none()
            }
            Message::NewLocationGridChanged(s) => {
                self.new_location_grid = s;
                Task::none()
            }
            Message::CreateLocationPressed => self.create_station_location(),
            Message::LocationCreated(Ok(loc)) => {
                self.creating_location = false;
                self.new_location_name.clear();
                self.new_location_call.clear();
                self.new_location_grid.clear();
                self.station_locations.push(loc.clone());
                self.station_locations
                    .sort_by(|a, b| a.name.cmp(&b.name));
                self.status = Some(format!("created station '{}'", loc.name));
                self.select_station_location(loc.id)
            }
            Message::LocationCreated(Err(e)) => {
                self.creating_location = false;
                self.status = Some(format!("create station error: {e}"));
                Task::none()
            }
            Message::SpotEvent(SpotEvent::Spot(spot)) => {
                self.spots.push_front(spot);
                while self.spots.len() > SPOT_HISTORY_LIMIT {
                    self.spots.pop_back();
                }
                Task::none()
            }
            Message::SpotEvent(SpotEvent::Withdrawn { call, freq_hz }) => {
                self.spots
                    .retain(|s| !(s.call == call && s.freq_hz == freq_hz));
                Task::none()
            }
            Message::SpotEvent(SpotEvent::SourceStatus { source_id, status }) => {
                self.spots_status = Some(format!("{source_id}: {status}"));
                Task::none()
            }
            Message::SpotEvent(SpotEvent::Error { message }) => {
                tracing::warn!(error = %message, "spot feed error");
                Task::none()
            }
            Message::SpotClicked(spot) => self.apply_spot(spot),
            Message::ToggleSpotsNeededOnly => {
                self.spots_needed_only = !self.spots_needed_only;
                Task::none()
            }
            Message::PaneClicked(_pane) => {
                // Future: track focused pane for keyboard navigation.
                Task::none()
            }
            // pane_grid events come only from the QSL and Logbook views;
            // Operating uses free-floating panels instead.
            Message::PaneDragged(pane_grid::DragEvent::Dropped { pane, target }) => {
                match self.current_view {
                    super::views::ViewKind::Logbook => self.logbook_panes.drop(pane, target),
                    super::views::ViewKind::Qsl => self.qsl_panes.drop(pane, target),
                    super::views::ViewKind::Operating => {}
                }
                Task::none()
            }
            Message::PaneDragged(_) => Task::none(),
            Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => {
                match self.current_view {
                    super::views::ViewKind::Logbook => self.logbook_panes.resize(split, ratio),
                    super::views::ViewKind::Qsl => self.qsl_panes.resize(split, ratio),
                    super::views::ViewKind::Operating => {}
                }
                Task::none()
            }

            // ---- Operating panels ----
            Message::PanelPressed(kind) => {
                self.panels.raise(kind);
                Task::none()
            }
            Message::PanelDragStarted { kind, mode, canvas } => {
                self.panels.start_drag(kind, mode, canvas);
                Task::none()
            }
            Message::PanelPointerMoved(position) => {
                self.panels.drag_to(position);
                Task::none()
            }
            Message::PanelPointerReleased => {
                if self.panels.end_drag() {
                    self.save_panel_layout()
                } else {
                    Task::none()
                }
            }
            Message::PanelToggled(kind) => {
                self.panels.toggle(kind);
                self.save_panel_layout()
            }
            Message::PanelsReset => {
                self.panels.reset();
                self.save_panel_layout()
            }
            Message::LayoutSaveCompleted => Task::none(),

            // ---- Logbook drawer ----
            Message::LogbookCallPrefixChanged(s) => {
                self.logbook_search.call_prefix = s;
                Task::none()
            }
            Message::LogbookExactCallChanged(s) => {
                self.logbook_search.exact_call = s;
                Task::none()
            }
            Message::LogbookBandChanged(bf) => {
                self.logbook_search.band = bf;
                Task::none()
            }
            Message::LogbookModeChanged(mf) => {
                self.logbook_search.mode = mf;
                Task::none()
            }
            Message::LogbookLotwFilterChanged(lf) => {
                self.logbook_search.lotw_filter = lf;
                Task::none()
            }
            Message::LogbookSearchPressed => self.run_logbook_search(),
            Message::LogbookSearchFinished { generation, result } => {
                if generation != self.logbook_search.generation {
                    return Task::none();
                }
                self.logbook_search.searching = false;
                match result {
                    Ok(LogbookSearchResult { results, total }) => {
                        // Selection is per-id; preserve any selections
                        // that are still present in the new result set.
                        self.logbook_search
                            .selected
                            .retain(|id| results.iter().any(|q| q.id == *id));
                        self.logbook_search.results = results;
                        self.logbook_search.total_count = Some(total);
                    }
                    Err(e) => {
                        self.logbook_search.last_action =
                            Some(format!("search error: {e}"));
                    }
                }
                Task::none()
            }
            Message::LogbookRowToggle(id) => {
                if !self.logbook_search.selected.remove(&id) {
                    self.logbook_search.selected.insert(id);
                }
                Task::none()
            }
            Message::LogbookClearSelection => {
                self.logbook_search.selected.clear();
                Task::none()
            }
            Message::LogbookBulkDelete => self.run_logbook_bulk_delete(),
            Message::LogbookBulkMarkUploaded(service) => {
                self.run_logbook_bulk_mark_uploaded(service)
            }
            Message::AwardsKindChanged(kind) => {
                self.awards_drawer.selected_kind =
                    super::drawers::awards::AwardKindOpt(kind);
                // Clear any active spot-highlight target — it would point
                // at a unit of the old kind, which the spots panel can't
                // sensibly cross-reference against the new view.
                self.awards_drawer.target_unit = None;
                Task::none()
            }
            Message::AwardsBandFilterChanged(bf) => {
                self.awards_drawer.band_filter = bf;
                Task::none()
            }
            Message::AwardsSetTarget(target) => {
                self.awards_drawer.target_unit = Some(target);
                Task::none()
            }
            Message::AwardsClearTarget => {
                self.awards_drawer.target_unit = None;
                Task::none()
            }
            Message::KeyerSendMacro(text) => self.send_cw(text),
            Message::KeyerSendFinished(result) => {
                if let Err(e) = result {
                    self.status = Some(format!("keyer error: {e}"));
                }
                Task::none()
            }
            Message::Tick => Task::none(),
            Message::FontScaleUp => self.set_font_scale(self.font_scale + theme::FONT_SCALE_STEP),
            Message::FontScaleDown => self.set_font_scale(self.font_scale - theme::FONT_SCALE_STEP),
            Message::FontScaleReset => self.set_font_scale(1.0),
            Message::ThemeSelected(t) => {
                self.theme = t;
                self.save_panel_layout()
            }
            Message::OpenModal(modal) => {
                self.modal = Some(modal);
                Task::none()
            }
            Message::CloseModal => {
                self.modal = None;
                Task::none()
            }
            Message::ModalClicked => Task::none(),
            Message::FocusNext => iced::widget::operation::focus_next(),
            Message::FocusPrevious => iced::widget::operation::focus_previous(),
            Message::EscapePressed => self.escape(),
            Message::FunctionKey(n) => {
                let Some(m) = self.keyer_macros().into_iter().nth(n as usize - 1) else {
                    return Task::none();
                };
                match self.expand_macro(&m) {
                    Ok(text) => self.send_cw(text),
                    Err(missing) => {
                        self.status = Some(format!(
                            "F{n} {}: nothing to send for {missing} yet",
                            m.label
                        ));
                        Task::none()
                    }
                }
            }
            Message::KeyerTextChanged(s) => {
                self.keyer_text = s.to_ascii_uppercase();
                Task::none()
            }
            Message::KeyerTextSubmitted => {
                let text = self.keyer_text.trim().to_string();
                if text.is_empty() || self.keyer_handle.is_none() {
                    // Keep the text if there's no keyer to send it to.
                    return self.send_cw(text);
                }
                self.keyer_text.clear();
                self.send_cw(text)
            }
            Message::KeyerAbortPressed => self.abort_cw(),
            Message::KeyerSetWpm(wpm) => {
                let Some(handle) = self.keyer_handle.clone() else {
                    return Task::none();
                };
                let wpm = wpm.clamp(5, 50);
                Task::perform(
                    async move {
                        handle.set_wpm(wpm).await.map(|()| wpm).map_err(|e| e.to_string())
                    },
                    Message::KeyerSetWpmFinished,
                )
            }
            Message::KeyerSetWpmFinished(Ok(wpm)) => {
                // Reflect the new speed immediately; the keyer only reports
                // speed changes made on its own pot.
                if let Some(snap) = self.keyer_snapshot.as_mut() {
                    snap.wpm = wpm;
                }
                Task::none()
            }
            Message::KeyerSetWpmFinished(Err(e)) => {
                self.status = Some(format!("keyer speed error: {e}"));
                Task::none()
            }
            Message::KeyerAbortFinished(result) => {
                self.status = Some(match result {
                    Ok(()) => "CW aborted".into(),
                    Err(e) => format!("keyer abort error: {e}"),
                });
                Task::none()
            }
            Message::ViewChanged(view) => {
                // An edit belongs to the tab it started in: leaving that
                // tab abandons it, so a QSO picked in the Logbook never
                // shows up in the Operating entry form (or the reverse).
                let ended = if view != self.current_view && self.editing_qso.is_some() {
                    self.end_edit()
                } else {
                    Task::none()
                };
                self.current_view = view;
                // Auto-refresh the QSL Pending pane on first switch
                // into QSL so the operator sees fresh data immediately.
                if matches!(view, super::views::ViewKind::Qsl)
                    && self.qsl_view.pending.is_empty()
                    && !self.qsl_view.loading_pending
                {
                    return Task::batch([ended, self.run_qsl_pending_refresh()]);
                }
                ended
            }
            Message::QslServiceFilterChanged(filter) => {
                self.qsl_view.service_filter = filter;
                // Selection refers to QSO IDs from the previous filter;
                // those may not be in the new pending list, so clear.
                self.qsl_view.selected.clear();
                self.run_qsl_pending_refresh()
            }
            Message::QslPendingRefreshPressed => self.run_qsl_pending_refresh(),
            Message::QslPendingRefreshed(result) => {
                self.qsl_view.loading_pending = false;
                match result {
                    Ok(rows) => {
                        // Drop stale selections that aren't in the new list.
                        self.qsl_view
                            .selected
                            .retain(|id| rows.iter().any(|q| q.id == *id));
                        self.qsl_view.pending = rows;
                        self.qsl_view.last_action = None;
                    }
                    Err(e) => {
                        self.qsl_view.last_action = Some(format!("refresh error: {e}"));
                    }
                }
                Task::none()
            }
            Message::QslPendingRowToggle(id) => {
                if !self.qsl_view.selected.remove(&id) {
                    self.qsl_view.selected.insert(id);
                }
                Task::none()
            }
            Message::QslClearSelection => {
                self.qsl_view.selected.clear();
                Task::none()
            }
            Message::QslBulkMarkUploaded(service) => {
                let Some(svc) = self.service.clone() else {
                    return Task::none();
                };
                let ids: Vec<radio_core::QsoId> =
                    self.qsl_view.selected.iter().copied().collect();
                if ids.is_empty() {
                    return Task::none();
                }
                let attempted = ids.len();
                let now = Utc::now();
                Task::perform(
                    async move {
                        let marked = svc
                            .bulk_mark_uploaded(&ids, &service, now)
                            .await;
                        Ok(format!(
                            "marked {marked} / {attempted} as uploaded to {service}"
                        ))
                    },
                    Message::QslBulkFinished,
                )
            }
            Message::QslBulkDelete => {
                let Some(svc) = self.service.clone() else {
                    return Task::none();
                };
                let ids: Vec<radio_core::QsoId> =
                    self.qsl_view.selected.iter().copied().collect();
                if ids.is_empty() {
                    return Task::none();
                }
                let attempted = ids.len();
                Task::perform(
                    async move {
                        let deleted = svc.bulk_soft_delete(&ids).await;
                        Ok(format!("deleted {deleted} / {attempted} QSOs"))
                    },
                    Message::QslBulkFinished,
                )
            }
            Message::QslBulkFinished(result) => {
                self.qsl_view.last_action = Some(match result {
                    Ok(s) => s,
                    Err(e) => format!("bulk error: {e}"),
                });
                self.qsl_view.selected.clear();
                let refresh = self.run_qsl_pending_refresh();
                Task::batch([refresh, self.refresh_task()])
            }
            Message::LogbookBulkFinished { generation, result } => {
                if generation != self.logbook_search.generation {
                    return Task::none();
                }
                let message = match result {
                    Ok(m) => m,
                    Err(e) => format!("bulk error: {e}"),
                };
                self.logbook_search.last_action = Some(message);
                // After a bulk action the result list may be stale (rows
                // deleted, service state changed). Re-run the same search
                // to refresh + clear stale selections.
                self.logbook_search.selected.clear();
                let refresh = self.run_logbook_search();
                Task::batch([refresh, self.refresh_task()])
            }
            Message::DupeCheckFinished { generation, dupe } => {
                // Drop stale results: if the operator kept typing while
                // the debounced query was in flight, dupe_check_gen has
                // moved on and this answer no longer matches the form.
                if generation == self.dupe_check_gen {
                    self.dupe_match = dupe;
                }
                Task::none()
            }
        }
    }

    /// Esc. Stops CW first if the keyer is mid-message (the operator's
    /// reflex when something goes wrong on the air); otherwise, on the
    /// Operating view, cancels an edit or wipes the call and refocuses it.
    fn escape(&mut self) -> Task<Message> {
        let keying = self.keyer_snapshot.as_ref().is_some_and(|s| s.keying);
        if keying && self.keyer_handle.is_some() {
            return self.abort_cw();
        }
        if self.modal.take().is_some() {
            return Task::none();
        }
        if self.current_view != super::views::ViewKind::Operating {
            return Task::none();
        }
        if self.editing_qso.is_some() {
            return self.update(Message::CancelEditPressed);
        }
        self.call_input.clear();
        self.dupe_match = None;
        self.dupe_check_gen = self.dupe_check_gen.wrapping_add(1);
        focus_call()
    }

    /// The operator's own callsign: active station location first, then
    /// the `[station]` config fallback.
    pub(super) fn my_call(&self) -> Option<String> {
        self.active_location
            .as_ref()
            .and_then(|loc| loc.station_callsign.as_ref().map(|c| c.as_str().to_string()))
            .or_else(|| {
                self.config
                    .as_ref()
                    .and_then(|c| c.station.default_callsign.clone())
            })
    }

    /// The CW message buttons: `[[keyer.macro]]` from config, or the
    /// built-in set. Position in this list is the F-key number minus one.
    pub(super) fn keyer_macros(&self) -> Vec<KeyerMacro> {
        match &self.config {
            Some(cfg) => cfg.keyer.macros(),
            None => KeyerMacro::defaults(),
        }
    }

    /// Fills a message's placeholders from the live operating context.
    /// `Err` names the placeholder that has no value yet.
    pub(super) fn expand_macro(&self, m: &KeyerMacro) -> Result<String, &'static str> {
        let my_call = self.my_call();
        m.expand(MacroContext {
            my_call: my_call.as_deref(),
            call: Some(self.call_input.as_str()),
            rst_sent: Some(self.rst_sent.as_str()),
        })
    }

    /// Queue CW on the keyer.
    /// Finishes an edit (saved, deleted, cancelled or abandoned) and hands
    /// the entry form back: whatever it held before the edit is restored,
    /// then it is resynced with the rig, which may have moved meanwhile.
    fn end_edit(&mut self) -> Task<Message> {
        self.editing_qso = None;
        self.qso_detail = None;
        match self.entry_draft.take() {
            Some(draft) => {
                self.call_input = draft.call_input;
                self.band = draft.band;
                self.mode = draft.mode;
                self.freq_input = draft.freq_input;
                self.rst_sent = draft.rst_sent;
                self.rst_rcvd = draft.rst_rcvd;
            }
            None => self.call_input.clear(),
        }
        let follow = self.follow_rig(None);
        Task::batch([follow, self.schedule_dupe_check()])
    }

    /// The SO2R headphone mode as last reported by the switch: `"stereo"`
    /// (one radio per ear) or `"mono"` (the active radio in both ears).
    /// Reverse stereo counts as stereo; unknown counts as mono.
    pub(super) fn so2r_rx_mode(&self) -> &'static str {
        match self.so2r_snapshot.as_ref().map(|s| s.rx_mode.as_str()) {
            Some("stereo" | "reverse_stereo") => "stereo",
            _ => "mono",
        }
    }

    fn send_cw(&mut self, text: String) -> Task<Message> {
        if text.is_empty() {
            return Task::none();
        }
        let Some(handle) = self.keyer_handle.clone() else {
            self.status = Some("keyer not connected".into());
            return Task::none();
        };
        Task::perform(
            async move { handle.send_message(&text).await.map_err(|e| e.to_string()) },
            Message::KeyerSendFinished,
        )
    }

    /// Stop sending immediately and flush the keyer's buffer.
    fn abort_cw(&mut self) -> Task<Message> {
        let Some(handle) = self.keyer_handle.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { handle.abort().await.map_err(|e| e.to_string()) },
            Message::KeyerAbortFinished,
        )
    }

    /// Builds a `QsoSearch` from the logbook drawer filter form and runs
    /// it. Bumps the generation so any in-flight result from a prior
    /// search is dropped.
    fn run_logbook_search(&mut self) -> Task<Message> {
        let Some(svc) = self.service.clone() else {
            return Task::none();
        };
        self.logbook_search.generation =
            self.logbook_search.generation.wrapping_add(1);
        let generation = self.logbook_search.generation;
        let search = self.build_logbook_search();
        self.logbook_search.searching = true;
        Task::perform(
            async move {
                let count = svc.count_matching(search.clone()).await.map_err(|e| e.to_string())?;
                let results = svc.search_qsos(search).await.map_err(|e| e.to_string())?;
                Ok(LogbookSearchResult {
                    results,
                    total: count,
                })
            },
            move |result| Message::LogbookSearchFinished { generation, result },
        )
    }

    /// Bulk soft-delete every currently-selected QSO. No-op if no rows
    /// are selected. Reports the success count via `last_action`.
    fn run_logbook_bulk_delete(&mut self) -> Task<Message> {
        let Some(svc) = self.service.clone() else {
            return Task::none();
        };
        let ids: Vec<QsoId> = self.logbook_search.selected.iter().copied().collect();
        if ids.is_empty() {
            return Task::none();
        }
        let generation = self.logbook_search.generation;
        let attempted = ids.len();
        Task::perform(
            async move {
                let deleted = svc.bulk_soft_delete(&ids).await;
                Ok(format!("deleted {deleted} / {attempted} QSOs"))
            },
            move |result| Message::LogbookBulkFinished { generation, result },
        )
    }

    /// Bulk mark-uploaded against the given service for every selected
    /// QSO. Same shape as the delete path.
    fn run_logbook_bulk_mark_uploaded(&mut self, service: String) -> Task<Message> {
        let Some(svc) = self.service.clone() else {
            return Task::none();
        };
        let ids: Vec<QsoId> = self.logbook_search.selected.iter().copied().collect();
        if ids.is_empty() {
            return Task::none();
        }
        let generation = self.logbook_search.generation;
        let attempted = ids.len();
        let now = Utc::now();
        Task::perform(
            async move {
                let marked = svc
                    .bulk_mark_uploaded(&ids, &service, now)
                    .await;
                Ok(format!(
                    "marked {marked} / {attempted} as uploaded to {service}"
                ))
            },
            move |result| Message::LogbookBulkFinished { generation, result },
        )
    }

    /// Converts the drawer's filter form into a `QsoSearch`. Empty text
    /// fields become `None` filters; the result-row cap is set high
    /// enough for typical inspection workflows but bounded so we never
    /// load 100k rows into memory.
    fn build_logbook_search(&self) -> QsoSearch {
        let state = &self.logbook_search;
        let call_prefix = if state.call_prefix.trim().is_empty() {
            None
        } else {
            Some(state.call_prefix.trim().to_string())
        };
        let exact_call = Callsign::parse(state.exact_call.trim()).ok();
        QsoSearch {
            call_prefix,
            exact_call,
            band: state.band.0,
            mode: state.mode.0.clone(),
            lotw_confirmed: state.lotw_filter.as_search_filter(),
            limit: Some(500),
            ..Default::default()
        }
    }

    /// Refreshes the Pending QSOs list in the QSL view from
    /// `repo.list_pending_uploads`. Called when the operator presses
    /// Refresh, switches into QSL, or after a bulk action. Filter
    /// `None` falls back to LotW since that's the most common workflow
    /// (filtering across all services would require a separate query).
    fn run_qsl_pending_refresh(&mut self) -> Task<Message> {
        let Some(repo) = self.repo.clone() else {
            return Task::none();
        };
        let Some(svc) = self.qsl_view.service_filter.0 else {
            // "Any" — clear the list; per-service is the operator's
            // explicit choice and the union view is a future refinement.
            self.qsl_view.pending = Vec::new();
            return Task::none();
        };
        let service_key = svc.key().to_string();
        self.qsl_view.loading_pending = true;
        Task::perform(
            async move {
                repo.list_pending_uploads(&service_key, Some(500))
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::QslPendingRefreshed,
        )
    }

    /// Persists the current panel arrangement.
    /// Writes the panel layout, theme and text size to `ui-layout.json`.
    fn save_panel_layout(&self) -> Task<Message> {
        let ui = SavedUi {
            panels: self.panels.to_saved(),
            font_scale: Some(self.font_scale),
            theme: Some(self.theme.to_string()),
        };
        Task::perform(save_layout(ui), |()| Message::LayoutSaveCompleted)
    }

    fn set_font_scale(&mut self, scale: f32) -> Task<Message> {
        // Round to the step so repeated presses don't drift (0.1 is not
        // exact in binary).
        let scale = (scale * 10.0).round() / 10.0;
        self.font_scale = scale.clamp(theme::FONT_SCALE_MIN, theme::FONT_SCALE_MAX);
        self.save_panel_layout()
    }

    /// Schedules a debounced dupe-check for the current (call, band).
    /// Called from `CallChanged` and `BandChanged`. Clears any stale match
    /// immediately so the UI doesn't show "DUP B4" against an outdated key
    /// while the new check is in flight.
    fn schedule_dupe_check(&mut self) -> Task<Message> {
        self.dupe_match = None;
        self.dupe_check_gen = self.dupe_check_gen.wrapping_add(1);
        let generation = self.dupe_check_gen;
        let Some(svc) = self.service.clone() else {
            return Task::none();
        };
        let Ok(call) = Callsign::parse(self.call_input.trim()) else {
            // Operator hasn't typed a valid callsign yet; nothing to check.
            return Task::none();
        };
        let band = self.band;
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let qsos = svc
                    .search_qsos(QsoSearch {
                        exact_call: Some(call),
                        band,
                        limit: Some(1),
                        ..Default::default()
                    })
                    .await
                    .ok();
                let dupe = qsos.and_then(|q| q.into_iter().next()).map(|q| DupeMatch {
                    qso_begin: q.qso_begin,
                    band: q.band,
                });
                (generation, dupe)
            },
            |(generation, dupe)| Message::DupeCheckFinished { generation, dupe },
        )
    }

    pub(super) fn refresh_task(&self) -> Task<Message> {
        match (self.repo.clone(), self.service.clone()) {
            (Some(repo), Some(svc)) => Task::perform(refresh(repo, svc), Message::Refreshed),
            _ => Task::none(),
        }
    }

    fn submit_qso(&mut self) -> Task<Message> {
        let Some(svc) = self.service.clone() else {
            return Task::none();
        };
        let call = match Callsign::parse(&self.call_input) {
            Ok(c) => c,
            Err(e) => {
                self.status = Some(format!("invalid callsign: {e}"));
                return Task::none();
            }
        };
        // Rig fallback: if the operator didn't type a freq/mode, take the
        // active rig's snapshot. Operator-typed values still win.
        let rig_freq_hz: Option<i64> = self
            .active_rig_snapshot()
            .and_then(|s| s.freq_hz)
            .map(|v| v as i64);
        let rig_mode: Option<Mode> = self
            .active_rig_snapshot()
            .and_then(|s| s.mode.as_deref())
            .map(Mode::from_adif);
        let freq_hz = parse_mhz_to_hz(&self.freq_input).or(rig_freq_hz);
        let band = self
            .band
            .or_else(|| freq_hz.and_then(Band::from_freq_hz));
        let station_callsign = self
            .active_location
            .as_ref()
            .and_then(|loc| loc.station_callsign.clone())
            .or_else(|| {
                self.config
                    .as_ref()
                    .and_then(|c| station_call_from_config(&c.station))
            });
        let mode = self.mode.clone().or(rig_mode);
        let cmd = CreateQsoCommand {
            band,
            freq_hz,
            mode,
            rst_sent: option_from_str(&self.rst_sent),
            rst_rcvd: option_from_str(&self.rst_rcvd),
            station_callsign,
            station_location_id: self.active_location.as_ref().map(|l| l.id),
            ..CreateQsoCommand::minimal(call, Utc::now())
        };

        // Editing path: route through update_qso, preserving the original
        // qso_begin so we don't accidentally re-time the contact.
        if let Some(id) = self.editing_qso {
            let mut edit_cmd = cmd;
            // For edits, keep original qso_begin from the form's bound
            // record. submit_qso doesn't currently surface that — we
            // approximate by reusing the loaded value via Utc::now() as
            // last resort, but UI loads qso_begin from get_qso when the
            // edit starts. Future improvement: editable timestamp.
            edit_cmd.qso_begin = self
                .recent
                .iter()
                .find(|q| q.id == id)
                .map(|q| q.qso_begin)
                .unwrap_or(edit_cmd.qso_begin);
            return Task::perform(
                async move { svc.update_qso(id, edit_cmd).await.map_err(|e| e.to_string()) },
                Message::QsoUpdated,
            );
        }

        Task::perform(create_qso(svc, cmd), Message::QsoCreated)
    }

    fn select_station_location(&mut self, id: StationLocationId) -> Task<Message> {
        let Some(loc) = self.station_locations.iter().find(|l| l.id == id).cloned() else {
            return Task::none();
        };
        self.active_location = Some(loc);
        Task::none()
    }

    /// Returns the active rig's handle if one is connected.
    pub(super) fn active_rig_handle(&self) -> Option<RigHandle> {
        self.rigs
            .get(self.active_rig)
            .and_then(|e| e.handle.clone())
    }

    /// Returns the active rig's most recent snapshot, if any.
    pub(super) fn active_rig_snapshot(&self) -> Option<&RigSnapshot> {
        self.rigs.get(self.active_rig).and_then(|e| e.snapshot.as_ref())
    }

    /// Push the form's freq + mode to the active rig. Operator-driven;
    /// we do nothing automatically. Conflict policy with WSJT-X / other
    /// apps: last writer wins. Slogger doesn't try to coordinate.
    fn send_to_rig(&mut self) -> Task<Message> {
        let Some(handle) = self.active_rig_handle() else {
            self.status = Some("active rig not connected".into());
            return Task::none();
        };
        let freq_hz = parse_mhz_to_hz(&self.freq_input).map(|v| v as u64);
        let mode_adif: Option<String> = self.mode.as_ref().map(|m| m.as_adif().to_string());
        if freq_hz.is_none() && mode_adif.is_none() {
            self.status = Some("nothing to send (set freq or mode in the form)".into());
            return Task::none();
        }
        Task::perform(
            async move {
                let mut parts: Vec<String> = Vec::new();
                if let Some(hz) = freq_hz {
                    handle
                        .set_frequency_hz(hz)
                        .await
                        .map_err(|e| format!("set freq: {e}"))?;
                    parts.push(format!("freq → {:.5} MHz", hz as f64 / 1_000_000.0));
                }
                if let Some(mode) = mode_adif {
                    match handle.set_mode_adif(&mode).await {
                        Ok(()) => parts.push(format!("mode → {mode}")),
                        Err(rig_control::CommandError::UnsupportedMode(m)) => {
                            parts.push(format!("mode {m} not mappable to rig — skipped"));
                        }
                        Err(e) => return Err(format!("set mode: {e}")),
                    }
                }
                Ok(parts.join(", "))
            },
            Message::SendToRigFinished,
        )
    }

    /// Keeps the entry form tracking the active rig. Called whenever the
    /// active rig reports a new snapshot; `previous` is the snapshot it
    /// replaced (None for the first report, or to force a full resync).
    ///
    /// Only what actually changed on the rig is copied, so values the
    /// operator set by hand survive until the rig itself moves:
    /// picking FT8 in the form sticks while tuning around, because the
    /// rig's own mode (USB) hasn't changed. Editing a logged QSO pauses
    /// following entirely — that form describes a past contact.
    fn follow_rig(&mut self, previous: Option<&RigSnapshot>) -> Task<Message> {
        if self.editing_qso.is_some() {
            return Task::none();
        }
        let Some(snap) = self.active_rig_snapshot().cloned() else {
            return Task::none();
        };
        let mut band_changed = false;
        if let Some(hz) = snap.freq_hz
            && previous.and_then(|p| p.freq_hz) != Some(hz)
        {
            self.freq_input = format!("{:.5}", hz as f64 / 1_000_000.0);
            if let Some(band) = Band::from_freq_hz(hz as i64)
                && self.band != Some(band)
            {
                self.band = Some(band);
                band_changed = true;
            }
        }
        if let Some(mode_str) = snap.mode.as_deref()
            && previous.and_then(|p| p.mode.as_deref()) != Some(mode_str)
        {
            // A rig mode with no logbook equivalent would blank the picker;
            // leave the operator's choice alone instead.
            let mode = Mode::from_adif(mode_str);
            if !matches!(mode, Mode::Other(_)) {
                self.mode = Some(mode);
            }
        }
        if band_changed {
            // Dupe status is per band.
            self.schedule_dupe_check()
        } else {
            Task::none()
        }
    }

    /// Copy the active rig's state into the entry form. Doesn't touch
    /// `call_input` (that's the operator's working call) or RST fields.
    fn use_rig_now(&mut self) -> Task<Message> {
        let Some(snap) = self.active_rig_snapshot() else {
            self.status = Some("active rig has no snapshot yet".into());
            return Task::none();
        };
        let snap = snap.clone();
        if let Some(hz) = snap.freq_hz {
            self.freq_input = format!("{:.5}", hz as f64 / 1_000_000.0);
            if let Some(b) = Band::from_freq_hz(hz as i64) {
                self.band = Some(b);
            }
        }
        if let Some(mode_str) = snap.mode.as_deref() {
            self.mode = Some(Mode::from_adif(mode_str));
        }
        Task::none()
    }

    /// Apply a clicked spot to the entry form: copy the call, freq, mode,
    /// and derive the band. Operator can edit before pressing Log.
    /// When a rig is connected, ALSO send freq/mode to the rig — this is
    /// the "click and the radio tunes" UX that DXLab popularized.
    fn apply_spot(&mut self, spot: Spot) -> Task<Message> {
        // Guard: if the operator is editing an existing QSO, don't
        // silently overwrite their form. They likely meant to cancel the
        // edit first — show a hint and skip rather than wreck the edit.
        if self.editing_qso.is_some() {
            self.status = Some(format!(
                "spot {} ignored — finish or cancel the QSO edit first",
                spot.call
            ));
            return Task::none();
        }
        self.call_input = spot.call.clone();
        let freq_hz_u64 = spot.freq_hz;
        let freq_i64 = freq_hz_u64 as i64;
        self.freq_input = format!("{:.5}", freq_i64 as f64 / 1_000_000.0);
        if let Some(b) = Band::from_freq_hz(freq_i64) {
            self.band = Some(b);
        }
        if let Some(mode_str) = spot.mode.as_deref() {
            self.mode = Some(Mode::from_adif(mode_str));
        }

        // If the active rig is connected, push freq + mode immediately.
        // Failure is non-fatal — we already updated the form, so the
        // operator can still log the QSO; the rig just won't be tuned.
        if let Some(handle) = self.active_rig_handle() {
            let mode_adif = spot.mode.clone();
            return Task::perform(
                async move {
                    let mut parts: Vec<String> = Vec::new();
                    if let Err(e) = handle.set_frequency_hz(freq_hz_u64).await {
                        return Err(format!("rig set freq: {e}"));
                    }
                    parts.push(format!("freq → {:.5} MHz", freq_hz_u64 as f64 / 1_000_000.0));
                    if let Some(m) = mode_adif {
                        match handle.set_mode_adif(&m).await {
                            Ok(()) => parts.push(format!("mode → {m}")),
                            Err(rig_control::CommandError::UnsupportedMode(_)) => {
                                // Spot lacked a rig-mappable mode — fine,
                                // operator can adjust by hand.
                            }
                            Err(e) => return Err(format!("rig set mode: {e}")),
                        }
                    }
                    Ok(parts.join(", "))
                },
                Message::SendToRigFinished,
            );
        }
        Task::none()
    }

    fn create_station_location(&mut self) -> Task<Message> {
        let Some(repo) = self.station_repo.clone() else {
            return Task::none();
        };
        let name = self.new_location_name.trim();
        if name.is_empty() {
            self.status = Some("station name is required".into());
            return Task::none();
        }
        let call = match Callsign::parse(&self.new_location_call) {
            Ok(c) => c,
            Err(e) => {
                self.status = Some(format!("station callsign: {e}"));
                return Task::none();
            }
        };
        let grid = option_from_str(&self.new_location_grid);
        let now = Utc::now();
        let loc = StationLocation {
            id: StationLocationId::new(),
            name: name.to_string(),
            station_callsign: Some(call.clone()),
            owner_callsign: Some(call),
            city: None,
            county: None,
            state: None,
            country: None,
            grid,
            latitude: None,
            longitude: None,
            cq_zone: None,
            itu_zone: None,
            iota: None,
            lotw_station_location: None,
            eqsl_account: None,
            created_at: now,
            updated_at: now,
        };
        self.creating_location = true;
        self.status = Some(format!("creating station '{}'…", loc.name));
        Task::perform(insert_location(repo, loc), Message::LocationCreated)
    }

    fn start_services_update(&mut self) -> Task<Message> {
        let Some(repo) = self.repo.clone() else {
            return Task::none();
        };
        let Some(cfg) = self.config.clone() else {
            self.status = Some("config not loaded".into());
            return Task::none();
        };
        let lotw_configured =
            cfg.lotw.is_configured_for_upload() || cfg.lotw.is_configured_for_fetch();
        let eqsl_configured = cfg.eqsl.is_configured();
        let clublog_configured = cfg.clublog.is_configured();
        let qrz_configured = cfg.qrz.is_configured();
        let hrdlog_configured = cfg.hrdlog.is_configured();
        if !lotw_configured
            && !eqsl_configured
            && !clublog_configured
            && !qrz_configured
            && !hrdlog_configured
        {
            self.status = Some(
                "no service configured (need [lotw], [eqsl], [clublog], [qrz], or [hrdlog] \
                 in config.toml)"
                    .into(),
            );
            return Task::none();
        }
        if self.syncing {
            return Task::none();
        }
        self.syncing = true;
        self.status = Some("updating services…".into());
        Task::perform(
            run_services_update(repo, cfg),
            Message::ServicesUpdateFinished,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::types::RigEntry;

    fn snapshot(freq_hz: u64, mode: &str) -> RigSnapshot {
        RigSnapshot {
            vendor: "icom".into(),
            model: "IC-7300".into(),
            freq_hz: Some(freq_hz),
            mode: Some(mode.into()),
            at: Utc::now(),
        }
    }

    fn app_with_rig() -> App {
        let (mut app, _) = App::init();
        app.rigs.push(RigEntry {
            label: "Main".into(),
            config: Default::default(),
            handle: None,
            snapshot: None,
            status: String::new(),
        });
        app
    }

    fn report(app: &mut App, freq_hz: u64, mode: &str) {
        let _ = app.update(Message::RigSnapshot(crate::app::types::TaggedRigSnapshot {
            rig_index: 0,
            snapshot: snapshot(freq_hz, mode),
        }));
    }

    #[test]
    fn keyer_messages_fill_in_from_the_entry_form() {
        let (mut app, _) = App::init();
        let report = KeyerMacro::new("Report", "{CALL} UR {RST}");
        assert_eq!(app.expand_macro(&report), Err("{CALL}"));
        let _ = app.update(Message::CallChanged("ja1nut".into()));
        app.rst_sent = "599".into();
        assert_eq!(app.expand_macro(&report).unwrap(), "JA1NUT UR 5NN");
    }

    #[test]
    fn unsent_keyer_text_is_kept_when_no_keyer_is_connected() {
        let (mut app, _) = App::init();
        let _ = app.update(Message::KeyerTextChanged("qrl?".into()));
        let _ = app.update(Message::KeyerTextSubmitted);
        assert_eq!(app.keyer_text, "QRL?");
        assert_eq!(app.status.as_deref(), Some("keyer not connected"));
    }

    #[test]
    fn form_follows_rig_frequency_band_and_mode() {
        let mut app = app_with_rig();
        report(&mut app, 7_005_000, "CW");
        assert_eq!(app.freq_input, "7.00500");
        assert_eq!(app.band, Some(Band::M40));
        assert_eq!(app.mode, Some(Mode::CW));
    }

    #[test]
    fn hand_picked_mode_survives_tuning_until_rig_mode_changes() {
        let mut app = app_with_rig();
        report(&mut app, 14_074_000, "USB");
        app.mode = Some(Mode::FT8);
        report(&mut app, 14_076_000, "USB");
        assert_eq!(app.freq_input, "14.07600");
        assert_eq!(app.mode, Some(Mode::FT8));
        report(&mut app, 14_030_000, "CW");
        assert_eq!(app.mode, Some(Mode::CW));
    }

    #[test]
    fn form_does_not_follow_rig_while_editing_a_qso() {
        let mut app = app_with_rig();
        report(&mut app, 14_074_000, "USB");
        app.editing_qso = Some(QsoId::new());
        report(&mut app, 7_005_000, "CW");
        assert_eq!(app.freq_input, "14.07400");
        assert_eq!(app.band, Some(Band::M20));
    }

    #[test]
    fn ending_an_edit_restores_the_form_as_it_was() {
        let (mut app, _) = App::init();
        app.call_input = "K1ABC".into();
        app.freq_input = "14.02500".into();
        app.rst_sent = "579".into();
        app.entry_draft = Some(EntryDraft {
            call_input: app.call_input.clone(),
            band: app.band,
            mode: app.mode.clone(),
            freq_input: app.freq_input.clone(),
            rst_sent: app.rst_sent.clone(),
            rst_rcvd: app.rst_rcvd.clone(),
        });
        app.editing_qso = Some(QsoId::new());
        // The edit loads an old QSO over the form.
        app.call_input = "DL1XYZ".into();
        app.freq_input = "7.01000".into();
        app.rst_sent = "599".into();

        let _ = app.end_edit();

        assert!(app.editing_qso.is_none());
        assert!(app.entry_draft.is_none());
        assert_eq!(app.call_input, "K1ABC");
        assert_eq!(app.freq_input, "14.02500");
        assert_eq!(app.rst_sent, "579");
    }

    #[test]
    fn leaving_the_tab_abandons_an_edit() {
        let (mut app, _) = App::init();
        app.current_view = super::super::views::ViewKind::Logbook;
        app.call_input = "K1ABC".into();
        app.entry_draft = Some(EntryDraft {
            call_input: app.call_input.clone(),
            band: app.band,
            mode: app.mode.clone(),
            freq_input: app.freq_input.clone(),
            rst_sent: app.rst_sent.clone(),
            rst_rcvd: app.rst_rcvd.clone(),
        });
        app.editing_qso = Some(QsoId::new());
        app.call_input = "DL1XYZ".into();

        let _ = app.update(Message::ViewChanged(super::super::views::ViewKind::Operating));

        assert!(app.editing_qso.is_none());
        assert_eq!(app.call_input, "K1ABC");
    }

    #[test]
    fn escape_closes_an_open_modal_and_nothing_else() {
        let (mut app, _) = App::init();
        app.call_input = "K1ABC".into();
        app.modal = Some(crate::app::modals::Modal::ThemePicker);

        let _ = app.update(Message::EscapePressed);
        assert!(app.modal.is_none());
        assert_eq!(app.call_input, "K1ABC", "the form under the modal is untouched");
    }

    #[test]
    fn a_modal_blocks_keyboard_shortcuts_for_the_ui_underneath() {
        let (mut app, _) = App::init();
        app.modal = Some(crate::app::modals::Modal::ThemePicker);

        let _ = app.update(Message::ViewChanged(super::super::views::ViewKind::Logbook));
        assert_eq!(app.current_view, super::super::views::ViewKind::Operating);
        assert!(app.modal.is_some());
    }
}
