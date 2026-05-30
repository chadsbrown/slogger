use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{OperatorId, StationLocationId};
use crate::value::Callsign;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operator {
    pub id: OperatorId,
    pub callsign: Callsign,
    pub name: Option<String>,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationLocation {
    pub id: StationLocationId,
    pub name: String,

    pub station_callsign: Option<Callsign>,
    pub owner_callsign: Option<Callsign>,

    pub city: Option<String>,
    pub county: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub grid: Option<String>,

    pub latitude: Option<f64>,
    pub longitude: Option<f64>,

    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    pub iota: Option<String>,

    pub lotw_station_location: Option<String>,
    pub eqsl_account: Option<String>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
