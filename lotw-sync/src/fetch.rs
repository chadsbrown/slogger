use chrono::{DateTime, NaiveDate, Utc};
use thiserror::Error;

const DEFAULT_REPORT_URL: &str = "https://lotw.arrl.org/lotwuser/lotwreport.adi";

#[derive(Debug, Clone)]
pub struct LotwFetchConfig {
    pub username: String,
    pub password: String,
    pub report_url: String,
}

impl LotwFetchConfig {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            report_url: DEFAULT_REPORT_URL.to_string(),
        }
    }

    pub fn with_report_url(mut self, url: impl Into<String>) -> Self {
        self.report_url = url.into();
        self
    }
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("HTTP error: {0}")]
    Http(String),

    #[error("ADIF parse error: {0}")]
    Parse(String),
}

/// One row of LotW's QSL report, normalized to the keys we care about for
/// matching against local QSOs and recording confirmation timestamps.
#[derive(Debug, Clone)]
pub struct ConfirmationRecord {
    pub station_callsign: Option<String>,
    pub worked_callsign: String,
    pub qso_date: String, // YYYY-MM-DD
    pub band: Option<String>,
    pub mode: Option<String>,
    pub qsl_rcvd: bool,
    pub qsl_rdate: Option<DateTime<Utc>>,
}

pub struct LotwFetchClient {
    config: LotwFetchConfig,
    http: reqwest::Client,
}

impl LotwFetchClient {
    pub fn new(config: LotwFetchConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn with_http(config: LotwFetchConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    /// Fetch QSO records from the user's LotW account.
    ///
    /// - `since` filters by QSO date (incremental sync).
    /// - `only_confirmed` adds `qso_qsl=yes`, returning only QSOs the
    ///   other station has matched. When `false`, returns every QSO in the
    ///   account — used to verify our uploads landed *and* to pick up
    ///   confirmations in a single round-trip via `qsl_rcvd` per record.
    pub async fn fetch(
        &self,
        since: Option<NaiveDate>,
        only_confirmed: bool,
    ) -> Result<Vec<ConfirmationRecord>, FetchError> {
        let mut req = self
            .http
            .get(&self.config.report_url)
            .query(&[
                ("login", self.config.username.as_str()),
                ("password", self.config.password.as_str()),
                ("qso_query", "1"),
                ("qso_qsldetail", "yes"),
            ]);
        if only_confirmed {
            req = req.query(&[("qso_qsl", "yes")]);
        }
        if let Some(date) = since {
            req = req.query(&[("qso_qsosince", date.format("%Y-%m-%d").to_string())]);
        }

        let resp = req.send().await.map_err(|e| FetchError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| FetchError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(FetchError::Http(format!("status {status}: {body}")));
        }
        parse_report_adif(&body)
    }
}

pub(crate) fn parse_report_adif(body: &str) -> Result<Vec<ConfirmationRecord>, FetchError> {
    let file = adif_parser::parse_adi(body).map_err(|e| FetchError::Parse(e.to_string()))?;
    let mut out = Vec::with_capacity(file.records.len());
    for rec in file.iter() {
        let Some(call) = rec.call() else { continue };
        let Some(date) = rec.qso_date() else { continue };
        let normalized_date = format_iso_date(date);
        out.push(ConfirmationRecord {
            station_callsign: rec.get_value("STATION_CALLSIGN").map(|s| s.to_ascii_uppercase()),
            worked_callsign: call.to_ascii_uppercase(),
            qso_date: normalized_date,
            band: rec.band().map(|s| s.to_ascii_uppercase()),
            mode: rec.mode().map(|s| s.to_ascii_uppercase()),
            qsl_rcvd: rec
                .get_value("QSL_RCVD")
                .map(|v| v.eq_ignore_ascii_case("Y"))
                .unwrap_or(false),
            qsl_rdate: rec
                .get_value("QSLRDATE")
                .and_then(|d| parse_qslrdate(d, rec.get_value("QSLRTIME"))),
        });
    }
    Ok(out)
}

fn format_iso_date(adif_date: &str) -> String {
    // Convert "YYYYMMDD" to "YYYY-MM-DD" for repository matching.
    let s = adif_date.trim();
    if s.len() == 8 {
        format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8])
    } else {
        s.to_string()
    }
}

fn parse_qslrdate(date: &str, time: Option<&str>) -> Option<DateTime<Utc>> {
    let nd = NaiveDate::parse_from_str(date.trim(), "%Y%m%d").ok()?;
    let nt = match time.unwrap_or("0000") {
        t if t.len() == 4 => chrono::NaiveTime::parse_from_str(t, "%H%M").ok()?,
        t if t.len() == 6 => chrono::NaiveTime::parse_from_str(t, "%H%M%S").ok()?,
        _ => chrono::NaiveTime::from_hms_opt(0, 0, 0)?,
    };
    let dt = chrono::NaiveDateTime::new(nd, nt);
    Some(chrono::TimeZone::from_utc_datetime(&Utc, &dt))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_REPORT: &str = "LoTW QSL Report\n\
        <PROGRAMID:4>LoTW<ADIF_VER:5>3.1.4<EOH>\n\
        <STATION_CALLSIGN:5>W1ABC<CALL:4>W1AW<QSO_DATE:8>20260508<TIME_ON:6>183045<BAND:3>20M<MODE:3>FT8<QSL_RCVD:1>Y<QSLRDATE:8>20260510<QSLRTIME:4>1200<EOR>\n\
        <STATION_CALLSIGN:5>W1ABC<CALL:6>JA1NUT<QSO_DATE:8>20260508<TIME_ON:4>1900<BAND:3>40M<MODE:2>CW<QSL_RCVD:1>Y<QSLRDATE:8>20260511<EOR>\n";

    #[test]
    fn parses_two_confirmation_records() {
        let recs = parse_report_adif(SAMPLE_REPORT).unwrap();
        assert_eq!(recs.len(), 2);
        let first = &recs[0];
        assert_eq!(first.station_callsign.as_deref(), Some("W1ABC"));
        assert_eq!(first.worked_callsign, "W1AW");
        assert_eq!(first.qso_date, "2026-05-08");
        assert_eq!(first.band.as_deref(), Some("20M"));
        assert_eq!(first.mode.as_deref(), Some("FT8"));
        assert!(first.qsl_rcvd);
        assert!(first.qsl_rdate.is_some());
    }

    #[test]
    fn handles_missing_optional_fields() {
        let body = "<ADIF_VER:5>3.1.4<EOH>\n<CALL:4>W1AW<QSO_DATE:8>20260508<EOR>\n";
        let recs = parse_report_adif(body).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].worked_callsign, "W1AW");
        assert!(recs[0].qsl_rdate.is_none());
        assert!(!recs[0].qsl_rcvd, "missing QSL_RCVD must default to not-confirmed");
    }

    #[test]
    fn distinguishes_uploaded_from_confirmed() {
        let body = "<ADIF_VER:5>3.1.4<EOH>\n\
            <STATION_CALLSIGN:5>W1ABC<CALL:4>W1AW<QSO_DATE:8>20260508<BAND:3>20M<MODE:3>FT8<QSL_RCVD:1>Y<EOR>\n\
            <STATION_CALLSIGN:5>W1ABC<CALL:6>JA1NUT<QSO_DATE:8>20260508<BAND:3>40M<MODE:2>CW<QSL_RCVD:1>N<EOR>\n";
        let recs = parse_report_adif(body).unwrap();
        assert_eq!(recs.len(), 2);
        assert!(recs[0].qsl_rcvd, "Y means confirmed");
        assert!(!recs[1].qsl_rcvd, "N means uploaded but not confirmed");
    }
}
