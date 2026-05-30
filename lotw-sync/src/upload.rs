use std::path::PathBuf;

use thiserror::Error;
use tokio::process::Command;

const DEFAULT_UPLOAD_URL: &str = "https://lotw.arrl.org/lotw/upload";

#[derive(Debug, Clone)]
pub struct LotwUploadConfig {
    pub tqsl_path: PathBuf,
    pub station_location: String,
    pub station_callsign: Option<String>,
    pub upload_url: String,
}

impl LotwUploadConfig {
    pub fn new(station_location: impl Into<String>) -> Self {
        Self {
            tqsl_path: PathBuf::from("tqsl"),
            station_location: station_location.into(),
            station_callsign: None,
            upload_url: DEFAULT_UPLOAD_URL.to_string(),
        }
    }

    pub fn with_tqsl_path(mut self, path: PathBuf) -> Self {
        self.tqsl_path = path;
        self
    }

    pub fn with_station_callsign(mut self, call: impl Into<String>) -> Self {
        self.station_callsign = Some(call.into());
        self
    }

    pub fn with_upload_url(mut self, url: impl Into<String>) -> Self {
        self.upload_url = url.into();
        self
    }
}

#[derive(Debug, Error)]
pub enum UploadError {
    #[error("temp file error: {0}")]
    TempFile(String),

    #[error("write error: {0}")]
    Write(String),

    #[error("tqsl invocation failed: {0}")]
    TqslSpawn(String),

    #[error("tqsl exited with status {status}: {stderr}")]
    TqslExit { status: i32, stderr: String },

    #[error("read signed file error: {0}")]
    ReadSigned(String),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("LotW rejected: {0}")]
    Rejected(String),
}

#[derive(Debug, Clone)]
pub struct UploadOutcome {
    /// Whether the LotW server accepted the upload at all (HTTP 200 + body
    /// looks like a normal response). Per-QSO accept counts are not always
    /// reliable from the response body, so we surface the raw body too.
    pub accepted: bool,
    pub raw_body: String,
}

pub struct LotwUploadClient {
    config: LotwUploadConfig,
    http: reqwest::Client,
}

impl LotwUploadClient {
    pub fn new(config: LotwUploadConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn with_http(config: LotwUploadConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    pub fn config(&self) -> &LotwUploadConfig {
        &self.config
    }

    /// End-to-end: write ADIF to temp file, invoke tqsl to sign it into a
    /// .tq8, POST the .tq8 to the LotW upload URL, return the response.
    pub async fn upload_adif(&self, adif: &str) -> Result<UploadOutcome, UploadError> {
        let dir = tempfile::tempdir()
            .map_err(|e| UploadError::TempFile(e.to_string()))?;
        let in_path = dir.path().join("upload.adi");
        let out_path = dir.path().join("upload.tq8");

        tokio::fs::write(&in_path, adif)
            .await
            .map_err(|e| UploadError::Write(e.to_string()))?;

        sign_with_tqsl(
            &self.config.tqsl_path,
            &self.config.station_location,
            self.config.station_callsign.as_deref(),
            &in_path,
            &out_path,
        )
        .await?;

        let bytes = tokio::fs::read(&out_path)
            .await
            .map_err(|e| UploadError::ReadSigned(e.to_string()))?;

        post_tq8(&self.http, &self.config.upload_url, bytes).await
    }
}

pub(crate) async fn sign_with_tqsl(
    tqsl_path: &std::path::Path,
    station_location: &str,
    station_callsign: Option<&str>,
    in_path: &std::path::Path,
    out_path: &std::path::Path,
) -> Result<(), UploadError> {
    let mut cmd = Command::new(tqsl_path);
    cmd.arg("-d") // skip dialog
        .arg("-a")
        .arg("all")
        .arg("-l")
        .arg(station_location)
        .arg("-x") // exit when done
        .arg("-o")
        .arg(out_path);
    if let Some(call) = station_callsign {
        cmd.arg("-c").arg(call);
    }
    cmd.arg(in_path);

    let output = cmd
        .output()
        .await
        .map_err(|e| UploadError::TqslSpawn(format!("{}: {e}", tqsl_path.display())))?;

    if !output.status.success() {
        return Err(UploadError::TqslExit {
            status: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    Ok(())
}

pub(crate) async fn post_tq8(
    http: &reqwest::Client,
    upload_url: &str,
    bytes: Vec<u8>,
) -> Result<UploadOutcome, UploadError> {
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("upload.tq8")
        .mime_str("application/octet-stream")
        .map_err(|e| UploadError::Http(e.to_string()))?;
    let form = reqwest::multipart::Form::new().part("upfile", part);

    let resp = http
        .post(upload_url)
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

    let lower = body.to_ascii_lowercase();
    if lower.contains("rejected") && !lower.contains("accepted") {
        return Err(UploadError::Rejected(body));
    }

    Ok(UploadOutcome {
        accepted: lower.contains("accepted") || lower.contains("result: ok"),
        raw_body: body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    /// Drop a fake `tqsl` shell script that just copies its input file to
    /// the output location specified by `-o`. Lets us exercise the spawn
    /// path without needing a real cert.
    fn fake_tqsl(dir: &std::path::Path, body: &str) -> PathBuf {
        let path = dir.join("fake_tqsl.sh");
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    const COPY_TQSL: &str = "#!/bin/sh
# Parse the args we care about: -o <out> <input>
out=
in=
while [ $# -gt 0 ]; do
  case \"$1\" in
    -o) out=$2; shift 2 ;;
    -d|-x) shift ;;
    -a|-c|-l) shift 2 ;;
    *) in=$1; shift ;;
  esac
done
if [ -z \"$in\" ] || [ -z \"$out\" ]; then
  echo missing args >&2
  exit 2
fi
cp \"$in\" \"$out\"
";

    #[tokio::test]
    async fn sign_with_tqsl_spawns_and_writes_output() {
        let dir = tempdir().unwrap();
        let tqsl = fake_tqsl(dir.path(), COPY_TQSL);
        let in_path = dir.path().join("in.adi");
        let out_path = dir.path().join("out.tq8");
        std::fs::write(&in_path, "<EOH>\n<CALL:4>W1AW<EOR>\n").unwrap();

        sign_with_tqsl(&tqsl, "Home", Some("W1ABC"), &in_path, &out_path)
            .await
            .unwrap();

        let signed = std::fs::read(&out_path).unwrap();
        assert!(!signed.is_empty());
    }

    #[tokio::test]
    async fn sign_with_tqsl_propagates_nonzero_exit() {
        let dir = tempdir().unwrap();
        let tqsl = fake_tqsl(dir.path(), "#!/bin/sh\nexit 7\n");
        let in_path = dir.path().join("in.adi");
        let out_path = dir.path().join("out.tq8");
        std::fs::write(&in_path, "x").unwrap();

        let err = sign_with_tqsl(&tqsl, "Home", None, &in_path, &out_path)
            .await
            .unwrap_err();
        match err {
            UploadError::TqslExit { status, .. } => assert_eq!(status, 7),
            other => panic!("expected TqslExit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn post_tq8_accepts_ok_response() {
        let server = mock_server(
            200,
            "<html>Result: ok\nAccepted 5 QSOs</html>",
        )
        .await;
        let http = reqwest::Client::new();
        let outcome = post_tq8(&http, &server.url, vec![1, 2, 3]).await.unwrap();
        assert!(outcome.accepted);
        assert!(outcome.raw_body.contains("Accepted"));
    }

    #[tokio::test]
    async fn post_tq8_treats_rejected_as_error() {
        let server = mock_server(200, "Result: Rejected — bad cert").await;
        let http = reqwest::Client::new();
        let err = post_tq8(&http, &server.url, vec![1, 2, 3]).await.unwrap_err();
        assert!(matches!(err, UploadError::Rejected(_)), "got {err:?}");
    }

    /// Spin up a one-shot HTTP server on a random port that returns the
    /// given body. Lives only for the test.
    async fn mock_server(status: u16, body: &'static str) -> MockServer {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

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
        MockServer {
            url: format!("http://{addr}/upload"),
        }
    }

    struct MockServer {
        url: String,
    }
}
