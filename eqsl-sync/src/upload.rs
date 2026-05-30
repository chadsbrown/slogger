//! eQSL.cc upload — POST ADIF text to ImportADIF.cfm with credentials.
//!
//! Unlike LotW there is no signing step; eQSL trusts the username/password
//! pair. Response is HTML with a status string. This module is deliberately
//! a sibling of `lotw_sync::upload` rather than a sub-module: eQSL has no
//! verification round-trip to mirror, and conflating the two would force
//! either crate into the other's quirks.

use thiserror::Error;

const DEFAULT_UPLOAD_URL: &str = "https://www.eqsl.cc/qslcard/ImportADIF.cfm";

#[derive(Debug, Clone)]
pub struct EqslUploadConfig {
    pub username: String,
    pub password: String,
    pub upload_url: String,
}

impl EqslUploadConfig {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
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

    #[error("eQSL rejected: {0}")]
    Rejected(String),
}

#[derive(Debug, Clone)]
pub struct UploadOutcome {
    pub accepted: bool,
    pub raw_body: String,
}

pub struct EqslUploadClient {
    config: EqslUploadConfig,
    http: reqwest::Client,
}

impl EqslUploadClient {
    pub fn new(config: EqslUploadConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn with_http(config: EqslUploadConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    pub fn config(&self) -> &EqslUploadConfig {
        &self.config
    }

    pub async fn upload_adif(&self, adif: &str) -> Result<UploadOutcome, UploadError> {
        let form = reqwest::multipart::Form::new()
            .text("EQSL_USER", self.config.username.clone())
            .text("EQSL_PSWD", self.config.password.clone())
            .text("ADIFData", adif.to_string());
        post_form(&self.http, &self.config.upload_url, form).await
    }
}

pub(crate) async fn post_form(
    http: &reqwest::Client,
    url: &str,
    form: reqwest::multipart::Form,
) -> Result<UploadOutcome, UploadError> {
    let resp = http
        .post(url)
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

fn classify(body: &str) -> Result<UploadOutcome, UploadError> {
    let lower = body.to_ascii_lowercase();
    // eQSL responses include an explicit "Result: Error" or
    // "Result: ..." line. Treat anything obvious as a hard reject; treat
    // ambiguous output as a soft success and let the operator review.
    if lower.contains("result: error") || lower.contains("authentication failed") {
        return Err(UploadError::Rejected(body.to_string()));
    }
    let accepted = lower.contains("result: ok")
        || lower.contains("successfully")
        || lower.contains("qso records added");
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
        let url = mock_server(200, "<html>Result: OK\n3 out of 3 QSO records added.</html>").await;
        let cfg = EqslUploadConfig::new("W1ABC", "secret").with_upload_url(url);
        let client = EqslUploadClient::new(cfg);
        let outcome = client.upload_adif("<EOH>\n<CALL:4>W1AW<EOR>\n").await.unwrap();
        assert!(outcome.accepted);
    }

    #[tokio::test]
    async fn error_response_is_rejected() {
        let url = mock_server(200, "Result: Error\nAuthentication failed.").await;
        let cfg = EqslUploadConfig::new("W1ABC", "wrong").with_upload_url(url);
        let client = EqslUploadClient::new(cfg);
        let err = client.upload_adif("x").await.unwrap_err();
        assert!(matches!(err, UploadError::Rejected(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn ambiguous_response_is_soft_success() {
        // eQSL occasionally returns boilerplate without obvious markers;
        // we accept HTTP 200 and let the operator verify on the website.
        let url = mock_server(200, "OK").await;
        let cfg = EqslUploadConfig::new("W1ABC", "secret").with_upload_url(url);
        let client = EqslUploadClient::new(cfg);
        let outcome = client.upload_adif("x").await.unwrap();
        assert!(!outcome.accepted, "ambiguous body should not auto-claim acceptance");
    }
}
