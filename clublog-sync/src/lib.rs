//! Club Log upload — POST ADIF text to `realtime.php`.
//!
//! Club Log doesn't expose a public confirmation-report endpoint the way
//! LotW or eQSL do; the typical use case is "back up my log to Club Log
//! and let it generate DXCC/Marathon stats on its side." So this crate
//! is upload-only by design. If/when Club Log publishes a structured
//! report API, add a fetch module here.

use thiserror::Error;

const DEFAULT_UPLOAD_URL: &str = "https://clublog.org/realtime.php";

#[derive(Debug, Clone)]
pub struct ClubLogConfig {
    /// Club Log account email.
    pub email: String,
    /// Club Log account password.
    pub password: String,
    /// The callsign whose log this upload belongs to. Required because a
    /// Club Log account may host multiple callsigns.
    pub callsign: String,
    pub upload_url: String,
}

impl ClubLogConfig {
    pub fn new(
        email: impl Into<String>,
        password: impl Into<String>,
        callsign: impl Into<String>,
    ) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
            callsign: callsign.into(),
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

    #[error("Club Log rejected: {0}")]
    Rejected(String),
}

#[derive(Debug, Clone)]
pub struct UploadOutcome {
    pub accepted: bool,
    pub raw_body: String,
}

pub struct ClubLogUploadClient {
    config: ClubLogConfig,
    http: reqwest::Client,
}

impl ClubLogUploadClient {
    pub fn new(config: ClubLogConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn with_http(config: ClubLogConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    pub fn config(&self) -> &ClubLogConfig {
        &self.config
    }

    pub async fn upload_adif(&self, adif: &str) -> Result<UploadOutcome, UploadError> {
        let form = reqwest::multipart::Form::new()
            .text("email", self.config.email.clone())
            .text("password", self.config.password.clone())
            .text("callsign", self.config.callsign.clone())
            .text("api", "slogger")
            .text("adif", adif.to_string());
        let resp = self
            .http
            .post(&self.config.upload_url)
            .multipart(form)
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

fn classify(body: &str) -> Result<UploadOutcome, UploadError> {
    // Club Log's realtime.php returns plain text. "OK" / "1" indicates
    // success; "FAIL" / "Authentication failed" / etc. indicate rejection.
    let trimmed = body.trim();
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("fail")
        || lower.contains("authentication")
        || lower.contains("denied")
        || lower.contains("error")
    {
        return Err(UploadError::Rejected(body.to_string()));
    }
    let accepted = trimmed == "OK"
        || trimmed == "1"
        || lower.contains("upload received")
        || lower.contains("queued");
    Ok(UploadOutcome {
        accepted,
        raw_body: body.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn mock_server(status: u16, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
                    status = status,
                    len = body.len(),
                    body = body,
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        format!("http://{addr}/upload")
    }

    #[tokio::test]
    async fn ok_response_marks_accepted() {
        let url = mock_server(200, "OK").await;
        let cfg = ClubLogConfig::new("user@example.com", "pw", "W1ABC").with_upload_url(url);
        let client = ClubLogUploadClient::new(cfg);
        let outcome = client.upload_adif("<EOH>\n<CALL:4>W1AW<EOR>\n").await.unwrap();
        assert!(outcome.accepted);
    }

    #[tokio::test]
    async fn fail_response_is_rejected() {
        let url = mock_server(200, "FAIL: authentication failed").await;
        let cfg = ClubLogConfig::new("user@example.com", "wrong", "W1ABC").with_upload_url(url);
        let client = ClubLogUploadClient::new(cfg);
        let err = client.upload_adif("x").await.unwrap_err();
        assert!(matches!(err, UploadError::Rejected(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn ambiguous_response_is_soft_success() {
        let url = mock_server(200, "Thanks").await;
        let cfg = ClubLogConfig::new("u", "p", "W1ABC").with_upload_url(url);
        let client = ClubLogUploadClient::new(cfg);
        let outcome = client.upload_adif("x").await.unwrap();
        assert!(!outcome.accepted);
    }
}
