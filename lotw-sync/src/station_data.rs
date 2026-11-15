//! TQSL's own list of station locations.
//!
//! TQSL signs every QSO against one of its "Station Locations" (callsign,
//! grid, DXCC entity, zones, state/county). It has no command-line way to
//! list them, so slogger reads the XML file TQSL keeps them in,
//! `station_data`, to offer the names in the station editor and to refuse
//! to sign against a name TQSL doesn't have.
//!
//! The file is flat — one `<StationData name="…">` per location, each a
//! list of `<TAG>value</TAG>` children — so it is parsed by hand rather
//! than with an XML crate. Unknown child elements are ignored.

use std::path::{Path, PathBuf};

/// One location from TQSL's `station_data`. Everything but the name is
/// optional: TQSL only stores the fields the operator filled in, and which
/// subdivision tags exist depends on the DXCC entity.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TqslStationLocation {
    pub name: String,
    pub call: Option<String>,
    pub grid: Option<String>,
    pub dxcc: Option<u16>,
    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    /// `US_STATE`, or `CA_PROVINCE` for a Canadian location.
    pub state: Option<String>,
    /// `US_COUNTY`.
    pub county: Option<String>,
    pub iota: Option<String>,
}

/// What slogger knows about TQSL's location list: either the list, or why
/// it couldn't be read. "Unavailable" is a normal state (TQSL not
/// installed, or installed somewhere unexpected), not an error — callers
/// fall back to trusting names the operator typed.
#[derive(Debug, Clone)]
pub enum TqslStationData {
    Read {
        path: PathBuf,
        locations: Vec<TqslStationLocation>,
    },
    Unavailable {
        /// The file that was tried; `None` when no default could be
        /// worked out (no home directory).
        path: Option<PathBuf>,
        reason: String,
    },
}

impl TqslStationData {
    /// Reads `path_override`, or TQSL's default file for this platform.
    /// Never fails: an unreadable or unparseable file becomes
    /// [`TqslStationData::Unavailable`].
    pub fn load(path_override: Option<&Path>) -> Self {
        let Some(path) = path_override
            .map(Path::to_path_buf)
            .or_else(default_station_data_path)
        else {
            return Self::Unavailable {
                path: None,
                reason: "no home directory to look in".into(),
            };
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match parse_station_data(&text) {
                Ok(locations) => Self::Read { path, locations },
                Err(reason) => Self::Unavailable {
                    path: Some(path),
                    reason,
                },
            },
            Err(e) => Self::Unavailable {
                path: Some(path),
                reason: e.to_string(),
            },
        }
    }

    /// TQSL's locations, or `None` when the list couldn't be read.
    pub fn locations(&self) -> Option<&[TqslStationLocation]> {
        match self {
            Self::Read { locations, .. } => Some(locations),
            Self::Unavailable { .. } => None,
        }
    }

    /// The location TQSL knows by exactly this name. Always `None` when
    /// the list is unavailable.
    pub fn find(&self, name: &str) -> Option<&TqslStationLocation> {
        self.locations()?.iter().find(|l| l.name == name)
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Read { path, .. } => Some(path),
            Self::Unavailable { path, .. } => path.as_deref(),
        }
    }
}

/// Where TQSL keeps `station_data`: `%APPDATA%\TrustedQSL` on Windows,
/// `~/.tqsl` elsewhere. The non-Windows location is TQSL's documented
/// default but has not been checked against every install, which is why
/// `[lotw] tqsl_station_data` can override it.
pub fn default_station_data_path() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(dirs::config_dir)
            .map(|d| d.join("TrustedQSL").join("station_data"))
    } else {
        dirs::home_dir().map(|h| h.join(".tqsl").join("station_data"))
    }
}

/// Parses the contents of a `station_data` file. Errors only when the
/// text isn't recognisably that file (no `StationDataFile` root, or a
/// location left unclosed); oddities inside a location are tolerated.
pub fn parse_station_data(xml: &str) -> Result<Vec<TqslStationLocation>, String> {
    let mut locations = Vec::new();
    let mut saw_root = false;
    let mut current: Option<TqslStationLocation> = None;
    // The child element being read inside a location, and its text.
    let mut field: Option<(String, String)> = None;

    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        if let Some((_, value)) = field.as_mut() {
            value.push_str(&rest[..lt]);
        }
        rest = &rest[lt..];

        // Comments, declarations and processing instructions carry no
        // data; skip them whole.
        if let Some(skip) = skip_markup(rest) {
            rest = &rest[skip..];
            continue;
        }
        let gt = rest
            .find('>')
            .ok_or_else(|| "unterminated tag".to_string())?;
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];

        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            if name == "StationData" {
                let loc = current
                    .take()
                    .ok_or_else(|| "</StationData> without a matching start".to_string())?;
                if !loc.name.is_empty() {
                    locations.push(loc);
                }
                field = None;
            } else if let (Some(loc), Some((open, value))) = (current.as_mut(), field.take()) {
                if open == name {
                    set_field(loc, name, &unescape(value.trim()));
                }
            }
            continue;
        }

        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (name, attrs) = match tag.find(char::is_whitespace) {
            Some(i) => (&tag[..i], &tag[i..]),
            None => (tag, ""),
        };
        match name {
            "StationDataFile" => saw_root = true,
            "StationData" => {
                if current.is_some() {
                    return Err("nested <StationData>".into());
                }
                let loc = TqslStationLocation {
                    name: attribute(attrs, "name")
                        .map(|n| unescape(&n))
                        .unwrap_or_default(),
                    ..Default::default()
                };
                if self_closing {
                    if !loc.name.is_empty() {
                        locations.push(loc);
                    }
                } else {
                    current = Some(loc);
                }
            }
            _ if current.is_some() && !self_closing => {
                field = Some((name.to_string(), String::new()));
            }
            _ => {}
        }
    }

    if !saw_root {
        return Err("not a TQSL station_data file (no <StationDataFile>)".into());
    }
    if current.is_some() {
        return Err("file ends inside a <StationData>".into());
    }
    Ok(locations)
}

/// Length of a `<!-- -->`, `<!…>` or `<?…?>` at the start of `s`, if
/// that's what starts it.
fn skip_markup(s: &str) -> Option<usize> {
    if s.starts_with("<!--") {
        return Some(s.find("-->").map(|i| i + 3).unwrap_or(s.len()));
    }
    if s.starts_with("<!") || s.starts_with("<?") {
        return Some(s.find('>').map(|i| i + 1).unwrap_or(s.len()));
    }
    None
}

fn set_field(loc: &mut TqslStationLocation, tag: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    let text = Some(value.to_string());
    match tag {
        "CALL" => loc.call = text,
        "GRIDSQUARE" => loc.grid = text,
        "DXCC" => loc.dxcc = value.parse().ok(),
        "CQZ" => loc.cq_zone = value.parse().ok(),
        "ITUZ" => loc.itu_zone = value.parse().ok(),
        "US_STATE" => loc.state = text,
        "CA_PROVINCE" if loc.state.is_none() => loc.state = text,
        "US_COUNTY" => loc.county = text,
        "IOTA" => loc.iota = text,
        _ => {}
    }
}

/// The raw (still escaped) value of attribute `key` in a tag's attribute
/// text, quoted with either `"` or `'`.
fn attribute(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let k = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let body = &after[1..];
        let end = body.find(quote)?;
        if k == key {
            return Some(body[..end].to_string());
        }
        rest = &body[end + 1..];
    }
}

/// Resolves the five predefined XML entities and numeric character
/// references. Anything else that looks like an entity is left as is.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else { break };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<StationDataFile>\r\n  <StationData name=\"Home\">\r\n    <CALL>W1ABC</CALL>\r\n    <CQZ>5</CQZ>\r\n    <DXCC>291</DXCC>\r\n    <GRIDSQUARE>FN31pr</GRIDSQUARE>\r\n    <ITUZ>8</ITUZ>\r\n    <US_COUNTY>Hartford</US_COUNTY>\r\n    <US_STATE>CT</US_STATE>\r\n  </StationData>\r\n  <StationData name=\"Field Day &amp; Friends\">\r\n    <CALL>W1ABC/P</CALL>\r\n    <GRIDSQUARE>FN32</GRIDSQUARE>\r\n    <IOTA>NA-046</IOTA>\r\n    <SOME_NEW_TAG>whatever</SOME_NEW_TAG>\r\n  </StationData>\r\n</StationDataFile>\r\n";

    #[test]
    fn parses_locations_with_crlf_and_escapes() {
        let locs = parse_station_data(SAMPLE).unwrap();
        assert_eq!(locs.len(), 2);
        assert_eq!(
            locs[0],
            TqslStationLocation {
                name: "Home".into(),
                call: Some("W1ABC".into()),
                grid: Some("FN31pr".into()),
                dxcc: Some(291),
                cq_zone: Some(5),
                itu_zone: Some(8),
                state: Some("CT".into()),
                county: Some("Hartford".into()),
                iota: None,
            }
        );
        assert_eq!(locs[1].name, "Field Day & Friends");
        assert_eq!(locs[1].call.as_deref(), Some("W1ABC/P"));
        assert_eq!(locs[1].iota.as_deref(), Some("NA-046"));
        assert_eq!(locs[1].dxcc, None);
    }

    #[test]
    fn tolerates_declaration_comments_single_quotes_and_odd_values() {
        let xml = "<?xml version=\"1.0\"?>\n<!-- saved by tqsl -->\n<StationDataFile>\n\
                   <StationData name='VE3 &#65;&#x42;'>\n<CA_PROVINCE>ON</CA_PROVINCE>\n\
                   <CQZ>not a number</CQZ><EMPTY/><CALL></CALL>\n</StationData>\n\
                   <StationData name=\"\"><CALL>X1X</CALL></StationData>\n</StationDataFile>";
        let locs = parse_station_data(xml).unwrap();
        assert_eq!(locs.len(), 1, "a location without a name is dropped");
        assert_eq!(locs[0].name, "VE3 AB");
        assert_eq!(locs[0].state.as_deref(), Some("ON"));
        assert_eq!(locs[0].cq_zone, None);
        assert_eq!(locs[0].call, None);
    }

    #[test]
    fn empty_file_has_no_locations() {
        assert!(parse_station_data("<StationDataFile/>").unwrap().is_empty());
        assert!(
            parse_station_data("<StationDataFile>\n</StationDataFile>\n")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rejects_text_that_is_not_station_data() {
        assert!(parse_station_data("hello").is_err());
        assert!(parse_station_data("<StationDataFile><StationData name=\"A\"><CALL>X").is_err());
    }

    #[test]
    fn unescape_leaves_unknown_entities() {
        assert_eq!(unescape("a &amp; b &bogus; &lt;c&gt;"), "a & b &bogus; <c>");
        assert_eq!(unescape("trailing &"), "trailing &");
    }

    #[test]
    fn load_reports_a_missing_file_as_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("station_data");
        let data = TqslStationData::load(Some(&path));
        assert!(data.locations().is_none());
        assert_eq!(data.path(), Some(path.as_path()));

        std::fs::write(&path, SAMPLE).unwrap();
        let data = TqslStationData::load(Some(&path));
        assert_eq!(data.locations().map(<[_]>::len), Some(2));
        assert!(data.find("Home").is_some());
        assert!(data.find("home").is_none(), "names match exactly");
    }
}
