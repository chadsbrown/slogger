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

    pub dxcc_id: Option<u16>,
    pub dxcc_prefix: Option<String>,
    pub continent: Option<String>,
    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    pub grid: Option<String>,
    pub state: Option<String>,
    pub county: Option<String>,
    pub province: Option<String>,
    pub iota: Option<String>,

    pub tx_power_w: Option<f32>,
    pub rx_power_w: Option<f32>,

    pub propagation_mode: Option<PropagationMode>,
    pub sat_name: Option<String>,
    pub sat_mode: Option<String>,

    pub distance_km: Option<f64>,
    pub bearing_deg: Option<f64>,

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
