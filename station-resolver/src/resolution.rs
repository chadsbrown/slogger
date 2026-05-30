use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resolution {
    pub dxcc_id: Option<u16>,
    pub dxcc_prefix: Option<String>,
    pub country: Option<String>,
    pub continent: Option<String>,
    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

impl Resolution {
    pub fn empty() -> Self {
        Self {
            dxcc_id: None,
            dxcc_prefix: None,
            country: None,
            continent: None,
            cq_zone: None,
            itu_zone: None,
            latitude: None,
            longitude: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.dxcc_id.is_none()
            && self.dxcc_prefix.is_none()
            && self.country.is_none()
            && self.continent.is_none()
            && self.cq_zone.is_none()
            && self.itu_zone.is_none()
            && self.latitude.is_none()
            && self.longitude.is_none()
    }
}
