use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio::process::Command;

const DEFAULT_UPLOAD_URL: &str = "https://lotw.arrl.org/lotw/upload";

/// How to reach TQSL and LoTW. Which TQSL station location and callsign
/// to sign with are per call to [`LotwUploadClient::upload_adif`]: one
/// logbook can hold QSOs from several locations and callsigns, and each
/// batch must be signed with its own.
#[derive(Debug, Clone)]
pub struct LotwUploadConfig {
    pub tqsl_path: PathBuf,
    pub upload_url: String,
}

impl Default for LotwUploadConfig {
    fn default() -> Self {
        Self {
            tqsl_path: PathBuf::from("tqsl"),
            upload_url: DEFAULT_UPLOAD_URL.to_string(),
        }
    }
}

impl LotwUploadConfig {
    pub fn with_tqsl_path(mut self, path: PathBuf) -> Self {
        self.tqsl_path = path;
        self
    }

    pub fn with_upload_url(mut self, url: impl Into<String>) -> Self {
        self.upload_url = url.into();
        self
    }
}

/// TQSL's verdict on a signing run: the `Final Status: Description (Code)`
/// line it prints last in batch mode, or the exit status when that line
/// is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TqslStatus {
    /// TQSL's status code; `None` when TQSL printed no status and was
    /// killed by a signal.
    pub code: Option<i32>,
    pub description: String,
    /// The last other line TQSL wrote to stderr, which usually names the
    /// actual problem (a missing certificate, an unknown location).
    pub detail: Option<String>,
}

impl std::fmt::Display for TqslStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description)?;
        if let Some(code) = self.code {
            write!(f, " ({code})")?;
        }
        if let Some(detail) = &self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
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

    #[error("TQSL: {0}")]
    Tqsl(TqslStatus),

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

/// What became of one batch handed to [`LotwUploadClient::upload_adif`].
#[derive(Debug, Clone)]
pub enum SignedUpload {
    /// TQSL signed the batch and the .tq8 was posted to LoTW.
    Posted(UploadOutcome),
    /// TQSL signed nothing because, by its own records, every QSO was
    /// already uploaded (or outside the certificate's date range).
    /// Nothing was posted.
    AlreadyUploaded(TqslStatus),
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

    /// End-to-end for one batch: write the ADIF to a temp file, have TQSL
    /// sign it against `station_location` with `station_callsign`'s
    /// certificate, and POST the .tq8 to LoTW.
    pub async fn upload_adif(
        &self,
        adif: &str,
        station_location: &str,
        station_callsign: &str,
    ) -> Result<SignedUpload, UploadError> {
        let dir = tempfile::tempdir()
            .map_err(|e| UploadError::TempFile(e.to_string()))?;
        let in_path = dir.path().join("upload.adi");
        let out_path = dir.path().join("upload.tq8");

        tokio::fs::write(&in_path, adif)
            .await
            .map_err(|e| UploadError::Write(e.to_string()))?;

        let signed = sign_with_tqsl(
            &self.config.tqsl_path,
            station_location,
            station_callsign,
            &in_path,
            &out_path,
        )
        .await?;
        if let SignResult::AlreadyUploaded(status) = signed {
            return Ok(SignedUpload::AlreadyUploaded(status));
        }

        let bytes = tokio::fs::read(&out_path)
            .await
            .map_err(|e| UploadError::ReadSigned(e.to_string()))?;

        post_tq8(&self.http, &self.config.upload_url, bytes)
            .await
            .map(SignedUpload::Posted)
    }
}

/// How a TQSL run ended, once it didn't fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SignResult {
    /// The output .tq8 holds signed QSOs, ready to post.
    Signed,
    AlreadyUploaded(TqslStatus),
}

pub(crate) async fn sign_with_tqsl(
    tqsl_path: &Path,
    station_location: &str,
    station_callsign: &str,
    in_path: &Path,
    out_path: &Path,
) -> Result<SignResult, UploadError> {
    let mut cmd = Command::new(tqsl_path);
    cmd.arg("-d") // no date-range dialog
        // Sign what can be signed; skip QSOs TQSL knows were already
        // uploaded or that fall outside the certificate's dates instead
        // of signing them anyway (`all`) or asking (`ask`).
        .arg("-a")
        .arg("compliant")
        .arg("-l")
        .arg(station_location)
        // Overrides the location's callsign: picks the certificate for
        // the call the QSOs were made under (e.g. a /P). TQSL refuses if
        // it has no such certificate or its DXCC differs.
        .arg("-c")
        .arg(station_callsign)
        .arg("-x") // batch mode: status on stderr, then exit
        .arg("-o")
        .arg(out_path)
        .arg(in_path);

    let output = cmd
        .output()
        .await
        .map_err(|e| UploadError::TqslSpawn(format!("{}: {e}", tqsl_path.display())))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let output_present = std::fs::metadata(out_path).is_ok_and(|m| m.len() > 0);
    classify(&stderr, output.status.code(), output_present)
}

/// Turns what TQSL printed and how it exited into a result. The
/// `Final Status` line wins over the exit status: TQSL's documentation
/// doesn't promise the two agree.
pub(crate) fn classify(
    stderr: &str,
    exit_code: Option<i32>,
    output_present: bool,
) -> Result<SignResult, UploadError> {
    let (code, description) = match final_status(stderr) {
        Some((code, description)) => (Some(code), description),
        None => (exit_code, describe_code(exit_code).to_string()),
    };
    let status = TqslStatus {
        code,
        description,
        detail: last_detail_line(stderr),
    };
    match code {
        // Success, or "some signed, some skipped as already uploaded".
        Some(0) | Some(9) if output_present => Ok(SignResult::Signed),
        Some(0) | Some(9) => Err(UploadError::Tqsl(TqslStatus {
            detail: Some("TQSL reported success but wrote no signed file".into()),
            ..status
        })),
        // "Already uploaded". Should TQSL still have written a file, post
        // it: LoTW ignores duplicates, while marking QSOs uploaded that
        // never reached LoTW would lose them silently.
        Some(8) | Some(14) if output_present => Ok(SignResult::Signed),
        Some(8) | Some(14) => Ok(SignResult::AlreadyUploaded(status)),
        _ => Err(UploadError::Tqsl(status)),
    }
}

/// Parses TQSL's last `… Final Status: Description (Code)` line.
pub(crate) fn final_status(stderr: &str) -> Option<(i32, String)> {
    let line = stderr.lines().rev().find(|l| l.contains("Final Status:"))?;
    let rest = line[line.find("Final Status:")? + "Final Status:".len()..].trim();
    let open = rest.rfind('(')?;
    let code = rest[open + 1..].strip_suffix(')')?.trim().parse().ok()?;
    Some((code, rest[..open].trim().to_string()))
}

/// The last non-empty stderr line that isn't the final status, trimmed
/// to a length that fits a one-line summary.
fn last_detail_line(stderr: &str) -> Option<String> {
    const MAX: usize = 160;
    let line = stderr
        .lines()
        .map(str::trim)
        .rev()
        .find(|l| !l.is_empty() && !l.contains("Final Status:"))?;
    Some(if line.chars().count() > MAX {
        format!("{}…", line.chars().take(MAX).collect::<String>())
    } else {
        line.to_string()
    })
}

/// TQSL's documented meaning of each status code, for when it exits
/// without printing one.
fn describe_code(code: Option<i32>) -> &'static str {
    match code {
        Some(0) => "Success",
        Some(1) => "Cancelled by user",
        Some(2) => "Rejected by LoTW",
        Some(3) => "Unexpected response from LoTW server",
        Some(4) => "TQSL error",
        Some(5) => "TQSLlib error",
        Some(6) => "Unable to open input file",
        Some(7) => "Unable to open output file",
        Some(8) => "No QSOs processed: already uploaded or out of date range",
        Some(9) => "Some QSOs processed, some ignored as already uploaded or out of date range",
        Some(10) => "Command syntax error",
        Some(11) => "LoTW connection error",
        Some(12) => "Unknown error",
        Some(13) => "Upload tracking database locked",
        Some(14) => "Some QSOs already uploaded",
        Some(15) => "Invalid passphrase",
        Some(_) => "Unrecognised TQSL status",
        None => "TQSL was killed by a signal",
    }
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

    /// Script tests run one at a time. Writing an executable while another
    /// test forks lets the child inherit the open file, and exec'ing it
    /// then fails with "Text file busy".
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

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

    /// A fake `tqsl` that records its arguments (one per line) in
    /// `args.txt` beside itself, optionally copies its input to the `-o`
    /// file, prints `stderr` and exits with `exit`.
    fn scripted_tqsl(dir: &std::path::Path, copy: bool, stderr: &str, exit: i32) -> PathBuf {
        let args_file = dir.join("args.txt");
        let body = format!(
            "#!/bin/sh
: > '{args}'
for a in \"$@\"; do printf '%s\\n' \"$a\" >> '{args}'; done
out=
in=
while [ $# -gt 0 ]; do
  case \"$1\" in
    -o) out=$2; shift 2 ;;
    -d|-x|-q) shift ;;
    -a|-c|-l) shift 2 ;;
    *) in=$1; shift ;;
  esac
done
if [ {copy} = 1 ]; then cp \"$in\" \"$out\"; fi
printf '%s' '{stderr}' >&2
exit {exit}
",
            args = args_file.display(),
            copy = if copy { 1 } else { 0 },
        );
        fake_tqsl(dir, &body)
    }

    async fn run_sign(tqsl: &std::path::Path, dir: &std::path::Path) -> Result<SignResult, UploadError> {
        let in_path = dir.join("in.adi");
        let out_path = dir.join("out.tq8");
        std::fs::write(&in_path, "<EOH>\n<CALL:4>W1AW<EOR>\n").unwrap();
        sign_with_tqsl(tqsl, "Field Day", "N9UNX/P", &in_path, &out_path).await
    }

    #[tokio::test]
    async fn sign_passes_location_callsign_and_compliant_action() {
        let _serial = serial();
        let dir = tempdir().unwrap();
        let tqsl = scripted_tqsl(dir.path(), true, "10:01:02 AM Final Status: Success (0)\n", 0);
        assert_eq!(run_sign(&tqsl, dir.path()).await.unwrap(), SignResult::Signed);

        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        let after = |flag: &str| args[args.iter().position(|a| *a == flag).unwrap() + 1];
        assert_eq!(after("-a"), "compliant");
        assert_eq!(after("-l"), "Field Day", "a name with a space stays one argument");
        assert_eq!(after("-c"), "N9UNX/P");
        assert!(args.contains(&"-x") && args.contains(&"-d"));
        assert!(!std::fs::read(dir.path().join("out.tq8")).unwrap().is_empty());
    }

    #[tokio::test]
    async fn partly_already_uploaded_still_counts_as_signed() {
        let _serial = serial();
        let dir = tempdir().unwrap();
        let tqsl = scripted_tqsl(
            dir.path(),
            true,
            "Signing...\n10:01:02 AM Final Status: Some QSOs were processed, some ignored (9)\n",
            9,
        );
        assert_eq!(run_sign(&tqsl, dir.path()).await.unwrap(), SignResult::Signed);
    }

    #[tokio::test]
    async fn nothing_signed_because_already_uploaded() {
        let _serial = serial();
        let dir = tempdir().unwrap();
        let tqsl = scripted_tqsl(
            dir.path(),
            false,
            "10:01:02 AM Final Status: No QSOs were processed since some QSOs were duplicates or out of date range (8)\n",
            8,
        );
        match run_sign(&tqsl, dir.path()).await.unwrap() {
            SignResult::AlreadyUploaded(status) => assert_eq!(status.code, Some(8)),
            other => panic!("expected AlreadyUploaded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn final_status_line_wins_over_exit_status() {
        let _serial = serial();
        let dir = tempdir().unwrap();
        let tqsl = scripted_tqsl(
            dir.path(),
            false,
            "Unable to sign: bad passphrase for certificate\n10:01:02 PM Final Status: Invalid passphrase (15)\n",
            0,
        );
        match run_sign(&tqsl, dir.path()).await.unwrap_err() {
            UploadError::Tqsl(status) => {
                assert_eq!(status.code, Some(15));
                assert_eq!(status.description, "Invalid passphrase");
                assert_eq!(
                    status.detail.as_deref(),
                    Some("Unable to sign: bad passphrase for certificate")
                );
            }
            other => panic!("expected Tqsl, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn falls_back_to_exit_status_without_a_final_status_line() {
        let _serial = serial();
        let dir = tempdir().unwrap();
        let tqsl = scripted_tqsl(dir.path(), false, "", 7);
        match run_sign(&tqsl, dir.path()).await.unwrap_err() {
            UploadError::Tqsl(status) => {
                assert_eq!(status.code, Some(7));
                assert_eq!(status.description, "Unable to open output file");
                assert_eq!(status.detail, None);
            }
            other => panic!("expected Tqsl, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn success_without_an_output_file_is_a_failure() {
        let _serial = serial();
        let dir = tempdir().unwrap();
        let tqsl = scripted_tqsl(dir.path(), false, "Final Status: Success (0)\n", 0);
        assert!(matches!(
            run_sign(&tqsl, dir.path()).await,
            Err(UploadError::Tqsl(TqslStatus { code: Some(0), .. }))
        ));
    }

    #[tokio::test]
    async fn already_uploaded_batch_is_not_posted() {
        let _serial = serial();
        let dir = tempdir().unwrap();
        let tqsl = scripted_tqsl(dir.path(), false, "Final Status: No QSOs processed (8)\n", 8);
        // Nothing listens on this URL: posting would fail the test.
        let client = LotwUploadClient::new(
            LotwUploadConfig::default()
                .with_tqsl_path(tqsl)
                .with_upload_url("http://127.0.0.1:9/upload"),
        );
        let outcome = client.upload_adif("<EOH>\n", "Home", "W1ABC").await.unwrap();
        assert!(matches!(outcome, SignedUpload::AlreadyUploaded(_)));
    }

    #[test]
    fn classify_treats_unknown_codes_as_failures() {
        assert!(classify("Final Status: Upload tracking database locked (13)", Some(13), true).is_err());
        assert!(classify("", None, true).is_err(), "killed by a signal");
        // A file written despite "already uploaded" is posted, not dropped.
        assert_eq!(
            classify("Final Status: Some QSOs already uploaded (14)", Some(14), true).unwrap(),
            SignResult::Signed
        );
    }

    #[test]
    fn final_status_parses_description_and_code() {
        assert_eq!(
            final_status("x\n08:15:00 AM Final Status: Success (0)\r\n"),
            Some((0, "Success".to_string()))
        );
        assert_eq!(
            final_status("Final Status: Rejected (by LoTW) (2)"),
            Some((2, "Rejected (by LoTW)".to_string()))
        );
        assert_eq!(final_status("Final Status: garbled"), None);
        assert_eq!(final_status("no status here"), None);
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
