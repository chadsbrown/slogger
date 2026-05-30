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
}

impl KeyerConfig {
    pub fn is_configured(&self) -> bool {
        self.enabled && self.serial_port.is_some()
    }
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
    /// TQSL station-location *name* — must match a location defined inside
    /// the user's tqsl install. This is what tqsl signs against.
    pub station_location: Option<String>,
    /// Path to the tqsl binary. Defaults to whatever's on PATH.
    pub tqsl_path: Option<PathBuf>,
}

impl LotwConfig {
    pub fn is_configured_for_upload(&self) -> bool {
        self.station_location.is_some()
    }

    pub fn is_configured_for_fetch(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }
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

    /// Write a starter config file with empty sections, for a user to fill in.
    pub fn write_template(path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::Io {
                path: parent.to_path_buf(),
                error: e.to_string(),
            })?;
        }
        let text = "# slogger config\n\n[station]\n# default_callsign = \"W1ABC\"\n\n[lotw]\n# username = \"W1ABC\"\n# password = \"\"\n# station_location = \"Home\"\n# tqsl_path = \"/usr/bin/tqsl\"\n\n[eqsl]\n# username = \"W1ABC\"\n# password = \"\"\n\n[clublog]\n# email = \"you@example.com\"\n# password = \"\"\n# callsign = \"W1ABC\"\n\n[qrz]\n# api_key = \"YOUR-KEY-FROM-QRZ-LOGBOOK\"\n\n[hrdlog]\n# callsign = \"W1ABC\"\n# code = \"YOUR-HRDLOG-UPLOAD-CODE\"\n\n[dxcluster]\n# my_callsign = \"W1ABC\"\n# sources = [\n#     { host = \"dxc.kbx.org\", port = 7300 },\n# ]\n\n[wsjtx]\n# enabled = true\n# bind_addr = \"127.0.0.1:2237\"\n\n[[rig]]\n# enabled = true\n# vendor = \"icom\"  # icom / yaesu / kenwood / elecraft / flex\n# model = \"IC-7300\"\n# serial_port = \"/dev/ttyUSB0\"  # for serial vendors\n# baud_rate = 115200\n# host = \"192.168.1.50\"  # for flex (instead of serial_port)\n# label = \"Main\"           # optional friendly name for multi-rig setups\n\n# Add another [[rig]] block for a second radio (SO2R / parallel ops).\n\n[keyer]\n# enabled = true\n# serial_port = \"/dev/ttyUSB1\"\n# initial_wpm = 25\n\n[so2r]\n# enabled = true\n# serial_port = \"/dev/ttyUSB2\"\n# initial_tx = 1                    # 1 or 2\n# initial_rx_mode = \"stereo\"        # mono / stereo / reverse_stereo\n";
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

    #[test]
    fn missing_file_returns_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("absent.toml");
        let cfg = Config::load_from(&path).unwrap();
        assert!(!cfg.lotw.is_configured_for_upload());
        assert!(!cfg.lotw.is_configured_for_fetch());
    }

    #[test]
    fn parses_lotw_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[lotw]\nusername = \"W1ABC\"\npassword = \"hunter2\"\nstation_location = \"Home\"\n",
        )
        .unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.lotw.username.as_deref(), Some("W1ABC"));
        assert_eq!(cfg.lotw.station_location.as_deref(), Some("Home"));
        assert!(cfg.lotw.is_configured_for_upload());
        assert!(cfg.lotw.is_configured_for_fetch());
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
        assert!(!cfg.lotw.is_configured_for_upload());
    }
}
