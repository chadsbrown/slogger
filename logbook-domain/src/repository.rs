use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use radio_core::{
    Band, Callsign, Mode, OperatingSession, OperatingSessionId, OperatorId, Qso, QsoExchangeField,
    QsoId, StationLocation, StationLocationId,
};

use crate::queries::{QsoSearch, QsoSummary};

/// Slim per-QSO snapshot for award computation. Pulled in one query that
/// joins qsos with qso_service_state.lotw so the awards module can compute
/// worked + confirmed counts without further DB hits.
#[derive(Debug, Clone)]
pub struct AwardQso {
    pub id: QsoId,
    pub call: Callsign,
    pub qso_begin: DateTime<Utc>,
    pub band: Option<Band>,
    pub mode: Option<Mode>,
    pub dxcc_id: Option<u16>,
    pub dxcc_prefix: Option<String>,
    pub continent: Option<String>,
    pub state: Option<String>,
    pub iota: Option<String>,
    pub lotw_confirmed: bool,
}

/// QSO upload lifecycle for a remote service:
///
/// - `Pending` — never attempted (or no row in qso_service_state).
/// - `Uploaded` — we POSTed and the server returned 200, but we have not
///   round-tripped a fetch yet to confirm the server kept the QSO.
/// - `Verified` — a subsequent report fetch saw the QSO in our account at
///   the service. This is the strong success signal.
/// - `Failed` — upload attempt errored (TQSL exit, HTTP, server reject).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadState {
    Pending,
    Uploaded,
    Verified,
    Failed,
}

impl UploadState {
    pub fn as_storage_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Uploaded => "uploaded",
            Self::Verified => "verified",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmationState {
    Unknown,
    Pending,
    Confirmed,
    Mismatch,
}

impl ConfirmationState {
    pub fn as_storage_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Mismatch => "mismatch",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingUpload {
    pub qso_id: QsoId,
}

#[derive(Debug, Clone)]
pub struct ConfirmationMatch {
    pub qso_id: QsoId,
    pub confirmed_at: DateTime<Utc>,
    pub remote_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("not found")]
    NotFound,

    #[error("constraint violation: {0}")]
    Constraint(String),

    #[error("storage error: {0}")]
    Storage(String),
}

pub type RepoResult<T> = Result<T, RepositoryError>;

/// Canonical dedup key for a QSO during ADIF import — matches the keys
/// used by `is_duplicate` in `LogbookService`. Tuple of
/// (station_callsign, worked_call, yyyy-mm-dd, band, mode). The
/// preloaded `HashSet<DedupKey>` lets `import_qsos` skip per-record
/// SELECTs against the growing `qsos` table.
pub type DedupKey = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

/// One row of synthesized `qso_service_state` derived from ADIF
/// per-service fields during import (e.g. `LOTW_QSL_SENT=V` →
/// upload_state=verified). `upload_state` and `confirmation_state` are
/// stored as `&'static str` to match the schema's TEXT enum strings
/// (`pending` / `uploaded` / `verified` / `failed` for upload;
/// `pending` / `confirmed` / `mismatch` / `unknown` for confirmation).
/// `None` means "leave at default" — the SQL coalesces appropriately.
#[derive(Debug, Clone)]
pub struct ImportedServiceState {
    pub qso_id: QsoId,
    pub service: &'static str,
    pub upload_state: Option<&'static str>,
    pub confirmation_state: Option<&'static str>,
    pub uploaded_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait QsoRepository: std::fmt::Debug + Send + Sync {
    async fn insert_qso(&self, qso: &Qso) -> RepoResult<()>;

    async fn update_qso(&self, qso: &Qso) -> RepoResult<()>;

    async fn get_qso(&self, id: &QsoId) -> RepoResult<Option<Qso>>;

    async fn soft_delete_qso(&self, id: &QsoId) -> RepoResult<()>;

    async fn add_exchange_field(
        &self,
        qso_id: &QsoId,
        field: &QsoExchangeField,
    ) -> RepoResult<()>;

    async fn list_exchange_fields(
        &self,
        qso_id: &QsoId,
    ) -> RepoResult<Vec<QsoExchangeField>>;

    async fn search_qsos(&self, query: QsoSearch) -> RepoResult<Vec<QsoSummary>>;

    /// Same filter shape as `search_qsos` but returns full `Qso` records.
    /// Used when callers need every field (export, edit, bulk-update).
    async fn search_full_qsos(&self, query: QsoSearch) -> RepoResult<Vec<Qso>>;

    /// Count QSOs matching the filter without fetching them. Used for
    /// "Delete N QSOs?" confirmations and result-set previews.
    async fn count_matching(&self, query: QsoSearch) -> RepoResult<usize>;

    /// Returns QSOs that have not yet been uploaded to the named service.
    /// "Not yet uploaded" = no row in qso_service_state for (qso_id, service)
    /// OR a row with upload_state = 'pending' / 'failed'.
    async fn list_pending_uploads(
        &self,
        service: &str,
        limit: Option<u32>,
    ) -> RepoResult<Vec<Qso>>;

    /// Mark a QSO as uploaded to a service. Inserts or updates the
    /// qso_service_state row. `remote_id` is the service's identifier for
    /// the upload (e.g. LotW assigns one per ADIF batch / record).
    async fn mark_uploaded(
        &self,
        qso_id: &QsoId,
        service: &str,
        uploaded_at: DateTime<Utc>,
        remote_id: Option<&str>,
    ) -> RepoResult<()>;

    /// Mark an upload as failed (transient or hard) so a retry pass can
    /// pick it up. Stored in qso_service_state.last_error.
    async fn mark_upload_failed(
        &self,
        qso_id: &QsoId,
        service: &str,
        error: &str,
    ) -> RepoResult<()>;

    /// Mark a QSO as upload-verified — i.e. a subsequent fetch saw it at
    /// the service. Distinct from `mark_uploaded` (optimistic post-POST)
    /// and from `mark_confirmed` (other station also matched at the
    /// service).
    async fn mark_upload_verified(
        &self,
        qso_id: &QsoId,
        service: &str,
        verified_at: DateTime<Utc>,
        remote_id: Option<&str>,
    ) -> RepoResult<()>;

    /// Mark a QSO as confirmed by the service.
    async fn mark_confirmed(
        &self,
        qso_id: &QsoId,
        service: &str,
        confirmed_at: DateTime<Utc>,
        remote_id: Option<&str>,
    ) -> RepoResult<()>;

    /// Pull every non-deleted QSO joined with the LotW service state, in
    /// the slim shape needed for award aggregation. Awards code is pure
    /// Rust on top of this — keeps SQL out of the awards layer.
    async fn list_award_qsos(&self) -> RepoResult<Vec<AwardQso>>;

    /// Look up a QSO by the canonical match keys used by LotW and friends:
    /// station callsign + worked callsign + UTC date + band + mode. Returns
    /// at most one match — if multiple exist, the most recent qso_begin wins.
    async fn find_match_for_confirmation(
        &self,
        station_call: &str,
        worked_call: &str,
        qso_date: &str,
        band: Option<&str>,
        mode: Option<&str>,
    ) -> RepoResult<Option<QsoId>>;

    /// One-shot load of every existing dedup key. Used by `import_qsos`
    /// to replace per-record dedup SELECTs with O(1) HashSet lookups.
    /// Only QSOs that aren't soft-deleted and have a `station_callsign`
    /// (matching `is_duplicate` semantics) are returned.
    async fn load_dedup_keys(&self) -> RepoResult<std::collections::HashSet<DedupKey>>;

    /// Batch insert path used by ADIF import. Wraps all three tables
    /// (qsos, qso_exchange_fields, qso_service_state) in a single
    /// transaction so a 50k-record import is one fsync instead of
    /// ~1.5M. Per-row INSERT bodies match the existing single-row
    /// methods; only the transaction wrapping differs.
    async fn insert_qsos_batch(
        &self,
        qsos: &[Qso],
        exchange_fields: &[(QsoId, QsoExchangeField)],
        service_states: &[ImportedServiceState],
    ) -> RepoResult<()>;
}

#[async_trait]
pub trait StationRepository: std::fmt::Debug + Send + Sync {
    async fn insert_location(&self, location: &StationLocation) -> RepoResult<()>;

    async fn update_location(&self, location: &StationLocation) -> RepoResult<()>;

    async fn get_location(
        &self,
        id: &StationLocationId,
    ) -> RepoResult<Option<StationLocation>>;

    async fn list_locations(&self) -> RepoResult<Vec<StationLocation>>;

    /// Open a new operating session — a time-bounded marker stamped onto
    /// every QSO logged during it. Returns the new session id.
    async fn start_session(
        &self,
        operator_id: Option<&OperatorId>,
        station_location_id: Option<&StationLocationId>,
        name: Option<&str>,
    ) -> RepoResult<OperatingSessionId>;

    /// Close a session. Sets `ended_at = now`. Existing QSOs already
    /// stamped with this session keep that stamp.
    async fn end_session(&self, id: &OperatingSessionId) -> RepoResult<()>;

    async fn get_session(
        &self,
        id: &OperatingSessionId,
    ) -> RepoResult<Option<OperatingSession>>;

    /// Update which station_location an in-flight session is associated
    /// with — when the user changes their selected station mid-session,
    /// we preserve session continuity rather than fragmenting.
    async fn set_session_station_location(
        &self,
        id: &OperatingSessionId,
        station_location_id: Option<&StationLocationId>,
    ) -> RepoResult<()>;

    /// Close any sessions left open from prior runs (`ended_at IS NULL`).
    /// Called at boot before opening the current run's session, so the
    /// operating_sessions table doesn't accumulate orphaned rows.
    /// Returns how many sessions were closed.
    async fn close_open_sessions(&self) -> RepoResult<usize>;

    /// List recent sessions ordered by `started_at DESC`. `limit` caps
    /// the result set; `None` returns everything (use only on small
    /// datasets — UI should pass a bounded limit).
    async fn list_sessions(
        &self,
        limit: Option<u32>,
    ) -> RepoResult<Vec<OperatingSession>>;
}
