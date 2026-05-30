//! WSJT-X UDP bridge — listens for WSJT-X NetworkMessage broadcasts and
//! surfaces logged QSOs to slogger.
//!
//! Protocol reference: WSJT-X source `Common/NetworkMessage.hpp`. All
//! messages share a header:
//!
//!   magic (u32 BE) = 0xadbc_cbda
//!   schema (u32 BE) = 3 (current as of WSJT-X 2.7)
//!   type (u32 BE) = message kind
//!   id (QString) = WSJT-X instance id
//!
//! Then per-type fields. We only care about type 12 (Logged ADIF) — when
//! the operator clicks "Log QSO" in WSJT-X, it broadcasts a single QString
//! containing the ADIF record. We re-export that text and let logbook-domain
//! parse it through the same import pipeline used for file imports.
//!
//! Strings are length-prefixed: u32 BE byte-count, or 0xFFFFFFFF for null.

pub mod listener;
pub mod parser;

pub use listener::{BridgeError, WsjtxBridge, spawn_bridge};
pub use parser::{ParseError, WsjtxMessage};