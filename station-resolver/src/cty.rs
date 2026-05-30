use std::io::Read;
use std::path::Path;

use radio_core::Callsign;
use thiserror::Error;

use crate::dxcc_table::dxcc_id_for_prefix;
use crate::resolution::Resolution;
use crate::resolver::Resolver;

#[derive(Debug, Error)]
pub enum CtyLoadError {
    #[error("cty.dat load error: {0}")]
    Load(String),
}

pub struct CtyDbResolver {
    inner: station_data::CtyDb,
}

impl std::fmt::Debug for CtyDbResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CtyDbResolver").finish_non_exhaustive()
    }
}

impl CtyDbResolver {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, CtyLoadError> {
        let inner = station_data::CtyDb::from_path(path.as_ref())
            .map_err(|e| CtyLoadError::Load(e.to_string()))?;
        Ok(Self { inner })
    }

    pub fn from_reader<R: Read>(reader: R) -> Result<Self, CtyLoadError> {
        let inner = station_data::CtyDb::from_reader(reader)
            .map_err(|e| CtyLoadError::Load(e.to_string()))?;
        Ok(Self { inner })
    }
}

fn strip_ssid(call: &str) -> &str {
    match call.find('-') {
        Some(i) => &call[..i],
        None => call,
    }
}

fn keep_zone(z: u8) -> Option<u8> {
    if z == 0 { None } else { Some(z) }
}

impl Resolver for CtyDbResolver {
    fn resolve(&self, call: &Callsign) -> Option<Resolution> {
        let base = strip_ssid(call.as_str());
        let resolved = self.inner.lookup(base)?;
        let dxcc_id = dxcc_id_for_prefix(&resolved.dxcc);
        Some(Resolution {
            dxcc_id,
            dxcc_prefix: Some(resolved.dxcc.clone()),
            country: None,
            continent: Some(resolved.continent.clone()),
            cq_zone: resolved.cq_zone.and_then(keep_zone),
            itu_zone: resolved.itu_zone.and_then(keep_zone),
            latitude: None,
            longitude: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI_CTY: &str = "United States of America:           05:  08:  NA:   37.53:   97.00:     5.0:  K:
    K,W,N,AA,AB,AC,AD,AE,AF,AG,AH,AI,AJ,AK,AL;
England:                            14:  27:  EU:   52.00:   -2.00:     0.0:  G:
    G,M,2E,GX,MX;
Japan:                              25:  45:  AS:   36.24: -139.00:    -9.0:  JA:
    JA,JE,JF,JG,JH,JI,JJ,JK,JL,JM,JN,JO,JP,JQ,JR,JS;
";

    fn loader() -> CtyDbResolver {
        CtyDbResolver::from_reader(MINI_CTY.as_bytes()).unwrap()
    }

    #[test]
    fn resolves_us_call() {
        let r = loader();
        let call = Callsign::parse("W1AW").unwrap();
        let res = r.resolve(&call).expect("W1AW should resolve");
        assert_eq!(res.continent.as_deref(), Some("NA"));
        assert_eq!(res.cq_zone, Some(5));
        assert_eq!(res.itu_zone, Some(8));
        assert_eq!(res.dxcc_id, Some(291));
    }

    #[test]
    fn resolves_japanese_call() {
        let r = loader();
        let call = Callsign::parse("JA1NUT").unwrap();
        let res = r.resolve(&call).expect("JA1NUT should resolve");
        assert_eq!(res.continent.as_deref(), Some("AS"));
        assert_eq!(res.cq_zone, Some(25));
        assert_eq!(res.dxcc_id, Some(339));
    }

    #[test]
    fn unknown_prefix_returns_none() {
        let r = loader();
        let call = Callsign::parse("ZZ9ZZZ").unwrap();
        assert!(r.resolve(&call).is_none());
    }

    #[test]
    fn ssid_suffix_stripped() {
        // Callsign rejects '-', so we test strip_ssid directly. Real input
        // arrives from spot/cluster pipelines as raw strings before they
        // reach Callsign::parse.
        assert_eq!(strip_ssid("W3LPL-2"), "W3LPL");
        assert_eq!(strip_ssid("W1AW"), "W1AW");
    }
}
