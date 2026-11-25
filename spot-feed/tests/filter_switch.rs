//! Switching the filter on a running feed, against a stand-in cluster
//! node on a local port.

use std::path::PathBuf;
use std::time::Duration;

use spot_feed::{ClusterSource, SpotEvent, SpotFeedConfig, spawn_spot_feed};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::Receiver;

const LINES: &str = "DX de W3LPL:     14025.0  JA1CW        599                            1234Z\r\n\
                     DX de W3LPL:     14200.0  JA1SSB       59                             1234Z\r\n";

/// A node that greets every connection, takes the login, sends the same
/// two spots (one CW, one SSB) and then stays quiet.
async fn node() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let _ = stream.write_all(b"login:\r\n").await;
                let mut buf = [0u8; 64];
                let _ = stream.read(&mut buf).await;
                let _ = stream.write_all(b"Welcome\r\n").await;
                let _ = stream.write_all(LINES.as_bytes()).await;
                let mut rest = Vec::new();
                let _ = stream.read_to_end(&mut rest).await;
            });
        }
    });
    port
}

/// Spot calls and filter changes, in order, until the feed goes quiet.
async fn drain(rx: &mut Receiver<SpotEvent>) -> Vec<String> {
    let mut seen = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(1500), rx.recv()).await {
        match event {
            SpotEvent::Spot(spot) => seen.push(spot.call),
            SpotEvent::FilterChanged { filter } => seen.push(match filter {
                Some(path) => format!("-> {}", path.file_name().unwrap().to_string_lossy()),
                None => "-> none".to_string(),
            }),
            _ => {}
        }
    }
    seen
}

fn config(port: u16, filter_path: Option<PathBuf>) -> SpotFeedConfig {
    SpotFeedConfig {
        my_callsign: "N0CALL".into(),
        sources: vec![ClusterSource { host: "127.0.0.1".into(), port }],
        filter_path,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_the_filter_changes_what_comes_through() {
    let dir = tempfile::tempdir().unwrap();
    let cw = dir.path().join("cw.json");
    let ssb = dir.path().join("ssb.json");
    let broken = dir.path().join("broken.json");
    std::fs::write(&cw, r#"{"rf": {"mode_allow": ["CW"]}}"#).unwrap();
    std::fs::write(&ssb, r#"{"rf": {"mode_allow": ["SSB"]}}"#).unwrap();
    std::fs::write(&broken, r#"{"rf": {"mode_allow": ["cw"]}}"#).unwrap();

    let port = node().await;
    let (mut rx, control, problem) = spawn_spot_feed(&config(port, Some(cw.clone())), None).await.unwrap();
    assert!(problem.is_none());
    assert_eq!(drain(&mut rx).await, ["JA1CW"]);

    // A file that is no good is refused and nothing changes.
    let err = control.set_filter(Some(broken.clone())).unwrap_err().to_string();
    assert!(err.contains("broken.json"), "{err}");
    assert!(control.set_filter(Some(dir.path().join("missing.json"))).is_err());
    assert_eq!(drain(&mut rx).await, Vec::<String>::new());

    // The change is announced before any spot the new filter passes.
    control.set_filter(Some(ssb.clone())).unwrap();
    assert_eq!(drain(&mut rx).await, ["-> ssb.json", "JA1SSB"]);

    control.set_filter(None).unwrap();
    assert_eq!(drain(&mut rx).await, ["-> none", "JA1CW", "JA1SSB"]);

    // Reload: the same path again, after the file was edited.
    control.set_filter(Some(ssb.clone())).unwrap();
    assert_eq!(drain(&mut rx).await, ["-> ssb.json", "JA1SSB"]);
    std::fs::write(&ssb, r#"{"rf": {"mode_allow": ["CW"]}}"#).unwrap();
    control.set_filter(Some(ssb)).unwrap();
    assert_eq!(drain(&mut rx).await, ["-> ssb.json", "JA1CW"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_filter_at_startup_shows_nothing_until_one_is_chosen() {
    let dir = tempfile::tempdir().unwrap();
    let broken = dir.path().join("broken.json");
    std::fs::write(&broken, "{ not json").unwrap();

    let port = node().await;
    let (mut rx, control, problem) =
        spawn_spot_feed(&config(port, Some(broken.clone())), None).await.unwrap();
    assert!(problem.unwrap().to_string().contains("broken.json"));
    assert_eq!(drain(&mut rx).await, Vec::<String>::new());

    // Fixing the file and choosing it again brings spots back.
    std::fs::write(&broken, r#"{"rf": {"mode_allow": ["CW"]}}"#).unwrap();
    control.set_filter(Some(broken)).unwrap();
    assert_eq!(drain(&mut rx).await, ["-> broken.json", "JA1CW"]);
}
