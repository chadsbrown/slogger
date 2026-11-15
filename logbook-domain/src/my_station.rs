//! The operator's own station as an ADIF record describes it, and how
//! that maps onto `station_locations`.
//!
//! DXKeeper names the "my QTH" a QSO was made from in
//! `APP_DXKEEPER_MY_QTHID` and keeps the details in its own table, so
//! its exports carry the name alone. Other loggers write the standard
//! `MY_*` fields instead. Import turns either into a link to a
//! `station_locations` row (see `LogbookService::import_qsos`); export
//! writes both back from the linked row.

use radio_core::StationLocation;

/// DXKeeper's name for the station location a QSO was made from.
pub const QTH_ID_FIELD: &str = "APP_DXKEEPER_MY_QTHID";

/// The standard ADIF fields describing the operator's own station that
/// `station_locations` has a column for.
pub const MY_FIELDS: &[&str] = &[
    "MY_GRIDSQUARE",
    "MY_CITY",
    "MY_CNTY",
    "MY_STATE",
    "MY_COUNTRY",
    "MY_CQ_ZONE",
    "MY_ITU_ZONE",
    "MY_IOTA",
    "MY_LAT",
    "MY_LON",
];

/// The station location an imported record names, resolved against the
/// existing locations at import time. Matching is by exact name.
#[derive(Debug, Clone, PartialEq)]
pub struct StationLocationRef {
    pub name: String,
    /// The record's `MY_*` fields, raw. They fill in a location that the
    /// import creates; any that the linked location does not reproduce
    /// stay on the QSO as extra fields so nothing is lost.
    pub my_fields: Vec<(String, String)>,
}

impl StationLocationRef {
    /// Build from a record's QTH id and `MY_*` fields. Without a QTH id
    /// the name is derived from the `MY_*` fields (city and grid, else
    /// county / state / country, else IOTA). Returns `None` when the
    /// record says nothing about where it was made: slogger does not
    /// guess a location, the operator assigns one afterwards.
    pub fn from_record(qth_id: Option<&str>, my_fields: Vec<(String, String)>) -> Option<Self> {
        let name = match qth_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => id.to_string(),
            None => derive_name(&my_fields)?,
        };
        Some(Self { name, my_fields })
    }
}

fn derive_name(fields: &[(String, String)]) -> Option<String> {
    let get = |name: &str| {
        fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.trim())
            .filter(|v| !v.is_empty())
    };
    let join = |names: &[&str], sep: &str| {
        let parts: Vec<&str> = names.iter().filter_map(|n| get(n)).collect();
        (!parts.is_empty()).then(|| parts.join(sep))
    };
    join(&["MY_CITY", "MY_GRIDSQUARE"], " ")
        .or_else(|| join(&["MY_CNTY", "MY_STATE", "MY_COUNTRY"], ", "))
        .or_else(|| get("MY_IOTA").map(String::from))
}

/// Fill a newly created location's details from `MY_*` fields. Values
/// that don't parse (a malformed `MY_LAT`, say) are left unset — the
/// caller keeps those as extra fields on the QSO.
pub fn fill_location(loc: &mut StationLocation, fields: &[(String, String)]) {
    for (name, value) in fields {
        let v = value.trim();
        if v.is_empty() {
            continue;
        }
        match name.as_str() {
            "MY_GRIDSQUARE" => loc.grid = Some(v.to_string()),
            "MY_CITY" => loc.city = Some(v.to_string()),
            "MY_CNTY" => loc.county = Some(v.to_string()),
            "MY_STATE" => loc.state = Some(v.to_string()),
            "MY_COUNTRY" => loc.country = Some(v.to_string()),
            "MY_CQ_ZONE" => loc.cq_zone = v.parse().ok(),
            "MY_ITU_ZONE" => loc.itu_zone = v.parse().ok(),
            "MY_IOTA" => loc.iota = Some(v.to_string()),
            "MY_LAT" => loc.latitude = parse_latitude(v),
            "MY_LON" => loc.longitude = parse_longitude(v),
            _ => {}
        }
    }
}

/// The `MY_*` fields export writes for a location: every detail it has.
pub fn location_fields(loc: &StationLocation) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut push = |name: &'static str, v: Option<String>| {
        if let Some(v) = v {
            out.push((name, v));
        }
    };
    push("MY_GRIDSQUARE", loc.grid.clone());
    push("MY_CITY", loc.city.clone());
    push("MY_CNTY", loc.county.clone());
    push("MY_STATE", loc.state.clone());
    push("MY_COUNTRY", loc.country.clone());
    push("MY_CQ_ZONE", loc.cq_zone.map(|z| z.to_string()));
    push("MY_ITU_ZONE", loc.itu_zone.map(|z| z.to_string()));
    push("MY_IOTA", loc.iota.clone());
    push("MY_LAT", loc.latitude.map(format_latitude));
    push("MY_LON", loc.longitude.map(format_longitude));
    out
}

/// True when exporting `loc` reproduces the record's `name` = `value`,
/// i.e. the field can be dropped from the QSO's extras without loss.
/// Compares parsed values so `EM48ku` / `EM48KU` or two spellings of the
/// same latitude count as the same.
pub fn location_covers(loc: &StationLocation, name: &str, value: &str) -> bool {
    let v = value.trim();
    let same_text = |stored: &Option<String>| {
        stored.as_deref().is_some_and(|s| s.trim().eq_ignore_ascii_case(v))
    };
    let same_coord = |stored: Option<f64>, parsed: Option<f64>| match (stored, parsed) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-6,
        _ => false,
    };
    match name {
        "MY_GRIDSQUARE" => same_text(&loc.grid),
        "MY_CITY" => same_text(&loc.city),
        "MY_CNTY" => same_text(&loc.county),
        "MY_STATE" => same_text(&loc.state),
        "MY_COUNTRY" => same_text(&loc.country),
        "MY_IOTA" => same_text(&loc.iota),
        "MY_CQ_ZONE" => loc.cq_zone.is_some() && loc.cq_zone == v.parse().ok(),
        "MY_ITU_ZONE" => loc.itu_zone.is_some() && loc.itu_zone == v.parse().ok(),
        "MY_LAT" => same_coord(loc.latitude, parse_latitude(v)),
        "MY_LON" => same_coord(loc.longitude, parse_longitude(v)),
        _ => false,
    }
}

/// Parse an ADIF Location latitude (`N041 23.456`: hemisphere, three
/// digit degrees, space, decimal minutes) into decimal degrees, south
/// negative.
pub fn parse_latitude(s: &str) -> Option<f64> {
    parse_location(s, 'N', 'S', 90.0)
}

/// As `parse_latitude`, for longitude (`W087 12.345`), west negative.
pub fn parse_longitude(s: &str) -> Option<f64> {
    parse_location(s, 'E', 'W', 180.0)
}

fn parse_location(s: &str, pos: char, neg: char, limit: f64) -> Option<f64> {
    let s = s.trim();
    let hemi = s.chars().next()?.to_ascii_uppercase();
    let sign = if hemi == pos {
        1.0
    } else if hemi == neg {
        -1.0
    } else {
        return None;
    };
    let mut parts = s[1..].split_whitespace();
    let degrees: u32 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(0.0..60.0).contains(&minutes) {
        return None;
    }
    let value = degrees as f64 + minutes / 60.0;
    (value <= limit).then_some(sign * value)
}

/// Decimal degrees to ADIF Location latitude, e.g. `N041 23.456`.
pub fn format_latitude(value: f64) -> String {
    format_location(value, 'N', 'S')
}

/// Decimal degrees to ADIF Location longitude, e.g. `W087 12.345`.
pub fn format_longitude(value: f64) -> String {
    format_location(value, 'E', 'W')
}

fn format_location(value: f64, pos: char, neg: char) -> String {
    let hemi = if value < 0.0 { neg } else { pos };
    // Whole thousandths of a minute, so rounding can carry into the
    // degrees instead of printing "60.000".
    let thousandths = (value.abs() * 60_000.0).round() as u64;
    let degrees = thousandths / 60_000;
    let rem = thousandths % 60_000;
    format!("{hemi}{degrees:03} {:02}.{:03}", rem / 1000, rem % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adif_locations_with_sign_by_hemisphere() {
        let lat = parse_latitude("N041 23.456").unwrap();
        assert!((lat - (41.0 + 23.456 / 60.0)).abs() < 1e-9);
        let lon = parse_longitude("W087 12.345").unwrap();
        assert!((lon + (87.0 + 12.345 / 60.0)).abs() < 1e-9);
        assert!(parse_latitude("S033 52.000").unwrap() < 0.0);
        assert!(parse_longitude("E151 12.500").unwrap() > 0.0);
    }

    #[test]
    fn rejects_malformed_locations() {
        assert_eq!(parse_latitude("W087 12.345"), None, "longitude hemisphere");
        assert_eq!(parse_latitude("N091 00.000"), None, "beyond the pole");
        assert_eq!(parse_latitude("N041 61.000"), None, "minutes over 60");
        assert_eq!(parse_latitude("41.5"), None, "decimal degrees");
        assert_eq!(parse_latitude(""), None);
    }

    #[test]
    fn format_round_trips_and_carries_rounding() {
        for s in ["N041 23.456", "S000 00.001", "N090 00.000"] {
            assert_eq!(format_latitude(parse_latitude(s).unwrap()), s);
        }
        for s in ["W087 12.345", "E151 12.500", "W180 00.000"] {
            assert_eq!(format_longitude(parse_longitude(s).unwrap()), s);
        }
        assert_eq!(format_latitude(41.999_999_9), "N042 00.000");
    }

    #[test]
    fn location_name_prefers_qth_id_then_city_and_grid() {
        let my = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect::<Vec<_>>()
        };
        let r = StationLocationRef::from_record(Some("Home"), my(&[("MY_GRIDSQUARE", "FN31")]));
        assert_eq!(r.unwrap().name, "Home");
        let r = StationLocationRef::from_record(
            None,
            my(&[("MY_GRIDSQUARE", "FN31pr"), ("MY_CITY", "Newington")]),
        );
        assert_eq!(r.unwrap().name, "Newington FN31pr");
        let r = StationLocationRef::from_record(None, my(&[("MY_STATE", "CT")]));
        assert_eq!(r.unwrap().name, "CT");
        assert_eq!(StationLocationRef::from_record(None, Vec::new()), None);
        assert_eq!(StationLocationRef::from_record(Some("  "), Vec::new()), None);
    }
}
