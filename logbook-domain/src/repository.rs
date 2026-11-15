use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use radio_core::{
    Band, Callsign, Mode, Qso, QsoExchangeField, QsoId, StationLocation, StationLocationId,
};

use crate::credit::AwardCredit;
use crate::queries::{QsoSearch, QsoSummary};

/// Slim per-QSO snapshot for award computation. Pulled in one query that
/// joins qsos with qso_service_state.lotw (and the QSO's granted award
/// credits) so the awards module can compute worked / confirmed /
/// credited counts without further DB hits.
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
    /// Names of the awards this QSO has granted credit for (`DXCC`,
    /// `DXCC_BAND`, `WAS`, …). Submitted-only credit is not included.
    pub granted_credits: Vec<String>,
}

impl AwardQso {
    pub fn has_granted_credit(&self, award: &str) -> bool {
        self.granted_credits.iter().any(|a| a == award)
    }
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

/// One service's upload / confirmation record for a QSO, as stored.
/// Services with no record yet (never uploaded) are simply absent.
#[derive(Debug, Clone, PartialEq)]
pub struct QsoServiceState {
    /// Service key: `lotw`, `eqsl`, `clublog`, `qrz`, `hrdlog`.
    pub service: String,
    /// `pending` / `uploaded` / `verified` / `failed`.
    pub upload_state: String,
    /// `unknown` / `pending` / `confirmed` / `mismatch`.
    pub confirmation_state: String,
    pub uploaded_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

/// Everything stored about one QSO: the record itself, the extra fields
/// kept alongside it (ADIF fields with no column of their own), where it
/// stands with each QSL service, and the award credit it carries.
#[derive(Debug, Clone)]
pub struct QsoDetail {
    pub qso: Qso,
    pub exchange_fields: Vec<QsoExchangeField>,
    pub service_states: Vec<QsoServiceState>,
    pub credits: Vec<AwardCredit>,
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

/// Canonical dedup key for a QSO during ADIF import. Tuple of
/// (station_callsign, worked_call, start time as `yyyy-mm-ddThh:mm:ss`,
/// band, mode). The
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
    /// The service's id for the QSO, e.g. DXKeeper's
    /// `APP_DXKEEPER_QRZCOM_LOGID` for QRZ.
    pub remote_id: Option<String>,
}

/// One award credit to store at import: an `award_credit_imports` row
/// (the credit as the sponsor sees it — call, date, band, mode, unit)
/// linked to the QSO it came from by an `award_credit_links` row.
#[derive(Debug, Clone)]
pub struct ImportedCredit {
    pub qso_id: QsoId,
    pub credit: AwardCredit,
    pub call: String,
    /// UTC date of the QSO, `YYYY-MM-DD`.
    pub qso_date: String,
    pub band: Option<String>,
    pub mode: Option<String>,
    /// What the credit counts towards within the award: the DXCC entity
    /// id for the DXCC family, the state for WAS, the grid square for
    /// VUCC; empty when slogger doesn't know the award's unit.
    pub unit: String,
}

/// Everything one ADIF import writes, committed together by
/// `QsoRepository::insert_qsos_batch`.
#[derive(Debug, Clone, Default)]
pub struct ImportBatch {
    /// Station locations first seen in this import. Inserted before the
    /// QSOs that link to them.
    pub locations: Vec<StationLocation>,
    pub qsos: Vec<Qso>,
    pub exchange_fields: Vec<(QsoId, QsoExchangeField)>,
    pub service_states: Vec<ImportedServiceState>,
    pub credits: Vec<ImportedCredit>,
}

/// A local QSO a QSL service's record might describe, with where it
/// already stands with that service. Loaded in bulk for a whole report
/// by `QsoRepository::load_sync_candidates`.
#[derive(Debug, Clone)]
pub struct SyncCandidate {
    pub qso_id: QsoId,
    pub call: String,
    pub station_callsign: Option<String>,
    pub qso_begin: DateTime<Utc>,
    pub band: Option<String>,
    pub mode: Option<String>,
    /// What award credit counts towards when the service doesn't say.
    pub dxcc_id: Option<u16>,
    pub state: Option<String>,
    pub grid: Option<String>,
    pub upload_verified: bool,
    pub confirmed: bool,
}

/// One QSO's state change from a sync: always upload-verified, and
/// confirmed when `confirmed_at` is set. Applied without downgrading
/// anything already recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncStateChange {
    pub qso_id: QsoId,
    /// When the service received the upload; kept only if the QSO has no
    /// upload time yet.
    pub uploaded_at: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
}

/// An award credit already on a QSO as submitted that the service now
/// reports granted. Identified by award and medium, like the credit
/// list entry it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct CreditUpgrade {
    pub qso_id: QsoId,
    pub award: String,
    pub medium: Option<String>,
}

/// Where a service's incremental report left off: the value to send as
/// `since` next time, stored per (service, report).
#[derive(Debug, Clone, PartialEq)]
pub struct SyncWatermark {
    pub service: String,
    pub key: String,
    pub value: String,
}

/// Everything one service report changes, committed together by
/// `QsoRepository::apply_sync_batch` — including the report's new
/// watermark, so it only advances once the report's results are stored.
#[derive(Debug, Clone, Default)]
pub struct SyncBatch {
    pub service: String,
    pub states: Vec<SyncStateChange>,
    pub new_credits: Vec<ImportedCredit>,
    pub credit_upgrades: Vec<CreditUpgrade>,
    pub watermark: Option<SyncWatermark>,
}

impl SyncBatch {
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
            && self.new_credits.is_empty()
            && self.credit_upgrades.is_empty()
            && self.watermark.is_none()
    }
}

/// How many QSOs at one station location are waiting for a service, split
/// by whether they carry a station callsign. What an upload that can only
/// send some locations' QSOs reports about the ones it holds back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingUploadCount {
    pub station_location_id: Option<StationLocationId>,
    pub has_station_callsign: bool,
    pub count: usize,
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

    /// Upload / confirmation records for one QSO, ordered by service.
    async fn list_service_states(&self, qso_id: &QsoId) -> RepoResult<Vec<QsoServiceState>>;

    /// Award credit linked to one QSO, granted before submitted, then by
    /// award name.
    async fn list_credits(&self, qso_id: &QsoId) -> RepoResult<Vec<AwardCredit>>;

    async fn search_qsos(&self, query: QsoSearch) -> RepoResult<Vec<QsoSummary>>;

    /// Same filter shape as `search_qsos` but returns full `Qso` records.
    /// Used when callers need every field (export, edit, bulk-update).
    async fn search_full_qsos(&self, query: QsoSearch) -> RepoResult<Vec<Qso>>;

    /// Count QSOs matching the filter without fetching them. Used for
    /// "Delete N QSOs?" confirmations and result-set previews.
    async fn count_matching(&self, query: QsoSearch) -> RepoResult<usize>;

    /// Point every QSO matching `query` at `location` in one statement.
    /// Ignores the query's sort and limit: it applies to all matches, not
    /// one page of them. Returns the number of QSOs changed.
    async fn assign_station_location(
        &self,
        query: QsoSearch,
        location: StationLocationId,
    ) -> RepoResult<usize>;

    /// Returns QSOs that have not yet been uploaded to the named service.
    /// "Not yet uploaded" = no row in qso_service_state for (qso_id, service)
    /// OR a row with upload_state = 'pending' / 'failed'.
    async fn list_pending_uploads(
        &self,
        service: &str,
        limit: Option<u32>,
    ) -> RepoResult<Vec<Qso>>;

    /// Pending uploads (same meaning as `list_pending_uploads`) counted
    /// per (station location, has a station callsign).
    async fn count_pending_uploads_by_location(
        &self,
        service: &str,
    ) -> RepoResult<Vec<PendingUploadCount>>;

    /// The newest `limit` pending uploads that are linked to one of
    /// `locations` and carry a station callsign. LoTW can only sign QSOs
    /// it can map to a TQSL location; selecting just those before the
    /// limit is applied keeps QSOs it must hold back from filling every
    /// batch and stalling the upload.
    async fn list_pending_uploads_at(
        &self,
        service: &str,
        locations: &[StationLocationId],
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

    /// Look up a QSO by station callsign + worked callsign + UTC date +
    /// band + mode — what the eQSL inbox fetch matches on. Returns at most
    /// one match — if multiple exist, the most recent qso_begin wins.
    /// (LoTW matches by time instead; see `qsl_sync::match_records`.)
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
    /// (without one the key can't tell operators apart) are returned.
    async fn load_dedup_keys(&self) -> RepoResult<std::collections::HashSet<DedupKey>>;

    /// Every station location, oldest first — what `import_qsos` matches
    /// an imported record's QTH name against. The same rows as
    /// `StationRepository::list_locations`; here so the import, which
    /// only holds this repository, can resolve names up front (one query,
    /// like `load_dedup_keys`) and write new locations in its batch.
    async fn load_station_locations(&self) -> RepoResult<Vec<StationLocation>>;

    /// Batch insert path used by ADIF import. Writes every table in the
    /// batch (station_locations, qsos, qso_exchange_fields,
    /// qso_service_state, award credit) in a single transaction so a
    /// 50k-record import is one fsync instead of ~1.5M. Per-row INSERT
    /// bodies match the existing single-row methods; only the
    /// transaction wrapping differs.
    async fn insert_qsos_batch(&self, batch: &ImportBatch) -> RepoResult<()>;

    /// Every QSO that isn't soft-deleted whose worked call is one of
    /// `calls` (uppercase), with its state at `service`. One query per
    /// chunk of calls, so matching a whole report costs a handful of
    /// round trips rather than one per record.
    async fn load_sync_candidates(
        &self,
        service: &str,
        calls: &[String],
    ) -> RepoResult<Vec<SyncCandidate>>;

    /// Award credit on each of `qso_ids`, as `list_credits` returns it,
    /// for many QSOs at once. QSOs with none are absent from the map.
    async fn list_credits_for(
        &self,
        qso_ids: &[QsoId],
    ) -> RepoResult<std::collections::HashMap<QsoId, Vec<AwardCredit>>>;

    /// Write one sync report's changes and its watermark in a single
    /// transaction.
    async fn apply_sync_batch(&self, batch: &SyncBatch) -> RepoResult<()>;

    /// The stored watermark for a service's report, if any.
    async fn get_sync_watermark(&self, service: &str, key: &str) -> RepoResult<Option<String>>;
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
}
