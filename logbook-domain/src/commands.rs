use chrono::{DateTime, Utc};

use radio_core::{
    Band, Callsign, Mode, OperatorId, PropagationMode, QsoExchangeField, StationLocationId,
};

use crate::credit::AwardCredit;
use crate::my_station::StationLocationRef;

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
    pub operator_callsign: Option<Callsign>,

    pub dxcc_id: Option<u16>,
    pub dxcc_prefix: Option<String>,
    pub wpx_prefix: Option<String>,
    pub continent: Option<String>,
    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    pub grid: Option<String>,
    pub state: Option<String>,
    pub county: Option<String>,
    pub province: Option<String>,
    pub iota: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,

    pub name: Option<String>,
    pub qth: Option<String>,
    pub address: Option<String>,
    pub email: Option<String>,
    pub age: Option<u16>,
    pub qsl_via: Option<String>,
    pub comment: Option<String>,

    pub contest_id: Option<String>,
    pub serial_sent: Option<i64>,
    pub serial_rcvd: Option<i64>,
    pub exchange_sent: Option<String>,
    pub exchange_rcvd: Option<String>,
    pub arrl_section: Option<String>,

    pub band_rx: Option<Band>,
    pub freq_rx_hz: Option<i64>,

    pub tx_power_w: Option<f32>,
    pub rx_power_w: Option<f32>,

    pub propagation_mode: Option<PropagationMode>,
    pub sat_name: Option<String>,
    pub sat_mode: Option<String>,

    pub distance_km: Option<f64>,
    pub bearing_deg: Option<f64>,

    pub sfi: Option<u16>,
    pub a_index: Option<u16>,
    pub k_index: Option<u8>,

    /// Set by ADIF import: the station location the record names, for
    /// `import_qsos` to link (creating it if new). Ignored when
    /// `station_location_id` is already set.
    pub station_location_ref: Option<StationLocationRef>,
    /// Award credits from ADIF `CREDIT_GRANTED` / `CREDIT_SUBMITTED`.
    /// Stored by `import_qsos`; `create_qso` does not record credit.
    pub credits: Vec<AwardCredit>,

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
            operator_callsign: None,
            dxcc_id: None,
            dxcc_prefix: None,
            wpx_prefix: None,
            continent: None,
            cq_zone: None,
            itu_zone: None,
            grid: None,
            state: None,
            county: None,
            province: None,
            iota: None,
            latitude: None,
            longitude: None,
            name: None,
            qth: None,
            address: None,
            email: None,
            age: None,
            qsl_via: None,
            comment: None,
            contest_id: None,
            serial_sent: None,
            serial_rcvd: None,
            exchange_sent: None,
            exchange_rcvd: None,
            arrl_section: None,
            band_rx: None,
            freq_rx_hz: None,
            tx_power_w: None,
            rx_power_w: None,
            propagation_mode: None,
            sat_name: None,
            sat_mode: None,
            distance_km: None,
            bearing_deg: None,
            sfi: None,
            a_index: None,
            k_index: None,
            station_location_ref: None,
            credits: Vec::new(),
            exchange_fields: Vec::new(),
        }
    }

    /// A command that would recreate `qso` as stored — the starting point
    /// for an edit that changes a few fields, so the rest survive
    /// `LogbookService::update_qso` (which replaces the whole record).
    pub fn from_qso(qso: &radio_core::Qso) -> Self {
        let q = qso.clone();
        Self {
            call: q.call,
            qso_begin: q.qso_begin,
            qso_end: q.qso_end,
            band: q.band,
            freq_hz: q.freq_hz,
            mode: q.mode,
            submode: q.submode,
            rst_sent: q.rst_sent,
            rst_rcvd: q.rst_rcvd,
            operator_id: q.operator_id,
            station_location_id: q.station_location_id,
            station_callsign: q.station_callsign,
            owner_callsign: q.owner_callsign,
            operator_callsign: q.operator_callsign,
            dxcc_id: q.dxcc_id,
            dxcc_prefix: q.dxcc_prefix,
            wpx_prefix: q.wpx_prefix,
            continent: q.continent,
            cq_zone: q.cq_zone,
            itu_zone: q.itu_zone,
            grid: q.grid,
            state: q.state,
            county: q.county,
            province: q.province,
            iota: q.iota,
            latitude: q.latitude,
            longitude: q.longitude,
            name: q.name,
            qth: q.qth,
            address: q.address,
            email: q.email,
            age: q.age,
            qsl_via: q.qsl_via,
            comment: q.comment,
            contest_id: q.contest_id,
            serial_sent: q.serial_sent,
            serial_rcvd: q.serial_rcvd,
            exchange_sent: q.exchange_sent,
            exchange_rcvd: q.exchange_rcvd,
            arrl_section: q.arrl_section,
            band_rx: q.band_rx,
            freq_rx_hz: q.freq_rx_hz,
            tx_power_w: q.tx_power_w,
            rx_power_w: q.rx_power_w,
            propagation_mode: q.propagation_mode,
            sat_name: q.sat_name,
            sat_mode: q.sat_mode,
            distance_km: q.distance_km,
            bearing_deg: q.bearing_deg,
            sfi: q.sfi,
            a_index: q.a_index,
            k_index: q.k_index,
            station_location_ref: None,
            credits: Vec::new(),
            exchange_fields: Vec::new(),
        }
    }
}
