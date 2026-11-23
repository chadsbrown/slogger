//! What QRZ's logbook API puts on the wire: `NAME=value` pairs joined
//! with `&`, with any ADIF payload HTML-escaped (`&lt;call:4&gt;W1AW`).
//!
//! The format is looser than the developer guide suggests, so this
//! follows what the server sends: fields come in any order, values are
//! not URL-encoded (a book name arrives with its spaces), a reason can
//! end in a newline, and `STATUS` returns its fields at the top level
//! rather than inside `DATA`.

use std::collections::HashMap;

/// The fields of a response, by upper-cased name.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Response {
    fields: HashMap<String, String>,
}

impl Response {
    pub(crate) fn parse(body: &str) -> Self {
        // The ADIF payload is the one value that may hold a bare `&`
        // (every escaped bracket has one), and it comes last, so it is
        // cut off first and the rest split into pairs.
        let (head, adif) = match find_field(body, "ADIF") {
            Some(at) => (&body[..at], Some(&body[at + "ADIF=".len()..])),
            None => (body, None),
        };
        let mut fields = HashMap::new();
        for pair in head.split('&') {
            if let Some((name, value)) = pair.split_once('=') {
                fields.insert(name.trim().to_ascii_uppercase(), value.trim().to_string());
            }
        }
        if let Some(adif) = adif {
            fields.insert("ADIF".into(), unescape(adif));
        }
        Response { fields }
    }

    /// A field's value; `None` when absent or empty.
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .get(name)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    pub(crate) fn result(&self) -> Option<&str> {
        self.get("RESULT")
    }

    pub(crate) fn number(&self, name: &str) -> Option<u64> {
        self.get(name)?.parse().ok()
    }
}

/// Where `NAME=` starts as a field of its own: at the start of the body
/// or straight after an `&`.
fn find_field(body: &str, name: &str) -> Option<usize> {
    let needle = format!("{name}=");
    let mut from = 0;
    while let Some(rel) = body[from..].find(&needle) {
        let at = from + rel;
        if at == 0 || body.as_bytes()[at - 1] == b'&' {
            return Some(at);
        }
        from = at + needle.len();
    }
    None
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The records of an ADIF body, each as its fields by lower-cased name.
/// Anything after the last `<eor>` is an unfinished record and dropped.
pub(crate) fn parse_adif(adif: &str) -> Vec<HashMap<String, String>> {
    let mut records = Vec::new();
    let mut current = HashMap::new();
    let mut rest = adif;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        let mut parts = tag.split(':');
        let name = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        match (name.as_str(), parts.next().and_then(|n| n.trim().parse::<usize>().ok())) {
            ("eor", _) => records.push(std::mem::take(&mut current)),
            ("eoh", _) => current.clear(),
            (_, Some(len)) if !name.is_empty() => {
                let value = take_value(rest, len);
                rest = &rest[value.len()..];
                current.insert(name, value.trim().to_string());
            }
            _ => {}
        }
    }
    records
}

/// The `len`-long value at the start of `rest`. ADIF counts bytes, but
/// not every exporter does once a name has an accent in it, so if `len`
/// bytes doesn't end where the next tag begins, `len` characters is
/// tried; failing both, the value runs to the next tag.
fn take_value(rest: &str, len: usize) -> &str {
    let ends_at_tag = |end: usize| {
        let after = rest[end..].trim_start();
        after.is_empty() || after.starts_with('<')
    };
    if len <= rest.len() && rest.is_char_boundary(len) && ends_at_tag(len) {
        return &rest[..len];
    }
    let by_chars = rest
        .char_indices()
        .nth(len)
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    if ends_at_tag(by_chars) {
        return &rest[..by_chars];
    }
    &rest[..rest.find('<').unwrap_or(rest.len())]
}

/// The first record of an ADIF text, without any header and ending in
/// its `<EOR>`; `None` if there is no complete record.
pub(crate) fn first_record(adif: &str) -> Option<&str> {
    let lower = adif.to_ascii_lowercase();
    let start = lower.find("<eoh>").map(|i| i + "<eoh>".len()).unwrap_or(0);
    let end = start + lower[start..].find("<eor>")? + "<eor>".len();
    let record = adif[start..end].trim();
    (record.len() > "<eor>".len()).then_some(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_come_in_any_order_and_are_not_url_encoded() {
        let r = Response::parse(
            "BOOKID=42&COUNT=100&CALLSIGN=K1ABC&BOOK_NAME=K1ABC Logbook&RESULT=OK&ACTION=STATUS",
        );
        assert_eq!(r.result(), Some("OK"));
        assert_eq!(r.get("BOOK_NAME"), Some("K1ABC Logbook"));
        assert_eq!(r.number("COUNT"), Some(100));
        assert_eq!(r.get("MISSING"), None);
    }

    #[test]
    fn a_reason_loses_its_trailing_newline_and_empty_fields_read_as_absent() {
        let r = Response::parse("STATUS=AUTH&RESULT=AUTH&REASON=invalid api key\n&EXTENDED=");
        assert_eq!(r.result(), Some("AUTH"));
        assert_eq!(r.get("REASON"), Some("invalid api key"));
        assert_eq!(r.get("EXTENDED"), None);
    }

    #[test]
    fn the_adif_payload_is_unescaped_and_kept_whole() {
        let r = Response::parse(
            "COUNT=1&RESULT=OK&ADIF=&lt;call:4&gt;W1AW\n&lt;comment:7&gt;A&amp;B=C D\n&lt;eor&gt;\n",
        );
        assert_eq!(r.number("COUNT"), Some(1));
        let records = parse_adif(r.get("ADIF").unwrap());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["call"], "W1AW");
        assert_eq!(records[0]["comment"], "A&B=C D");
    }

    #[test]
    fn adif_records_split_at_eor_and_drop_an_unfinished_tail() {
        let records = parse_adif(
            "<CALL:4>W1AW<band:3:S>20M<eor>\n<call:6>JA1NUT <BAND:3>40M<EOR><call:4>K1AB",
        );
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["band"], "20M");
        assert_eq!(records[1]["call"], "JA1NUT");
    }

    #[test]
    fn a_length_counted_in_characters_still_reads() {
        // "José" is four characters but five bytes.
        let records = parse_adif("<name:4>José<call:4>EA1A<eor>");
        assert_eq!(records[0]["name"], "José");
        assert_eq!(records[0]["call"], "EA1A");
        let records = parse_adif("<name:5>José<call:4>EA1A<eor>");
        assert_eq!(records[0]["name"], "José");
        assert_eq!(records[0]["call"], "EA1A");
    }

    #[test]
    fn first_record_skips_the_header() {
        let adif = "<ADIF_VER:5>3.1.4<EOH>\n<CALL:4>W1AW<EOR>\n<CALL:4>K1AB<EOR>\n";
        assert_eq!(first_record(adif), Some("<CALL:4>W1AW<EOR>"));
        assert_eq!(first_record("<CALL:4>W1AW"), None);
        assert_eq!(first_record("<eoh><eor>"), None);
    }
}
