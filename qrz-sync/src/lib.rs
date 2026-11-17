//! QRZ.com logbook client: check the access key, insert QSOs one at a
//! time, and read back what the logbook holds and has confirmed.
//!
//! Everything goes through one endpoint, `https://logbook.qrz.com/api`,
//! as a form POST carrying `KEY`, `ACTION` and the action's own fields.
//! A key selects one logbook, and a logbook serves exactly one station
//! callsign. Every action needs the logbook's owner to hold a QRZ
//! subscription; without one (or with a bad key) the answer is `AUTH`.
//!
//! The developer guide is at
//! <https://www.qrz.com/docs/logbook/QRZLogbookAPI.html>. Where the
//! server differs from it, the code follows the server; see `wire`.

mod wire;

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use thiserror::Error;

use wire::Response;

const DEFAULT_API_URL: &str = "https://logbook.qrz.com/api";

/// QRZ requires an identifiable user agent and may rate-limit requests
/// without one.
const USER_AGENT: &str = concat!("slogger/", env!("CARGO_PKG_VERSION"));

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A page of a thousand records arrives in about a second; this only has
/// to stop a stalled connection from holding up the update forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Records asked for per fetch request. QRZ advises paging because a
/// whole logbook in one response can time out.
const PAGE_SIZE: usize = 1000;

#[derive(Debug, Clone)]
pub struct QrzConfig {
    /// The logbook's API access key, from the QRZ logbook settings page.
    pub api_key: String,
    pub api_url: String,
}

impl QrzConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_url: DEFAULT_API_URL.to_string(),
        }
    }

    pub fn with_api_url(mut self, url: impl Into<String>) -> Self {
        self.api_url = url.into();
        self
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum QrzError {
    /// The request never got an answer QRZ's API wrote.
    #[error("HTTP error: {0}")]
    Http(String),

    /// The key is wrong, or the logbook's owner has no subscription.
    /// Nothing else will work until that changes.
    #[error("QRZ refused the API key: {0}")]
    Auth(String),

    /// QRZ answered `FAIL`.
    #[error("QRZ reported a failure: {0}")]
    Failed(String),

    /// An answer that can't be acted on.
    #[error("unexpected answer from QRZ: {0}")]
    Unexpected(String),
}

/// What `STATUS` says about the logbook a key selects.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BookStatus {
    /// The one station callsign the logbook serves.
    pub callsign: Option<String>,
    pub book_name: Option<String>,
    pub owner: Option<String>,
    pub book_id: Option<u64>,
    /// QSOs in the logbook.
    pub count: Option<u64>,
    pub confirmed: Option<u64>,
    /// The logbook only accepts QSOs dated within this range.
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

/// What became of one QSO sent with `INSERT`.
#[derive(Debug, Clone, PartialEq)]
pub enum InsertOutcome {
    /// Added to the logbook under this id.
    Inserted { logid: Option<u64> },
    /// QRZ overwrote a QSO it held as a duplicate.
    Replaced { logid: Option<u64> },
    /// The logbook already holds this QSO; nothing changed.
    Duplicate,
    /// QRZ would not take it, for the reason given.
    Rejected(String),
}

/// One QSO as the logbook holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct QrzRecord {
    /// QRZ's id for the record.
    pub logid: Option<u64>,
    pub station_callsign: Option<String>,
    pub call: String,
    pub start: DateTime<Utc>,
    pub band: Option<String>,
    pub mode: Option<String>,
    /// QRZ has matched it with the other station's log.
    pub confirmed: bool,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub uploaded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct QrzReport {
    pub records: Vec<QrzRecord>,
    /// Records without a call or a readable date and time.
    pub unreadable: usize,
}

pub struct QrzClient {
    config: QrzConfig,
    http: reqwest::Client,
}

impl QrzClient {
    pub fn new(config: QrzConfig) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "QRZ http client: falling back to defaults");
                reqwest::Client::new()
            });
        Self { config, http }
    }

    pub fn with_http(config: QrzConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    /// Ask which logbook the key selects. Cheap, and the way to find a
    /// bad key or a lapsed subscription before sending any QSOs.
    pub async fn status(&self) -> Result<BookStatus, QrzError> {
        let r = self.request(&[("ACTION", "STATUS")]).await?;
        match r.result() {
            Some("OK") => {}
            _ => return Err(failure(&r)),
        }
        let date = |name| r.get(name).and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
        let text = |name| r.get(name).map(str::to_owned);
        Ok(BookStatus {
            callsign: text("CALLSIGN").map(|c| c.to_ascii_uppercase()),
            book_name: text("BOOK_NAME"),
            owner: text("OWNER"),
            book_id: r.number("BOOKID"),
            count: r.number("COUNT"),
            confirmed: r.number("CONFIRMED"),
            start_date: date("START_DATE"),
            end_date: date("END_DATE"),
        })
    }

    /// Insert one QSO. `adif` is that QSO's record, with or without an
    /// ADIF header; only its first record is sent, since QRZ takes one
    /// per request.
    ///
    /// Never sends `OPTION=REPLACE`: that makes QRZ overwrite the QSO it
    /// already holds, discarding its confirmation until the two logs are
    /// matched again. A duplicate is reported as [`InsertOutcome::Duplicate`]
    /// and left as it is.
    pub async fn insert(&self, adif: &str) -> Result<InsertOutcome, QrzError> {
        let record = wire::first_record(adif)
            .ok_or_else(|| QrzError::Unexpected("no complete ADIF record to send".into()))?;
        let r = self.request(&[("ACTION", "INSERT"), ("ADIF", record)]).await?;
        let logid = r.number("LOGID").or_else(|| r.number("LOGIDS"));
        match r.result() {
            Some("OK") => Ok(InsertOutcome::Inserted { logid }),
            Some("REPLACE") => Ok(InsertOutcome::Replaced { logid }),
            Some("FAIL") => {
                let reason = r.get("REASON").unwrap_or("no reason given").to_string();
                if reason.to_ascii_lowercase().contains("duplicate") {
                    Ok(InsertOutcome::Duplicate)
                } else {
                    Ok(InsertOutcome::Rejected(reason))
                }
            }
            _ => Err(failure(&r)),
        }
    }

    /// Every record in the logbook modified on or after `since` (all of
    /// them when `None`), read a page at a time in logid order.
    ///
    /// QRZ compares whole dates, so a caller resuming from where it left
    /// off should pass a date at or before its last read and expect to
    /// see some records again.
    pub async fn fetch_modified_since(
        &self,
        since: Option<NaiveDate>,
    ) -> Result<QrzReport, QrzError> {
        let mut report = QrzReport::default();
        let mut after = 0u64;
        loop {
            let mut option = String::new();
            if let Some(date) = since {
                option.push_str(&format!("MODSINCE:{},", date.format("%Y-%m-%d")));
            }
            option.push_str(&format!("MAX:{PAGE_SIZE},AFTERLOGID:{after}"));
            let r = self.request(&[("ACTION", "FETCH"), ("OPTION", &option)]).await?;
            match r.result() {
                Some("OK") => {}
                // "Nothing matched" also comes back as FAIL, with a
                // count of zero and no reason.
                Some("FAIL") if r.get("REASON").is_none() && r.number("COUNT") == Some(0) => break,
                _ => return Err(failure(&r)),
            }
            let fields = wire::parse_adif(r.get("ADIF").unwrap_or(""));
            let in_page = fields.len();
            let highest = fields.iter().filter_map(logid).max();
            for f in &fields {
                match record(f) {
                    Some(rec) => report.records.push(rec),
                    None => report.unreadable += 1,
                }
            }
            tracing::info!(
                after_logid = after,
                records = in_page,
                total = report.records.len(),
                "QRZ: fetched a page"
            );
            if in_page < PAGE_SIZE {
                break;
            }
            // The next page starts just past this one's highest id. A
            // full page that doesn't move that on would repeat forever.
            match highest {
                Some(id) if id >= after => after = id + 1,
                _ => {
                    return Err(QrzError::Unexpected(
                        "a full page of records carried no usable logid to page from".into(),
                    ));
                }
            }
        }
        Ok(report)
    }

    async fn request(&self, fields: &[(&str, &str)]) -> Result<Response, QrzError> {
        let mut body = format!("KEY={}", urlencoding::encode(self.config.api_key.trim()));
        for (name, value) in fields {
            body.push_str(&format!("&{name}={}", urlencoding::encode(value)));
        }
        let resp = self
            .http
            .post(&self.config.api_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| QrzError::Http(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| QrzError::Http(e.to_string()))?;
        let r = Response::parse(&text);
        // A refused key comes with HTTP 401 and a body saying why, so the
        // body is read before the status is judged.
        if r.result() == Some("AUTH") {
            return Err(QrzError::Auth(
                r.get("REASON").unwrap_or("no reason given").to_string(),
            ));
        }
        if r.result().is_none() {
            let shown: String = text.chars().take(200).collect();
            return Err(if status.is_success() {
                QrzError::Unexpected(shown)
            } else {
                QrzError::Http(format!("status {status}: {shown}"))
            });
        }
        Ok(r)
    }
}

fn failure(r: &Response) -> QrzError {
    match (r.result(), r.get("REASON")) {
        (Some("FAIL"), reason) => QrzError::Failed(reason.unwrap_or("no reason given").to_string()),
        (other, _) => QrzError::Unexpected(format!("RESULT={}", other.unwrap_or(""))),
    }
}

fn logid(fields: &HashMap<String, String>) -> Option<u64> {
    fields.get("app_qrzlog_logid")?.parse().ok()
}

fn record(f: &HashMap<String, String>) -> Option<QrzRecord> {
    let text = |name: &str| f.get(name).filter(|v| !v.is_empty()).cloned();
    let date = |name: &str| {
        let d = NaiveDate::parse_from_str(f.get(name)?, "%Y%m%d").ok()?;
        Some(d.and_time(NaiveTime::MIN).and_utc())
    };
    let call = text("call")?.to_ascii_uppercase();
    let day = NaiveDate::parse_from_str(f.get("qso_date")?, "%Y%m%d").ok()?;
    let start = day.and_time(parse_time(f.get("time_on")?)?).and_utc();
    Some(QrzRecord {
        logid: logid(f),
        station_callsign: text("station_callsign").map(|c| c.to_ascii_uppercase()),
        call,
        start,
        band: text("band").map(|b| b.to_ascii_uppercase()),
        mode: text("mode").map(|m| m.to_ascii_uppercase()),
        // QRZ's own verdict: C once both logs match, N until then.
        confirmed: f
            .get("app_qrzlog_status")
            .is_some_and(|s| s.eq_ignore_ascii_case("C")),
        confirmed_at: date("app_qrzlog_qsldate"),
        uploaded_at: date("qrzcom_qso_upload_date"),
    })
}

/// `HHMM` or `HHMMSS`.
fn parse_time(t: &str) -> Option<NaiveTime> {
    let n = |range: std::ops::Range<usize>| t.get(range)?.parse::<u32>().ok();
    match t.len() {
        4 => NaiveTime::from_hms_opt(n(0..2)?, n(2..4)?, 0),
        6 => NaiveTime::from_hms_opt(n(0..2)?, n(2..4)?, n(4..6)?),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A server that answers each request with the next `(status, body)`
    /// and records what it was sent.
    async fn mock(responses: Vec<(u16, String)>) -> (QrzClient, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 16384];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                seen2
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        let config = QrzConfig::new("KEY-123").with_api_url(format!("http://{addr}/api"));
        (QrzClient::new(config), seen)
    }

    fn ok(body: &str) -> (u16, String) {
        (200, body.to_string())
    }

    const QSO: &str = "<ADIF_VER:5>3.1.4<EOH>\n\
        <STATION_CALLSIGN:5>K1ABC<CALL:4>W1AW<QSO_DATE:8>20260508<TIME_ON:6>183045<BAND:3>20M<MODE:3>FT8<EOR>\n";

    #[tokio::test]
    async fn requests_identify_the_application_and_carry_the_key() {
        let (client, seen) = mock(vec![ok("RESULT=OK&CALLSIGN=k1abc&COUNT=12")]).await;
        client.status().await.unwrap();
        let request = seen.lock().unwrap()[0].to_ascii_lowercase();
        assert!(request.contains(&format!("user-agent: {}", USER_AGENT.to_ascii_lowercase())));
        assert!(request.contains("key=key-123&action=status"));
    }

    #[tokio::test]
    async fn status_reads_the_logbook_it_selects() {
        let (client, _) = mock(vec![ok(
            "BOOKID=42&COUNT=100&CONFIRMED=40&START_DATE=2019-06-01&CALLSIGN=k1abc\
             &END_DATE=2030-04-27&BOOK_NAME=K1ABC Logbook&RESULT=OK&OWNER=K1ABC&ACTION=STATUS",
        )])
        .await;
        let status = client.status().await.unwrap();
        assert_eq!(status.callsign.as_deref(), Some("K1ABC"));
        assert_eq!(status.book_name.as_deref(), Some("K1ABC Logbook"));
        assert_eq!((status.count, status.confirmed), (Some(100), Some(40)));
        assert_eq!(status.start_date, NaiveDate::from_ymd_opt(2019, 6, 1));
        assert_eq!(status.end_date, NaiveDate::from_ymd_opt(2030, 4, 27));
    }

    #[tokio::test]
    async fn a_refused_key_is_an_auth_error_whatever_the_action() {
        let refused = || (401, "STATUS=AUTH&RESULT=AUTH&REASON=invalid api key\n&EXTENDED=".to_string());
        let (client, _) = mock(vec![refused(), refused(), refused()]).await;
        let auth = QrzError::Auth("invalid api key".into());
        assert_eq!(client.status().await, Err(auth));
        assert!(matches!(client.insert(QSO).await, Err(QrzError::Auth(_))));
        assert!(matches!(client.fetch_modified_since(None).await, Err(QrzError::Auth(_))));
    }

    #[tokio::test]
    async fn an_answer_that_is_not_the_apis_is_an_http_error() {
        let (client, _) = mock(vec![(502, "<html>Bad Gateway</html>".into())]).await;
        assert!(matches!(client.status().await, Err(QrzError::Http(_))));
    }

    #[tokio::test]
    async fn insert_sends_one_record_without_the_header_and_never_replaces() {
        let (client, seen) = mock(vec![ok("RESULT=OK&LOGID=130877825&COUNT=1")]).await;
        let outcome = client.insert(QSO).await.unwrap();
        assert_eq!(outcome, InsertOutcome::Inserted { logid: Some(130877825) });
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.contains("ACTION=INSERT&ADIF=%3CSTATION_CALLSIGN"));
        assert!(!request.contains("ADIF_VER"));
        assert!(!request.contains("OPTION"));
    }

    #[tokio::test]
    async fn insert_tells_a_duplicate_from_a_rejection() {
        let (client, _) = mock(vec![
            ok("RESULT=FAIL&REASON=Unable to add QSO to database: duplicate&EXTENDED="),
            ok("RESULT=FAIL&REASON=outside the logbook date range"),
            ok("RESULT=REPLACE&LOGID=7&COUNT=1"),
            ok("RESULT=OK&LOGIDS=8&COUNT=1"),
        ])
        .await;
        assert_eq!(client.insert(QSO).await, Ok(InsertOutcome::Duplicate));
        assert_eq!(
            client.insert(QSO).await,
            Ok(InsertOutcome::Rejected("outside the logbook date range".into()))
        );
        assert_eq!(client.insert(QSO).await, Ok(InsertOutcome::Replaced { logid: Some(7) }));
        assert_eq!(client.insert(QSO).await, Ok(InsertOutcome::Inserted { logid: Some(8) }));
    }

    #[tokio::test]
    async fn insert_refuses_text_with_no_record_without_asking_qrz() {
        let (client, seen) = mock(vec![]).await;
        assert!(matches!(client.insert("<CALL:4>W1AW").await, Err(QrzError::Unexpected(_))));
        assert!(seen.lock().unwrap().is_empty());
    }

    fn page(logids: std::ops::Range<u64>) -> (u16, String) {
        let mut adif = String::new();
        for id in logids.clone() {
            adif.push_str(&format!(
                "&lt;app_qrzlog_logid:{}&gt;{id}\n&lt;call:4&gt;W1AW\n&lt;qso_date:8&gt;20260508\n\
                 &lt;time_on:4&gt;1830\n&lt;band:3&gt;20m\n&lt;eor&gt;\n\n",
                id.to_string().len()
            ));
        }
        ok(&format!("COUNT={}&RESULT=OK&ADIF={adif}", logids.count()))
    }

    #[tokio::test]
    async fn fetch_pages_by_logid_until_a_short_page() {
        let first = 100..100 + PAGE_SIZE as u64;
        let (client, seen) = mock(vec![page(first.clone()), page(5000..5003)]).await;
        let since = NaiveDate::from_ymd_opt(2026, 11, 1);
        let report = client.fetch_modified_since(since).await.unwrap();
        assert_eq!(report.records.len(), PAGE_SIZE + 3);
        assert_eq!(report.unreadable, 0);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let option = |after: u64| {
            format!("OPTION=MODSINCE%3A2026-11-01%2CMAX%3A{PAGE_SIZE}%2CAFTERLOGID%3A{after}")
        };
        assert!(seen[0].contains(&option(0)));
        assert!(seen[1].contains(&option(first.end)));
    }

    #[tokio::test]
    async fn fetch_reads_nothing_matched_as_an_empty_report() {
        let (client, seen) = mock(vec![ok("RESULT=FAIL&COUNT=0")]).await;
        assert_eq!(client.fetch_modified_since(None).await, Ok(QrzReport::default()));
        assert!(seen.lock().unwrap()[0].contains("OPTION=MAX%3A"));
    }

    #[tokio::test]
    async fn fetch_reports_a_real_failure() {
        let (client, _) = mock(vec![ok("RESULT=FAIL&COUNT=")]).await;
        assert!(matches!(client.fetch_modified_since(None).await, Err(QrzError::Failed(_))));
    }

    #[test]
    fn a_record_carries_qrzs_confirmation_and_its_dates() {
        let adif = "<app_qrzlog_logid:9>423002969<station_callsign:5>k1abc<call:5>k0mld\
            <qso_date:8>20190606<time_on:4>0143<band:2>6M<mode:3>FT8<app_qrzlog_status:1>C\
            <app_qrzlog_qsldate:8>20240506<qrzcom_qso_upload_date:8>20190606<eor>\
            <call:4>K5MM<qso_date:8>20190607<time_on:6>000930<app_qrzlog_status:1>N<eor>\
            <call:4>K1AB<qso_date:8>20190607<eor>";
        let fields = wire::parse_adif(adif);
        let first = record(&fields[0]).unwrap();
        assert_eq!(first.logid, Some(423002969));
        assert_eq!(first.station_callsign.as_deref(), Some("K1ABC"));
        assert_eq!(first.call, "K0MLD");
        assert_eq!(first.start.to_rfc3339(), "2019-06-06T01:43:00+00:00");
        assert!(first.confirmed);
        assert_eq!(first.confirmed_at.unwrap().to_rfc3339(), "2024-05-06T00:00:00+00:00");
        assert_eq!(first.uploaded_at.unwrap().to_rfc3339(), "2019-06-06T00:00:00+00:00");

        let second = record(&fields[1]).unwrap();
        assert!(!second.confirmed);
        assert_eq!(second.start.to_rfc3339(), "2019-06-07T00:09:30+00:00");
        assert_eq!((second.confirmed_at, second.band.clone()), (None, None));

        // No time: can't be matched to a QSO.
        assert_eq!(record(&fields[2]), None);
    }
}
