use thiserror::Error;

pub const WSJTX_MAGIC: u32 = 0xadbc_cbda;
pub const SUPPORTED_SCHEMA: u32 = 3;

const MSG_TYPE_LOGGED_ADIF: u32 = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsjtxMessage {
    /// QSO logged in WSJT-X. ADIF is the raw record text broadcast on
    /// each "Log QSO" click. The ID is the WSJT-X instance identifier
    /// (e.g. "WSJT-X" or a user-set name).
    LoggedAdif { id: String, adif: String },
    /// Catch-all: a well-formed message with a type slogger doesn't
    /// currently use. Carries the type so callers can log/ignore.
    Other { id: String, msg_type: u32 },
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("bad magic: expected {expected:#x}, got {actual:#x}")]
    BadMagic { expected: u32, actual: u32 },

    #[error("unsupported schema: {0}")]
    UnsupportedSchema(u32),

    #[error("truncated datagram (need {need} more bytes at offset {offset})")]
    Truncated { offset: usize, need: usize },

    #[error("invalid utf-8 in string field at offset {0}")]
    BadUtf8(usize),
}

/// Parse a single UDP datagram. Returns `Ok(Other { .. })` for valid but
/// uninteresting message types so callers can log or count without
/// rejecting them.
pub fn parse(buf: &[u8]) -> Result<WsjtxMessage, ParseError> {
    let mut r = Reader::new(buf);
    let magic = r.read_u32()?;
    if magic != WSJTX_MAGIC {
        return Err(ParseError::BadMagic {
            expected: WSJTX_MAGIC,
            actual: magic,
        });
    }
    let schema = r.read_u32()?;
    if schema != SUPPORTED_SCHEMA {
        // Be tolerant: WSJT-X has bumped schemas before. Log via the
        // error chain rather than crashing.
        return Err(ParseError::UnsupportedSchema(schema));
    }
    let msg_type = r.read_u32()?;
    let id = r.read_string()?.unwrap_or_default();

    match msg_type {
        MSG_TYPE_LOGGED_ADIF => {
            let adif = r.read_string()?.unwrap_or_default();
            Ok(WsjtxMessage::LoggedAdif { id, adif })
        }
        _ => Ok(WsjtxMessage::Other { id, msg_type }),
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn read_u32(&mut self) -> Result<u32, ParseError> {
        if self.buf.len() - self.pos < 4 {
            return Err(ParseError::Truncated {
                offset: self.pos,
                need: 4,
            });
        }
        let v = u32::from_be_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    /// QString: u32 byte-count, or 0xFFFFFFFF for null. Returns
    /// `Ok(None)` for null, `Ok(Some(s))` otherwise.
    fn read_string(&mut self) -> Result<Option<String>, ParseError> {
        let len = self.read_u32()?;
        if len == 0xFFFF_FFFF {
            return Ok(None);
        }
        let len = len as usize;
        if self.buf.len() - self.pos < len {
            return Err(ParseError::Truncated {
                offset: self.pos,
                need: len,
            });
        }
        let bytes = &self.buf[self.pos..self.pos + len];
        let s = std::str::from_utf8(bytes)
            .map_err(|_| ParseError::BadUtf8(self.pos))?
            .to_string();
        self.pos += len;
        Ok(Some(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a WSJT-X datagram for the given type and string fields.
    fn build(msg_type: u32, id: Option<&str>, extras: &[Option<&str>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&WSJTX_MAGIC.to_be_bytes());
        out.extend_from_slice(&SUPPORTED_SCHEMA.to_be_bytes());
        out.extend_from_slice(&msg_type.to_be_bytes());
        push_string(&mut out, id);
        for s in extras {
            push_string(&mut out, *s);
        }
        out
    }

    fn push_string(out: &mut Vec<u8>, s: Option<&str>) {
        match s {
            Some(s) => {
                out.extend_from_slice(&(s.len() as u32).to_be_bytes());
                out.extend_from_slice(s.as_bytes());
            }
            None => out.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()),
        }
    }

    #[test]
    fn parses_logged_adif() {
        let adif = "<EOH>\n<CALL:4>W1AW<QSO_DATE:8>20260508<EOR>\n";
        let dg = build(12, Some("WSJT-X"), &[Some(adif)]);
        let msg = parse(&dg).unwrap();
        match msg {
            WsjtxMessage::LoggedAdif { id, adif: a } => {
                assert_eq!(id, "WSJT-X");
                assert_eq!(a, adif);
            }
            other => panic!("expected LoggedAdif, got {other:?}"),
        }
    }

    #[test]
    fn other_types_become_other_variant() {
        let dg = build(0, Some("WSJT-X"), &[]); // 0 = heartbeat
        let msg = parse(&dg).unwrap();
        assert!(matches!(msg, WsjtxMessage::Other { msg_type: 0, .. }));
    }

    #[test]
    fn null_id_is_empty_string() {
        let dg = build(12, None, &[Some("")]);
        let msg = parse(&dg).unwrap();
        match msg {
            WsjtxMessage::LoggedAdif { id, .. } => assert_eq!(id, ""),
            other => panic!("expected LoggedAdif, got {other:?}"),
        }
    }

    #[test]
    fn bad_magic_rejected() {
        let mut dg = vec![0u8; 16];
        dg[0..4].copy_from_slice(&0xdead_beefu32.to_be_bytes());
        let err = parse(&dg).unwrap_err();
        assert!(matches!(err, ParseError::BadMagic { .. }));
    }

    #[test]
    fn truncated_rejected() {
        let dg = vec![0xad, 0xbc, 0xcb]; // not even the magic word
        let err = parse(&dg).unwrap_err();
        assert!(matches!(err, ParseError::Truncated { .. }));
    }

    #[test]
    fn unsupported_schema_rejected() {
        let mut dg = Vec::new();
        dg.extend_from_slice(&WSJTX_MAGIC.to_be_bytes());
        dg.extend_from_slice(&99u32.to_be_bytes()); // wrong schema
        dg.extend_from_slice(&12u32.to_be_bytes());
        push_string(&mut dg, Some("WSJT-X"));
        let err = parse(&dg).unwrap_err();
        assert!(matches!(err, ParseError::UnsupportedSchema(99)));
    }
}
