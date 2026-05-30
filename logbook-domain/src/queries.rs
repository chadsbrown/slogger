use chrono::{DateTime, Utc};

use radio_core::{Band, Callsign, Mode, QsoId, StationLocationId};

#[derive(Debug, Clone, Default)]
pub struct QsoSearch {
    pub call_prefix: Option<String>,
    pub exact_call: Option<Callsign>,
    pub band: Option<Band>,
    pub mode: Option<Mode>,
    pub dxcc_id: Option<u16>,
    pub station_location_id: Option<StationLocationId>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// US state filter (case-sensitive on stored value, typically 2-letter).
    pub state: Option<String>,
    /// IOTA reference (e.g. "EU-005").
    pub iota: Option<String>,
    /// Continent code ("NA"/"EU"/"AS"/...).
    pub continent: Option<String>,
    /// When `Some(true)`, only QSOs that are confirmed via LotW. When
    /// `Some(false)`, only QSOs that are NOT confirmed via LotW (regardless
    /// of upload state). When `None`, no filter on confirmation.
    pub lotw_confirmed: Option<bool>,
    /// Result ordering. Default (and the historic behavior) is
    /// `QsoBeginDesc` — most-recent QSOs first.
    pub sort: SortOrder,
    pub limit: Option<u32>,
}

/// Result ordering for `search_qsos` / `search_full_qsos` /
/// `count_matching`. Count doesn't actually order, but accepting the
/// same QsoSearch shape keeps the API symmetric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    QsoBeginDesc,
    QsoBeginAsc,
    CallAsc,
    CallDesc,
    BandAsc,
    BandDesc,
}

impl SortOrder {
    pub fn sql_clause(self) -> &'static str {
        match self {
            Self::QsoBeginDesc => "q.qso_begin DESC",
            Self::QsoBeginAsc => "q.qso_begin ASC",
            Self::CallAsc => "q.call ASC",
            Self::CallDesc => "q.call DESC",
            Self::BandAsc => "q.band ASC",
            Self::BandDesc => "q.band DESC",
        }
    }
}

#[derive(Debug, Clone)]
pub struct QsoSummary {
    pub id: QsoId,
    pub call: Callsign,
    pub qso_begin: DateTime<Utc>,
    pub band: Option<Band>,
    pub mode: Option<Mode>,
    pub freq_hz: Option<i64>,
    pub dxcc_id: Option<u16>,
    pub dxcc_prefix: Option<String>,
    pub continent: Option<String>,
    pub cq_zone: Option<u8>,
}
