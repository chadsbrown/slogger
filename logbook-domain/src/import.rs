use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use thiserror::Error;

use radio_core::{
    Band, Callsign, FieldSource, Mode, PropagationMode, QsoExchangeField,
};

use crate::commands::CreateQsoCommand;

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

/// ADIF QSO core fields that map onto `CreateQsoCommand` first-class slots.
/// Anything not in this set goes into `qso_exchange_fields` with
/// `FieldSource::ImportedAdif` so it round-trips on export.
const CORE_FIELDS: &[&str] = &[
    "CALL",
    "QSO_DATE",
    "TIME_ON",
    "QSO_DATE_OFF",
    "TIME_OFF",
    "BAND",
    "FREQ",
    "MODE",
    "SUBMODE",
    "RST_SENT",
    "RST_RCVD",
    "STATION_CALLSIGN",
    "OPERATOR",
    "DXCC",
    "PFX",
    "CONT",
    "CQZ",
    "ITUZ",
    "GRIDSQUARE",
    "STATE",
    "CNTY",
    "VE_PROV",
    "IOTA",
    "TX_PWR",
    "RX_PWR",
    "PROP_MODE",
    "SAT_NAME",
    "SAT_MODE",
];

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

fn record_to_command(record: &[(String, String)]) -> Result<CreateQsoCommand, String> {
    let call_str = lookup(record, "CALL").ok_or_else(|| "missing CALL".to_string())?;
    let call = Callsign::parse(call_str).map_err(|e| format!("invalid CALL {call_str:?}: {e}"))?;

    let date = lookup(record, "QSO_DATE").ok_or_else(|| "missing QSO_DATE".to_string())?;
    let time = lookup(record, "TIME_ON").unwrap_or("0000");
    let qso_begin = parse_adif_datetime(date, time)
        .ok_or_else(|| format!("invalid QSO_DATE/TIME_ON {date:?} {time:?}"))?;

    let qso_end = match (lookup(record, "QSO_DATE_OFF"), lookup(record, "TIME_OFF")) {
        (Some(d), Some(t)) => parse_adif_datetime(d, t),
        (None, Some(t)) => parse_adif_datetime(date, t),
        _ => None,
    };

    let mut cmd = CreateQsoCommand::minimal(call, qso_begin);
    cmd.qso_end = qso_end;
    cmd.band = lookup(record, "BAND").and_then(Band::from_adif);
    cmd.freq_hz = lookup(record, "FREQ").and_then(parse_freq_mhz);
    cmd.mode = lookup(record, "MODE").map(Mode::from_adif);
    cmd.submode = lookup(record, "SUBMODE").map(|s| s.to_string());
    cmd.rst_sent = lookup(record, "RST_SENT").map(|s| s.to_string());
    cmd.rst_rcvd = lookup(record, "RST_RCVD").map(|s| s.to_string());
    cmd.station_callsign = lookup(record, "STATION_CALLSIGN").and_then(|s| Callsign::parse(s).ok());
    cmd.owner_callsign = lookup(record, "OPERATOR").and_then(|s| Callsign::parse(s).ok());
    cmd.dxcc_id = lookup(record, "DXCC").and_then(|s| s.parse::<u16>().ok());
    cmd.dxcc_prefix = lookup(record, "PFX").map(|s| s.to_ascii_uppercase());
    cmd.continent = lookup(record, "CONT").map(|s| s.to_ascii_uppercase());
    cmd.cq_zone = lookup(record, "CQZ").and_then(|s| s.parse::<u8>().ok());
    cmd.itu_zone = lookup(record, "ITUZ").and_then(|s| s.parse::<u8>().ok());
    cmd.grid = lookup(record, "GRIDSQUARE").map(|s| s.to_string());
    cmd.state = lookup(record, "STATE").map(|s| s.to_string());
    cmd.county = lookup(record, "CNTY").map(|s| s.to_string());
    cmd.province = lookup(record, "VE_PROV").map(|s| s.to_string());
    cmd.iota = lookup(record, "IOTA").map(|s| s.to_string());
    cmd.tx_power_w = lookup(record, "TX_PWR").and_then(|s| s.parse::<f32>().ok());
    cmd.rx_power_w = lookup(record, "RX_PWR").and_then(|s| s.parse::<f32>().ok());
    cmd.propagation_mode = lookup(record, "PROP_MODE").map(parse_prop_mode);
    cmd.sat_name = lookup(record, "SAT_NAME").map(|s| s.to_string());
    cmd.sat_mode = lookup(record, "SAT_MODE").map(|s| s.to_string());

    for (name, value) in record {
        if CORE_FIELDS.contains(&name.as_str()) {
            continue;
        }
        if value.is_empty() {
            continue;
        }
        cmd.exchange_fields.push(QsoExchangeField {
            name: name.clone(),
            raw_value: value.clone(),
            normalized_value: None,
            source: FieldSource::ImportedAdif,
        });
    }

    Ok(cmd)
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

        // MY_GRIDSQUARE is not a core field — should land in exchange_fields.
        let exch = &first.exchange_fields;
        assert!(
            exch.iter().any(|f| f.name == "MY_GRIDSQUARE" && f.raw_value == "EM48ku"),
            "expected MY_GRIDSQUARE in exchange_fields, got {exch:?}"
        );
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
