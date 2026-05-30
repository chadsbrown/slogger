use chrono::{DateTime, Utc};

use radio_core::{
    Band, Callsign, Mode, OperatorId, PropagationMode, QsoExchangeField, StationLocationId,
};

#[derive(Debug, Clone)]
pub struct CreateQsoCommand {
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

    pub exchange_fields: Vec<QsoExchangeField>,
}

impl CreateQsoCommand {
    pub fn minimal(call: Callsign, qso_begin: DateTime<Utc>) -> Self {
        Self {
            call,
            qso_begin,
            qso_end: None,
            band: None,
            freq_hz: None,
            mode: None,
            submode: None,
            rst_sent: None,
            rst_rcvd: None,
            operator_id: None,
            station_location_id: None,
            station_callsign: None,
            owner_callsign: None,
            dxcc_id: None,
            dxcc_prefix: None,
            continent: None,
            cq_zone: None,
            itu_zone: None,
            grid: None,
            state: None,
            county: None,
            province: None,
            iota: None,
            tx_power_w: None,
            rx_power_w: None,
            propagation_mode: None,
            sat_name: None,
            sat_mode: None,
            exchange_fields: Vec::new(),
        }
    }
}
