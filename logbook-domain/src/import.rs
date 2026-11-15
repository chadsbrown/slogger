use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use thiserror::Error;

use radio_core::{Band, Callsign, FieldSource, Mode, PropagationMode, QsoExchangeField};

use crate::commands::CreateQsoCommand;
use crate::credit::{CreditStatus, parse_credit_list};
use crate::my_station::{MY_FIELDS, QTH_ID_FIELD, StationLocationRef, parse_latitude, parse_longitude};

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("ADIF parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Default)]
pub struct ImportOutcome {
    pub commands: Vec<CreateQsoCommand>,
    pub skipped: Vec<SkippedRecord>,
}

#[derive(Debug)]
pub struct SkippedRecord {
    pub index: usize,
    pub reason: String,
}

pub fn parse_adif(input: &str) -> Result<ImportOutcome, ImportError> {
    let records = fast_parse_adi(input);
    let mut outcome = ImportOutcome::default();
    for (idx, record) in records.into_iter().enumerate() {
        match record_to_command(&record) {
            Ok(cmd) => outcome.commands.push(cmd),
            Err(reason) => outcome.skipped.push(SkippedRecord { index: idx, reason }),
        }
    }
    Ok(outcome)
}

/// Parsed ADIF record as a flat list of (field_name, field_value)
/// pairs. Field names are normalized to uppercase by the parser to
/// match downstream lookups.
type ParsedRecord = Vec<(String, String)>;

/// Fast in-house ADIF parser. The upstream `adif_parser` crate calls
/// `remaining.to_uppercase()` on the entire remaining input twice per
/// parsed field (once for the EOR check, once for EOF) in its
/// `check_tag` helper. That's O(N²) on input size, and a 41 MB / ~50k
/// QSO export from DXKeeper takes hours to parse.
///
/// This walker scans bytes once, allocates only per captured (name,
/// value) pair, and uses `eq_ignore_ascii_case` on the tag-name byte
/// slice rather than uppercasing the input buffer. Empirically: a 41
/// MB DXKeeper file parses in under a second.
fn fast_parse_adi(input: &str) -> Vec<ParsedRecord> {
    let bytes = input.as_bytes();
    let mut records: Vec<ParsedRecord> = Vec::new();
    let mut current: ParsedRecord = Vec::new();

    // Anything before <EOH> is preamble or header — we don't need it.
    // Files without a header start at byte 0 (the loop's next-'<' scan
    // will find the first tag).
    let mut pos = find_marker_end(bytes, b"EOH").unwrap_or(0);

    while pos < bytes.len() {
        let lt = match find_byte(&bytes[pos..], b'<') {
            Some(i) => pos + i,
            None => break,
        };
        pos = lt + 1;

        // Read tag name until ':' or '>'.
        let name_start = pos;
        while pos < bytes.len() && bytes[pos] != b':' && bytes[pos] != b'>' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let name_bytes = &bytes[name_start..pos];

        // Marker tag (no length spec): <EOR>, <EOF>, …
        if bytes[pos] == b'>' {
            pos += 1;
            if name_bytes.eq_ignore_ascii_case(b"EOR") {
                if !current.is_empty() {
                    records.push(std::mem::take(&mut current));
                }
            } else if name_bytes.eq_ignore_ascii_case(b"EOF") {
                break;
            }
            // Unknown markers are silently skipped — matches the
            // upstream parser's tolerance.
            continue;
        }

        // Field with length: <NAME:LEN[:TYPE]>VALUE
        pos += 1; // skip ':'
        let len_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        let length: usize = std::str::from_utf8(&bytes[len_start..pos])
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // Optional type indicator: <NAME:LEN:T>
        if pos < bytes.len() && bytes[pos] == b':' {
            pos += 1;
            if pos < bytes.len() {
                pos += 1; // type character
            }
        }

        // Advance to '>'.
        while pos < bytes.len() && bytes[pos] != b'>' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        pos += 1;

        // Read VALUE (exactly `length` bytes, clamped to remaining
        // input for tolerance against truncated files).
        let value_end = (pos + length).min(bytes.len());
        let value = if length == 0 {
            String::new()
        } else {
            String::from_utf8_lossy(&bytes[pos..value_end]).into_owned()
        };
        pos = value_end;

        if name_bytes.is_empty() {
            continue;
        }
        let mut name = String::with_capacity(name_bytes.len());
        for &b in name_bytes {
            name.push(b.to_ascii_uppercase() as char);
        }
        current.push((name, value));
    }

    // Trailing record without an explicit <EOR>.
    if !current.is_empty() {
        records.push(current);
    }
    records
}

/// Find the first occurrence of `byte` in `haystack`, returning its
/// offset. LLVM auto-vectorizes byte-position scans on Iterator slices,
/// so this is effectively SIMD without pulling in the `memchr` crate.
fn find_byte(haystack: &[u8], byte: u8) -> Option<usize> {
    haystack.iter().position(|&b| b == byte)
}

/// Find a `<TARGET>` marker tag in `bytes` and return the byte offset
/// *after* the closing `>`. Returns None if not found. Case-insensitive
/// on `target`.
fn find_marker_end(bytes: &[u8], target: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let after_lt = i + 1;
        if after_lt + target.len() < bytes.len()
            && bytes[after_lt..after_lt + target.len()].eq_ignore_ascii_case(target)
            && bytes[after_lt + target.len()] == b'>'
        {
            return Some(after_lt + target.len() + 1);
        }
        i += 1;
    }
    None
}

fn lookup<'a>(record: &'a [(String, String)], name: &str) -> Option<&'a str> {
    record
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// Map one ADIF record onto a `CreateQsoCommand`.
///
/// Each field the match below names goes to its first-class slot (or,
/// for `MY_*` / the DXKeeper QTH id, to the station-location reference;
/// for `CREDIT_*`, to award credit). Everything else — and any mapped
/// field whose value doesn't fit its column, like a non-numeric `STX` —
/// goes into `exchange_fields` with `FieldSource::ImportedAdif`, raw, so
/// nothing in the file is dropped. The QSL-service fields
/// (`LOTW_QSL_SENT`, `QRZCOM_QSO_UPLOAD_STATUS`, …) deliberately fall
/// through too: `extract_service_states` reads them from there at import
/// and they are written back out from there on export.
fn record_to_command(record: &[(String, String)]) -> Result<CreateQsoCommand, String> {
    let call_str = lookup(record, "CALL").ok_or_else(|| "missing CALL".to_string())?;
    let call = Callsign::parse(call_str).map_err(|e| format!("invalid CALL {call_str:?}: {e}"))?;

    let date = lookup(record, "QSO_DATE").ok_or_else(|| "missing QSO_DATE".to_string())?;
    let time = lookup(record, "TIME_ON").unwrap_or("0000");
    let qso_begin = parse_adif_datetime(date, time)
        .ok_or_else(|| format!("invalid QSO_DATE/TIME_ON {date:?} {time:?}"))?;

    let mut cmd = CreateQsoCommand::minimal(call, qso_begin);
    let mut unmapped: Vec<&(String, String)> = Vec::new();
    let mut end_fields: Vec<&(String, String)> = Vec::new();
    let mut qth_id: Option<&str> = None;
    let mut my_fields: Vec<(String, String)> = Vec::new();

    for field in record {
        let (name, raw) = (field.0.as_str(), field.1.as_str());
        let v = raw.trim();
        if v.is_empty() {
            continue;
        }
        let c = &mut cmd;
        let mapped = match name {
            "CALL" | "QSO_DATE" | "TIME_ON" => true,
            "QSO_DATE_OFF" | "TIME_OFF" => {
                end_fields.push(field);
                true
            }
            "BAND" => set(&mut c.band, Band::from_adif(v)),
            "FREQ" => set(&mut c.freq_hz, parse_freq_mhz(v)),
            "MODE" => set(&mut c.mode, Some(Mode::from_adif(v))),
            "SUBMODE" => text(&mut c.submode, raw),
            "RST_SENT" => text(&mut c.rst_sent, raw),
            "RST_RCVD" => text(&mut c.rst_rcvd, raw),
            "STATION_CALLSIGN" => set(&mut c.station_callsign, Callsign::parse(v).ok()),
            "OWNER_CALLSIGN" => set(&mut c.owner_callsign, Callsign::parse(v).ok()),
            "OPERATOR" => set(&mut c.operator_callsign, Callsign::parse(v).ok()),
            "DXCC" => set(&mut c.dxcc_id, v.parse().ok()),
            // DXKeeper's entity prefix. Standard ADIF has no field for it;
            // `PFX` is the WPX prefix, which is something else.
            "APP_DXKEEPER_DXCCPREFIX" => set(&mut c.dxcc_prefix, Some(v.to_ascii_uppercase())),
            "PFX" => set(&mut c.wpx_prefix, Some(v.to_ascii_uppercase())),
            "CONT" => set(&mut c.continent, Some(v.to_ascii_uppercase())),
            "CQZ" => set(&mut c.cq_zone, v.parse().ok()),
            "ITUZ" => set(&mut c.itu_zone, v.parse().ok()),
            "GRIDSQUARE" => text(&mut c.grid, raw),
            "STATE" => text(&mut c.state, raw),
            "CNTY" => text(&mut c.county, raw),
            "VE_PROV" => text(&mut c.province, raw),
            "IOTA" => text(&mut c.iota, raw),
            "LAT" => set(&mut c.latitude, parse_latitude(v)),
            "LON" => set(&mut c.longitude, parse_longitude(v)),
            "NAME" => text(&mut c.name, raw),
            "QTH" => text(&mut c.qth, raw),
            "ADDRESS" => text(&mut c.address, raw),
            "EMAIL" => text(&mut c.email, raw),
            "AGE" => set(&mut c.age, v.parse().ok()),
            "QSL_VIA" => text(&mut c.qsl_via, raw),
            "COMMENT" => text(&mut c.comment, raw),
            "CONTEST_ID" => text(&mut c.contest_id, raw),
            "STX" => set(&mut c.serial_sent, v.parse().ok()),
            "SRX" => set(&mut c.serial_rcvd, v.parse().ok()),
            "STX_STRING" => text(&mut c.exchange_sent, raw),
            "SRX_STRING" => text(&mut c.exchange_rcvd, raw),
            "ARRL_SECT" => text(&mut c.arrl_section, raw),
            "BAND_RX" => set(&mut c.band_rx, Band::from_adif(v)),
            "FREQ_RX" => set(&mut c.freq_rx_hz, parse_freq_mhz(v)),
            "TX_PWR" => set(&mut c.tx_power_w, v.parse().ok()),
            "RX_PWR" => set(&mut c.rx_power_w, v.parse().ok()),
            "PROP_MODE" => set(&mut c.propagation_mode, Some(parse_prop_mode(v))),
            "SAT_NAME" => text(&mut c.sat_name, raw),
            "SAT_MODE" => text(&mut c.sat_mode, raw),
            "DISTANCE" => set(&mut c.distance_km, v.parse().ok()),
            "ANT_AZ" => set(&mut c.bearing_deg, v.parse().ok()),
            "SFI" => set(&mut c.sfi, v.parse().ok()),
            "A_INDEX" => set(&mut c.a_index, v.parse().ok()),
            "K_INDEX" => set(&mut c.k_index, v.parse().ok()),
            "CREDIT_GRANTED" => add_credits(c, v, CreditStatus::Granted),
            "CREDIT_SUBMITTED" => add_credits(c, v, CreditStatus::Submitted),
            QTH_ID_FIELD => {
                qth_id = Some(v);
                true
            }
            n if MY_FIELDS.contains(&n) => {
                my_fields.push((field.0.clone(), field.1.clone()));
                true
            }
            _ => false,
        };
        if !mapped {
            unmapped.push(field);
        }
    }

    // End time. ADIF allows TIME_OFF without QSO_DATE_OFF, meaning the
    // start date — unless the clock has gone past midnight, which shows
    // up as an end time earlier than the start time.
    let date_off = end_fields.iter().find(|f| f.0 == "QSO_DATE_OFF").map(|f| f.1.as_str());
    let time_off = end_fields.iter().find(|f| f.0 == "TIME_OFF").map(|f| f.1.as_str());
    cmd.qso_end = match (date_off, time_off) {
        (Some(d), Some(t)) => parse_adif_datetime(d, t),
        (None, Some(t)) => parse_adif_datetime(date, t).map(|end| {
            if end < qso_begin {
                end + chrono::Duration::days(1)
            } else {
                end
            }
        }),
        _ => None,
    };
    if cmd.qso_end.is_none() {
        unmapped.extend(end_fields);
    }

    // My station. With neither a QTH id nor a nameable MY_* field the
    // record doesn't say where it was made; keep whatever MY_* fields
    // there are as extras rather than inventing a location.
    cmd.station_location_ref = StationLocationRef::from_record(qth_id, my_fields.clone());
    if cmd.station_location_ref.is_none() {
        for (name, value) in my_fields {
            push_extra(&mut cmd.exchange_fields, name, value);
        }
    }

    for (name, value) in unmapped {
        push_extra(&mut cmd.exchange_fields, name.clone(), value.clone());
    }

    Ok(cmd)
}

/// Store `value` in `slot` unless the slot is already filled (a repeated
/// field: the first occurrence wins). Returns false when there's no value
/// — the field didn't parse — so the caller keeps the raw text.
fn set<T>(slot: &mut Option<T>, value: Option<T>) -> bool {
    match value {
        Some(v) => {
            if slot.is_none() {
                *slot = Some(v);
            }
            true
        }
        None => false,
    }
}

/// Text fields are stored exactly as written (an `ADDRESS` keeps its
/// line breaks).
fn text(slot: &mut Option<String>, raw: &str) -> bool {
    set(slot, Some(raw.to_string()))
}

/// Append the credits in an ADIF CreditList, skipping awards already
/// recorded with the same status. An unparseable list is left unmapped.
fn add_credits(cmd: &mut CreateQsoCommand, value: &str, status: CreditStatus) -> bool {
    let parsed = parse_credit_list(value, status);
    if parsed.is_empty() {
        return false;
    }
    for credit in parsed {
        if !cmd
            .credits
            .iter()
            .any(|c| c.award == credit.award && c.status == credit.status)
        {
            cmd.credits.push(credit);
        }
    }
    true
}

/// Extra fields are unique per QSO in storage, so a field repeated within
/// a record keeps its first value.
fn push_extra(fields: &mut Vec<QsoExchangeField>, name: String, raw_value: String) {
    if fields.iter().any(|f| f.name == name) {
        return;
    }
    fields.push(QsoExchangeField {
        name,
        raw_value,
        normalized_value: None,
        source: FieldSource::ImportedAdif,
    });
}

fn parse_adif_datetime(date: &str, time: &str) -> Option<chrono::DateTime<Utc>> {
    let nd = NaiveDate::parse_from_str(date.trim(), "%Y%m%d").ok()?;
    let t = time.trim();
    let nt = match t.len() {
        4 => NaiveTime::parse_from_str(t, "%H%M").ok()?,
        6 => NaiveTime::parse_from_str(t, "%H%M%S").ok()?,
        _ => return None,
    };
    let dt = NaiveDateTime::new(nd, nt);
    Some(Utc.from_utc_datetime(&dt))
}

fn parse_freq_mhz(s: &str) -> Option<i64> {
    let mhz = s.trim().parse::<f64>().ok()?;
    if mhz <= 0.0 {
        return None;
    }
    Some((mhz * 1_000_000.0).round() as i64)
}

fn parse_prop_mode(s: &str) -> PropagationMode {
    match s.to_ascii_uppercase().as_str() {
        "" => PropagationMode::Terrestrial,
        "SAT" => PropagationMode::Satellite,
        "EME" => PropagationMode::Eme,
        "MS" => PropagationMode::MeteorScatter,
        "AUR" | "AUE" => PropagationMode::Aurora,
        "AS" => PropagationMode::AircraftScatter,
        other => PropagationMode::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_ADIF: &str = "Generated by test\n\
        <ADIF_VER:5>3.1.4<EOH>\n\
        <CALL:4>W1AW<QSO_DATE:8>20260508<TIME_ON:6>183045<BAND:3>20M<FREQ:8>14.07400<MODE:3>FT8<RST_SENT:3>-12<RST_RCVD:3>-09<DXCC:3>291<CQZ:1>5<ITUZ:1>8<CONT:2>NA<GRIDSQUARE:4>FN31<STATE:2>CT<MY_GRIDSQUARE:6>EM48ku<EOR>\n\
        <CALL:6>JA1NUT<QSO_DATE:8>20260508<TIME_ON:4>1900<BAND:3>40M<MODE:2>CW<RST_SENT:3>599<RST_RCVD:3>569<DXCC:3>339<CONT:2>AS<EOR>\n\
        <CALL:0><QSO_DATE:8>20260508<TIME_ON:4>1901<EOR>\n";

    #[test]
    fn parses_two_records_skips_one() {
        let outcome = parse_adif(SAMPLE_ADIF).unwrap();
        assert_eq!(outcome.commands.len(), 2);
        assert_eq!(outcome.skipped.len(), 1);

        let first = &outcome.commands[0];
        assert_eq!(first.call.as_str(), "W1AW");
        assert_eq!(first.band, Some(Band::M20));
        assert_eq!(first.freq_hz, Some(14_074_000));
        assert_eq!(first.mode, Some(Mode::FT8));
        assert_eq!(first.rst_sent.as_deref(), Some("-12"));
        assert_eq!(first.rst_rcvd.as_deref(), Some("-09"));
        assert_eq!(first.dxcc_id, Some(291));
        assert_eq!(first.cq_zone, Some(5));
        assert_eq!(first.itu_zone, Some(8));
        assert_eq!(first.continent.as_deref(), Some("NA"));
        assert_eq!(first.grid.as_deref(), Some("FN31"));
        assert_eq!(first.state.as_deref(), Some("CT"));

        // MY_GRIDSQUARE names the station location (no QTH id here) and
        // travels with that reference rather than as an extra field.
        let loc = first.station_location_ref.as_ref().unwrap();
        assert_eq!(loc.name, "EM48ku");
        assert_eq!(loc.my_fields, [("MY_GRIDSQUARE".to_string(), "EM48ku".to_string())]);
        assert!(first.exchange_fields.is_empty(), "{:?}", first.exchange_fields);
    }

    /// One record of a DXKeeper export, synthetic values.
    fn one(fields: &str) -> CreateQsoCommand {
        let adif = format!("<EOH><CALL:5>K1ABC<QSO_DATE:8>20240315<TIME_ON:6>120000{fields}<EOR>");
        let mut out = parse_adif(&adif).unwrap();
        assert!(out.skipped.is_empty(), "{:?}", out.skipped);
        out.commands.remove(0)
    }

    fn extra<'a>(cmd: &'a CreateQsoCommand, name: &str) -> Option<&'a str> {
        cmd.exchange_fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.raw_value.as_str())
    }

    fn field(name: &str, value: &str) -> String {
        format!("<{name}:{}>{value}", value.len())
    }

    #[test]
    fn pfx_is_the_wpx_prefix_and_dxkeeper_prefix_is_the_entity() {
        let cmd = one(&(field("PFX", "k1") + &field("APP_DXKEEPER_DXCCPREFIX", "K")));
        assert_eq!(cmd.wpx_prefix.as_deref(), Some("K1"));
        assert_eq!(cmd.dxcc_prefix.as_deref(), Some("K"));
        // PFX alone no longer stands in for the entity prefix.
        let cmd = one(&field("PFX", "VP2E"));
        assert_eq!(cmd.dxcc_prefix, None);
        assert_eq!(cmd.wpx_prefix.as_deref(), Some("VP2E"));
        assert!(cmd.exchange_fields.is_empty());
    }

    #[test]
    fn operator_and_owner_are_separate() {
        let cmd = one(&(field("OPERATOR", "W1AW") + &field("OWNER_CALLSIGN", "K2A")));
        assert_eq!(cmd.operator_callsign.as_ref().map(|c| c.as_str()), Some("W1AW"));
        assert_eq!(cmd.owner_callsign.as_ref().map(|c| c.as_str()), Some("K2A"));
        let cmd = one(&field("OPERATOR", "W1AW"));
        assert_eq!(cmd.owner_callsign, None);
    }

    #[test]
    fn distance_and_azimuth_map_to_path_columns() {
        let cmd = one(&(field("DISTANCE", "1234.5") + &field("ANT_AZ", "87")));
        assert_eq!(cmd.distance_km, Some(1234.5));
        assert_eq!(cmd.bearing_deg, Some(87.0));
        assert!(cmd.exchange_fields.is_empty());
    }

    #[test]
    fn time_off_before_time_on_ends_the_next_day() {
        // Started 12:00:00, ended 00:01:30: past midnight.
        let cmd = one(&field("TIME_OFF", "000130"));
        let begin = cmd.qso_begin;
        let end = cmd.qso_end.unwrap();
        assert_eq!(end.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-03-16 00:01:30");
        assert!(end > begin);
        // Same day when the end is later; QSO_DATE_OFF is taken as given.
        let cmd = one(&field("TIME_OFF", "120130"));
        assert_eq!(cmd.qso_end.unwrap().format("%d %H%M%S").to_string(), "15 120130");
        let cmd = one(&(field("QSO_DATE_OFF", "20240317") + &field("TIME_OFF", "010000")));
        assert_eq!(cmd.qso_end.unwrap().format("%d").to_string(), "17");
    }

    #[test]
    fn new_columns_map_and_stay_out_of_extras() {
        let fields = [
            ("CONTEST_ID", "cwt"),
            ("STX", "17"),
            ("SRX", "1234"),
            ("STX_STRING", "CHAD 1"),
            ("SRX_STRING", "ART CT"),
            ("ARRL_SECT", "CT"),
            ("NAME", "Art"),
            ("QTH", "Newington"),
            ("ADDRESS", "1 Main St\r\nNewington CT"),
            ("EMAIL", "art@example.com"),
            ("AGE", "42"),
            ("COMMENT", "synthetic"),
            ("QSL_VIA", "W1AW"),
            ("BAND_RX", "20m"),
            ("FREQ_RX", "14.0255"),
            ("LAT", "N041 42.857"),
            ("LON", "W072 43.698"),
            ("SFI", "150"),
            ("A_INDEX", "7"),
            ("K_INDEX", "2"),
        ];
        let cmd = one(&fields.iter().map(|(n, v)| field(n, v)).collect::<String>());
        assert_eq!(cmd.contest_id.as_deref(), Some("cwt"), "case kept as given");
        assert_eq!(cmd.serial_sent, Some(17));
        assert_eq!(cmd.serial_rcvd, Some(1234));
        assert_eq!(cmd.exchange_sent.as_deref(), Some("CHAD 1"));
        assert_eq!(cmd.exchange_rcvd.as_deref(), Some("ART CT"));
        assert_eq!(cmd.arrl_section.as_deref(), Some("CT"));
        assert_eq!(cmd.name.as_deref(), Some("Art"));
        assert_eq!(cmd.qth.as_deref(), Some("Newington"));
        assert_eq!(cmd.address.as_deref(), Some("1 Main St\r\nNewington CT"));
        assert_eq!(cmd.email.as_deref(), Some("art@example.com"));
        assert_eq!(cmd.age, Some(42));
        assert_eq!(cmd.comment.as_deref(), Some("synthetic"));
        assert_eq!(cmd.qsl_via.as_deref(), Some("W1AW"));
        assert_eq!(cmd.band_rx, Some(Band::M20));
        assert_eq!(cmd.freq_rx_hz, Some(14_025_500));
        assert!((cmd.latitude.unwrap() - 41.71428).abs() < 1e-4);
        assert!((cmd.longitude.unwrap() + 72.7283).abs() < 1e-4);
        assert_eq!((cmd.sfi, cmd.a_index, cmd.k_index), (Some(150), Some(7), Some(2)));
        assert!(cmd.exchange_fields.is_empty(), "{:?}", cmd.exchange_fields);
    }

    #[test]
    fn unparseable_numbers_fall_through_to_extras() {
        let cmd = one(
            &(field("STX", "5NN") + &field("AGE", "old") + &field("LAT", "41.5")
                + &field("K_INDEX", "2.5") + &field("SRX", "12")),
        );
        assert_eq!(cmd.serial_sent, None);
        assert_eq!(extra(&cmd, "STX"), Some("5NN"));
        assert_eq!(extra(&cmd, "AGE"), Some("old"));
        assert_eq!(extra(&cmd, "LAT"), Some("41.5"));
        assert_eq!(extra(&cmd, "K_INDEX"), Some("2.5"));
        assert_eq!(cmd.serial_rcvd, Some(12));
        assert_eq!(extra(&cmd, "SRX"), None);
    }

    #[test]
    fn credits_parse_and_leave_extras() {
        let cmd = one(
            &(field("CREDIT_GRANTED", "DXCC:LOTW, DXCC_BAND:LOTW,WAS")
                + &field("CREDIT_SUBMITTED", "VUCC_BAND")),
        );
        let got: Vec<(&str, Option<&str>, CreditStatus)> = cmd
            .credits
            .iter()
            .map(|c| (c.award.as_str(), c.medium.as_deref(), c.status))
            .collect();
        assert_eq!(
            got,
            [
                ("DXCC", Some("LOTW"), CreditStatus::Granted),
                ("DXCC_BAND", Some("LOTW"), CreditStatus::Granted),
                ("WAS", None, CreditStatus::Granted),
                ("VUCC_BAND", None, CreditStatus::Submitted),
            ]
        );
        assert!(cmd.exchange_fields.is_empty());
    }

    #[test]
    fn qth_id_names_the_location_and_service_fields_stay_extras() {
        let cmd = one(
            &(field("APP_DXKEEPER_MY_QTHID", "Home 2")
                + &field("LOTW_QSL_SENT", "Y")
                + &field("APP_DXKEEPER_QRZCOM_LOGID", "123456")
                + &field("QSL_SENT", "N")),
        );
        let loc = cmd.station_location_ref.as_ref().unwrap();
        assert_eq!(loc.name, "Home 2");
        assert!(loc.my_fields.is_empty());
        assert_eq!(extra(&cmd, "APP_DXKEEPER_MY_QTHID"), None);
        assert_eq!(extra(&cmd, "LOTW_QSL_SENT"), Some("Y"));
        assert_eq!(extra(&cmd, "APP_DXKEEPER_QRZCOM_LOGID"), Some("123456"));
        assert_eq!(extra(&cmd, "QSL_SENT"), Some("N"));
        // No QTH id and no MY_* fields: no location, nothing assumed.
        assert!(one("").station_location_ref.is_none());
        // MY_* fields that can't name a location stay as extras.
        let cmd = one(&field("MY_CQ_ZONE", "5"));
        assert!(cmd.station_location_ref.is_none());
        assert_eq!(extra(&cmd, "MY_CQ_ZONE"), Some("5"));
    }

    #[test]
    fn repeated_unmapped_field_keeps_first_value() {
        let cmd = one(&(field("QSL_SENT", "Y") + &field("QSL_SENT", "N")));
        assert_eq!(cmd.exchange_fields.len(), 1);
        assert_eq!(extra(&cmd, "QSL_SENT"), Some("Y"));
    }

    #[test]
    fn time_on_4_digit_form_handled() {
        let outcome = parse_adif(SAMPLE_ADIF).unwrap();
        let second = &outcome.commands[1];
        assert_eq!(second.qso_begin.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-05-08 19:00:00");
    }

    #[test]
    fn missing_call_record_skipped_with_reason() {
        let outcome = parse_adif(SAMPLE_ADIF).unwrap();
        assert_eq!(outcome.skipped.len(), 1);
        assert!(
            outcome.skipped[0].reason.contains("missing CALL")
                || outcome.skipped[0].reason.contains("invalid CALL"),
            "unexpected reason: {}",
            outcome.skipped[0].reason
        );
    }

    /// Synthesize ~50k records (modeling the user's DXKeeper export)
    /// and confirm the parser finishes in well under a second. The
    /// upstream `adif_parser::parse_adi` takes hours on inputs this
    /// size because of its O(N²) `check_tag` implementation.
    #[test]
    fn fast_parse_handles_50k_records_in_under_a_second() {
        const RECORDS: usize = 50_000;
        let mut s = String::with_capacity(RECORDS * 80);
        s.push_str("<ADIF_VER:5>3.1.4<EOH>\n");
        for i in 0..RECORDS {
            // Use a fixed valid time; the test exercises parser speed
            // + record count, not timestamp variety.
            s.push_str(&format!(
                "<CALL:6>TEST{:02}<QSO_DATE:8>20260508<TIME_ON:6>183045<BAND:3>20M<MODE:3>FT8<RST_SENT:3>-12<RST_RCVD:3>-09<DXCC:3>291<EOR>\n",
                i % 100,
            ));
        }
        let start = std::time::Instant::now();
        let outcome = parse_adif(&s).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(outcome.commands.len(), RECORDS);
        assert_eq!(outcome.skipped.len(), 0);
        assert!(
            elapsed.as_secs() < 2,
            "parsing {RECORDS} records took {elapsed:?} (expected < 2s)"
        );
    }
}
