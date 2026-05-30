//! HRDLog upload — POST ADIF text to `NewEntry.aspx`.
//!
//! HRDLog is a free third-party log-hosting service used by many DXLab
//! and Ham Radio Deluxe operators. The upload protocol mirrors eQSL's
//! shape: form-encoded POST with credentials + ADIF body. HRDLog
//! distinguishes between the website password and a per-account
//! "upload code" — we use the upload code; user keeps the website
//! password out of the config.
//!
//! Confirmation fetch isn't implemented here — HRDLog's reporting
//! endpoint is less standardized than LotW/eQSL and the upload itself
//! is the high-value piece. Add a fetch module if/when needed.

use thiserror::Error;

const DEFAULT_UPLOAD_URL: &str = "https://www.hrdlog.net/NewEntry.aspx";

#[derive(Debug, Clone)]
pub struct HrdlogConfig {
    /// Account callsign (the "Callsign" field — whose log this is).
    pub callsign: String,
    /// Per-account upload code, distinct from the website password.
    /// Look it up at hrdlog.net under Account → Settings → Upload code.
    pub code: String,
    pub upload_url: String,
}

impl HrdlogConfig {
    pub fn new(callsign: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            callsign: callsign.into(),
            code: code.into(),
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

    #[error("HRDLog rejected: {0}")]
    Rejected(String),
}

#[derive(Debug, Clone)]
pub struct UploadOutcome {
    pub accepted: bool,
    pub raw_body: String,
}

pub struct HrdlogUploadClient {
    config: HrdlogConfig,
    http: reqwest::Client,
}

impl HrdlogUploadClient {
    pub fn new(config: HrdlogConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn with_http(config: HrdlogConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    pub fn config(&self) -> &HrdlogConfig {
        &self.config
    }

    pub async fn upload_adif(&self, adif: &str) -> Result<UploadOutcome, UploadError> {
        let form = reqwest::multipart::Form::new()
            .text("Callsign", self.config.callsign.clone())
            .text("Code", self.config.code.clone())
            .text("ADIFData", adif.to_string());
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
    let trimmed = body.trim();
    let lower = trimmed.to_ascii_lowercase();

    // HRDLog's response wording isn't fully documented from public
    // sources. Treat clear error signals as Rejected; otherwise default
    // to soft-success and surface the raw body so the operator can
    // verify on hrdlog.net.
    if lower.contains("error")
        || lower.contains("invalid")
        || lower.contains("denied")
        || lower.contains("authentication failed")
        || lower.starts_with("fail")
    {
        return Err(UploadError::Rejected(body.to_string()));
    }

    let accepted = trimmed.eq_ignore_ascii_case("OK")
        || lower.contains("upload received")
        || lower.contains("imported")
        || lower.contains("success");
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
        let cfg = HrdlogConfig::new("W1ABC", "secretcode").with_upload_url(url);
        let client = HrdlogUploadClient::new(cfg);
        let outcome = client.upload_adif("<EOH>\n<CALL:4>W1AW<EOR>\n").await.unwrap();
        assert!(outcome.accepted);
    }

    #[tokio::test]
    async fn error_response_is_rejected() {
        let url = mock_server(200, "Error: invalid Code").await;
        let cfg = HrdlogConfig::new("W1ABC", "wrong").with_upload_url(url);
        let client = HrdlogUploadClient::new(cfg);
        let err = client.upload_adif("x").await.unwrap_err();
        assert!(matches!(err, UploadError::Rejected(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn ambiguous_response_is_soft_success() {
        let url = mock_server(200, "Done.").await;
        let cfg = HrdlogConfig::new("W1ABC", "secretcode").with_upload_url(url);
        let client = HrdlogUploadClient::new(cfg);
        let outcome = client.upload_adif("x").await.unwrap();
        assert!(!outcome.accepted, "ambiguous body should not auto-claim acceptance");
    }
}
