use adif_parser::{AdifFile, AdifHeader, Field, Record};
use chrono::{DateTime, Utc};

use radio_core::{PropagationMode, Qso};

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

pub fn export_adif(qsos: &[Qso], opts: &ExportOptions) -> String {
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

    for qso in qsos {
        file.records.push(qso_to_record(qso));
    }

    file.to_adi_string()
}

fn qso_to_record(qso: &Qso) -> Record {
    let mut rec = Record::new();
    rec.add_field(Field::new("CALL", qso.call.as_str()));
    rec.add_field(Field::new(
        "QSO_DATE",
        qso.qso_begin.format("%Y%m%d").to_string(),
    ));
    rec.add_field(Field::new(
        "TIME_ON",
        qso.qso_begin.format("%H%M%S").to_string(),
    ));

    if let Some(end) = &qso.qso_end {
        if end.date_naive() != qso.qso_begin.date_naive() {
            rec.add_field(Field::new("QSO_DATE_OFF", end.format("%Y%m%d").to_string()));
        }
        rec.add_field(Field::new("TIME_OFF", end.format("%H%M%S").to_string()));
    }

    if let Some(band) = qso.band {
        rec.add_field(Field::new("BAND", band.as_adif()));
    }
    if let Some(hz) = qso.freq_hz {
        rec.add_field(Field::new(
            "FREQ",
            format!("{:.6}", hz as f64 / 1_000_000.0),
        ));
    }
    if let Some(mode) = &qso.mode {
        rec.add_field(Field::new("MODE", mode.as_adif()));
    }
    if let Some(submode) = &qso.submode {
        rec.add_field(Field::new("SUBMODE", submode));
    }
    if let Some(rst) = &qso.rst_sent {
        rec.add_field(Field::new("RST_SENT", rst));
    }
    if let Some(rst) = &qso.rst_rcvd {
        rec.add_field(Field::new("RST_RCVD", rst));
    }
    if let Some(call) = &qso.station_callsign {
        rec.add_field(Field::new("STATION_CALLSIGN", call.as_str()));
    }
    if let Some(call) = &qso.owner_callsign {
        rec.add_field(Field::new("OPERATOR", call.as_str()));
    }
    if let Some(id) = qso.dxcc_id {
        rec.add_field(Field::new("DXCC", id.to_string()));
    }
    if let Some(prefix) = &qso.dxcc_prefix {
        rec.add_field(Field::new("PFX", prefix));
    }
    if let Some(c) = &qso.continent {
        rec.add_field(Field::new("CONT", c));
    }
    if let Some(z) = qso.cq_zone {
        rec.add_field(Field::new("CQZ", z.to_string()));
    }
    if let Some(z) = qso.itu_zone {
        rec.add_field(Field::new("ITUZ", z.to_string()));
    }
    if let Some(g) = &qso.grid {
        rec.add_field(Field::new("GRIDSQUARE", g));
    }
    if let Some(s) = &qso.state {
        rec.add_field(Field::new("STATE", s));
    }
    if let Some(c) = &qso.county {
        rec.add_field(Field::new("CNTY", c));
    }
    if let Some(p) = &qso.province {
        rec.add_field(Field::new("VE_PROV", p));
    }
    if let Some(i) = &qso.iota {
        rec.add_field(Field::new("IOTA", i));
    }
    if let Some(p) = qso.tx_power_w {
        rec.add_field(Field::new("TX_PWR", format!("{p}")));
    }
    if let Some(p) = qso.rx_power_w {
        rec.add_field(Field::new("RX_PWR", format!("{p}")));
    }
    if let Some(p) = &qso.propagation_mode {
        rec.add_field(Field::new("PROP_MODE", prop_mode_to_adif(p)));
    }
    if let Some(s) = &qso.sat_name {
        rec.add_field(Field::new("SAT_NAME", s));
    }
    if let Some(s) = &qso.sat_mode {
        rec.add_field(Field::new("SAT_MODE", s));
    }
    rec
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
    use radio_core::{Band, Callsign, Mode, QsoId};

    fn sample_qso() -> Qso {
        Qso {
            id: QsoId::new(),
            call: Callsign::parse("W1AW").unwrap(),
            qso_begin: Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 45).unwrap(),
            qso_end: None,
            band: Some(Band::M20),
            freq_hz: Some(14_074_000),
            mode: Some(Mode::FT8),
            submode: None,
            rst_sent: Some("-12".into()),
            rst_rcvd: Some("-09".into()),
            operator_id: None,
            station_location_id: None,
            station_callsign: Some(Callsign::parse("K2A").unwrap()),
            owner_callsign: None,
            dxcc_id: Some(291),
            dxcc_prefix: Some("W".into()),
            continent: Some("NA".into()),
            cq_zone: Some(5),
            itu_zone: Some(8),
            grid: Some("FN31".into()),
            state: Some("CT".into()),
            county: None,
            province: None,
            iota: None,
            tx_power_w: Some(100.0),
            rx_power_w: None,
            propagation_mode: None,
            sat_name: None,
            sat_mode: None,
            distance_km: None,
            bearing_deg: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn export_writes_header_and_record() {
        let adi = export_adif_default(&[sample_qso()]);
        assert!(adi.contains("PROGRAMID"));
        assert!(adi.contains("slogger"));
        assert!(adi.contains("<EOH>"));
        assert!(adi.contains("<CALL:4>W1AW"));
        assert!(adi.contains("<BAND:3>20M"));
        assert!(adi.contains("<MODE:3>FT8"));
        assert!(adi.contains("<EOR>"));
    }

    #[test]
    fn round_trip_preserves_core_fields() {
        let original = sample_qso();
        let adi = export_adif_default(&[original.clone()]);
        let outcome = parse_adif(&adi).unwrap();
        assert_eq!(outcome.commands.len(), 1);
        let cmd = &outcome.commands[0];
        assert_eq!(cmd.call, original.call);
        assert_eq!(cmd.band, original.band);
        assert_eq!(cmd.freq_hz, original.freq_hz);
        assert_eq!(cmd.mode, original.mode);
        assert_eq!(cmd.rst_sent, original.rst_sent);
        assert_eq!(cmd.rst_rcvd, original.rst_rcvd);
        assert_eq!(cmd.dxcc_id, original.dxcc_id);
        assert_eq!(cmd.cq_zone, original.cq_zone);
        assert_eq!(cmd.itu_zone, original.itu_zone);
        assert_eq!(cmd.grid, original.grid);
        assert_eq!(cmd.state, original.state);
        assert_eq!(cmd.continent, original.continent);
    }
}
