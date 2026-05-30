//! QRZ.com logbook upload — POST ADIF records to `logbook.qrz.com/api`.
//!
//! Each request inserts ONE QSO and returns `RESULT=OK&LOGID=…` or
//! `RESULT=FAIL&REASON=…`. The wrapper here splits a multi-record ADIF
//! string at the `<EOR>` boundary and sends each record separately,
//! since QRZ doesn't accept multi-record bodies on the INSERT action.

use thiserror::Error;

const DEFAULT_UPLOAD_URL: &str = "https://logbook.qrz.com/api";

#[derive(Debug, Clone)]
pub struct QrzConfig {
    /// QRZ logbook API key (per-account; visible on the QRZ logbook page).
    pub api_key: String,
    pub upload_url: String,
}

impl QrzConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            upload_url: DEFAULT_UPLOAD_URL.to_string(),
        }
    }

    pub fn with_upload_url(mut self, url: impl Into<String>) -> Self {
        self.upload_url = url.into();
        self
    }
}

#[derive(Debug, Error)]
pub enum UploadError {
    #[error("HTTP error: {0}")]
    Http(String),

    #[error("QRZ rejected: {0}")]
    Rejected(String),
}

#[derive(Debug, Clone)]
pub struct UploadOutcome {
    /// Number of records that returned `RESULT=OK`.
    pub accepted: usize,
    /// Number of records that returned a non-OK response. Each is
    /// reflected once in `errors`.
    pub rejected: usize,
    pub errors: Vec<String>,
}

pub struct QrzUploadClient {
    config: QrzConfig,
    http: reqwest::Client,
}

impl QrzUploadClient {
    pub fn new(config: QrzConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn with_http(config: QrzConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    pub fn config(&self) -> &QrzConfig {
        &self.config
    }

    pub async fn upload_adif(&self, adif: &str) -> Result<UploadOutcome, UploadError> {
        let mut outcome = UploadOutcome {
            accepted: 0,
            rejected: 0,
            errors: Vec::new(),
        };
        for record in split_records(adif) {
            match self.upload_record(&record).await {
                Ok(()) => outcome.accepted += 1,
                Err(UploadError::Rejected(msg)) => {
                    outcome.rejected += 1;
                    outcome.errors.push(msg);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(outcome)
    }

    async fn upload_record(&self, adif_record: &str) -> Result<(), UploadError> {
        // QRZ wants application/x-www-form-urlencoded with the ADIF as
        // a single field value. Encode the ADIF text — angle brackets in
        // ADIF tags would break naive URL parsing otherwise.
        let body = format!(
            "KEY={}&ACTION=INSERT&ADIF={}",
            urlencoding::encode(&self.config.api_key),
            urlencoding::encode(adif_record),
        );
        let resp = self
            .http
            .post(&self.config.upload_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| UploadError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| UploadError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(UploadError::Http(format!("status {status}: {body}")));
        }
        classify(&body)
    }
}

/// Split a multi-record ADIF body at `<EOR>` boundaries (case-insensitive),
/// trimming surrounding whitespace and dropping the header (everything
/// before `<EOH>`). Returns one ADIF chunk per record, each ending in
/// `<EOR>` so QRZ's parser is happy.
fn split_records(adif: &str) -> Vec<String> {
    let body = match find_after_eoh(adif) {
        Some(idx) => &adif[idx..],
        None => adif,
    };
    body.split_terminator_inclusive_eor()
}

fn find_after_eoh(adif: &str) -> Option<usize> {
    let lower = adif.to_ascii_lowercase();
    lower.find("<eoh>").map(|i| i + "<eoh>".len())
}

trait SplitInclusiveEor {
    fn split_terminator_inclusive_eor(&self) -> Vec<String>;
}

impl SplitInclusiveEor for str {
    fn split_terminator_inclusive_eor(&self) -> Vec<String> {
        let mut out = Vec::new();
        let lower = self.to_ascii_lowercase();
        let mut start = 0usize;
        while let Some(rel) = lower[start..].find("<eor>") {
            let end = start + rel + "<eor>".len();
            let piece = self[start..end].trim();
            if !piece.is_empty() {
                out.push(piece.to_string());
            }
            start = end;
        }
        // Anything trailing without a final <EOR> is malformed for QRZ —
        // skip it. (Matches what dxlab and tqsl do.)
        out
    }
}

fn classify(body: &str) -> Result<(), UploadError> {
    let trimmed = body.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("result=ok") {
        return Ok(());
    }
    if lower.contains("result=replace") {
        // QRZ uses RESULT=REPLACE when a duplicate is overwritten —
        // treat as success.
        return Ok(());
    }
    Err(UploadError::Rejected(body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Multi-shot mock server that returns the next response from a
    /// rotating list. Lets us verify per-record handling.
    async fn mock_multi(responses: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let count2 = count.clone();
        tokio::spawn(async move {
            let mut idx = 0usize;
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 8192];
                let _ = stream.read(&mut buf).await;
                let body = responses.get(idx).copied().unwrap_or("RESULT=FAIL");
                idx += 1;
                count2.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
                    len = body.len(),
                    body = body,
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        (format!("http://{addr}/api"), count)
    }

    const TWO_RECORD_ADIF: &str = "<ADIF_VER:5>3.1.4<EOH>\n\
        <CALL:4>W1AW<QSO_DATE:8>20260508<TIME_ON:6>183045<BAND:3>20M<MODE:3>FT8<EOR>\n\
        <CALL:6>JA1NUT<QSO_DATE:8>20260508<TIME_ON:4>1900<BAND:3>40M<MODE:2>CW<EOR>\n";

    #[test]
    fn split_records_drops_header_keeps_eor() {
        let recs = split_records(TWO_RECORD_ADIF);
        assert_eq!(recs.len(), 2);
        for r in &recs {
            assert!(r.to_ascii_lowercase().ends_with("<eor>"));
            assert!(!r.to_ascii_lowercase().contains("<adif_ver"));
        }
    }

    #[tokio::test]
    async fn uploads_each_record_separately() {
        let (url, count) = mock_multi(vec![
            "RESULT=OK&LOGID=1",
            "RESULT=OK&LOGID=2",
        ])
        .await;
        let client = QrzUploadClient::new(QrzConfig::new("KEY123").with_upload_url(url));
        let outcome = client.upload_adif(TWO_RECORD_ADIF).await.unwrap();
        assert_eq!(outcome.accepted, 2);
        assert_eq!(outcome.rejected, 0);
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn replace_counts_as_accepted() {
        let (url, _) = mock_multi(vec![
            "RESULT=REPLACE&LOGID=1",
            "RESULT=OK&LOGID=2",
        ])
        .await;
        let client = QrzUploadClient::new(QrzConfig::new("KEY123").with_upload_url(url));
        let outcome = client.upload_adif(TWO_RECORD_ADIF).await.unwrap();
        assert_eq!(outcome.accepted, 2);
        assert_eq!(outcome.rejected, 0);
    }

    #[tokio::test]
    async fn fail_per_record_counted_separately() {
        let (url, _) = mock_multi(vec![
            "RESULT=OK&LOGID=1",
            "RESULT=FAIL&REASON=duplicate",
        ])
        .await;
        let client = QrzUploadClient::new(QrzConfig::new("KEY123").with_upload_url(url));
        let outcome = client.upload_adif(TWO_RECORD_ADIF).await.unwrap();
        assert_eq!(outcome.accepted, 1);
        assert_eq!(outcome.rejected, 1);
        assert_eq!(outcome.errors.len(), 1);
    }
}
