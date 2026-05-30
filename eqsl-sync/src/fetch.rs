//! eQSL.cc inbox download — confirmations from eQSL get returned as ADIF
//! via DownloadInBox.cfm.

use chrono::NaiveDate;
use thiserror::Error;

const DEFAULT_FETCH_URL: &str = "https://www.eqsl.cc/qslcard/DownloadInBox.cfm";

#[derive(Debug, Clone)]
pub struct EqslFetchConfig {
    pub username: String,
    pub password: String,
    pub fetch_url: String,
}

impl EqslFetchConfig {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            fetch_url: DEFAULT_FETCH_URL.to_string(),
        }
    }

    pub fn with_fetch_url(mut self, url: impl Into<String>) -> Self {
        self.fetch_url = url.into();
        self
    }
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("HTTP error: {0}")]
    Http(String),

    #[error("ADIF parse error: {0}")]
    Parse(String),

    #[error("eQSL rejected request: {0}")]
    Rejected(String),
}

#[derive(Debug, Clone)]
pub struct EqslInboxRecord {
    pub station_callsign: Option<String>,
    pub worked_callsign: String,
    pub qso_date: String, // YYYY-MM-DD
    pub band: Option<String>,
    pub mode: Option<String>,
    pub qsl_rdate: Option<String>, // YYYY-MM-DD
}

pub struct EqslFetchClient {
    config: EqslFetchConfig,
    http: reqwest::Client,
}

impl EqslFetchClient {
    pub fn new(config: EqslFetchConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn with_http(config: EqslFetchConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    pub async fn fetch(
        &self,
        rcvd_since: Option<NaiveDate>,
    ) -> Result<Vec<EqslInboxRecord>, FetchError> {
        let mut req = self.http.get(&self.config.fetch_url).query(&[
            ("UserName", self.config.username.as_str()),
            ("Password", self.config.password.as_str()),
        ]);
        if let Some(date) = rcvd_since {
            req = req.query(&[("RcvdSince", date.format("%Y%m%d").to_string())]);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| FetchError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| FetchError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(FetchError::Http(format!("status {status}: {body}")));
        }

        // eQSL fetch errors come back as 200 with "Error:" preamble, not as
        // an HTTP error code. Detect those before attempting ADIF parse —
        // an "Error:" body would parse as zero-record ADIF and silently
        // hide the real problem.
        let trimmed = body.trim_start().to_ascii_lowercase();
        if trimmed.starts_with("error:") || trimmed.contains("authentication failed") {
            return Err(FetchError::Rejected(body));
        }

        parse_inbox_adif(&body)
    }
}

pub(crate) fn parse_inbox_adif(body: &str) -> Result<Vec<EqslInboxRecord>, FetchError> {
    let file = adif_parser::parse_adi(body).map_err(|e| FetchError::Parse(e.to_string()))?;
    let mut out = Vec::with_capacity(file.records.len());
    for rec in file.iter() {
        let Some(call) = rec.call() else { continue };
        let Some(date) = rec.qso_date() else {
            continue;
        };
        out.push(EqslInboxRecord {
            station_callsign: rec
                .get_value("STATION_CALLSIGN")
                .map(|s| s.to_ascii_uppercase()),
            worked_callsign: call.to_ascii_uppercase(),
            qso_date: format_iso_date(date),
            band: rec.band().map(|s| s.to_ascii_uppercase()),
            mode: rec.mode().map(|s| s.to_ascii_uppercase()),
            qsl_rdate: rec
                .get_value("QSLRDATE")
                .map(|s| format_iso_date(s)),
        });
    }
    Ok(out)
}

fn format_iso_date(d: &str) -> String {
    let s = d.trim();
    if s.len() == 8 {
        format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_INBOX: &str = "eQSL.cc Inbox Download\n\
        <EOH>\n\
        <STATION_CALLSIGN:5>W1ABC<CALL:4>W1AW<QSO_DATE:8>20260508<BAND:3>20M<MODE:3>FT8<QSLRDATE:8>20260510<EOR>\n\
        <STATION_CALLSIGN:5>W1ABC<CALL:6>JA1NUT<QSO_DATE:8>20260508<BAND:3>40M<MODE:2>CW<EOR>\n";

    #[test]
    fn parses_two_inbox_records() {
        let recs = parse_inbox_adif(SAMPLE_INBOX).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].station_callsign.as_deref(), Some("W1ABC"));
        assert_eq!(recs[0].worked_callsign, "W1AW");
        assert_eq!(recs[0].qso_date, "2026-05-08");
        assert_eq!(recs[0].qsl_rdate.as_deref(), Some("2026-05-10"));
    }
}
