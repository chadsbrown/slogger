//! Reading LoTW's reports of the QSOs in our account.
//!
//! Sync is incremental: each report kind is asked only for what changed
//! since the last time it was read, using the watermark LoTW itself
//! hands back in the report header (`APP_LoTW_LASTQSL` /
//! `APP_LoTW_LASTQSORX`). The caller stores that value and passes it as
//! `since` next time. See ARRL's "Developer: Query QSOs and QSLs" page.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use thiserror::Error;

const DEFAULT_REPORT_URL: &str = "https://lotw.arrl.org/lotwuser/lotwreport.adi";

/// The `since` to send when there is no watermark: the first sync, or a
/// full resync. LoTW substitutes a value it remembers from an earlier
/// query when the parameter is missing or empty, so "everything" has to
/// be asked for with an explicit early date.
pub const SINCE_BEGINNING: &str = "1900-01-01";

/// Longest error text taken from an LoTW error page; the rest is cut so a
/// status line stays one line.
const MAX_ERROR_LEN: usize = 160;

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

    /// LoTW answered with its HTML error page instead of ADIF. Holds the
    /// page's text, tags stripped and shortened.
    #[error("LoTW: {0}")]
    Service(String),

    /// Records arrived but the end-of-file marker didn't: the download
    /// may have been cut short, so none of it is applied.
    #[error("report looks incomplete (no end-of-file marker); nothing applied")]
    Truncated,
}

/// The two reports an incremental sync reads, each with its own
/// watermark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LotwReportKind {
    /// `qso_qsl=yes`: QSOs the other station has matched, by when the
    /// match was made or updated (which includes award credit changes).
    Confirmations,
    /// `qso_qsl=no`: every QSO LoTW accepted from us, by upload time —
    /// what proves an upload landed.
    Uploads,
}

impl LotwReportKind {
    /// Key this report's watermark is stored under.
    pub fn watermark_key(self) -> &'static str {
        match self {
            Self::Confirmations => "qsl_since",
            Self::Uploads => "qso_rx_since",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Confirmations => "confirmations",
            Self::Uploads => "uploads",
        }
    }

    /// Header field holding the value to send as `since` next time.
    fn watermark_field(self) -> &'static str {
        match self {
            Self::Confirmations => "APP_LOTW_LASTQSL",
            Self::Uploads => "APP_LOTW_LASTQSORX",
        }
    }
}

/// The report parameters for `kind`, credentials excluded. `since` is
/// always sent: LoTW fills a missing one from server-side memory.
pub fn report_query(kind: LotwReportKind, since: &str) -> Vec<(&'static str, String)> {
    let mut q = vec![
        ("qso_query", "1".to_string()),
        // STATION_CALLSIGN on every record, so a QSO made under another
        // call in the same account can't match.
        ("qso_withown", "yes".to_string()),
    ];
    match kind {
        LotwReportKind::Confirmations => {
            q.push(("qso_qsl", "yes".to_string()));
            q.push(("qso_qsldetail", "yes".to_string()));
            q.push(("qso_qslsince", since.to_string()));
        }
        LotwReportKind::Uploads => {
            q.push(("qso_qsl", "no".to_string()));
            q.push(("qso_qsorxsince", since.to_string()));
        }
    }
    q
}

/// One record of an LoTW report: one of our QSOs as LoTW holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct LotwRecord {
    pub station_callsign: Option<String>,
    pub worked_callsign: String,
    /// QSO start, from `QSO_DATE` + `TIME_ON` (or
    /// `APP_LoTW_QSO_TIMESTAMP`). LoTW may drop the seconds.
    pub qso_start: DateTime<Utc>,
    pub band: Option<String>,
    pub mode: Option<String>,
    /// `APP_LoTW_MODEGROUP`: CW / PHONE / DATA.
    pub mode_group: Option<String>,
    pub qsl_rcvd: bool,
    /// When the confirmation arrived: `QSLRDATE`, else `APP_LoTW_RXQSL`.
    pub qsl_rdate: Option<DateTime<Utc>>,
    /// `APP_LoTW_RXQSO`: when LoTW received our upload.
    pub rx_qso: Option<DateTime<Utc>>,
    /// ADIF credit lists, raw. LoTW's own `APP_LoTW_CREDIT_*`
    /// enumeration is not read.
    pub credit_granted: Option<String>,
    pub credit_submitted: Option<String>,
    /// The QSLing station's details (`qso_qsldetail=yes`).
    pub dxcc: Option<u16>,
    pub state: Option<String>,
    pub gridsquare: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LotwReport {
    pub records: Vec<LotwRecord>,
    /// Records without a call or a readable QSO date and time.
    pub unreadable: usize,
    /// The header's `APP_LoTW_LASTQSL` / `APP_LoTW_LASTQSORX`, verbatim
    /// (its timezone isn't documented, so it is sent back as is). `None`
    /// when the report held no records: what LoTW puts there then isn't
    /// documented, and the previous watermark is still right.
    pub watermark: Option<String>,
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

    /// Read one report, everything LoTW has for it on or after `since`
    /// (a watermark from an earlier report, or `SINCE_BEGINNING`).
    ///
    /// The password travels in the query string (LoTW's design), so no
    /// error from here carries the URL.
    pub async fn fetch(&self, kind: LotwReportKind, since: &str) -> Result<LotwReport, FetchError> {
        let resp = self
            .http
            .get(&self.config.report_url)
            .query(&[
                ("login", self.config.username.as_str()),
                ("password", self.config.password.as_str()),
            ])
            .query(&report_query(kind, since))
            .send()
            .await
            .map_err(|e| self.http_error(e))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| self.http_error(e))?;
        if !status.is_success() {
            let text = self.redact(page_text(&body));
            return Err(FetchError::Http(if text.is_empty() {
                format!("status {status}")
            } else {
                format!("status {status}: {text}")
            }));
        }
        parse_report(&body, kind).map_err(|e| match e {
            FetchError::Service(msg) => FetchError::Service(self.redact(msg)),
            other => other,
        })
    }

    fn http_error(&self, e: reqwest::Error) -> FetchError {
        FetchError::Http(self.redact(e.without_url().to_string()))
    }

    /// Belt and braces: never let the password through in a message.
    fn redact(&self, msg: String) -> String {
        if self.config.password.is_empty() {
            msg
        } else {
            msg.replace(&self.config.password, "***")
        }
    }
}

/// Parse a report body. An HTML page (no ADIF end-of-header tag) is how
/// LoTW reports a failed query.
pub fn parse_report(body: &str, kind: LotwReportKind) -> Result<LotwReport, FetchError> {
    let adi = tokenize(body).ok_or_else(|| {
        let text = page_text(body);
        FetchError::Service(if text.is_empty() {
            "unexpected response (no ADIF header)".into()
        } else {
            text
        })
    })?;
    let total = adi.records.len();
    if total > 0 && !adi.complete {
        return Err(FetchError::Truncated);
    }

    let mut report = LotwReport::default();
    for rec in &adi.records {
        match to_record(rec) {
            Some(r) => report.records.push(r),
            None => report.unreadable += 1,
        }
    }
    if total > 0 {
        report.watermark = lookup(&adi.header, kind.watermark_field())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(String::from);
    }
    Ok(report)
}

type Fields = Vec<(String, String)>;

struct Adi {
    header: Fields,
    records: Vec<Fields>,
    /// `<APP_LoTW_EOF>` was seen: the file arrived whole.
    complete: bool,
}

/// Split an ADI body into header fields and records. `None` when there
/// is no `<EOH>`.
///
/// Not `adif_parser::parse_adi`: it uppercases the whole remaining input
/// on every tag, which is quadratic and takes hours on a full report of
/// tens of thousands of QSOs (see `fast_parse_adi` in the importer, which
/// this follows). Lengths count bytes; a value cut mid-character is
/// decoded lossily rather than panicking.
fn tokenize(body: &str) -> Option<Adi> {
    let bytes = body.as_bytes();
    let mut adi = Adi {
        header: Vec::new(),
        records: Vec::new(),
        complete: false,
    };
    let mut in_header = true;
    let mut current: Fields = Vec::new();
    let mut pos = 0;

    while let Some(i) = bytes[pos..].iter().position(|&b| b == b'<') {
        pos += i + 1;
        let name_start = pos;
        while pos < bytes.len() && bytes[pos] != b':' && bytes[pos] != b'>' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let name = String::from_utf8_lossy(&bytes[name_start..pos]).to_ascii_uppercase();

        if bytes[pos] == b'>' {
            pos += 1;
            match name.as_str() {
                "EOH" if in_header => in_header = false,
                "EOR" if !in_header => {
                    if !current.is_empty() {
                        adi.records.push(std::mem::take(&mut current));
                    }
                }
                "APP_LOTW_EOF" if !in_header => {
                    adi.complete = true;
                    break;
                }
                // Before <EOH> an HTML page is all "marker tags"; skip
                // them along with any marker we don't know.
                _ => {}
            }
            continue;
        }

        // <NAME:LEN[:TYPE]>VALUE
        pos += 1;
        let len_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        let len: usize = std::str::from_utf8(&bytes[len_start..pos])
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        while pos < bytes.len() && bytes[pos] != b'>' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        pos += 1;
        let end = (pos + len).min(bytes.len());
        let value = String::from_utf8_lossy(&bytes[pos..end]).into_owned();
        pos = end;
        if name.is_empty() {
            continue;
        }
        if in_header {
            adi.header.push((name, value));
        } else {
            current.push((name, value));
        }
    }
    // A record with no <EOR> is what a cut-off download ends with; it
    // only counts if the file claims to be whole.
    if !current.is_empty() {
        adi.records.push(current);
    }
    (!in_header).then_some(adi)
}

fn lookup<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.trim().is_empty())
}

fn to_record(f: &[(String, String)]) -> Option<LotwRecord> {
    let call = lookup(f, "CALL")?.trim().to_ascii_uppercase();
    let qso_start = match (lookup(f, "QSO_DATE"), lookup(f, "TIME_ON")) {
        (Some(d), Some(t)) => parse_adif_datetime(d, t),
        _ => None,
    }
    .or_else(|| lookup(f, "APP_LOTW_QSO_TIMESTAMP").and_then(parse_lotw_datetime))?;
    let upper = |name| lookup(f, name).map(|s| s.trim().to_ascii_uppercase());
    Some(LotwRecord {
        station_callsign: upper("STATION_CALLSIGN"),
        worked_callsign: call,
        qso_start,
        band: upper("BAND"),
        mode: upper("MODE"),
        mode_group: upper("APP_LOTW_MODEGROUP"),
        qsl_rcvd: lookup(f, "QSL_RCVD").is_some_and(|v| v.trim().eq_ignore_ascii_case("Y")),
        qsl_rdate: lookup(f, "QSLRDATE")
            .and_then(|d| parse_adif_date(d, lookup(f, "QSLRTIME")))
            .or_else(|| lookup(f, "APP_LOTW_RXQSL").and_then(parse_lotw_datetime)),
        rx_qso: lookup(f, "APP_LOTW_RXQSO").and_then(parse_lotw_datetime),
        credit_granted: lookup(f, "CREDIT_GRANTED").map(String::from),
        credit_submitted: lookup(f, "CREDIT_SUBMITTED").map(String::from),
        dxcc: lookup(f, "DXCC").and_then(|v| v.trim().parse().ok()),
        state: upper("STATE"),
        gridsquare: upper("GRIDSQUARE"),
    })
}

fn parse_time(t: &str) -> Option<NaiveTime> {
    let t = t.trim();
    match t.len() {
        4 => NaiveTime::parse_from_str(t, "%H%M").ok(),
        6 => NaiveTime::parse_from_str(t, "%H%M%S").ok(),
        _ => None,
    }
}

/// ADIF `QSO_DATE` (YYYYMMDD) + `TIME_ON` (HHMM or HHMMSS — LoTW's
/// choice isn't documented).
fn parse_adif_datetime(date: &str, time: &str) -> Option<DateTime<Utc>> {
    let d = NaiveDate::parse_from_str(date.trim(), "%Y%m%d").ok()?;
    Some(Utc.from_utc_datetime(&NaiveDateTime::new(d, parse_time(time)?)))
}

/// `QSLRDATE` with optional `QSLRTIME`; midnight when the time is absent
/// or unreadable.
fn parse_adif_date(date: &str, time: Option<&str>) -> Option<DateTime<Utc>> {
    let d = NaiveDate::parse_from_str(date.trim(), "%Y%m%d").ok()?;
    let t = time.and_then(parse_time).unwrap_or(NaiveTime::MIN);
    Some(Utc.from_utc_datetime(&NaiveDateTime::new(d, t)))
}

/// LoTW's own date-times (`APP_LoTW_RXQSO`, `APP_LoTW_QSO_TIMESTAMP`,
/// …). Documented as `YYYY-MM-DD HH:MM:SS` or ISO-8601 without saying
/// exactly which, so either separator, an optional `Z`, missing seconds
/// and a bare date are all accepted, read as UTC.
fn parse_lotw_datetime(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    let s = s.trim_end_matches(['Z', 'z']).replacen('T', " ", 1);
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&s, fmt) {
            return Some(Utc.from_utc_datetime(&dt));
        }
    }
    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
        .ok()
        .map(|d| Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN)))
}

/// The readable text of an HTML page, for an error message: tags,
/// scripts and styles removed, common entities decoded, whitespace
/// collapsed, cut to `MAX_ERROR_LEN` characters.
fn page_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        out.push(' ');
        let after = &rest[lt..];
        // A script or style block is skipped whole, up to its closing tag.
        let opening: String = after
            .chars()
            .skip(1)
            .take(6)
            .collect::<String>()
            .to_ascii_lowercase();
        let block = ["script", "style"]
            .into_iter()
            .find(|t| opening.starts_with(t));
        let skip_to = block.and_then(|t| after.to_ascii_lowercase().find(&format!("</{t}")));
        let from = skip_to.unwrap_or(0);
        rest = match after[from..].find('>') {
            Some(gt) => &after[from + gt + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_ERROR_LEN {
        let cut: String = collapsed.chars().take(MAX_ERROR_LEN).collect();
        format!("{}…", cut.trim_end())
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QSL_REPORT: &str = "ARRL Logbook of the World Status Report\n\
        Generated at 2026-05-12 10:00:00\n\
        <PROGRAMID:4>LoTW\n\
        <APP_LoTW_LASTQSL:19>2026-05-11 14:02:33\n\
        <APP_LoTW_NUMREC:1>2\n\
        <eoh>\n\
        <APP_LoTW_OWNCALL:5>K1ABC<STATION_CALLSIGN:5>K1ABC<CALL:4>W1AW<BAND:3>20M<MODE:3>FT8\
        <APP_LoTW_MODEGROUP:4>DATA<QSO_DATE:8>20260508<TIME_ON:6>183045\
        <APP_LoTW_QSO_TIMESTAMP:20>2026-05-08T18:30:45Z<QSL_RCVD:1>Y<QSLRDATE:8>20260510\
        <APP_LoTW_RXQSL:19>2026-05-10 12:00:01<DXCC:3>291<STATE:2>CT<GRIDSQUARE:6>FN31pr\
        <CREDIT_GRANTED:9>DXCC:LOTW<CREDIT_SUBMITTED:13>WAS:LOTW,DXCC<APP_LoTW_CREDIT_GRANTED:4>DXCC\n\
        <eor>\n\
        <STATION_CALLSIGN:5>K1ABC<CALL:6>JA1NUT<BAND:3>40M<MODE:2>CW<QSO_DATE:8>20260508\
        <TIME_ON:4>1900<QSL_RCVD:1>Y<APP_LoTW_RXQSL:19>2026-05-11 14:02:33\n\
        <eor>\n\
        <APP_LoTW_EOF>\n";

    #[test]
    fn query_always_sends_since_and_owncall() {
        let q = report_query(LotwReportKind::Confirmations, "2026-05-11 14:02:33");
        let get = |k| q.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("qso_query"), Some("1"));
        assert_eq!(get("qso_qsl"), Some("yes"));
        assert_eq!(get("qso_qsldetail"), Some("yes"));
        assert_eq!(get("qso_withown"), Some("yes"));
        assert_eq!(get("qso_qslsince"), Some("2026-05-11 14:02:33"));
        assert_eq!(get("qso_qsorxsince"), None);

        let q = report_query(LotwReportKind::Uploads, SINCE_BEGINNING);
        let get = |k| q.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("qso_qsl"), Some("no"));
        assert_eq!(get("qso_withown"), Some("yes"));
        assert_eq!(get("qso_qsorxsince"), Some("1900-01-01"));
        assert_eq!(get("qso_qslsince"), None);
        assert!(q.iter().all(|(n, _)| *n != "password" && *n != "login"));
    }

    #[test]
    fn parses_confirmations_with_detail_credit_and_watermark() {
        let r = parse_report(QSL_REPORT, LotwReportKind::Confirmations).unwrap();
        assert_eq!(r.watermark.as_deref(), Some("2026-05-11 14:02:33"));
        assert_eq!(r.unreadable, 0);
        assert_eq!(r.records.len(), 2);
        let a = &r.records[0];
        assert_eq!(a.station_callsign.as_deref(), Some("K1ABC"));
        assert_eq!(a.worked_callsign, "W1AW");
        assert_eq!(
            a.qso_start,
            Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 45).unwrap()
        );
        assert_eq!(a.band.as_deref(), Some("20M"));
        assert_eq!(a.mode_group.as_deref(), Some("DATA"));
        assert!(a.qsl_rcvd);
        assert_eq!(
            a.qsl_rdate,
            Some(Utc.with_ymd_and_hms(2026, 5, 10, 0, 0, 0).unwrap())
        );
        assert_eq!(a.credit_granted.as_deref(), Some("DXCC:LOTW"));
        assert_eq!(a.credit_submitted.as_deref(), Some("WAS:LOTW,DXCC"));
        assert_eq!(a.dxcc, Some(291));
        assert_eq!(a.state.as_deref(), Some("CT"));
        assert_eq!(a.gridsquare.as_deref(), Some("FN31PR"));

        // HHMM time; no QSLRDATE, so the receive time stands in.
        let b = &r.records[1];
        assert_eq!(
            b.qso_start,
            Utc.with_ymd_and_hms(2026, 5, 8, 19, 0, 0).unwrap()
        );
        assert_eq!(
            b.qsl_rdate,
            Some(Utc.with_ymd_and_hms(2026, 5, 11, 14, 2, 33).unwrap())
        );
    }

    #[test]
    fn uploads_report_reads_its_own_watermark_and_rx_time() {
        let body = "<APP_LoTW_LASTQSORX:19>2026-05-12 09:15:00<EOH>\
            <STATION_CALLSIGN:5>K1ABC<CALL:4>W1AW<BAND:3>20M<MODE:2>CW\
            <APP_LoTW_QSO_TIMESTAMP:19>2026-05-08T18:30:00<APP_LoTW_RXQSO:19>2026-05-12 09:15:00\
            <QSL_RCVD:1>N<EOR><APP_LoTW_EOF>";
        let r = parse_report(body, LotwReportKind::Uploads).unwrap();
        assert_eq!(r.watermark.as_deref(), Some("2026-05-12 09:15:00"));
        let rec = &r.records[0];
        assert!(!rec.qsl_rcvd);
        // No QSO_DATE/TIME_ON: the combined timestamp is used.
        assert_eq!(
            rec.qso_start,
            Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 0).unwrap()
        );
        assert_eq!(
            rec.rx_qso,
            Some(Utc.with_ymd_and_hms(2026, 5, 12, 9, 15, 0).unwrap())
        );
        // The confirmations watermark field isn't this report's.
        assert!(
            parse_report(body, LotwReportKind::Confirmations)
                .unwrap()
                .watermark
                .is_none()
        );
    }

    #[test]
    fn empty_report_leaves_the_watermark_alone() {
        let body =
            "<APP_LoTW_LASTQSL:19>2026-05-12 09:15:00<APP_LoTW_NUMREC:1>0<EOH><APP_LoTW_EOF>";
        let r = parse_report(body, LotwReportKind::Confirmations).unwrap();
        assert!(r.records.is_empty());
        assert_eq!(r.watermark, None);
        // Not even an EOF marker: still fine when there are no records.
        let r = parse_report("<EOH>\n", LotwReportKind::Uploads).unwrap();
        assert!(r.records.is_empty());
    }

    #[test]
    fn records_without_eof_marker_are_treated_as_truncated() {
        let body = "<APP_LoTW_LASTQSL:19>2026-05-12 09:15:00<EOH>\
            <CALL:4>W1AW<QSO_DATE:8>20260508<TIME_ON:4>1830<BAND:3>20M<EOR>\
            <CALL:4>W1AW<QSO_DATE:8>20260508<TIME_ON:4>18";
        assert!(matches!(
            parse_report(body, LotwReportKind::Confirmations),
            Err(FetchError::Truncated)
        ));
    }

    #[test]
    fn html_error_page_becomes_a_short_message() {
        let body = "<!DOCTYPE html><html><head><title>LoTW</title>\
            <style>body { color: red; }</style><script>var x = '<b>';</script></head>\
            <body><h1>Logbook of the World</h1><p>Username/password incorrect &amp; \
            please try again.</p></body></html>";
        match parse_report(body, LotwReportKind::Uploads) {
            Err(FetchError::Service(msg)) => {
                assert_eq!(
                    msg,
                    "LoTW Logbook of the World Username/password incorrect & please try again."
                );
            }
            other => panic!("expected a service error, got {other:?}"),
        }
        let long = format!("<html><body>{}</body></html>", "word ".repeat(200));
        match parse_report(&long, LotwReportKind::Uploads) {
            Err(FetchError::Service(msg)) => {
                assert!(msg.chars().count() <= MAX_ERROR_LEN + 1);
                assert!(msg.ends_with('…'));
            }
            other => panic!("expected a service error, got {other:?}"),
        }
    }

    #[test]
    fn records_missing_call_or_time_are_counted_not_guessed() {
        let body = "<EOH><CALL:4>W1AW<QSO_DATE:8>20260508<BAND:3>20M<EOR>\
            <QSO_DATE:8>20260508<TIME_ON:4>1830<EOR>\
            <CALL:5>K1ABC<QSO_DATE:8>20260508<TIME_ON:4>1830<BAND:3>20M<EOR><APP_LoTW_EOF>";
        let r = parse_report(body, LotwReportKind::Uploads).unwrap();
        assert_eq!(r.records.len(), 1);
        assert_eq!(r.unreadable, 2);
        assert!(
            !r.records[0].qsl_rcvd,
            "missing QSL_RCVD is not a confirmation"
        );
        assert_eq!(r.records[0].station_callsign, None);
    }

    #[test]
    fn lotw_datetimes_in_either_layout() {
        let want = Utc.with_ymd_and_hms(2026, 5, 8, 18, 30, 45).unwrap();
        for s in [
            "2026-05-08 18:30:45",
            "2026-05-08T18:30:45",
            "2026-05-08T18:30:45Z",
            "2026-05-08T18:30:45+00:00",
        ] {
            assert_eq!(parse_lotw_datetime(s), Some(want), "{s}");
        }
        assert_eq!(
            parse_lotw_datetime("2026-05-08"),
            Some(Utc.with_ymd_and_hms(2026, 5, 8, 0, 0, 0).unwrap())
        );
        assert_eq!(parse_lotw_datetime("garbage"), None);
    }
}
