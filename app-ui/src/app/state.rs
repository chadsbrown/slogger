use std::collections::VecDeque;
use std::sync::Arc;

use app_config::Config;
use condx_feed::CondXSnapshot;
use iced::Task;
use iced::widget::pane_grid;
use keyer_control::{KeyerHandle, KeyerSnapshot};
use logbook_domain::{AwardsSnapshot, LogbookService, QsoRepository, QsoSummary, StationRepository};
use radio_core::{Band, Mode, QsoId, StationLocation};
use so2r_control::{So2rHandle, So2rSnapshot};
use spot_feed::Spot;
use station_resolver::Resolver;

use super::boot::boot_app;
use super::constants::SPOT_HISTORY_LIMIT;
use super::drawers::awards::AwardsDrawerState;
use super::drawers::logbook::LogbookSearchState;
use super::message::{DupeMatch, Message, MultiUpdateSummary};
use super::panels::Panels;
use super::types::RigEntry;
use super::views::ViewKind;
use super::views::logbook::{LogbookPaneKind, default_logbook_configuration};
use super::views::qsl::{QslPaneKind, QslViewState, default_qsl_configuration};

/// The entry form's fields, set aside while an existing QSO is edited.
#[derive(Debug, Clone)]
pub(super) struct EntryDraft {
    pub(super) call_input: String,
    pub(super) band: Option<Band>,
    pub(super) mode: Option<Mode>,
    pub(super) freq_input: String,
    pub(super) rst_sent: String,
    pub(super) rst_rcvd: String,
}

pub struct App {
    pub(super) service: Option<Arc<LogbookService>>,
    pub(super) repo: Option<Arc<dyn QsoRepository>>,
    pub(super) station_repo: Option<Arc<dyn StationRepository>>,
    pub(super) resolver: Option<Arc<dyn Resolver>>,
    pub(super) config: Option<Config>,
    pub(super) boot_error: Option<String>,

    pub(super) call_input: String,
    pub(super) band: Option<Band>,
    pub(super) mode: Option<Mode>,
    pub(super) freq_input: String,
    pub(super) rst_sent: String,
    pub(super) rst_rcvd: String,

    pub(super) adif_path: String,
    pub(super) importing: bool,
    pub(super) syncing: bool,

    pub(super) station_locations: Vec<StationLocation>,
    pub(super) active_location: Option<StationLocation>,

    pub(super) recent: Vec<QsoSummary>,
    pub(super) pending_lotw: usize,
    pub(super) pending_eqsl: usize,
    pub(super) pending_clublog: usize,
    pub(super) pending_qrz: usize,
    pub(super) pending_hrdlog: usize,
    pub(super) awards: AwardsSnapshot,

    /// When set, the entry form is editing an existing QSO rather than
    /// creating a new one. Submit calls update_qso instead of create_qso.
    pub(super) editing_qso: Option<QsoId>,
    /// Everything stored about the QSO being edited — the Logbook detail
    /// pane shows the parts the entry form has no field for.
    pub(super) qso_detail: Option<logbook_domain::QsoDetail>,
    /// The active colour theme, chosen in the bottom bar.
    pub(super) theme: iced::Theme,
    /// Every theme the picker offers, re-read from the theme directories
    /// each time the picker opens.
    pub(super) themes: Vec<super::theme::ThemeEntry>,
    /// Whole-UI scale factor (the A− / A+ buttons). 1.0 is normal size.
    pub(super) font_scale: f32,
    /// The modal dialog currently open over the UI, if any.
    pub(super) modal: Option<super::modals::Modal>,
    /// What the entry form held before the current edit began. Editing a
    /// QSO borrows the one shared form, so this is put back when the edit
    /// ends — a logbook lookup never disturbs a QSO in progress.
    pub(super) entry_draft: Option<EntryDraft>,

    pub(super) spots_active: bool,
    pub(super) spots: VecDeque<Spot>,
    pub(super) spots_status: Option<String>,
    pub(super) spots_needed_only: bool,
    /// Column sort chosen in the Spots table; `None` is the default
    /// order (newest spot first, stable under re-reports).
    pub(super) spots_sort: Option<super::spots::SpotSort>,
    pub(super) wsjtx_active: bool,
    pub(super) wsjtx_bind_addr: Option<String>,
    pub(super) wsjtx_imported: usize,

    /// One entry per configured rig (zero or more). UI commands target
    /// `rigs[active_rig]`. Auto-reconnect happens inside each handle's
    /// rig-control task, so manual Reconnect is no longer needed.
    pub(super) rigs: Vec<RigEntry>,
    pub(super) active_rig: usize,

    pub(super) keyer_active: bool,
    pub(super) keyer_status: Option<String>,
    pub(super) keyer_snapshot: Option<KeyerSnapshot>,
    pub(super) keyer_handle: Option<KeyerHandle>,
    /// Free-text CW waiting to be sent from the keyer pane's text box.
    pub(super) keyer_text: String,

    pub(super) so2r_active: bool,
    pub(super) so2r_status: Option<String>,
    pub(super) so2r_snapshot: Option<So2rSnapshot>,
    /// Handle for issuing SO2R switch commands. Backend ready; UI flows
    /// (TX-radio button row, RX-mode picker, etc.) are pending the
    /// upcoming UI redesign.
    #[allow(dead_code)]
    pub(super) so2r_handle: Option<So2rHandle>,

    /// Whether the CondX poller is running; false means `[condx]` is off.
    pub(super) condx_active: bool,
    /// The latest CondX reading. `None` until the first poll succeeds.
    pub(super) condx: Option<CondXSnapshot>,
    /// Per-band set of worked DXCC entity IDs. Recomputed from
    /// `awards.dxcc_by_band` on each refresh; used to annotate spots.
    pub(super) worked_by_band:
        std::collections::BTreeMap<Band, std::collections::HashSet<u16>>,

    pub(super) status: Option<String>,

    /// The Operating view's free-floating panels: where each one sits,
    /// which are open, their stacking order, and any drag in progress.
    pub(super) panels: Panels,

    /// Generation counter for the dupe-check debounce. Every keystroke /
    /// band change increments this; pending async checks tag their result
    /// with the snapshot they were scheduled at, and the result is
    /// applied only if the generation hasn't moved on.
    pub(super) dupe_check_gen: u64,
    /// Most recent dupe found for the current (call, band). None means
    /// either no match in the log, or the operator hasn't typed enough to
    /// resolve a callsign yet.
    pub(super) dupe_match: Option<DupeMatch>,

    /// Logbook drawer state — filter form values, last search results,
    /// per-row selection, bulk-action status. Lives on App so the drawer
    /// preserves its state across open/close cycles.
    pub(super) logbook_search: LogbookSearchState,

    /// Awards drawer state — selected award, optional band filter, and
    /// the optional cross-reference target that highlights spots in the
    /// live Spots pane.
    pub(super) awards_drawer: AwardsDrawerState,

    /// Which top-level view is currently selected (Operating / QSL /
    /// Logbook). Defaults to Operating on launch. Not persisted to disk
    /// — operators should always land in Operating after a restart.
    pub(super) current_view: ViewKind,

    /// pane_grid state for the Logbook view. Separate from `panes`
    /// (Operating) so each view has its own resizable layout.
    pub(super) logbook_panes: pane_grid::State<LogbookPaneKind>,

    /// pane_grid state for the QSL view.
    pub(super) qsl_panes: pane_grid::State<QslPaneKind>,

    /// Most recent outcome of `run_services_update`. Held on App
    /// (instead of dropped into self.status) so the QSL view's Service
    /// status pane can render last-update detail per service.
    pub(super) last_services_update: Option<MultiUpdateSummary>,
    /// Result of the last Full LoTW resync this session, in full. The
    /// status bar clips long messages, so the QSL tab shows this whole.
    pub(super) last_lotw_resync: Option<String>,

    /// QSL view state — service filter for the Pending pane, the
    /// current pending QSOs list, per-row selection, and last action
    /// outcome.
    pub(super) qsl_view: QslViewState,
}

impl App {
    pub fn init() -> (Self, Task<Message>) {
        let app = Self {
            service: None,
            repo: None,
            station_repo: None,
            resolver: None,
            config: None,
            boot_error: None,
            call_input: String::new(),
            band: Some(Band::M20),
            mode: Some(Mode::SSB),
            freq_input: String::new(),
            rst_sent: "59".into(),
            rst_rcvd: "59".into(),
            adif_path: String::new(),
            importing: false,
            syncing: false,
            editing_qso: None,
            qso_detail: None,
            theme: super::theme::app_theme(),
            themes: super::theme::all_themes(),
            font_scale: 1.0,
            modal: None,
            entry_draft: None,
            station_locations: vec![],
            active_location: None,
            recent: vec![],
            pending_lotw: 0,
            pending_eqsl: 0,
            pending_clublog: 0,
            pending_qrz: 0,
            pending_hrdlog: 0,
            awards: AwardsSnapshot::default(),
            spots_active: false,
            spots: VecDeque::with_capacity(SPOT_HISTORY_LIMIT),
            spots_status: None,
            spots_needed_only: false,
            spots_sort: None,
            wsjtx_active: false,
            wsjtx_bind_addr: None,
            wsjtx_imported: 0,
            rigs: Vec::new(),
            active_rig: 0,
            keyer_active: false,
            keyer_status: None,
            keyer_snapshot: None,
            keyer_handle: None,
            keyer_text: String::new(),
            so2r_active: false,
            so2r_status: None,
            so2r_snapshot: None,
            so2r_handle: None,
            condx_active: false,
            condx: None,
            worked_by_band: Default::default(),
            status: None,
            panels: Panels::default(),
            dupe_check_gen: 0,
            dupe_match: None,
            logbook_search: LogbookSearchState::default(),
            awards_drawer: AwardsDrawerState::default(),
            current_view: ViewKind::default(),
            logbook_panes: pane_grid::State::with_configuration(
                default_logbook_configuration(),
            ),
            qsl_panes: pane_grid::State::with_configuration(default_qsl_configuration()),
            last_services_update: None,
            last_lotw_resync: None,
            qsl_view: QslViewState::default(),
        };
        (app, Task::perform(boot_app(), Message::Booted))
    }
}
