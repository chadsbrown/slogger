use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Callsign(String);

#[derive(Debug, Error)]
pub enum CallsignError {
    #[error("callsign is empty")]
    Empty,
    #[error("callsign contains invalid character: {0:?}")]
    InvalidChar(char),
    #[error("callsign too long: {0} > 16")]
    TooLong(usize),
}

impl Callsign {
    pub fn parse(input: &str) -> Result<Self, CallsignError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(CallsignError::Empty);
        }
        if trimmed.len() > 16 {
            return Err(CallsignError::TooLong(trimmed.len()));
        }
        for c in trimmed.chars() {
            if !c.is_ascii_alphanumeric() && c != '/' {
                return Err(CallsignError::InvalidChar(c));
            }
        }
        Ok(Self(trimmed.to_ascii_uppercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for Callsign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Callsign {
    type Err = CallsignError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Band {
    M2200,
    M630,
    M160,
    M80,
    M60,
    M40,
    M30,
    M20,
    M17,
    M15,
    M12,
    M10,
    M6,
    M4,
    M2,
    Cm125,
    Cm70,
    Cm33,
    Cm23,
    Cm13,
    Cm9,
    Cm6,
    Cm3,
    Mm1_25,
    Mm6,
}

impl Band {
    pub fn as_adif(&self) -> &'static str {
        match self {
            Self::M2200 => "2200M",
            Self::M630 => "630M",
            Self::M160 => "160M",
            Self::M80 => "80M",
            Self::M60 => "60M",
            Self::M40 => "40M",
            Self::M30 => "30M",
            Self::M20 => "20M",
            Self::M17 => "17M",
            Self::M15 => "15M",
            Self::M12 => "12M",
            Self::M10 => "10M",
            Self::M6 => "6M",
            Self::M4 => "4M",
            Self::M2 => "2M",
            Self::Cm125 => "1.25M",
            Self::Cm70 => "70CM",
            Self::Cm33 => "33CM",
            Self::Cm23 => "23CM",
            Self::Cm13 => "13CM",
            Self::Cm9 => "9CM",
            Self::Cm6 => "6CM",
            Self::Cm3 => "3CM",
            Self::Mm1_25 => "1.25CM",
            Self::Mm6 => "6MM",
        }
    }

    pub fn from_adif(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "2200M" => Some(Self::M2200),
            "630M" => Some(Self::M630),
            "160M" => Some(Self::M160),
            "80M" => Some(Self::M80),
            "60M" => Some(Self::M60),
            "40M" => Some(Self::M40),
            "30M" => Some(Self::M30),
            "20M" => Some(Self::M20),
            "17M" => Some(Self::M17),
            "15M" => Some(Self::M15),
            "12M" => Some(Self::M12),
            "10M" => Some(Self::M10),
            "6M" => Some(Self::M6),
            "4M" => Some(Self::M4),
            "2M" => Some(Self::M2),
            "1.25M" => Some(Self::Cm125),
            "70CM" => Some(Self::Cm70),
            "33CM" => Some(Self::Cm33),
            "23CM" => Some(Self::Cm23),
            "13CM" => Some(Self::Cm13),
            "9CM" => Some(Self::Cm9),
            "6CM" => Some(Self::Cm6),
            "3CM" => Some(Self::Cm3),
            "1.25CM" => Some(Self::Mm1_25),
            "6MM" => Some(Self::Mm6),
            _ => None,
        }
    }

    pub fn from_freq_hz(hz: i64) -> Option<Self> {
        let khz = hz / 1_000;
        match khz {
            135..=137 => Some(Self::M2200),
            472..=479 => Some(Self::M630),
            1_800..=2_000 => Some(Self::M160),
            3_500..=4_000 => Some(Self::M80),
            5_330..=5_410 => Some(Self::M60),
            7_000..=7_300 => Some(Self::M40),
            10_100..=10_150 => Some(Self::M30),
            14_000..=14_350 => Some(Self::M20),
            18_068..=18_168 => Some(Self::M17),
            21_000..=21_450 => Some(Self::M15),
            24_890..=24_990 => Some(Self::M12),
            28_000..=29_700 => Some(Self::M10),
            50_000..=54_000 => Some(Self::M6),
            70_000..=70_500 => Some(Self::M4),
            144_000..=148_000 => Some(Self::M2),
            222_000..=225_000 => Some(Self::Cm125),
            420_000..=450_000 => Some(Self::Cm70),
            902_000..=928_000 => Some(Self::Cm33),
            1_240_000..=1_300_000 => Some(Self::Cm23),
            _ => None,
        }
    }
}

impl fmt::Display for Band {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_adif())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    CW,
    SSB,
    AM,
    FM,
    RTTY,
    PSK,
    FT8,
    FT4,
    JT65,
    JT9,
    JS8,
    MFSK,
    OLIVIA,
    DIGITALVOICE,
    Other(String),
}

impl Mode {
    pub fn as_adif(&self) -> &str {
        match self {
            Self::CW => "CW",
            Self::SSB => "SSB",
            Self::AM => "AM",
            Self::FM => "FM",
            Self::RTTY => "RTTY",
            Self::PSK => "PSK",
            Self::FT8 => "FT8",
            Self::FT4 => "FT4",
            Self::JT65 => "JT65",
            Self::JT9 => "JT9",
            Self::JS8 => "JS8",
            Self::MFSK => "MFSK",
            Self::OLIVIA => "OLIVIA",
            Self::DIGITALVOICE => "DIGITALVOICE",
            Self::Other(s) => s.as_str(),
        }
    }

    pub fn from_adif(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "CW" => Self::CW,
            "SSB" | "USB" | "LSB" => Self::SSB,
            "AM" => Self::AM,
            "FM" => Self::FM,
            "RTTY" => Self::RTTY,
            "PSK" | "PSK31" | "PSK63" | "PSK125" => Self::PSK,
            "FT8" => Self::FT8,
            "FT4" => Self::FT4,
            "JT65" => Self::JT65,
            "JT9" => Self::JT9,
            "JS8" => Self::JS8,
            "MFSK" => Self::MFSK,
            "OLIVIA" => Self::OLIVIA,
            "DIGITALVOICE" => Self::DIGITALVOICE,
            other => Self::Other(other.to_string()),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_adif())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropagationMode {
    Terrestrial,
    Satellite,
    Eme,
    MeteorScatter,
    Aurora,
    AircraftScatter,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldSource {
    OperatorEntered,
    RigDerived,
    ImportedAdif,
    StationDataResolved,
    ServiceSync(String),
    ManualOverride,
}

impl FieldSource {
    pub fn as_storage_str(&self) -> String {
        match self {
            Self::OperatorEntered => "operator_entered".into(),
            Self::RigDerived => "rig_derived".into(),
            Self::ImportedAdif => "imported_adif".into(),
            Self::StationDataResolved => "station_data_resolved".into(),
            Self::ServiceSync(svc) => format!("service_sync:{svc}"),
            Self::ManualOverride => "manual_override".into(),
        }
    }

    pub fn parse_storage(s: &str) -> Self {
        if let Some(rest) = s.strip_prefix("service_sync:") {
            return Self::ServiceSync(rest.to_string());
        }
        match s {
            "operator_entered" => Self::OperatorEntered,
            "rig_derived" => Self::RigDerived,
            "imported_adif" => Self::ImportedAdif,
            "station_data_resolved" => Self::StationDataResolved,
            "manual_override" => Self::ManualOverride,
            _ => Self::OperatorEntered,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callsign_uppercases() {
        let c = Callsign::parse("w1aw").unwrap();
        assert_eq!(c.as_str(), "W1AW");
    }

    #[test]
    fn callsign_allows_slash_for_portable() {
        let c = Callsign::parse("VE3/W1AW").unwrap();
        assert_eq!(c.as_str(), "VE3/W1AW");
    }

    #[test]
    fn callsign_rejects_empty() {
        assert!(matches!(Callsign::parse("   "), Err(CallsignError::Empty)));
    }

    #[test]
    fn callsign_rejects_invalid_chars() {
        assert!(matches!(
            Callsign::parse("W1AW!"),
            Err(CallsignError::InvalidChar('!'))
        ));
    }

    #[test]
    fn band_from_freq_classifies_20m() {
        assert_eq!(Band::from_freq_hz(14_074_000), Some(Band::M20));
    }

    #[test]
    fn band_adif_roundtrip() {
        for band in [Band::M160, Band::M40, Band::M20, Band::M2, Band::Cm70] {
            let adif = band.as_adif();
            assert_eq!(Band::from_adif(adif), Some(band));
        }
    }

    #[test]
    fn mode_from_adif_normalizes_ssb() {
        assert_eq!(Mode::from_adif("USB"), Mode::SSB);
        assert_eq!(Mode::from_adif("LSB"), Mode::SSB);
    }

    #[test]
    fn field_source_storage_roundtrip() {
        let original = FieldSource::ServiceSync("lotw".into());
        let stored = original.as_storage_str();
        assert_eq!(FieldSource::parse_storage(&stored), original);
    }
}
