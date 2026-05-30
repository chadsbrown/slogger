use iced::widget::pane_grid;
use logbook_domain::AwardsSnapshot;
use radio_core::{Band, Mode, QsoId, StationLocation};
use spot_feed::SpotEvent;
use wsjtx_bridge::WsjtxMessage;

use super::drawers::awards::{AwardKind, TargetUnit};
use super::drawers::logbook::{BandFilter, LotwFilter, ModeFilter};
use super::types::{BootBundle, StationOption, TaggedRigSnapshot};
use super::views::ViewKind;
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
    StationLocationSelected(StationOption),
    NewLocationNameChanged(String),
    NewLocationCallsignChanged(String),
    NewLocationGridChanged(String),
    CreateLocationPressed,
    LocationCreated(Result<StationLocation, String>),
    SpotEvent(SpotEvent),
    SpotClicked(spot_feed::Spot),
    ToggleSpotsNeededOnly,
    QsoSelected(QsoId),
    QsoLoadedForEdit(Result<radio_core::Qso, String>),
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
    PaneClicked(pane_grid::Pane),
    PaneDragged(pane_grid::DragEvent),
    PaneResized(pane_grid::ResizeEvent),
    /// Result of a debounced dupe-check fired from CallChanged /
    /// BandChanged. `generation` is the generation counter at the time
    /// the check was scheduled; the update handler drops the result if
    /// a newer check has been scheduled since.
    DupeCheckFinished {
        generation: u64,
        dupe: Option<DupeMatch>,
    },
    /// Debounced tick fired after a pane resize/drag. Carries the
    /// generation that scheduled it; the handler saves only if the
    /// generation still matches `App.layout_save_gen`.
    LayoutSaveTick(u64),
    /// Fired when the async file write finishes. No state change; the
    /// variant exists because iced `Task::perform` requires the future's
    /// result to be mapped to a Message.
    LayoutSaveCompleted,

    // ---- Logbook drawer ----
    LogbookCallPrefixChanged(String),
    LogbookExactCallChanged(String),
    LogbookBandChanged(BandFilter),
    LogbookModeChanged(ModeFilter),
    LogbookLotwFilterChanged(LotwFilter),
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

    // ---- Keyer macros (Station pane) ----
    /// Send arbitrary CW text via the keyer.
    KeyerSendMacro(String),
    KeyerSendFinished(Result<(), String>),

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
    pub band: Option<Band>,
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
}

#[derive(Debug, Clone)]
pub struct ImportSummary {
    pub created: usize,
    pub skipped: usize,
    pub parse_errors: usize,
    pub first_errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct UploadSummary {
    pub uploaded: usize,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct FetchSummary {
    pub fetched: usize,
    pub verified: usize,
    pub confirmed: usize,
    pub unmatched: usize,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSummary {
    pub upload: Option<UploadSummary>,
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
    pub uploaded: usize,
    pub rejected: usize,
    pub upload_error: Option<String>,
    pub upload_skipped_reason: Option<String>,
    pub first_errors: Vec<String>,
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
