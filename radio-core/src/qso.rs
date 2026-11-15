use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{OperatorId, QsoId, StationLocationId};
use crate::value::{Band, Callsign, FieldSource, Mode, PropagationMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Qso {
    pub id: QsoId,

    pub call: Callsign,
    pub qso_begin: DateTime<Utc>,
    pub qso_end: Option<DateTime<Utc>>,

    pub band: Option<Band>,
    pub freq_hz: Option<i64>,
    pub mode: Option<Mode>,
    pub submode: Option<String>,

    pub rst_sent: Option<String>,
    pub rst_rcvd: Option<String>,

    pub operator_id: Option<OperatorId>,
    pub station_location_id: Option<StationLocationId>,

    pub station_callsign: Option<Callsign>,
    pub owner_callsign: Option<Callsign>,
    /// The callsign of whoever operated (ADIF `OPERATOR`) — distinct
    /// from the owner when a guest operates someone else's station.
    pub operator_callsign: Option<Callsign>,

    pub dxcc_id: Option<u16>,
    /// DXCC entity prefix (e.g. `W`, `VP2E`) — DXKeeper's
    /// `APP_DXKEEPER_DXCCPREFIX`, or the resolver's when not imported.
    pub dxcc_prefix: Option<String>,
    /// CQ WPX prefix of the worked call (ADIF `PFX`, e.g. `W1`, `VP2E`).
    pub wpx_prefix: Option<String>,
    pub continent: Option<String>,
    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    pub grid: Option<String>,
    pub state: Option<String>,
    pub county: Option<String>,
    pub province: Option<String>,
    pub iota: Option<String>,
    /// The other station's position in decimal degrees (south / west
    /// negative), from ADIF `LAT` / `LON`.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,

    // The other operator, as looked up or logged at the time.
    pub name: Option<String>,
    pub qth: Option<String>,
    pub address: Option<String>,
    pub email: Option<String>,
    pub age: Option<u16>,
    pub qsl_via: Option<String>,
    pub comment: Option<String>,

    // Contest exchange. `contest_id` is stored as logged (ADIF has a
    // canonical list but loggers disagree on case).
    pub contest_id: Option<String>,
    pub serial_sent: Option<i64>,
    pub serial_rcvd: Option<i64>,
    pub exchange_sent: Option<String>,
    pub exchange_rcvd: Option<String>,
    pub arrl_section: Option<String>,

    /// Receive side, for split / cross-band contacts (ADIF `BAND_RX`,
    /// `FREQ_RX`). Usually equal to the transmit side when present.
    pub band_rx: Option<Band>,
    pub freq_rx_hz: Option<i64>,

    pub tx_power_w: Option<f32>,
    pub rx_power_w: Option<f32>,

    pub propagation_mode: Option<PropagationMode>,
    pub sat_name: Option<String>,
    pub sat_mode: Option<String>,

    pub distance_km: Option<f64>,
    pub bearing_deg: Option<f64>,

    // Space weather at the time of the QSO (ADIF `SFI`, `A_INDEX`,
    // `K_INDEX`).
    pub sfi: Option<u16>,
    pub a_index: Option<u16>,
    pub k_index: Option<u8>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QsoExchangeField {
    pub name: String,
    pub raw_value: String,
    pub normalized_value: Option<String>,
    pub source: FieldSource,
}
