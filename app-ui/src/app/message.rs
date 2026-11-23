use logbook_domain::AwardsSnapshot;
use radio_core::{Band, Mode, QsoId, StationLocation};
use spot_feed::SpotEvent;
use wsjtx_bridge::WsjtxMessage;

use super::drawers::awards::{AwardKind, TargetUnit};
use super::drawers::logbook::{BandFilter, LocationFilterOpt, LotwFilter, ModeFilter};
use super::panels::{DragMode, PanelKind};
use super::types::{BootBundle, StationOption, TaggedRigSnapshot};
use super::views::ViewKind;
use condx_feed::CondXSnapshot;
use keyer_control::KeyerSnapshot;
use so2r_control::So2rSnapshot;

#[derive(Debug, Clone)]
pub enum Message {
    Booted(Result<BootBundle, String>),
    CallChanged(String),
    BandChanged(Band),
    ModeChanged(Mode),
    FreqChanged(String),
    RstSentChanged(String),
    RstRcvdChanged(String),
    LogPressed,
    QsoCreated(Result<QsoId, String>),
    Refreshed(Result<RefreshSnapshot, String>),
    AdifPathChanged(String),
    ImportPressed,
    ImportFinished(Result<ImportSummary, String>),
    ServicesUpdatePressed,
    ServicesUpdateFinished(MultiUpdateSummary),
    /// Read everything at LoTW again, ignoring where the last sync left
    /// off. LoTW only: no uploads, no other services.
    LotwFullResyncPressed,
    LotwFullResyncFinished(Result<FetchSummary, String>),
    StationLocationSelected(StationOption),
    /// Open the station editor on the active location.
    EditStationPressed,
    /// Open the station editor on a new, empty location.
    NewStationPressed,
    /// Input to the open station editor.
    StationEditor(super::modals::station::StationEditorMsg),
    /// A location saved from the editor: inserted (new) or updated.
    StationSaved(Result<StationLocation, String>),
    SpotEvent(SpotEvent),
    SpotClicked(spot_feed::Spot),
    ToggleSpotsNeededOnly,
    /// A POTA poll finished: the whole current list of activator spots.
    PotaSpots(Vec<pota_feed::PotaSpot>),
    /// The Spots pane's POTA button: switch POTA spots on or off.
    TogglePota,
    QsoSelected(QsoId),
    QsoLoadedForEdit(Result<logbook_domain::QsoDetail, String>),
    CancelEditPressed,
    DeletePressed,
    QsoDeleted(Result<(), String>),
    QsoUpdated(Result<(), String>),
    WsjtxMessage(WsjtxMessage),
    WsjtxImportFinished(Result<usize, String>),
    RigSnapshot(TaggedRigSnapshot),
    UseRigPressed,
    SendToRigPressed,
    SendToRigFinished(Result<String, String>),
    ActiveRigChanged(usize),
    KeyerSnapshot(KeyerSnapshot),
    So2rSnapshotMsg(So2rSnapshot),
    /// A fresh solar/propagation reading from hamqsl.com.
    CondXSnapshot(CondXSnapshot),
    /// Operator picked an SO2R headphone mode: `"mono"` or `"stereo"`.
    So2rRxModeSelected(&'static str),
    /// A Spots table column header was clicked.
    SpotsSortClicked(super::spots::SpotColumn),
    /// Result of a debounced dupe-check fired from CallChanged /
    /// BandChanged. `generation` is the generation counter at the time
    /// the check was scheduled; the update handler drops the result if
    /// a newer check has been scheduled since.
    DupeCheckFinished {
        generation: u64,
        dupe: Option<DupeMatch>,
    },
    /// Fired when the async layout write finishes. No state change; the
    /// variant exists because iced `Task::perform` requires the future's
    /// result to be mapped to a Message.
    LayoutSaveCompleted,
    FontScaleUp,
    FontScaleDown,
    FontScaleReset,
    ThemeSelected(iced::Theme),
    /// Open the theme editor on the theme in use: on a new copy of it,
    /// or (for a theme of the user's own, when `copy` is false) on the
    /// theme itself.
    EditTheme { copy: bool },
    /// Delete the file of the theme in use. The first press only arms it.
    DeleteTheme,
    ThemeEditor(super::editor::EditorMessage),
    OpenModal(super::modals::Modal),
    CloseModal,
    /// A click inside an open modal that hit no control. Does nothing; it
    /// exists so the click is not passed on to the backdrop.
    ModalClicked,

    // ---- Floating panels (every tab) ----
    /// A panel was clicked somewhere inert: bring it to the front.
    PanelPressed(PanelKind),
    /// The pointer went down on a panel's title bar (move) or corner
    /// grip (resize). `canvas` is the canvas size at that moment.
    PanelDragStarted {
        kind: PanelKind,
        mode: DragMode,
        canvas: iced::Size,
    },
    /// Pointer moved while a panel drag is in progress (window coords).
    PanelPointerMoved(iced::Point),
    /// Pointer released: the drag, if any, is over.
    PanelPointerReleased,
    /// Open a closed panel or close an open one.
    PanelToggled(PanelKind),
    /// Put every panel of the tab being shown back to its default
    /// place, size and visibility.
    PanelsReset,

    // ---- Logbook drawer ----
    LogbookCallPrefixChanged(String),
    LogbookExactCallChanged(String),
    LogbookBandChanged(BandFilter),
    LogbookModeChanged(ModeFilter),
    LogbookLotwFilterChanged(LotwFilter),
    LogbookLocationFilterChanged(LocationFilterOpt),
    /// Location picked for "assign to all matches".
    LogbookAssignTargetChanged(StationOption),
    /// Link every QSO the last search matched to the picked location.
    LogbookBulkAssignLocation,
    LogbookSearchPressed,
    LogbookSearchFinished {
        generation: u64,
        result: Result<LogbookSearchResult, String>,
    },
    LogbookRowToggle(QsoId),
    LogbookClearSelection,
    LogbookBulkDelete,
    LogbookBulkMarkUploaded(String),
    /// Outcome of a bulk action (delete / mark uploaded). String is the
    /// human-readable summary written into `last_action`.
    LogbookBulkFinished {
        generation: u64,
        result: Result<String, String>,
    },

    // ---- Awards drawer ----
    AwardsKindChanged(AwardKind),
    AwardsBandFilterChanged(BandFilter),
    /// Operator clicked an award unit to target it. Spots panel will
    /// border-highlight matching spots.
    AwardsSetTarget(TargetUnit),
    AwardsClearTarget,

    // ---- CW keyer pane ----
    /// Send this exact CW text via the keyer.
    KeyerSendMacro(String),
    KeyerSendFinished(Result<(), String>),
    /// Free-text box edited.
    KeyerTextChanged(String),
    /// Enter in the free-text box: send its contents and clear it.
    KeyerTextSubmitted,
    /// Stop sending and flush the keyer buffer.
    KeyerAbortPressed,
    /// Change CW speed to this many WPM.
    KeyerSetWpm(u8),
    KeyerSetWpmFinished(Result<u8, String>),

    // ---- Keyboard ----
    /// Esc: abort CW if the keyer is sending, otherwise cancel the edit /
    /// wipe the entry form and return focus to the Call field.
    EscapePressed,
    /// Tab / Shift+Tab: move focus between text fields.
    FocusNext,
    FocusPrevious,
    /// F1..F12: fire the numbered keyer message.
    FunctionKey(u8),
    KeyerAbortFinished(Result<(), String>),
    /// One-second heartbeat. Carries no state; it exists so the UTC clock,
    /// spot ages and rig staleness re-render while the app is otherwise idle.
    Tick,

    // ---- View switching ----
    /// Switch which top-level view (Operating / QSL / Logbook) is shown.
    ViewChanged(ViewKind),

    // ---- QSL view ----
    QslServiceFilterChanged(super::views::qsl::QslServiceFilter),
    QslPendingRefreshPressed,
    QslPendingRefreshed(Result<Vec<radio_core::Qso>, String>),
    QslPendingRowToggle(radio_core::QsoId),
    QslClearSelection,
    QslBulkMarkUploaded(String),
    QslBulkDelete,
    QslBulkFinished(Result<String, String>),
    /// The Confirmations pane's list, loaded with entity names resolved.
    ConfirmationsLoaded(Result<Vec<super::views::qsl::ConfirmationRow>, String>),
    /// A Confirmations row was clicked: show that QSO in the Logbook tab.
    ConfirmationClicked(QsoId),
}

/// Payload for a successful logbook search. `total` is the count from
/// `count_matching` (which sees every match, ignoring the result limit);
/// `results` is what was actually returned by `search_qsos`.
#[derive(Debug, Clone)]
pub struct LogbookSearchResult {
    pub results: Vec<logbook_domain::QsoSummary>,
    pub total: usize,
}

#[derive(Debug, Clone)]
pub struct DupeMatch {
    pub qso_begin: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct RefreshSnapshot {
    pub recent: Vec<logbook_domain::QsoSummary>,
    pub pending_lotw: usize,
    pub pending_eqsl: usize,
    pub pending_clublog: usize,
    pub pending_qrz: usize,
    pub pending_hrdlog: usize,
    pub awards: AwardsSnapshot,
    /// Re-read each refresh: an import can create locations.
    pub station_locations: Vec<StationLocation>,
}

#[derive(Debug, Clone)]
pub struct ImportSummary {
    pub created: usize,
    pub duplicates: usize,
    pub locations_created: usize,
    pub skipped: usize,
    pub parse_errors: usize,
    pub first_errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct UploadSummary {
    pub uploaded: usize,
    pub note: String,
}

/// What a LoTW sync did across its two reports (confirmations, accepted
/// uploads). Counts are of QSOs whose state changed, so a sync with
/// nothing new reads all zeros.
#[derive(Debug, Clone, Default)]
pub struct FetchSummary {
    /// Stored watermarks were ignored and everything was read again.
    pub full_resync: bool,
    pub new_confirmations: usize,
    pub uploads_verified: usize,
    /// LoTW QSOs that match nothing in the log, counted once even when
    /// both reports carry them.
    pub not_in_log: usize,
    /// Award credits added or raised from submitted to granted.
    pub credits: usize,
    /// QSOs with location fields filled in from LoTW's confirmations.
    pub qsos_filled: usize,
    /// QSOs where LoTW newly reports a location field other than the
    /// log holds.
    pub qsos_differing: usize,
    /// Records LoTW sent without a call or a readable date and time.
    pub unreadable: usize,
    /// A report that failed while the other was applied.
    pub errors: Vec<String>,
}

/// LoTW's part of a services update.
#[derive(Debug, Clone, Default)]
pub struct UpdateSummary {
    pub upload: Option<super::lotw_upload::LotwUploadSummary>,
    pub upload_error: Option<String>,
    pub upload_skipped_reason: Option<String>,
    pub fetch: Option<FetchSummary>,
    pub fetch_error: Option<String>,
    pub fetch_skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EqslUpdateSummary {
    pub upload: Option<UploadSummary>,
    pub upload_error: Option<String>,
    pub upload_skipped_reason: Option<String>,
    pub fetched: usize,
    pub confirmed: usize,
    pub unmatched: usize,
    pub fetch_error: Option<String>,
    pub fetch_skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ClubLogUpdateSummary {
    pub upload: Option<UploadSummary>,
    pub upload_error: Option<String>,
    pub upload_skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct QrzUpdateSummary {
    /// The station callsign the key's logbook serves.
    pub book_callsign: Option<String>,
    /// QSOs QRZ added to the logbook.
    pub uploaded: usize,
    /// QSOs QRZ already held; now marked uploaded.
    pub duplicates: usize,
    pub rejected: usize,
    /// Pending QSOs not sent because they don't belong in this logbook:
    /// another station callsign (or none), or a date outside its range.
    pub held_back: usize,
    /// What stopped the update: a refused key, or a failure part-way
    /// through the upload.
    pub upload_error: Option<String>,
    pub upload_skipped_reason: Option<String>,
    /// `CALL: reason` for the first few rejections.
    pub first_errors: Vec<String>,
    pub fetch: Option<QrzFetchSummary>,
    pub fetch_error: Option<String>,
}

/// What reading the QRZ logbook back did.
#[derive(Debug, Clone, Default)]
pub struct QrzFetchSummary {
    /// The whole logbook was read (no earlier sync point).
    pub full: bool,
    pub records: usize,
    pub new_confirmations: usize,
    pub uploads_verified: usize,
    /// Records QRZ holds that match no QSO in the log.
    pub not_in_log: usize,
    pub unreadable: usize,
}

#[derive(Debug, Clone, Default)]
pub struct HrdlogUpdateSummary {
    pub upload: Option<UploadSummary>,
    pub upload_error: Option<String>,
    pub upload_skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MultiUpdateSummary {
    pub lotw: Option<UpdateSummary>,
    pub eqsl: Option<EqslUpdateSummary>,
    pub clublog: Option<ClubLogUpdateSummary>,
    pub qrz: Option<QrzUpdateSummary>,
    pub hrdlog: Option<HrdlogUpdateSummary>,
}
