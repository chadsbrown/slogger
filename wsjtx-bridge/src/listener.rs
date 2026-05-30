use std::net::SocketAddr;

use thiserror::Error;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::parser::{ParseError, WsjtxMessage, parse};

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("bind error on {addr}: {error}")]
    Bind { addr: String, error: String },
}

#[derive(Debug, Clone)]
pub struct WsjtxBridge {
    pub bind_addr: SocketAddr,
}

const CHANNEL_DEPTH: usize = 64;
const RECV_BUF: usize = 64 * 1024;

/// Spawn the bridge task. Returns the bound address (so callers know
/// which ephemeral port the kernel picked when binding to :0) and the
/// receive channel of [`WsjtxMessage`]s.
pub async fn spawn_bridge(
    bind: SocketAddr,
) -> Result<(SocketAddr, mpsc::Receiver<WsjtxMessage>), BridgeError> {
    let socket = UdpSocket::bind(bind).await.map_err(|e| BridgeError::Bind {
        addr: bind.to_string(),
        error: e.to_string(),
    })?;
    let actual = socket.local_addr().map_err(|e| BridgeError::Bind {
        addr: bind.to_string(),
        error: format!("local_addr: {e}"),
    })?;
    let (tx, rx) = mpsc::channel(CHANNEL_DEPTH);

    tokio::spawn(async move {
        let mut buf = vec![0u8; RECV_BUF];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((n, peer)) => match parse(&buf[..n]) {
                    Ok(msg) => {
                        if tx.send(msg).await.is_err() {
                            return; // receiver dropped
                        }
                    }
                    Err(ParseError::UnsupportedSchema(s)) => {
                        // Don't spam the log; one-off mismatches mean
                        // WSJT-X bumped the schema and we should update.
                        tracing::warn!(schema = s, %peer, "wsjtx: unsupported schema");
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, %peer, "wsjtx: parse error");
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "wsjtx: recv error");
                    // Don't tight-loop on persistent socket errors.
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
    });

    Ok((actual, rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{SUPPORTED_SCHEMA, WSJTX_MAGIC};

    fn build_logged_adif(id: &str, adif: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&WSJTX_MAGIC.to_be_bytes());
        out.extend_from_slice(&SUPPORTED_SCHEMA.to_be_bytes());
        out.extend_from_slice(&12u32.to_be_bytes()); // type: Logged ADIF
        out.extend_from_slice(&(id.len() as u32).to_be_bytes());
        out.extend_from_slice(id.as_bytes());
        out.extend_from_slice(&(adif.len() as u32).to_be_bytes());
        out.extend_from_slice(adif.as_bytes());
        out
    }

    #[tokio::test]
    async fn end_to_end_loopback() {
        let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (addr, mut rx) = spawn_bridge(bind).await.unwrap();

        // Pretend to be WSJT-X.
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let dg = build_logged_adif("WSJT-X", "<EOH>\n<CALL:4>W1AW<EOR>\n");
        sender.send_to(&dg, addr).await.unwrap();

        let msg = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout waiting for msg")
            .expect("channel closed");
        match msg {
            WsjtxMessage::LoggedAdif { id, adif } => {
                assert_eq!(id, "WSJT-X");
                assert!(adif.contains("W1AW"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
