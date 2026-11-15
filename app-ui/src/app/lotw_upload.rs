//! LoTW upload, one TQSL signing run per (station location, callsign).
//!
//! Each slogger station location names the TQSL station location its QSOs
//! are signed with (`StationLocation::lotw_station_location`). A QSO is
//! only ever handed to TQSL when that mapping is certain; anything that
//! can't be mapped is held back — left pending, not marked failed — and
//! the summary says why, so the operator can fix the mapping and retry.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use app_config::LotwConfig;
use chrono::Utc;
use logbook_domain::{ExportOptions, PendingUploadCount, QsoRepository, export_adif};
use lotw_sync::{LotwUploadClient, LotwUploadConfig, SignedUpload, TqslStationData};
use radio_core::{Qso, StationLocation, StationLocationId};

use super::constants::LOTW_SERVICE;

/// The most QSOs one run signs. Applied after unmappable QSOs are set
/// aside, so held-back QSOs never take the place of sendable ones; any
/// left over go on the next run.
const BATCH_LIMIT: u32 = 500;

/// Why pending QSOs were not sent to TQSL.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum HoldReason {
    /// The QSO isn't linked to a slogger station location.
    NoStationLocation,
    /// The QSO's location has no TQSL location set.
    NoTqslLocation { location: String },
    /// The location names a TQSL location TQSL's list doesn't have.
    NotInTqsl { location: String, tqsl_name: String },
    /// The QSO has no station callsign, so there's no certificate to pick.
    NoStationCallsign { location: String },
}

impl std::fmt::Display for HoldReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoStationLocation => f.write_str("no station location"),
            Self::NoTqslLocation { location } => write!(f, "\"{location}\" has no TQSL location"),
            Self::NotInTqsl {
                location,
                tqsl_name,
            } => {
                write!(f, "\"{location}\": TQSL has no location \"{tqsl_name}\"")
            }
            Self::NoStationCallsign { location } => {
                write!(f, "at \"{location}\" without a station callsign")
            }
        }
    }
}

/// A batch TQSL refused or LoTW didn't take. Its QSOs stay pending.
#[derive(Debug, Clone)]
pub struct GroupFailure {
    pub location: String,
    pub callsign: String,
    pub count: usize,
    pub error: String,
}

/// Outcome of one LoTW upload run.
#[derive(Debug, Clone, Default)]
pub struct LotwUploadSummary {
    /// Signed, posted and marked uploaded.
    pub uploaded: usize,
    /// TQSL said these were already uploaded; marked uploaded unposted.
    pub already_uploaded: usize,
    /// Sendable but beyond this run's batch limit.
    pub deferred: usize,
    pub held_back: Vec<(HoldReason, usize)>,
    pub failed: Vec<GroupFailure>,
    /// LoTW answered at least one post with a body that didn't clearly
    /// say "accepted".
    pub response_unclear: bool,
    /// TQSL's location list couldn't be read, so names were passed to
    /// TQSL unchecked.
    pub tqsl_list_unread: bool,
}

impl LotwUploadSummary {
    pub fn held_back_total(&self) -> usize {
        self.held_back.iter().map(|(_, n)| n).sum()
    }
}

/// One line: what was sent, what was held back and why, what failed.
pub(super) fn format_lotw_upload(s: &LotwUploadSummary) -> String {
    let mut parts = vec![format!("{} uploaded", s.uploaded)];
    if s.already_uploaded > 0 {
        parts.push(format!("{} already at LoTW", s.already_uploaded));
    }
    if s.deferred > 0 {
        parts.push(format!("{} more next run", s.deferred));
    }
    let held = s.held_back_total();
    if held > 0 {
        let reasons: Vec<String> = s
            .held_back
            .iter()
            .map(|(r, n)| format!("{n} {r}"))
            .collect();
        parts.push(format!("{held} held back ({})", reasons.join(", ")));
    }
    for f in &s.failed {
        parts.push(format!(
            "{} failed (\"{}\" as {}: {})",
            f.count, f.location, f.callsign, f.error
        ));
    }
    if s.response_unclear {
        parts.push("LoTW response unclear — verify in your LoTW account".into());
    }
    if s.tqsl_list_unread && (s.uploaded > 0 || !s.failed.is_empty()) {
        parts.push("TQSL location list unreadable, names not checked".into());
    }
    parts.join(" · ")
}

/// Whether LoTW upload has anything to work with: some station location
/// names a TQSL location. This replaces the old `[lotw] station_location`
/// check — the mapping now lives on the locations, not in the config.
pub(super) fn upload_configured(locations: &[StationLocation]) -> bool {
    locations.iter().any(|l| {
        l.lotw_station_location
            .as_deref()
            .is_some_and(|n| !n.trim().is_empty())
    })
}

/// The TQSL name to sign `location`'s QSOs with, or why there is none.
/// With TQSL's list unreadable, an explicitly set name is trusted: TQSL
/// refuses a name it doesn't know, signing nothing.
pub(super) fn tqsl_name_for(
    location: &StationLocation,
    tqsl: &TqslStationData,
) -> Result<String, HoldReason> {
    let name = location
        .lotw_station_location
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| HoldReason::NoTqslLocation {
            location: location.name.clone(),
        })?;
    if tqsl.locations().is_some() && tqsl.find(name).is_none() {
        return Err(HoldReason::NotInTqsl {
            location: location.name.clone(),
            tqsl_name: name.to_string(),
        });
    }
    Ok(name.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sendable {
    pub location_name: String,
    pub tqsl_name: String,
}

/// Which locations' QSOs may go to TQSL, and what is held back.
#[derive(Debug, Default)]
pub(super) struct UploadPlan {
    pub sendable: HashMap<StationLocationId, Sendable>,
    /// How many pending QSOs the sendable locations hold in total.
    pub sendable_count: usize,
    /// Largest group first.
    pub held_back: Vec<(HoldReason, usize)>,
}

pub(super) fn plan_upload(
    counts: &[PendingUploadCount],
    locations: &[StationLocation],
    tqsl: &TqslStationData,
) -> UploadPlan {
    let mut plan = UploadPlan::default();
    let mut held: BTreeMap<HoldReason, usize> = BTreeMap::new();
    for c in counts {
        let location = c
            .station_location_id
            .and_then(|id| locations.iter().find(|l| l.id == id));
        let Some(location) = location else {
            *held.entry(HoldReason::NoStationLocation).or_default() += c.count;
            continue;
        };
        match tqsl_name_for(location, tqsl) {
            Err(reason) => *held.entry(reason).or_default() += c.count,
            Ok(_) if !c.has_station_callsign => {
                *held
                    .entry(HoldReason::NoStationCallsign {
                        location: location.name.clone(),
                    })
                    .or_default() += c.count;
            }
            Ok(tqsl_name) => {
                plan.sendable.insert(
                    location.id,
                    Sendable {
                        location_name: location.name.clone(),
                        tqsl_name,
                    },
                );
                plan.sendable_count += c.count;
            }
        }
    }
    plan.held_back = held.into_iter().collect();
    plan.held_back
        .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    plan
}

/// QSOs signed together: same TQSL location, same certificate.
#[derive(Debug)]
pub(super) struct UploadGroup {
    pub location_name: String,
    pub tqsl_name: String,
    pub callsign: String,
    pub qsos: Vec<Qso>,
}

/// Splits `qsos` by (location, station callsign). A QSO the plan doesn't
/// cover is dropped rather than sent — the query that produced `qsos`
/// already excludes those, so this only guards the rule.
pub(super) fn group_for_upload(qsos: Vec<Qso>, plan: &UploadPlan) -> Vec<UploadGroup> {
    // Keyed by name first so batches run in a stable, readable order.
    let mut groups: BTreeMap<(String, String, u128), UploadGroup> = BTreeMap::new();
    for qso in qsos {
        let (Some(id), Some(call)) = (qso.station_location_id, qso.station_callsign.as_ref())
        else {
            continue;
        };
        let Some(sendable) = plan.sendable.get(&id) else {
            continue;
        };
        let callsign = call.as_str().to_string();
        groups
            .entry((
                sendable.location_name.clone(),
                callsign.clone(),
                id.as_uuid().as_u128(),
            ))
            .or_insert_with(|| UploadGroup {
                location_name: sendable.location_name.clone(),
                tqsl_name: sendable.tqsl_name.clone(),
                callsign,
                qsos: Vec::new(),
            })
            .qsos
            .push(qso);
    }
    groups.into_values().collect()
}

pub(super) async fn run_lotw_upload(
    repo: Arc<dyn QsoRepository>,
    locations: &[StationLocation],
    cfg: &LotwConfig,
) -> Result<LotwUploadSummary, String> {
    let tqsl = TqslStationData::load(cfg.tqsl_station_data.as_deref());
    if let TqslStationData::Unavailable { path, reason } = &tqsl {
        tracing::info!(
            path = %path.as_deref().map(|p| p.display().to_string()).unwrap_or_default(),
            %reason,
            "TQSL station list unreadable; using station-location names unchecked"
        );
    }
    let counts = repo
        .count_pending_uploads_by_location(LOTW_SERVICE)
        .await
        .map_err(|e| e.to_string())?;
    let plan = plan_upload(&counts, locations, &tqsl);
    let ids: Vec<StationLocationId> = plan.sendable.keys().copied().collect();
    let qsos = repo
        .list_pending_uploads_at(LOTW_SERVICE, &ids, Some(BATCH_LIMIT))
        .await
        .map_err(|e| e.to_string())?;

    let mut summary = LotwUploadSummary {
        deferred: plan.sendable_count.saturating_sub(qsos.len()),
        held_back: plan.held_back.clone(),
        tqsl_list_unread: tqsl.locations().is_none(),
        ..Default::default()
    };

    let mut upload_cfg = LotwUploadConfig::default();
    if let Some(p) = cfg.tqsl_path.clone() {
        upload_cfg = upload_cfg.with_tqsl_path(p);
    }
    let client = LotwUploadClient::new(upload_cfg);

    // Groups are independent: one TQSL or LoTW failure leaves the others
    // to go ahead.
    for group in group_for_upload(qsos, &plan) {
        let adif = export_adif(&group.qsos, &ExportOptions::default());
        match client
            .upload_adif(&adif, &group.tqsl_name, &group.callsign)
            .await
        {
            Ok(SignedUpload::Posted(outcome)) => {
                summary.uploaded += mark_uploaded(&repo, &group.qsos).await;
                summary.response_unclear |= !outcome.accepted;
            }
            Ok(SignedUpload::AlreadyUploaded(status)) => {
                tracing::info!(
                    location = %group.location_name,
                    callsign = %group.callsign,
                    %status,
                    "TQSL reports these QSOs already uploaded"
                );
                summary.already_uploaded += mark_uploaded(&repo, &group.qsos).await;
            }
            Err(e) => {
                tracing::warn!(
                    location = %group.location_name,
                    callsign = %group.callsign,
                    error = %e,
                    "LoTW upload of one batch failed"
                );
                summary.failed.push(GroupFailure {
                    location: group.location_name.clone(),
                    callsign: group.callsign.clone(),
                    count: group.qsos.len(),
                    error: e.to_string(),
                });
            }
        }
    }
    Ok(summary)
}

async fn mark_uploaded(repo: &Arc<dyn QsoRepository>, qsos: &[Qso]) -> usize {
    let now = Utc::now();
    let mut marked = 0;
    for qso in qsos {
        match repo.mark_uploaded(&qso.id, LOTW_SERVICE, now, None).await {
            Ok(()) => marked += 1,
            Err(e) => tracing::warn!(qso_id = %qso.id, error = %e, "failed to mark qso uploaded"),
        }
    }
    marked
}

#[cfg(test)]
mod tests {
    use super::*;
    use lotw_sync::TqslStationLocation;
    use radio_core::{Callsign, QsoId};
    use std::path::PathBuf;

    fn location(name: &str, tqsl: Option<&str>) -> StationLocation {
        let now = Utc::now();
        StationLocation {
            id: StationLocationId::new(),
            name: name.into(),
            station_callsign: None,
            owner_callsign: None,
            city: None,
            county: None,
            state: None,
            country: None,
            grid: None,
            latitude: None,
            longitude: None,
            cq_zone: None,
            itu_zone: None,
            iota: None,
            lotw_station_location: tqsl.map(str::to_string),
            eqsl_account: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn tqsl_with(names: &[&str]) -> TqslStationData {
        TqslStationData::Read {
            path: PathBuf::from("/x"),
            locations: names
                .iter()
                .map(|n| TqslStationLocation {
                    name: n.to_string(),
                    ..Default::default()
                })
                .collect(),
        }
    }

    fn unreadable() -> TqslStationData {
        TqslStationData::Unavailable {
            path: None,
            reason: "missing".into(),
        }
    }

    fn count(loc: Option<&StationLocation>, has_call: bool, n: usize) -> PendingUploadCount {
        PendingUploadCount {
            station_location_id: loc.map(|l| l.id),
            has_station_callsign: has_call,
            count: n,
        }
    }

    fn qso(loc: Option<&StationLocation>, call: Option<&str>) -> Qso {
        let now = Utc::now();
        Qso {
            id: QsoId::new(),
            call: Callsign::parse("DL1XYZ").unwrap(),
            qso_begin: now,
            qso_end: None,
            band: None,
            freq_hz: None,
            mode: None,
            submode: None,
            rst_sent: None,
            rst_rcvd: None,
            operator_id: None,
            station_location_id: loc.map(|l| l.id),
            station_callsign: call.map(|c| Callsign::parse(c).unwrap()),
            owner_callsign: None,
            operator_callsign: None,
            dxcc_id: None,
            dxcc_prefix: None,
            wpx_prefix: None,
            continent: None,
            cq_zone: None,
            itu_zone: None,
            grid: None,
            state: None,
            county: None,
            province: None,
            iota: None,
            latitude: None,
            longitude: None,
            name: None,
            qth: None,
            address: None,
            email: None,
            age: None,
            qsl_via: None,
            comment: None,
            contest_id: None,
            serial_sent: None,
            serial_rcvd: None,
            exchange_sent: None,
            exchange_rcvd: None,
            arrl_section: None,
            band_rx: None,
            freq_rx_hz: None,
            tx_power_w: None,
            rx_power_w: None,
            propagation_mode: None,
            sat_name: None,
            sat_mode: None,
            distance_km: None,
            bearing_deg: None,
            sfi: None,
            a_index: None,
            k_index: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn holds_back_everything_that_cannot_be_mapped() {
        let home = location("Home", Some("Home"));
        let versailles = location("Versailles-House", None);
        let stale = location("Cabin", Some("Old Cabin"));
        let blank = location("Blank", Some("  "));
        let locations = [
            home.clone(),
            versailles.clone(),
            stale.clone(),
            blank.clone(),
        ];
        let counts = [
            count(Some(&home), true, 120),
            count(Some(&home), false, 2),
            count(Some(&versailles), true, 5),
            count(Some(&stale), true, 3),
            count(Some(&blank), true, 1),
            count(None, true, 9),
        ];
        let plan = plan_upload(&counts, &locations, &tqsl_with(&["Home"]));
        assert_eq!(plan.sendable_count, 120);
        assert_eq!(plan.sendable.len(), 1);
        assert_eq!(plan.sendable[&home.id].tqsl_name, "Home");
        assert_eq!(
            plan.held_back,
            vec![
                (HoldReason::NoStationLocation, 9),
                (
                    HoldReason::NoTqslLocation {
                        location: "Versailles-House".into()
                    },
                    5
                ),
                (
                    HoldReason::NotInTqsl {
                        location: "Cabin".into(),
                        tqsl_name: "Old Cabin".into()
                    },
                    3
                ),
                (
                    HoldReason::NoStationCallsign {
                        location: "Home".into()
                    },
                    2
                ),
                (
                    HoldReason::NoTqslLocation {
                        location: "Blank".into()
                    },
                    1
                ),
            ]
        );
    }

    #[test]
    fn a_link_to_an_unknown_location_counts_as_no_location() {
        let ghost = location("Ghost", Some("Home"));
        let plan = plan_upload(&[count(Some(&ghost), true, 4)], &[], &tqsl_with(&["Home"]));
        assert_eq!(plan.held_back, vec![(HoldReason::NoStationLocation, 4)]);
    }

    #[test]
    fn an_unreadable_tqsl_list_trusts_names_the_operator_set() {
        let cabin = location("Cabin", Some("Old Cabin"));
        let bare = location("Bare", None);
        let plan = plan_upload(
            &[count(Some(&cabin), true, 3), count(Some(&bare), true, 1)],
            &[cabin.clone(), bare],
            &unreadable(),
        );
        assert_eq!(plan.sendable[&cabin.id].tqsl_name, "Old Cabin");
        assert_eq!(
            plan.held_back,
            vec![(
                HoldReason::NoTqslLocation {
                    location: "Bare".into()
                },
                1
            )]
        );
    }

    #[test]
    fn tqsl_names_are_trimmed_but_matched_exactly() {
        let loc = location("Home", Some(" Home "));
        assert_eq!(tqsl_name_for(&loc, &tqsl_with(&["Home"])).unwrap(), "Home");
        assert!(tqsl_name_for(&loc, &tqsl_with(&["home"])).is_err());
    }

    #[test]
    fn groups_split_by_location_and_station_callsign() {
        let home = location("Home", Some("Home QTH"));
        let field = location("Field", Some("Field Day"));
        let unmapped = location("Unmapped", None);
        let locations = [home.clone(), field.clone(), unmapped.clone()];
        let counts = [count(Some(&home), true, 3), count(Some(&field), true, 1)];
        let plan = plan_upload(&counts, &locations, &tqsl_with(&["Home QTH", "Field Day"]));

        let qsos = vec![
            qso(Some(&home), Some("N9UNX")),
            qso(Some(&home), Some("N9UNX/P")),
            qso(Some(&home), Some("N9UNX")),
            qso(Some(&field), Some("N9UNX")),
            // Never sent, whatever the caller passes in.
            qso(Some(&unmapped), Some("N9UNX")),
            qso(None, Some("N9UNX")),
            qso(Some(&home), None),
        ];
        let groups = group_for_upload(qsos, &plan);
        let shape: Vec<(&str, &str, &str, usize)> = groups
            .iter()
            .map(|g| {
                (
                    g.location_name.as_str(),
                    g.tqsl_name.as_str(),
                    g.callsign.as_str(),
                    g.qsos.len(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("Field", "Field Day", "N9UNX", 1),
                ("Home", "Home QTH", "N9UNX", 2),
                ("Home", "Home QTH", "N9UNX/P", 1),
            ]
        );
    }

    #[test]
    fn summary_line_names_counts_reasons_and_failures() {
        let s = LotwUploadSummary {
            uploaded: 120,
            held_back: vec![
                (HoldReason::NoStationLocation, 9),
                (
                    HoldReason::NoTqslLocation {
                        location: "Versailles-House".into(),
                    },
                    5,
                ),
            ],
            ..Default::default()
        };
        assert_eq!(
            format_lotw_upload(&s),
            "120 uploaded · 14 held back (9 no station location, 5 \"Versailles-House\" has no TQSL location)"
        );

        let s = LotwUploadSummary {
            uploaded: 0,
            already_uploaded: 4,
            deferred: 30,
            failed: vec![GroupFailure {
                location: "Home".into(),
                callsign: "N9UNX/P".into(),
                count: 3,
                error: "TQSL: Invalid passphrase (15)".into(),
            }],
            tqsl_list_unread: true,
            ..Default::default()
        };
        assert_eq!(
            format_lotw_upload(&s),
            "0 uploaded · 4 already at LoTW · 30 more next run · 3 failed (\"Home\" as N9UNX/P: \
             TQSL: Invalid passphrase (15)) · TQSL location list unreadable, names not checked"
        );
    }

    #[test]
    fn upload_is_configured_once_any_location_names_a_tqsl_location() {
        assert!(!upload_configured(&[]));
        assert!(!upload_configured(&[
            location("A", None),
            location("B", Some(" "))
        ]));
        assert!(upload_configured(&[
            location("A", None),
            location("B", Some("B"))
        ]));
    }
}
