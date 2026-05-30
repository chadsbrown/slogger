use std::time::Duration;

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
use super::layout::{configuration_from_tree, save_layout, tree_from_state};
use super::message::{DupeMatch, LogbookSearchResult, Message};
use super::services::{format_multi_summary, run_services_update};
use super::spots::build_worked_by_band;
use super::state::App;

impl App {
    pub fn update(&mut self, message: Message) -> Task<Message> {
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
                // Restore a saved pane layout if present. If the saved
                // tree references a PaneKind that no longer exists in the
                // current build, configuration_from_tree just builds the
                // tree it sees — operators editing the JSON manually take
                // their own risk.
                if let Some(tree) = bundle.pane_layout {
                    self.panes = pane_grid::State::with_configuration(
                        configuration_from_tree(&tree),
                    );
                }
                Task::perform(refresh(bundle.repo, bundle.service), Message::Refreshed)
            }
            Message::Booted(Err(e)) => {
                self.boot_error = Some(e);
                Task::none()
            }
            Message::CallChanged(s) => {
                self.call_input = s;
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
                self.editing_qso = Some(id);
                Task::perform(
                    async move {
                        match svc.get_qso(&id).await {
                            Ok(Some(q)) => Ok(q),
                            Ok(None) => Err("qso not found".into()),
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    Message::QsoLoadedForEdit,
                )
            }
            Message::QsoLoadedForEdit(Ok(q)) => {
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
                self.editing_qso = None;
                self.status = Some(format!("load qso error: {e}"));
                Task::none()
            }
            Message::CancelEditPressed => {
                self.editing_qso = None;
                self.call_input.clear();
                self.dupe_match = None;
                focus_call()
            }
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
                self.editing_qso = None;
                self.call_input.clear();
                self.status = Some("qso deleted".into());
                self.refresh_task()
            }
            Message::QsoDeleted(Err(e)) => {
                self.status = Some(format!("delete error: {e}"));
                Task::none()
            }
            Message::QsoUpdated(Ok(())) => {
                self.editing_qso = None;
                self.status = Some("qso updated".into());
                self.call_input.clear();
                self.dupe_match = None;
                Task::batch([self.refresh_task(), focus_call()])
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
                if let Some(entry) = self.rigs.get_mut(tagged.rig_index) {
                    entry.snapshot = Some(tagged.snapshot);
                }
                Task::none()
            }
            Message::ActiveRigChanged(idx) => {
                if idx >= self.rigs.len() {
                    return Task::none();
                }
                self.active_rig = idx;
                // Active-radio abstraction: when SO2R is configured, the
                // same click that switches the entry-form / rig-command
                // target also moves the OTRSP TX line. Otherwise an
                // operator who clicked "R2" in the Station pane would
                // tune Radio 2 but transmit on Radio 1 — silent footgun.
                if let Some(handle) = self.so2r_handle.clone() {
                    let radio = (idx as u8) + 1;
                    return Task::perform(
                        async move {
                            handle
                                .set_tx_radio(radio)
                                .await
                                .map(|_| format!("SO2R TX → R{radio}"))
                                .map_err(|e| format!("SO2R set_tx error: {e}"))
                        },
                        Message::SendToRigFinished,
                    );
                }
                Task::none()
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
            Message::SpotEvent(SpotEvent::Withdrawn { call }) => {
                self.spots.retain(|s| s.call != call);
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
            Message::PaneDragged(pane_grid::DragEvent::Dropped { pane, target }) => {
                // Route to whichever view's pane_grid is currently
                // visible. Pane IDs are per-State, so events can only
                // refer to the current view's grid.
                match self.current_view {
                    super::views::ViewKind::Operating => self.panes.drop(pane, target),
                    super::views::ViewKind::Logbook => {
                        self.logbook_panes.drop(pane, target)
                    }
                    super::views::ViewKind::Qsl => self.qsl_panes.drop(pane, target),
                }
                self.schedule_layout_save()
            }
            Message::PaneDragged(_) => Task::none(),
            Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => {
                match self.current_view {
                    super::views::ViewKind::Operating => {
                        self.panes.resize(split, ratio)
                    }
                    super::views::ViewKind::Logbook => {
                        self.logbook_panes.resize(split, ratio)
                    }
                    super::views::ViewKind::Qsl => {
                        self.qsl_panes.resize(split, ratio)
                    }
                }
                self.schedule_layout_save()
            }
            Message::LayoutSaveTick(generation) => {
                if generation == self.layout_save_gen {
                    let tree = tree_from_state(&self.panes);
                    Task::perform(save_layout(tree), |()| Message::LayoutSaveCompleted)
                } else {
                    Task::none()
                }
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
            Message::KeyerSendMacro(text) => {
                let Some(handle) = self.keyer_handle.clone() else {
                    self.status = Some("keyer not connected".into());
                    return Task::none();
                };
                Task::perform(
                    async move {
                        handle.send_message(&text).await.map_err(|e| e.to_string())
                    },
                    Message::KeyerSendFinished,
                )
            }
            Message::KeyerSendFinished(result) => {
                if let Err(e) = result {
                    self.status = Some(format!("keyer error: {e}"));
                }
                Task::none()
            }
            Message::ViewChanged(view) => {
                self.current_view = view;
                // Auto-refresh the QSL Pending pane on first switch
                // into QSL so the operator sees fresh data immediately.
                if matches!(view, super::views::ViewKind::Qsl)
                    && self.qsl_view.pending.is_empty()
                    && !self.qsl_view.loading_pending
                {
                    return self.run_qsl_pending_refresh();
                }
                Task::none()
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

    /// Schedules a debounced layout save. Bumps the generation, then
    /// waits 500ms; if no further resize/drag bumps the generation, the
    /// tick handler actually writes the file. 500ms is comfortably long
    /// enough to coalesce a multi-pixel drag into one save.
    fn schedule_layout_save(&mut self) -> Task<Message> {
        self.layout_save_gen = self.layout_save_gen.wrapping_add(1);
        let generation = self.layout_save_gen;
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                generation
            },
            Message::LayoutSaveTick,
        )
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
