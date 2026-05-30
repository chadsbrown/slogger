use std::sync::Arc;

use app_config::{Config, RigConfig as RigConfigToml};
use keyer_control::KeyerHandle;
use logbook_domain::{LogbookService, QsoRepository, StationRepository};
use radio_core::{StationLocation, StationLocationId};
use rig_control::{RigHandle, RigSnapshot};
use so2r_control::So2rHandle;
use station_resolver::Resolver;

use super::layout::LayoutTree;

/// Display wrapper for the station picker. iced's pick_list needs ToString
/// + Clone + PartialEq, and we don't want to leak Display onto the domain
/// type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationOption {
    pub id: StationLocationId,
    pub label: String,
}

impl std::fmt::Display for StationOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

impl From<&StationLocation> for StationOption {
    fn from(loc: &StationLocation) -> Self {
        let label = match &loc.station_callsign {
            Some(c) => format!("{} ({})", loc.name, c.as_str()),
            None => loc.name.clone(),
        };
        Self { id: loc.id, label }
    }
}

/// Tagged rig snapshot multiplexed onto the unified channel. Per-rig
/// forwarder tasks wrap each `RigSnapshot` with the rig's index before
/// pushing it to the shared receiver.
#[derive(Debug, Clone)]
pub struct TaggedRigSnapshot {
    pub rig_index: usize,
    pub snapshot: RigSnapshot,
}

/// One entry per configured rig. Index corresponds to the order in the
/// `[[rig]]` array. Snapshots are populated as the rig task forwards
/// events; status is the boot-time connect outcome.
#[derive(Debug, Clone)]
pub struct RigEntry {
    pub label: String,
    pub config: RigConfigToml,
    pub handle: Option<RigHandle>,
    pub snapshot: Option<RigSnapshot>,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct BootBundle {
    pub service: Arc<LogbookService>,
    pub repo: Arc<dyn QsoRepository>,
    pub station_repo: Arc<dyn StationRepository>,
    pub resolver: Arc<dyn Resolver>,
    pub config: Config,
    pub station_locations: Vec<StationLocation>,
    pub active_location: Option<StationLocation>,
    pub spots_active: bool,
    pub wsjtx_active: bool,
    pub wsjtx_bind_addr: Option<String>,
    pub rigs: Vec<RigEntry>,
    pub keyer_active: bool,
    pub keyer_status: Option<String>,
    pub keyer_handle: Option<KeyerHandle>,
    pub so2r_active: bool,
    pub so2r_status: Option<String>,
    pub so2r_handle: Option<So2rHandle>,
    /// Saved pane layout from `ui-layout.json`, if present. `None` means
    /// the operator hasn't customized the layout yet (or the file was
    /// unparseable) — caller falls back to the default split.
    pub pane_layout: Option<LayoutTree>,
}
