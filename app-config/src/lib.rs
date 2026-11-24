use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file IO error at {path}: {error}")]
    Io { path: PathBuf, error: String },

    #[error("config parse error at {path}: {error}")]
    Parse { path: PathBuf, error: String },

    #[error("no platform config directory available")]
    NoConfigDir,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub station: StationConfig,
    pub lotw: LotwConfig,
    pub eqsl: EqslConfig,
    pub clublog: ClubLogConfig,
    pub qrz: QrzConfig,
    pub hrdlog: HrdlogConfig,
    pub dxcluster: DxClusterConfig,
    pub wsjtx: WsjtxConfig,
    /// Multi-rig support: zero or more `[[rig]]` entries. Single-rig
    /// operators write exactly one. `[rig]` (singular) is also accepted
    /// as a one-rig shortcut and gets folded into this Vec by `Config::rigs()`.
    #[serde(rename = "rig", default)]
    pub rigs: Vec<RigConfig>,
    pub keyer: KeyerConfig,
    pub so2r: So2rConfig,
    pub condx: CondXConfig,
    pub pota: PotaConfig,
}

impl Config {
    /// Returns the configured rigs, including a singleton from a legacy
    /// `[rig]` (singular) section if the user wrote one. Use this when
    /// iterating to spin up rig handles at boot.
    pub fn rigs(&self) -> &[RigConfig] {
        &self.rigs
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyerConfig {
    /// Whether to attempt keyer connection at boot. Default false because
    /// most users don't have a WinKeyer wired up.
    pub enabled: bool,
    /// Serial device path (e.g. `/dev/ttyUSB1`).
    pub serial_port: Option<String>,
    /// Initial CW speed in WPM. Default 25 if unset.
    pub initial_wpm: Option<u8>,
    /// Whether the WinKeyer makes its own sidetone. Default true if
    /// unset. Turn it off when the rig's sidetone is enough — otherwise
    /// the keyer's tone is heard as well (through its speaker, or mixed
    /// into the headphones by an SO2R box).
    pub sidetone: Option<bool>,
    /// Message buttons for the CW keyer pane, written as `[[keyer.macro]]`
    /// tables. Empty means "use the built-in set" — see
    /// [`KeyerConfig::macros`].
    #[serde(rename = "macro", skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<KeyerMacro>,
}

impl KeyerConfig {
    pub fn is_configured(&self) -> bool {
        self.enabled && self.serial_port.is_some()
    }

    /// The message buttons to show: the operator's `[[keyer.macro]]`
    /// entries, or the built-in defaults when none are configured. The
    /// first twelve are bound to F1..F12 in order.
    pub fn macros(&self) -> Vec<KeyerMacro> {
        if self.macros.is_empty() {
            KeyerMacro::defaults()
        } else {
            self.macros.clone()
        }
    }
}

/// One pre-populated CW message: a button label and the text it sends.
///
/// `text` may contain placeholders, filled in when the button is pressed:
///
/// - `{MYCALL}` — the operator's callsign (active station location, else
///   `[station].default_callsign`)
/// - `{CALL}` — the callsign currently in the entry form
/// - `{RST}` — the sent report from the entry form, with 9 keyed as N
///   (`599` → `5NN`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyerMacro {
    pub label: String,
    pub text: String,
}

/// Values substituted into a [`KeyerMacro`]'s placeholders.
#[derive(Debug, Clone, Copy, Default)]
pub struct MacroContext<'a> {
    pub my_call: Option<&'a str>,
    pub call: Option<&'a str>,
    pub rst_sent: Option<&'a str>,
}

impl KeyerMacro {
    pub fn new(label: &str, text: &str) -> Self {
        Self {
            label: label.into(),
            text: text.into(),
        }
    }

    /// The built-in message set used when no `[[keyer.macro]]` is configured.
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::new("CQ", "CQ CQ DE {MYCALL} {MYCALL} K"),
            Self::new("His call", "{CALL}"),
            Self::new("Report", "{CALL} UR {RST} {RST}"),
            Self::new("TU", "TU 73"),
            Self::new("My call", "{MYCALL}"),
            Self::new("AGN", "AGN?"),
        ]
    }

    /// Fills in the placeholders. Returns `Err` naming the first
    /// placeholder whose value is missing, so the UI can explain why a
    /// message can't be sent yet rather than keying a literal `{CALL}`.
    pub fn expand(&self, ctx: MacroContext<'_>) -> Result<String, &'static str> {
        let rst = ctx
            .rst_sent
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.replace('9', "N"));
        let values: [(&'static str, Option<String>); 3] = [
            ("{MYCALL}", non_empty(ctx.my_call)),
            ("{CALL}", non_empty(ctx.call)),
            ("{RST}", rst),
        ];
        let mut out = self.text.clone();
        for (placeholder, value) in values {
            if out.contains(placeholder) {
                out = out.replace(placeholder, &value.ok_or(placeholder)?);
            }
        }
        Ok(out.to_ascii_uppercase())
    }
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct So2rConfig {
    /// Whether to attempt SO2R switch connection at boot. Off by default
    /// since most users aren't running an OTRSP-compatible switch.
    pub enabled: bool,
    /// Serial device path (e.g. `/dev/ttyUSB2`).
    pub serial_port: Option<String>,
    /// Initial TX radio (1 or 2). Default 1.
    pub initial_tx: Option<u8>,
    /// Initial RX audio mode: `"mono"` / `"stereo"` / `"reverse_stereo"`.
    /// Default mono.
    pub initial_rx_mode: Option<String>,
}

impl So2rConfig {
    pub fn is_configured(&self) -> bool {
        self.enabled && self.serial_port.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CondXConfig {
    /// Whether to poll N0NBH's solar/propagation feed at hamqsl.com for
    /// the CondX panel. Off by default so an offline station never makes
    /// a network request it didn't ask for.
    pub enabled: bool,
    /// Seconds between polls. Default 3600 (hamqsl updates about
    /// hourly); values below 60 are raised to 60.
    pub refresh_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PotaConfig {
    /// Whether Parks on the Air activator spots are shown when slogger
    /// first starts. Off by default so slogger never contacts pota.app
    /// unasked. The POTA button in the Spots pane switches them on and
    /// off afterwards, and that choice is remembered in place of this.
    pub enabled: bool,
    /// Only show POTA spots on these bands, e.g. `["20m", "40m"]`.
    /// Empty (the default) shows every band.
    pub bands: Vec<String>,
    /// Only show POTA spots in these modes, as POTA names them, e.g.
    /// `["CW", "SSB"]`. Empty (the default) shows every mode.
    pub modes: Vec<String>,
    /// Seconds between polls while switched on. Default 60; values
    /// below 60 are raised to 60.
    pub refresh_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RigConfig {
    /// Whether to attempt rig connection at boot. Default false because
    /// most users start without a rig wired up; failed connection is
    /// noisy and uninteresting in that case.
    pub enabled: bool,
    /// Vendor: `icom` / `yaesu` / `kenwood` / `elecraft` / `flex`.
    pub vendor: Option<String>,
    /// Model name. Case- and hyphen-insensitive against riglib's tables.
    /// Examples: `IC-7300`, `FT-DX10`, `TS-890S`, `K4`, `6400`.
    pub model: Option<String>,
    /// Serial device path for icom/yaesu/kenwood/elecraft.
    pub serial_port: Option<String>,
    pub baud_rate: Option<u32>,
    /// Hostname or IP for FlexRadio (vendor = "flex").
    pub host: Option<String>,
    /// Optional friendly label for multi-rig setups (e.g. "Main", "Aux",
    /// "Run", "Mult"). Defaults to the model name when unset.
    pub label: Option<String>,
    /// How slogger learns that the operator turned the VFO knob.
    /// `false` (default): poll the rig at 4 Hz. `true`: listen for the
    /// rig's own broadcasts and do not poll — auto-information on
    /// Yaesu/Kenwood/Elecraft (enabled for you at connect), CI-V
    /// Transceive on Icom (you must also turn it on in the rig's menu).
    /// Polling can make a rig miss front-panel button presses; `true`
    /// avoids that.
    pub transceive: bool,
}

impl RigConfig {
    pub fn is_configured(&self) -> bool {
        if !self.enabled || self.vendor.is_none() || self.model.is_none() {
            return false;
        }
        match self.vendor.as_deref() {
            Some("flex") => self.host.is_some(),
            Some(_) => self.serial_port.is_some(),
            None => false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QrzConfig {
    /// QRZ logbook API key. Look it up on the QRZ logbook page after
    /// signing in. Different from QRZ XML subscription credentials.
    pub api_key: Option<String>,
}

impl QrzConfig {
    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HrdlogConfig {
    /// Account callsign (the one whose log this is).
    pub callsign: Option<String>,
    /// HRDLog "upload code" — distinct from the website password.
    pub code: Option<String>,
}

impl HrdlogConfig {
    pub fn is_configured(&self) -> bool {
        self.callsign.is_some() && self.code.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WsjtxConfig {
    /// Whether to bind a UDP listener for WSJT-X messages on boot.
    /// Default: true (the bridge stays idle until WSJT-X actually sends
    /// something, so leaving it on costs nothing).
    pub enabled: bool,
    /// Address to bind. Default `127.0.0.1:2237` matches WSJT-X's default
    /// "UDP server" target. For a multi-machine setup, change to
    /// `0.0.0.0:2237`.
    pub bind_addr: String,
}

impl Default for WsjtxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_addr: "127.0.0.1:2237".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClubLogConfig {
    /// Club Log account email (login).
    pub email: Option<String>,
    /// Club Log account password.
    pub password: Option<String>,
    /// The callsign whose log is uploaded. A Club Log account can host
    /// multiple callsigns; this picks which one.
    pub callsign: Option<String>,
}

impl ClubLogConfig {
    pub fn is_configured(&self) -> bool {
        self.email.is_some() && self.password.is_some() && self.callsign.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EqslConfig {
    /// eQSL.cc account username (typically your callsign).
    pub username: Option<String>,
    /// eQSL.cc account password. Plaintext — same trade-off as LotW.
    pub password: Option<String>,
}

impl EqslConfig {
    pub fn is_configured(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DxClusterConfig {
    /// Login callsign sent to the cluster. Required by virtually every
    /// cluster — anonymous logins are rejected.
    pub my_callsign: Option<String>,
    pub sources: Vec<ClusterSourceConfig>,
    /// Optional path to a dxfeed filter JSON file. When set, spots are
    /// filtered server-side (in dxfeed's pipeline) before reaching the
    /// UI. See dxfeed docs for the schema.
    pub filter_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSourceConfig {
    pub host: String,
    pub port: u16,
}

impl DxClusterConfig {
    pub fn is_configured(&self) -> bool {
        self.my_callsign.is_some() && !self.sources.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StationConfig {
    /// Default station callsign stamped onto every QSO logged through the
    /// entry form. Required for LotW to attribute uploads correctly. Until
    /// the operating-session aggregate lands, this is the simplest way to
    /// ensure new QSOs carry STATION_CALLSIGN.
    pub default_callsign: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LotwConfig {
    /// LotW website username (used for confirmation report fetch).
    pub username: Option<String>,
    /// LotW website password. **Plaintext.** Future: opt-in OS keyring.
    pub password: Option<String>,
    /// **Deprecated; ignored.** Once the one TQSL station location every
    /// upload was signed with. Each slogger station location now names
    /// its own TQSL location (set in the station editor), so QSOs from a
    /// second QTH are signed with that QTH's details. Still parsed so
    /// existing config files load; slogger logs a warning when it is set.
    pub station_location: Option<String>,
    /// Path to the tqsl binary. Defaults to whatever's on PATH.
    pub tqsl_path: Option<PathBuf>,
    /// Path to TQSL's `station_data` file, the list of TQSL station
    /// locations. Defaults to `%APPDATA%\TrustedQSL\station_data` on
    /// Windows and `~/.tqsl/station_data` elsewhere; set this when TQSL
    /// keeps it somewhere else.
    pub tqsl_station_data: Option<PathBuf>,
}

impl LotwConfig {
    pub fn is_configured_for_fetch(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }
}

fn unknown_keys_in(text: &str) -> Vec<String> {
    let mut unknown = Vec::new();
    let de = toml::Deserializer::new(text);
    let _: Result<Config, _> =
        serde_ignored::deserialize(de, |path| unknown.push(path.to_string()));
    unknown
}

impl Config {
    pub fn default_path() -> Result<PathBuf, ConfigError> {
        let dir = dirs::config_dir().ok_or(ConfigError::NoConfigDir)?;
        Ok(dir.join("slogger").join("config.toml"))
    }

    /// Load from the platform default location. Returns `Config::default()`
    /// if the file is missing — a clean install isn't an error.
    pub fn load_default() -> Result<Self, ConfigError> {
        let path = Self::default_path()?;
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| ConfigError::Parse {
                path: path.to_path_buf(),
                error: e.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no config file found; using defaults");
                Ok(Self::default())
            }
            Err(e) => Err(ConfigError::Io {
                path: path.to_path_buf(),
                error: e.to_string(),
            }),
        }
    }

    /// Keys in the config file that slogger does not recognise, as
    /// dotted paths ("dxcluster.filter", "dxcluster.sources.1.filter_file").
    /// Such keys are otherwise ignored without complaint, so a misspelt
    /// or misplaced one looks exactly like a setting that has no effect.
    /// Empty when the file is missing or does not parse: loading reports
    /// that.
    pub fn unknown_keys(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path).map(|text| unknown_keys_in(&text)).unwrap_or_default()
    }

    /// Write a starter config file with empty sections, for a user to fill in.
    pub fn write_template(path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::Io {
                path: parent.to_path_buf(),
                error: e.to_string(),
            })?;
        }
        let text = "# slogger config\n\n[station]\n# default_callsign = \"W1ABC\"\n\n[lotw]\n# username = \"W1ABC\"\n# password = \"\"\n# tqsl_path = \"/usr/bin/tqsl\"\n# tqsl_station_data = \"/home/you/.tqsl/station_data\"\n\n[eqsl]\n# username = \"W1ABC\"\n# password = \"\"\n\n[clublog]\n# email = \"you@example.com\"\n# password = \"\"\n# callsign = \"W1ABC\"\n\n[qrz]\n# api_key = \"YOUR-KEY-FROM-QRZ-LOGBOOK\"\n\n[hrdlog]\n# callsign = \"W1ABC\"\n# code = \"YOUR-HRDLOG-UPLOAD-CODE\"\n\n[dxcluster]\n# my_callsign = \"W1ABC\"\n# sources = [\n#     { host = \"dxc.kbx.org\", port = 7300 },\n# ]\n\n[wsjtx]\n# enabled = true\n# bind_addr = \"127.0.0.1:2237\"\n\n[[rig]]\n# enabled = true\n# vendor = \"icom\"  # icom / yaesu / kenwood / elecraft / flex\n# model = \"IC-7300\"\n# serial_port = \"/dev/ttyUSB0\"  # for serial vendors\n# baud_rate = 115200\n# host = \"192.168.1.50\"  # for flex (instead of serial_port)\n# label = \"Main\"           # optional friendly name for multi-rig setups\n\n# Add another [[rig]] block for a second radio (SO2R / parallel ops).\n\n[keyer]\n# enabled = true\n# serial_port = \"/dev/ttyUSB1\"\n# initial_wpm = 25\n\n[so2r]\n# enabled = true\n# serial_port = \"/dev/ttyUSB2\"\n# initial_tx = 1                    # 1 or 2\n# initial_rx_mode = \"stereo\"        # mono / stereo / reverse_stereo\n\n[condx]\n# enabled = true\n# refresh_interval_secs = 3600     # minimum 60\n";
        std::fs::write(path, text).map_err(|e| ConfigError::Io {
            path: path.to_path_buf(),
            error: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// A config with every field set. Written without `..Default` on
    /// purpose: adding a field fails to compile here until it is listed,
    /// and then `example_config_shows_every_option` fails until it is
    /// also in docs/example-config.toml.
    fn every_option() -> Config {
        let s = || Some(String::from("x"));
        let p = || Some(PathBuf::from("x"));
        Config {
            station: StationConfig { default_callsign: s() },
            lotw: LotwConfig {
                username: s(),
                password: s(),
                station_location: s(),
                tqsl_path: p(),
                tqsl_station_data: p(),
            },
            eqsl: EqslConfig { username: s(), password: s() },
            clublog: ClubLogConfig { email: s(), password: s(), callsign: s() },
            qrz: QrzConfig { api_key: s() },
            hrdlog: HrdlogConfig { callsign: s(), code: s() },
            dxcluster: DxClusterConfig {
                my_callsign: s(),
                sources: vec![ClusterSourceConfig { host: "x".into(), port: 1 }],
                filter_file: p(),
            },
            wsjtx: WsjtxConfig { enabled: true, bind_addr: "x".into() },
            rigs: vec![RigConfig {
                enabled: true,
                vendor: s(),
                model: s(),
                serial_port: s(),
                baud_rate: Some(1),
                host: s(),
                label: s(),
                transceive: true,
            }],
            keyer: KeyerConfig {
                enabled: true,
                serial_port: s(),
                initial_wpm: Some(1),
                sidetone: Some(true),
                macros: vec![KeyerMacro::new("x", "x")],
            },
            so2r: So2rConfig {
                enabled: true,
                serial_port: s(),
                initial_tx: Some(1),
                initial_rx_mode: s(),
            },
            condx: CondXConfig { enabled: true, refresh_interval_secs: Some(1) },
            pota: PotaConfig {
                enabled: true,
                bands: vec!["x".into()],
                modes: vec!["x".into()],
                refresh_interval_secs: Some(1),
            },
        }
    }

    /// Every key under `value`, however deeply nested (`sources` holds
    /// `host` and `port`; `macro` holds `label` and `text`).
    fn keys_of(value: &toml::Value, out: &mut Vec<String>) {
        match value {
            toml::Value::Table(table) => {
                for (key, inner) in table {
                    out.push(key.clone());
                    keys_of(inner, out);
                }
            }
            toml::Value::Array(items) => items.iter().for_each(|v| keys_of(v, out)),
            _ => {}
        }
    }

    #[test]
    fn example_config_shows_every_option() {
        let example = include_str!("../../docs/example-config.toml");
        // Each section starts at its banner line, "# [name] — …".
        let section = |name: &str| -> &str {
            let is_banner = |line: &str| line.starts_with("# [") && line.contains("] — ");
            let wanted = |line: &str| {
                line.starts_with(&format!("# [{name}] — "))
                    || line.starts_with(&format!("# [[{name}]] — "))
            };
            let mut start = None;
            let mut offset = 0;
            for line in example.split_inclusive('\n') {
                match start {
                    None if wanted(line) => start = Some(offset),
                    Some(from) if is_banner(line) => return &example[from..offset],
                    _ => {}
                }
                offset += line.len();
            }
            &example[start.unwrap_or_else(|| panic!("no [{name}] section in the example"))..]
        };

        let all = toml::Value::try_from(every_option()).unwrap();
        let mut missing = Vec::new();
        for (name, value) in all.as_table().unwrap() {
            let text = section(name);
            let mut keys = Vec::new();
            keys_of(value, &mut keys);
            for key in keys {
                // Shown either live or commented out: "key = …" at the
                // start of a line, after any "#" and spaces, or inside
                // an inline table.
                let shown = text.lines().any(|line| {
                    let line = line.trim_start_matches(['#', ' ']);
                    line.starts_with(&format!("{key} = "))
                        || line.contains(&format!("{{ {key} = "))
                        || line.contains(&format!(", {key} = "))
                });
                // `macro` is shown as its [[keyer.macro]] heading.
                let heading = text.contains(&format!("[[{name}.{key}]]"));
                if !shown && !heading {
                    missing.push(format!("[{name}] {key}"));
                }
            }
        }
        assert!(missing.is_empty(), "docs/example-config.toml lacks: {missing:?}");
    }

    #[test]
    fn missing_file_returns_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("absent.toml");
        let cfg = Config::load_from(&path).unwrap();
        assert!(!cfg.lotw.is_configured_for_fetch());
    }

    #[test]
    fn parses_lotw_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[lotw]\nusername = \"W1ABC\"\npassword = \"hunter2\"\n\
             tqsl_station_data = \"/tmp/station_data\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.lotw.username.as_deref(), Some("W1ABC"));
        assert_eq!(
            cfg.lotw.tqsl_station_data.as_deref(),
            Some(Path::new("/tmp/station_data"))
        );
        assert!(cfg.lotw.is_configured_for_fetch());
    }

    #[test]
    fn deprecated_lotw_station_location_still_loads() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[lotw]\nstation_location = \"Home\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.lotw.station_location.as_deref(), Some("Home"));
    }

    #[test]
    fn parses_station_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[station]\ndefault_callsign = \"W1ABC\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.station.default_callsign.as_deref(), Some("W1ABC"));
    }

    #[test]
    fn wsjtx_defaults_enabled_on_localhost() {
        let cfg = Config::default();
        assert!(cfg.wsjtx.enabled);
        assert_eq!(cfg.wsjtx.bind_addr, "127.0.0.1:2237");
    }

    #[test]
    fn parses_wsjtx_override() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[wsjtx]\nenabled = false\nbind_addr = \"0.0.0.0:2237\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert!(!cfg.wsjtx.enabled);
        assert_eq!(cfg.wsjtx.bind_addr, "0.0.0.0:2237");
    }

    #[test]
    fn parses_so2r_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[so2r]\nenabled = true\nserial_port = \"/dev/ttyUSB2\"\n\
             initial_tx = 2\ninitial_rx_mode = \"stereo\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert!(cfg.so2r.is_configured());
        assert_eq!(cfg.so2r.initial_tx, Some(2));
        assert_eq!(cfg.so2r.initial_rx_mode.as_deref(), Some("stereo"));
    }

    #[test]
    fn so2r_disabled_by_default() {
        assert!(!Config::default().so2r.is_configured());
    }

    #[test]
    fn parses_condx_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[condx]\nenabled = true\nrefresh_interval_secs = 900\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert!(cfg.condx.enabled);
        assert_eq!(cfg.condx.refresh_interval_secs, Some(900));
    }

    #[test]
    fn condx_disabled_by_default() {
        let cfg = Config::default();
        assert!(!cfg.condx.enabled);
        assert_eq!(cfg.condx.refresh_interval_secs, None);
    }

    #[test]
    fn parses_keyer_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[keyer]\nenabled = true\nserial_port = \"/dev/ttyUSB1\"\ninitial_wpm = 28\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert!(cfg.keyer.is_configured());
        assert_eq!(cfg.keyer.initial_wpm, Some(28));
    }

    #[test]
    fn keyer_disabled_by_default() {
        assert!(!Config::default().keyer.is_configured());
    }

    #[test]
    fn parses_single_rig_via_array_syntax() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[[rig]]\nenabled = true\nvendor = \"icom\"\nmodel = \"IC-7300\"\n\
             serial_port = \"/dev/ttyUSB0\"\nbaud_rate = 115200\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.rigs().len(), 1);
        assert!(cfg.rigs()[0].is_configured());
        assert_eq!(cfg.rigs()[0].model.as_deref(), Some("IC-7300"));
    }

    #[test]
    fn parses_two_rigs_for_so2r() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[[rig]]\nenabled = true\nvendor = \"icom\"\nmodel = \"IC-7300\"\n\
             serial_port = \"/dev/ttyUSB0\"\nlabel = \"Main\"\n\n\
             [[rig]]\nenabled = true\nvendor = \"yaesu\"\nmodel = \"FT-DX10\"\n\
             serial_port = \"/dev/ttyUSB1\"\nlabel = \"Aux\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.rigs().len(), 2);
        assert_eq!(cfg.rigs()[0].label.as_deref(), Some("Main"));
        assert_eq!(cfg.rigs()[1].label.as_deref(), Some("Aux"));
    }

    #[test]
    fn no_rigs_when_section_absent() {
        assert!(Config::default().rigs().is_empty());
    }

    #[test]
    fn parses_hrdlog_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[hrdlog]\ncallsign = \"W1ABC\"\ncode = \"abc123\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.hrdlog.callsign.as_deref(), Some("W1ABC"));
        assert!(cfg.hrdlog.is_configured());
    }

    #[test]
    fn parses_qrz_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[qrz]\napi_key = \"ABCD-1234\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.qrz.api_key.as_deref(), Some("ABCD-1234"));
        assert!(cfg.qrz.is_configured());
    }

    #[test]
    fn parses_clublog_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[clublog]\nemail = \"u@e.com\"\npassword = \"x\"\ncallsign = \"W1ABC\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.clublog.email.as_deref(), Some("u@e.com"));
        assert_eq!(cfg.clublog.callsign.as_deref(), Some("W1ABC"));
        assert!(cfg.clublog.is_configured());
    }

    #[test]
    fn parses_eqsl_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[eqsl]\nusername = \"W1ABC\"\npassword = \"hunter2\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.eqsl.username.as_deref(), Some("W1ABC"));
        assert!(cfg.eqsl.is_configured());
    }

    #[test]
    fn misplaced_and_misspelt_config_keys_are_found() {
        // After an array-of-tables header a key belongs to that source,
        // not to [dxcluster].
        let text = "[dxcluster]\nmy_callsign = \"W1ABC\"\nfilter = \"a.json\"\n\
                    [[dxcluster.sources]]\nhost = \"h\"\nport = 1\nfilter_file = \"a.json\"\n";
        assert_eq!(unknown_keys_in(text), ["dxcluster.filter", "dxcluster.sources.0.filter_file"]);
        assert!(unknown_keys_in("[dxcluster]\nfilter_file = \"a.json\"\n").is_empty());
        assert_eq!(unknown_keys_in("filter_file = \"a.json\"\n[dxcluster]\n"), ["filter_file"]);
    }

    #[test]
    fn parses_dxcluster_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[dxcluster]\nmy_callsign = \"W1ABC\"\nsources = [{ host = \"dxc.kbx.org\", port = 7300 }]\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.dxcluster.my_callsign.as_deref(), Some("W1ABC"));
        assert_eq!(cfg.dxcluster.sources.len(), 1);
        assert_eq!(cfg.dxcluster.sources[0].host, "dxc.kbx.org");
        assert!(cfg.dxcluster.is_configured());
    }

    #[test]
    fn write_template_creates_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        Config::write_template(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[lotw]"));
        let cfg = Config::load_from(&path).unwrap();
        assert!(!cfg.lotw.is_configured_for_fetch());
    }

    #[test]
    fn keyer_macros_default_when_none_configured() {
        let cfg = Config::default();
        assert_eq!(cfg.keyer.macros(), KeyerMacro::defaults());
    }

    #[test]
    fn parses_keyer_macro_tables() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[keyer]\nenabled = true\n\n[[keyer.macro]]\nlabel = \"CQ\"\ntext = \"CQ TEST {MYCALL}\"\n\n\
             [[keyer.macro]]\nlabel = \"TU\"\ntext = \"TU\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(
            cfg.keyer.macros(),
            vec![KeyerMacro::new("CQ", "CQ TEST {MYCALL}"), KeyerMacro::new("TU", "TU")]
        );
    }

    #[test]
    fn keyer_macro_expands_placeholders() {
        let ctx = MacroContext {
            my_call: Some("w1abc"),
            call: Some(" JA1NUT "),
            rst_sent: Some("599"),
        };
        let m = KeyerMacro::new("Report", "{CALL} de {MYCALL} ur {RST} {RST}");
        assert_eq!(m.expand(ctx).unwrap(), "JA1NUT DE W1ABC UR 5NN 5NN");
    }

    #[test]
    fn keyer_macro_reports_missing_placeholder_value() {
        let ctx = MacroContext {
            my_call: Some("W1ABC"),
            call: Some(""),
            rst_sent: None,
        };
        assert_eq!(KeyerMacro::new("His", "{CALL}").expand(ctx), Err("{CALL}"));
        // A message that doesn't use the missing value still expands.
        assert_eq!(KeyerMacro::new("TU", "tu 73").expand(ctx).unwrap(), "TU 73");
    }
}
