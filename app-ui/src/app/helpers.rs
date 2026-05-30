use app_config::StationConfig;
use radio_core::{Band, Callsign};

pub(super) fn station_call_from_config(s: &StationConfig) -> Option<Callsign> {
    s.default_callsign
        .as_deref()
        .and_then(|c| Callsign::parse(c).ok())
}

pub(super) fn option_from_str(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub(super) fn parse_mhz_to_hz(s: &str) -> Option<i64> {
    let mhz = s.trim().parse::<f64>().ok()?;
    if mhz <= 0.0 {
        return None;
    }
    Some((mhz * 1_000_000.0).round() as i64)
}

pub(super) fn default_freq_for_band(band: Band) -> Option<i64> {
    let mhz = match band {
        Band::M160 => 1.840,
        Band::M80 => 3.700,
        Band::M60 => 5.357,
        Band::M40 => 7.150,
        Band::M30 => 10.130,
        Band::M20 => 14.200,
        Band::M17 => 18.130,
        Band::M15 => 21.250,
        Band::M12 => 24.940,
        Band::M10 => 28.400,
        Band::M6 => 50.130,
        Band::M2 => 144.200,
        Band::Cm70 => 432.100,
        Band::Cm23 => 1296.100,
        _ => return None,
    };
    Some((mhz * 1_000_000.0) as i64)
}
