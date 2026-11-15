use adif_parser::{AdifFile, AdifHeader, Field, Record};
use chrono::{DateTime, Utc};

use radio_core::{PropagationMode, Qso, QsoExchangeField, StationLocation};

use crate::credit::{AwardCredit, CreditStatus, format_credit_list};
use crate::my_station::{QTH_ID_FIELD, format_latitude, format_longitude, location_fields};

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub program_id: String,
    pub program_version: String,
    pub adif_version: String,
    pub preamble: Option<String>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            program_id: "slogger".to_string(),
            program_version: env!("CARGO_PKG_VERSION").to_string(),
            adif_version: "3.1.4".to_string(),
            preamble: None,
        }
    }
}

/// One QSO and what's stored alongside it, for a full-fidelity export.
#[derive(Debug, Clone, Copy)]
pub struct ExportRecord<'a> {
    pub qso: &'a Qso,
    /// The station location the QSO links to, written as
    /// `APP_DXKEEPER_MY_QTHID` plus a `MY_*` field per detail it has.
    pub location: Option<&'a StationLocation>,
    pub credits: &'a [AwardCredit],
    /// Extra fields, written as stored. One with the same name as a field
    /// written from a column or the location replaces it: it is what this
    /// QSO's own record said (a per-QSO `MY_GRIDSQUARE` that differs from
    /// the location's, for instance).
    pub extra_fields: &'a [QsoExchangeField],
}

/// The fields a QSL service needs to match and credit a QSO: the QSO
/// facts and where the other station was. This is an allow-list, so a
/// column added later is not sent to third parties until someone decides
/// it should be. It deliberately leaves out what the log holds about the
/// other operator as a person (name, address, email, age, comments) and
/// the operator's private notes — none of that is ours to pass on.
const UPLOAD_FIELDS: &[&str] = &[
    "CALL",
    "QSO_DATE",
    "TIME_ON",
    "QSO_DATE_OFF",
    "TIME_OFF",
    "BAND",
    "FREQ",
    "BAND_RX",
    "FREQ_RX",
    "MODE",
    "SUBMODE",
    "RST_SENT",
    "RST_RCVD",
    "STATION_CALLSIGN",
    "OWNER_CALLSIGN",
    "OPERATOR",
    "DXCC",
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

/// ADIF for the service uploads (LoTW, eQSL, Club Log, QRZ, HRDLog):
/// only [`UPLOAD_FIELDS`]. For a complete export use
/// [`export_adif_records`].
pub fn export_adif(qsos: &[Qso], opts: &ExportOptions) -> String {
    let mut file = file_with_header(opts);
    for qso in qsos {
        let mut rec = Record::new();
        for (name, value) in qso_fields(qso) {
            if UPLOAD_FIELDS.contains(&name) {
                rec.add_field(Field::new(name, value));
            }
        }
        file.records.push(rec);
    }
    file.to_adi_string()
}

/// A complete export: every column, the station location, award credit
/// and extra fields. Re-importing it reproduces the log.
pub fn export_adif_records(records: &[ExportRecord<'_>], opts: &ExportOptions) -> String {
    let mut file = file_with_header(opts);
    for r in records {
        file.records.push(to_record(r));
    }
    file.to_adi_string()
}

fn file_with_header(opts: &ExportOptions) -> AdifFile {
    let mut file = AdifFile::new();
    file.header = AdifHeader {
        preamble: opts
            .preamble
            .clone()
            .unwrap_or_else(|| format!("ADIF export by {}", opts.program_id)),
        fields: vec![
            Field::new("ADIF_VER", &opts.adif_version),
            Field::new("PROGRAMID", &opts.program_id),
            Field::new("PROGRAMVERSION", &opts.program_version),
        ],
        adif_version: Some(opts.adif_version.clone()),
        program_id: Some(opts.program_id.clone()),
        program_version: Some(opts.program_version.clone()),
        created_timestamp: None,
    };
    file
}

fn to_record(r: &ExportRecord<'_>) -> Record {
    let mut fields = qso_fields(r.qso);
    if let Some(loc) = r.location {
        fields.push((QTH_ID_FIELD, loc.name.clone()));
        fields.extend(location_fields(loc));
    }
    for status in [CreditStatus::Granted, CreditStatus::Submitted] {
        if let Some(list) = format_credit_list(r.credits, status) {
            fields.push((status.adif_field(), list));
        }
    }

    let mut rec = Record::new();
    let mut written: Vec<(String, String)> =
        fields.into_iter().map(|(n, v)| (n.to_string(), v)).collect();
    for f in r.extra_fields {
        match written.iter_mut().find(|(n, _)| *n == f.name) {
            Some(slot) => slot.1 = f.raw_value.clone(),
            None => written.push((f.name.clone(), f.raw_value.clone())),
        }
    }
    for (name, value) in written {
        rec.add_field(Field::new(name, value));
    }
    rec
}

/// Every QSO column as ADIF, under the field import reads it from.
fn qso_fields(qso: &Qso) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut put = |name: &'static str, v: Option<String>| {
        if let Some(v) = v {
            out.push((name, v));
        }
    };
    let mhz = |hz: i64| format!("{:.6}", hz as f64 / 1_000_000.0);

    put("CALL", Some(qso.call.as_str().to_string()));
    put("QSO_DATE", Some(qso.qso_begin.format("%Y%m%d").to_string()));
    put("TIME_ON", Some(qso.qso_begin.format("%H%M%S").to_string()));
    if let Some(end) = &qso.qso_end {
        if end.date_naive() != qso.qso_begin.date_naive() {
            put("QSO_DATE_OFF", Some(end.format("%Y%m%d").to_string()));
        }
        put("TIME_OFF", Some(end.format("%H%M%S").to_string()));
    }

    put("BAND", qso.band.map(|b| b.as_adif().to_string()));
    put("FREQ", qso.freq_hz.map(mhz));
    put("BAND_RX", qso.band_rx.map(|b| b.as_adif().to_string()));
    put("FREQ_RX", qso.freq_rx_hz.map(mhz));
    put("MODE", qso.mode.as_ref().map(|m| m.as_adif().to_string()));
    put("SUBMODE", qso.submode.clone());
    put("RST_SENT", qso.rst_sent.clone());
    put("RST_RCVD", qso.rst_rcvd.clone());

    let call = |c: &Option<radio_core::Callsign>| c.as_ref().map(|c| c.as_str().to_string());
    put("STATION_CALLSIGN", call(&qso.station_callsign));
    put("OWNER_CALLSIGN", call(&qso.owner_callsign));
    put("OPERATOR", call(&qso.operator_callsign));

    put("DXCC", qso.dxcc_id.map(|id| id.to_string()));
    put("APP_DXKEEPER_DXCCPREFIX", qso.dxcc_prefix.clone());
    put("PFX", qso.wpx_prefix.clone());
    put("CONT", qso.continent.clone());
    put("CQZ", qso.cq_zone.map(|z| z.to_string()));
    put("ITUZ", qso.itu_zone.map(|z| z.to_string()));
    put("GRIDSQUARE", qso.grid.clone());
    put("STATE", qso.state.clone());
    put("CNTY", qso.county.clone());
    put("VE_PROV", qso.province.clone());
    put("IOTA", qso.iota.clone());
    put("LAT", qso.latitude.map(format_latitude));
    put("LON", qso.longitude.map(format_longitude));

    put("NAME", qso.name.clone());
    put("QTH", qso.qth.clone());
    put("ADDRESS", qso.address.clone());
    put("EMAIL", qso.email.clone());
    put("AGE", qso.age.map(|a| a.to_string()));
    put("QSL_VIA", qso.qsl_via.clone());
    put("COMMENT", qso.comment.clone());

    put("CONTEST_ID", qso.contest_id.clone());
    put("STX", qso.serial_sent.map(|n| n.to_string()));
    put("SRX", qso.serial_rcvd.map(|n| n.to_string()));
    put("STX_STRING", qso.exchange_sent.clone());
    put("SRX_STRING", qso.exchange_rcvd.clone());
    put("ARRL_SECT", qso.arrl_section.clone());

    put("TX_PWR", qso.tx_power_w.map(|p| format!("{p}")));
    put("RX_PWR", qso.rx_power_w.map(|p| format!("{p}")));
    put("PROP_MODE", qso.propagation_mode.as_ref().map(prop_mode_to_adif));
    put("SAT_NAME", qso.sat_name.clone());
    put("SAT_MODE", qso.sat_mode.clone());
    put("DISTANCE", qso.distance_km.map(|d| format!("{d}")));
    put("ANT_AZ", qso.bearing_deg.map(|b| format!("{b}")));

    put("SFI", qso.sfi.map(|n| n.to_string()));
    put("A_INDEX", qso.a_index.map(|n| n.to_string()));
    put("K_INDEX", qso.k_index.map(|n| n.to_string()));
    out
}

fn prop_mode_to_adif(p: &PropagationMode) -> String {
    match p {
        PropagationMode::Terrestrial => "".into(),
        PropagationMode::Satellite => "SAT".into(),
        PropagationMode::Eme => "EME".into(),
        PropagationMode::MeteorScatter => "MS".into(),
        PropagationMode::Aurora => "AUR".into(),
        PropagationMode::AircraftScatter => "AS".into(),
        PropagationMode::Other(s) => s.clone(),
    }
}

/// Convenience for callers that just want a list of bare QSOs to round-trip
/// through ADIF with default header/program metadata.
pub fn export_adif_default(qsos: &[Qso]) -> String {
    export_adif(qsos, &ExportOptions::default())
}

/// Helper to format a single timestamp for diagnostics. Public so callers
/// (tests, UI) don't need to know the wire format.
pub fn format_qso_timestamp(dt: &DateTime<Utc>) -> String {
    dt.format("%Y%m%d %H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::parse_adif;
    use chrono::TimeZone;
    use radio_core::{Band, Callsign, FieldSource, Mode, QsoId, StationLocationId};

    fn sample_qso() -> Qso {
        Qso {
            id: QsoId::new(),
            call: Callsign::parse("W1AW").unwrap(),
            qso_begin: Utc.with_ymd_and_hms(2026, 5, 8, 23, 58, 45).unwrap(),
            qso_end: Some(Utc.with_ymd_and_hms(2026, 5, 9, 0, 1, 0).unwrap()),
            band: Some(Band::M20),
            freq_hz: Some(14_074_000),
            mode: Some(Mode::FT8),
            submode: None,
            rst_sent: Some("-12".into()),
            rst_rcvd: Some("-09".into()),
            operator_id: None,
            station_location_id: None,
            station_callsign: Some(Callsign::parse("K2A").unwrap()),
            owner_callsign: Some(Callsign::parse("K2A").unwrap()),
            operator_callsign: Some(Callsign::parse("K1ABC").unwrap()),
            dxcc_id: Some(291),
            dxcc_prefix: Some("K".into()),
            wpx_prefix: Some("W1".into()),
            continent: Some("NA".into()),
            cq_zone: Some(5),
            itu_zone: Some(8),
            grid: Some("FN31".into()),
            state: Some("CT".into()),
            county: Some("CT,HARTFORD".into()),
            province: None,
            iota: None,
            latitude: Some(41.0 + 23.456 / 60.0),
            longitude: Some(-(72.0 + 12.345 / 60.0)),
            name: Some("Test Operator".into()),
            qth: Some("Newington".into()),
            address: Some("225 Main St\r\nNewington CT".into()),
            email: Some("op@example.com".into()),
            age: Some(42),
            qsl_via: Some("LOTW".into()),
            comment: Some("synthetic".into()),
            contest_id: Some("CW-OPS".into()),
            serial_sent: Some(17),
            serial_rcvd: Some(1234),
            exchange_sent: Some("CHAD 1".into()),
            exchange_rcvd: Some("ART 1".into()),
            arrl_section: Some("CT".into()),
            band_rx: Some(Band::M20),
            freq_rx_hz: Some(14_074_500),
            tx_power_w: Some(100.0),
            rx_power_w: Some(0.0),
            propagation_mode: None,
            sat_name: None,
            sat_mode: None,
            distance_km: Some(1234.5),
            bearing_deg: Some(45.0),
            sfi: Some(150),
            a_index: Some(7),
            k_index: Some(2),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// A complete export of bare QSOs: no location, credit or extras.
    fn full_export(qsos: &[Qso]) -> String {
        let records: Vec<ExportRecord<'_>> = qsos
            .iter()
            .map(|qso| ExportRecord {
                qso,
                location: None,
                credits: &[],
                extra_fields: &[],
            })
            .collect();
        export_adif_records(&records, &ExportOptions::default())
    }

    #[test]
    fn upload_export_leaves_out_personal_and_private_fields() {
        let qso = sample_qso();
        // Guard the premise: the sample really has these.
        assert!(qso.name.is_some() && qso.email.is_some() && qso.address.is_some());
        assert!(qso.comment.is_some());
        let adi = export_adif_default(&[qso]);
        assert!(adi.contains("<CALL:4>W1AW"));
        assert!(adi.contains("<BAND:3>20M"));
        for field in [
            "<NAME:", "<QTH:", "<ADDRESS:", "<EMAIL:", "<AGE:", "<COMMENT:", "<QSL_VIA:",
            "<LAT:", "<LON:", "<CONTEST_ID:", "<SRX", "<STX", "<SFI:", "<PFX:",
        ] {
            assert!(!adi.contains(field), "upload export must not contain {field}");
        }
    }

    #[test]
    fn export_writes_header_and_record() {
        let adi = full_export(&[sample_qso()]);
        assert!(adi.contains("PROGRAMID"));
        assert!(adi.contains("slogger"));
        assert!(adi.contains("<EOH>"));
        assert!(adi.contains("<CALL:4>W1AW"));
        assert!(adi.contains("<BAND:3>20M"));
        assert!(adi.contains("<MODE:3>FT8"));
        assert!(adi.contains("<EOR>"));
        // Owner and operator under their own fields; the WPX prefix as
        // PFX and the entity prefix under DXKeeper's name for it.
        assert!(adi.contains("<OWNER_CALLSIGN:3>K2A"));
        assert!(adi.contains("<OPERATOR:5>K1ABC"));
        assert!(adi.contains("<PFX:2>W1"));
        assert!(adi.contains("<APP_DXKEEPER_DXCCPREFIX:1>K"));
        assert!(adi.contains("<LAT:11>N041 23.456"));
        assert!(adi.contains("<LON:11>W072 12.345"));
    }

    #[test]
    fn round_trip_preserves_every_column() {
        let original = sample_qso();
        let adi = full_export(&[original.clone()]);
        let outcome = parse_adif(&adi).unwrap();
        assert_eq!(outcome.commands.len(), 1);
        let cmd = &outcome.commands[0];
        // Compare via Debug: the command has no PartialEq and every field
        // matters. `from_qso` is what the record should parse back to.
        let expected = crate::commands::CreateQsoCommand::from_qso(&original);
        assert_eq!(format!("{:?}", strip(cmd)), format!("{:?}", strip(&expected)));
        assert!(cmd.exchange_fields.is_empty(), "{:?}", cmd.exchange_fields);
    }

    /// The command minus the import-only parts, for comparison.
    fn strip(c: &crate::commands::CreateQsoCommand) -> crate::commands::CreateQsoCommand {
        crate::commands::CreateQsoCommand {
            exchange_fields: Vec::new(),
            station_location_ref: None,
            credits: Vec::new(),
            ..c.clone()
        }
    }

    #[test]
    fn full_record_writes_location_credits_and_extras() {
        let qso = sample_qso();
        let now = Utc::now();
        let loc = StationLocation {
            id: StationLocationId::new(),
            name: "Home".into(),
            station_callsign: None,
            owner_callsign: None,
            city: None,
            county: None,
            state: None,
            country: None,
            grid: Some("FN31pr".into()),
            latitude: None,
            longitude: None,
            cq_zone: Some(5),
            itu_zone: None,
            iota: None,
            lotw_station_location: None,
            eqsl_account: None,
            created_at: now,
            updated_at: now,
        };
        let credits = crate::credit::parse_credit_list("DXCC:LOTW,WAS", CreditStatus::Granted);
        let extras = vec![
            QsoExchangeField {
                name: "QSL_SENT".into(),
                raw_value: "Y".into(),
                normalized_value: None,
                source: FieldSource::ImportedAdif,
            },
            // A per-QSO value that differs from the location's wins.
            QsoExchangeField {
                name: "MY_GRIDSQUARE".into(),
                raw_value: "FN31ps".into(),
                normalized_value: None,
                source: FieldSource::ImportedAdif,
            },
        ];
        let adi = export_adif_records(
            &[ExportRecord {
                qso: &qso,
                location: Some(&loc),
                credits: &credits,
                extra_fields: &extras,
            }],
            &ExportOptions::default(),
        );
        assert!(adi.contains("<APP_DXKEEPER_MY_QTHID:4>Home"));
        assert!(adi.contains("<MY_CQ_ZONE:1>5"));
        assert!(adi.contains("<MY_GRIDSQUARE:6>FN31ps"));
        assert!(!adi.contains("FN31pr"));
        assert!(adi.contains("<CREDIT_GRANTED:13>DXCC:LOTW,WAS"));
        assert!(adi.contains("<QSL_SENT:1>Y"));

        let cmd = &parse_adif(&adi).unwrap().commands[0];
        assert_eq!(cmd.station_location_ref.as_ref().unwrap().name, "Home");
        assert_eq!(cmd.credits, credits);
    }
}
