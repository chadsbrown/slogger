//! Applying a QSL service's report of our QSOs to the log (LoTW today).
//!
//! Each record is matched to the local QSO it describes, then turned into
//! state changes (upload verified, confirmed), award credit, and — from a
//! confirmation's detail about the other station — location fields filled
//! in or differences recorded (see `qsl_detail`). Matching and planning
//! are pure; `apply_report` wraps them with the bulk loads and the one
//! batched write they need, so a report of tens of thousands of records
//! costs a handful of queries, not one per record.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use radio_core::QsoId;

use crate::credit::{AwardCredit, CreditStatus};
use crate::qsl_detail::{DetailField, FieldProvenance, plan_detail};
use crate::repository::{
    CreditUpgrade, ImportedCredit, QsoRepository, RepoResult, SyncBatch, SyncCandidate,
    SyncStateChange, SyncWatermark,
};
use crate::service::credit_unit;

/// How far apart a record's start time and a local QSO's may be and
/// still be the same QSO, in seconds.
///
/// A service's record is our own upload played back, so the times
/// normally agree exactly; what the window has to absorb is LoTW
/// dropping seconds, another logger having uploaded the QSO with its own
/// rounding or with the end rather than the start time (a minute or two
/// for FT8), and small edits made after uploading. Five minutes covers
/// those. It is not what separates two QSOs with the same station on one
/// band and day: nearest-first, one-to-one assignment does that, so the
/// window can stay generous without merging them.
pub const MATCH_TOLERANCE_SECS: i64 = 5 * 60;

/// One record of a service's report, reduced to what matching and the
/// state changes need.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceQso {
    /// `None` when the service didn't say; then any station call matches.
    pub station_callsign: Option<String>,
    pub call: String,
    pub start: DateTime<Utc>,
    pub band: Option<String>,
    pub mode: Option<String>,
    /// The service's mode group (CW / PHONE / DATA), if it gave one.
    pub mode_group: Option<String>,
    pub confirmed: bool,
    pub confirmed_at: Option<DateTime<Utc>>,
    /// When the service received our upload.
    pub uploaded_at: Option<DateTime<Utc>>,
    /// Award credit the service reports for the QSO. Only recorded when
    /// the record is a confirmation.
    pub credits: Vec<AwardCredit>,
    /// What the service says the credit counts towards (the other
    /// station's entity, state, grid) — the sponsor's view. The local
    /// QSO's values fill in what it leaves out.
    pub dxcc_id: Option<u16>,
    pub state: Option<String>,
    pub grid: Option<String>,
    /// What the confirming station told the service about its location
    /// (LoTW's `DXCC`, `CONT`, `CQZ`, …), raw. Only used when the record
    /// is a confirmation; normalised by `qsl_detail::normalize`. Empty
    /// when the report wasn't asked for detail — LoTW always sends `DXCC`
    /// and `PFX` when it was — so then the stored detail is left alone.
    pub detail: Vec<(DetailField, String)>,
}

/// What applying one report did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncOutcome {
    /// QSOs that became upload-verified.
    pub newly_verified: usize,
    /// QSOs that became confirmed.
    pub newly_confirmed: usize,
    /// Award credits added, or raised from submitted to granted.
    pub credits_changed: usize,
    /// QSOs with at least one location field filled in (or a grid made
    /// more precise) from the service.
    pub qsos_filled: usize,
    /// QSOs where a difference from the service was newly recorded, or
    /// the service's differing value changed.
    pub qsos_differing: usize,
    /// Indexes into the report of records that match no QSO in the log —
    /// expected for QSOs uploaded from elsewhere.
    pub not_in_log: Vec<usize>,
}

/// Mode groups as LoTW draws them, for preferring the right QSO when two
/// differ only in mode. Coarse on purpose: it only breaks ties.
fn mode_group(mode: &str) -> &'static str {
    match mode.to_ascii_uppercase().as_str() {
        "CW" => "CW",
        "SSB" | "USB" | "LSB" | "AM" | "FM" | "DIGITALVOICE" | "PHONE" => "PHONE",
        _ => "DATA",
    }
}

/// 0 = same mode, 1 = same mode group, 2 = neither (or a mode missing).
/// Mode never stops a match — users remap modes before upload — it only
/// orders otherwise equal candidates.
fn mode_rank(r: &ServiceQso, c: &SyncCandidate) -> u8 {
    let (Some(rm), Some(cm)) = (r.mode.as_deref(), c.mode.as_deref()) else {
        return 2;
    };
    if rm.eq_ignore_ascii_case(cm) {
        return 0;
    }
    let r_group = r.mode_group.as_deref().unwrap_or_else(|| mode_group(rm));
    if r_group.eq_ignore_ascii_case(mode_group(cm)) {
        1
    } else {
        2
    }
}

/// For each record, the index of the candidate it describes, if any.
///
/// A record and a QSO can pair when the worked call and band are equal
/// (ignoring case), the station calls agree (when the record names one)
/// and the start times are within `MATCH_TOLERANCE_SECS`. Pairs are then
/// taken nearest in time first, preferring the same mode and then the
/// same mode group, and each record and each QSO is used at most once.
/// That one-to-one rule is what keeps two QSOs with one station on one
/// day and band apart: each record gets its own QSO even when both QSOs
/// fall inside its window, and a record whose time drifted towards the
/// neighbouring QSO still lands on its own once that neighbour has been
/// claimed by its exact match. Remaining ties go to the earlier QSO, then
/// the lower id, so the outcome never depends on load order.
pub fn match_records(records: &[ServiceQso], candidates: &[SyncCandidate]) -> Vec<Option<usize>> {
    let mut by_key: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, c) in candidates.iter().enumerate() {
        if let Some(band) = &c.band {
            by_key
                .entry((c.call.to_ascii_uppercase(), band.to_ascii_uppercase()))
                .or_default()
                .push(i);
        }
    }

    // (seconds apart, mode rank, record, candidate)
    let mut pairs: Vec<(i64, u8, usize, usize)> = Vec::new();
    for (ri, r) in records.iter().enumerate() {
        let Some(band) = &r.band else { continue };
        let key = (r.call.to_ascii_uppercase(), band.to_ascii_uppercase());
        let Some(cands) = by_key.get(&key) else {
            continue;
        };
        for &ci in cands {
            let c = &candidates[ci];
            if let Some(station) = &r.station_callsign {
                let same = c
                    .station_callsign
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case(station));
                if !same {
                    continue;
                }
            }
            let apart = (r.start - c.qso_begin).num_seconds().abs();
            if apart <= MATCH_TOLERANCE_SECS {
                pairs.push((apart, mode_rank(r, c), ri, ci));
            }
        }
    }
    pairs.sort_by(|a, b| {
        let (ca, cb) = (&candidates[a.3], &candidates[b.3]);
        (a.0, a.1)
            .cmp(&(b.0, b.1))
            .then(ca.qso_begin.cmp(&cb.qso_begin))
            .then(ca.qso_id.as_uuid().cmp(&cb.qso_id.as_uuid()))
            .then(a.2.cmp(&b.2))
    });

    let mut matched = vec![None; records.len()];
    let mut taken = vec![false; candidates.len()];
    for (_, _, ri, ci) in pairs {
        if matched[ri].is_none() && !taken[ci] {
            matched[ri] = Some(ci);
            taken[ci] = true;
        }
    }
    matched
}

/// The batch that brings the log in line with a report, given what
/// `match_records` paired up, and the credit and service provenance rows
/// already on the matched QSOs.
///
/// Only what changes is written: a QSO already verified (and confirmed,
/// if the record says so) gets no row, so re-reading a record — as every
/// incremental report does with the one on its boundary — is a no-op.
/// Nothing is downgraded: a record that isn't a confirmation leaves a
/// confirmed QSO confirmed, and granted credit is never lowered to
/// submitted.
pub fn plan_sync(
    service: &str,
    records: &[ServiceQso],
    candidates: &[SyncCandidate],
    matches: &[Option<usize>],
    existing_credits: &HashMap<QsoId, Vec<AwardCredit>>,
    existing_provenance: &HashMap<QsoId, Vec<FieldProvenance>>,
    watermark: Option<SyncWatermark>,
    now: DateTime<Utc>,
) -> (SyncBatch, SyncOutcome) {
    let mut batch = SyncBatch {
        service: service.to_string(),
        watermark,
        ..Default::default()
    };
    let mut outcome = SyncOutcome::default();

    for (ri, (r, m)) in records.iter().zip(matches).enumerate() {
        let Some(ci) = *m else {
            outcome.not_in_log.push(ri);
            continue;
        };
        let c = &candidates[ci];
        let confirm = r.confirmed && !c.confirmed;
        if !c.upload_verified || confirm {
            batch.states.push(SyncStateChange {
                qso_id: c.qso_id,
                uploaded_at: r.uploaded_at.unwrap_or(now),
                confirmed_at: confirm.then(|| r.confirmed_at.unwrap_or(now)),
            });
            outcome.newly_verified += usize::from(!c.upload_verified);
            outcome.newly_confirmed += usize::from(confirm);
        }
        if r.confirmed {
            let existing = existing_credits
                .get(&c.qso_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            plan_credits(r, c, existing, &mut batch, &mut outcome);
        }
        // Only a record that carries detail says anything about it: the
        // uploads report also lists confirmed QSOs, without detail (it
        // isn't asked for), and must not read as LoTW withdrawing it.
        if r.confirmed && !r.detail.is_empty() {
            let rows = existing_provenance
                .get(&c.qso_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let detail = plan_detail(service, c.qso_id, &c.detail, &r.detail, rows);
            outcome.qsos_differing += usize::from(detail.new_difference);
            if let Some(fill) = detail.fill {
                outcome.qsos_filled += 1;
                batch.detail_fills.push(fill);
            }
            batch.provenance.extend(detail.provenance);
        }
    }
    (batch, outcome)
}

fn same_credit(a: &AwardCredit, b: &AwardCredit) -> bool {
    a.award == b.award && a.medium == b.medium
}

/// Credit identity is award + medium, whichever source recorded it, so a
/// credit an ADIF import already brought in isn't added a second time.
fn plan_credits(
    r: &ServiceQso,
    c: &SyncCandidate,
    existing: &[AwardCredit],
    batch: &mut SyncBatch,
    outcome: &mut SyncOutcome,
) {
    // Granted first, so an award listed both ways counts as granted.
    let mut wanted: Vec<&AwardCredit> = Vec::new();
    for status in [CreditStatus::Granted, CreditStatus::Submitted] {
        for credit in r.credits.iter().filter(|x| x.status == status) {
            if !wanted.iter().any(|w| same_credit(w, credit)) {
                wanted.push(credit);
            }
        }
    }

    for credit in wanted {
        let mut have = existing.iter().filter(|e| same_credit(e, credit));
        let held_granted = have.clone().any(|e| e.status == CreditStatus::Granted);
        if held_granted {
            continue;
        }
        if have.next().is_some() {
            // Held as submitted.
            if credit.status == CreditStatus::Granted {
                batch.credit_upgrades.push(CreditUpgrade {
                    qso_id: c.qso_id,
                    award: credit.award.clone(),
                    medium: credit.medium.clone(),
                });
                outcome.credits_changed += 1;
            }
            continue;
        }
        let unit = credit_unit(
            &credit.award,
            r.dxcc_id.or(c.detail.dxcc_id),
            r.state.as_deref().or(c.detail.state.as_deref()),
            r.grid.as_deref().or(c.detail.grid.as_deref()),
        );
        batch.new_credits.push(ImportedCredit {
            qso_id: c.qso_id,
            credit: credit.clone(),
            call: c.call.clone(),
            qso_date: c.qso_begin.format("%Y-%m-%d").to_string(),
            band: c.band.clone(),
            mode: c.mode.clone(),
            unit,
        });
        outcome.credits_changed += 1;
    }
}

/// Match `records` against the log and store the result, together with
/// the report's new watermark (`watermark` under `watermark_key`), in one
/// transaction. On error nothing is written and the old watermark stays.
pub async fn apply_report(
    repo: &dyn QsoRepository,
    service: &str,
    records: &[ServiceQso],
    watermark_key: &str,
    watermark: Option<String>,
    now: DateTime<Utc>,
) -> RepoResult<SyncOutcome> {
    let mut calls: Vec<String> = records
        .iter()
        .map(|r| r.call.to_ascii_uppercase())
        .collect();
    calls.sort_unstable();
    calls.dedup();
    let candidates = if calls.is_empty() {
        Vec::new()
    } else {
        repo.load_sync_candidates(service, &calls).await?
    };
    let matches = match_records(records, &candidates);

    let matched_confirmations = |with: fn(&ServiceQso) -> bool| -> Vec<QsoId> {
        records
            .iter()
            .zip(&matches)
            .filter(|(r, _)| r.confirmed && with(r))
            .filter_map(|(_, m)| m.map(|ci| candidates[ci].qso_id))
            .collect()
    };
    let credit_qsos = matched_confirmations(|r| !r.credits.is_empty());
    let existing = if credit_qsos.is_empty() {
        HashMap::new()
    } else {
        repo.list_credits_for(&credit_qsos).await?
    };
    let detail_qsos = matched_confirmations(|r| !r.detail.is_empty());
    let provenance = if detail_qsos.is_empty() {
        HashMap::new()
    } else {
        repo.list_service_provenance_for(service, &detail_qsos).await?
    };

    let watermark = watermark.map(|value| SyncWatermark {
        service: service.to_string(),
        key: watermark_key.to_string(),
        value,
    });
    let (batch, outcome) = plan_sync(
        service,
        records,
        &candidates,
        &matches,
        &existing,
        &provenance,
        watermark,
        now,
    );
    if !batch.is_empty() {
        repo.apply_sync_batch(&batch).await?;
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::credit::parse_credit_list;
    use crate::qsl_detail::LocationDetail;
    use radio_core::FieldSource;

    fn t(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 8, h, m, s).unwrap()
    }

    fn rec(call: &str, band: &str, mode: &str, start: DateTime<Utc>) -> ServiceQso {
        ServiceQso {
            station_callsign: Some("K1ABC".into()),
            call: call.into(),
            start,
            band: Some(band.into()),
            mode: Some(mode.into()),
            mode_group: None,
            confirmed: false,
            confirmed_at: None,
            uploaded_at: None,
            credits: Vec::new(),
            dxcc_id: None,
            state: None,
            grid: None,
            detail: Vec::new(),
        }
    }

    fn cand(call: &str, band: &str, mode: Option<&str>, begin: DateTime<Utc>) -> SyncCandidate {
        SyncCandidate {
            qso_id: QsoId::new(),
            call: call.into(),
            station_callsign: Some("K1ABC".into()),
            qso_begin: begin,
            band: Some(band.into()),
            mode: mode.map(String::from),
            detail: LocationDetail {
                dxcc_id: Some(291),
                state: Some("CT".into()),
                grid: Some("FN31".into()),
                ..Default::default()
            },
            upload_verified: false,
            confirmed: false,
        }
    }

    #[test]
    fn same_station_twice_in_a_day_each_record_takes_its_own_qso() {
        // Three minutes apart: each local QSO is inside the other record's
        // window, so only "nearest wins" keeps them apart.
        let cands = vec![
            cand("W1AW", "20M", Some("CW"), t(10, 0, 0)),
            cand("W1AW", "20M", Some("CW"), t(10, 3, 0)),
        ];
        let recs = vec![
            rec("W1AW", "20M", "CW", t(10, 3, 0)),
            rec("W1AW", "20M", "CW", t(10, 0, 0)),
        ];
        assert_eq!(match_records(&recs, &cands), vec![Some(1), Some(0)]);

        // Hours apart, one record alone: it still finds the right one.
        let cands = vec![
            cand("W1AW", "20M", Some("CW"), t(9, 0, 0)),
            cand("W1AW", "20M", Some("CW"), t(17, 30, 0)),
        ];
        let recs = vec![rec("W1AW", "20M", "CW", t(17, 30, 0))];
        assert_eq!(match_records(&recs, &cands), vec![Some(1)]);
    }

    #[test]
    fn a_drifted_record_does_not_steal_its_neighbours_qso() {
        // The first record's time is nearer the second QSO, but that QSO
        // is the second record's exact match.
        let cands = vec![
            cand("W1AW", "20M", Some("CW"), t(10, 0, 0)),
            cand("W1AW", "20M", Some("CW"), t(10, 3, 0)),
        ];
        let recs = vec![
            rec("W1AW", "20M", "CW", t(10, 2, 0)),
            rec("W1AW", "20M", "CW", t(10, 3, 0)),
        ];
        assert_eq!(match_records(&recs, &cands), vec![Some(0), Some(1)]);
    }

    #[test]
    fn one_qso_is_matched_by_one_record_at_most() {
        let cands = vec![cand("W1AW", "20M", Some("CW"), t(10, 0, 0))];
        let recs = vec![
            rec("W1AW", "20M", "CW", t(10, 1, 0)),
            rec("W1AW", "20M", "CW", t(10, 0, 0)),
        ];
        assert_eq!(match_records(&recs, &cands), vec![None, Some(0)]);
    }

    #[test]
    fn tolerance_is_five_minutes_either_way() {
        let cands = vec![cand("W1AW", "20M", Some("CW"), t(10, 0, 0))];
        let at = |d: Duration| vec![rec("W1AW", "20M", "CW", t(10, 0, 0) + d)];
        assert_eq!(
            match_records(&at(Duration::seconds(300)), &cands),
            vec![Some(0)]
        );
        assert_eq!(
            match_records(&at(Duration::seconds(-300)), &cands),
            vec![Some(0)]
        );
        assert_eq!(
            match_records(&at(Duration::seconds(301)), &cands),
            vec![None]
        );
        // LoTW dropping the seconds is well inside.
        let cands = vec![cand("W1AW", "20M", Some("CW"), t(10, 0, 59))];
        assert_eq!(match_records(&at(Duration::zero()), &cands), vec![Some(0)]);
    }

    #[test]
    fn mode_breaks_ties_but_is_not_required() {
        // Mode differs entirely, or is missing locally: still a match.
        let cands = vec![cand("W1AW", "20M", Some("RTTY"), t(10, 0, 0))];
        assert_eq!(
            match_records(&[rec("W1AW", "20M", "FT8", t(10, 0, 0))], &cands),
            vec![Some(0)]
        );
        let cands = vec![cand("W1AW", "20M", None, t(10, 0, 0))];
        assert_eq!(
            match_records(&[rec("W1AW", "20M", "FT8", t(10, 0, 0))], &cands),
            vec![Some(0)]
        );

        // Equally near: exact mode wins, then same group.
        let cands = vec![
            cand("W1AW", "20M", Some("CW"), t(10, 0, 0)),
            cand("W1AW", "20M", Some("FT8"), t(10, 0, 0)),
            cand("W1AW", "20M", Some("SSB"), t(10, 0, 0)),
        ];
        assert_eq!(
            match_records(&[rec("W1AW", "20M", "SSB", t(10, 0, 0))], &cands),
            vec![Some(2)]
        );
        assert_eq!(
            match_records(&[rec("W1AW", "20M", "FT4", t(10, 0, 0))], &cands),
            vec![Some(1)]
        );
        let mut am = rec("W1AW", "20M", "AM", t(10, 0, 0));
        assert_eq!(
            match_records(std::slice::from_ref(&am), &cands),
            vec![Some(2)]
        );
        // The service's own group is used when it gives one.
        am.mode_group = Some("DATA".into());
        assert_eq!(match_records(&[am], &cands), vec![Some(1)]);
        // But time beats mode.
        let cands = vec![
            cand("W1AW", "20M", Some("SSB"), t(10, 1, 0)),
            cand("W1AW", "20M", Some("CW"), t(10, 0, 30)),
        ];
        assert_eq!(
            match_records(&[rec("W1AW", "20M", "SSB", t(10, 0, 0))], &cands),
            vec![Some(1)]
        );
    }

    #[test]
    fn call_band_and_station_must_agree() {
        let mut other_station = cand("W1AW", "20M", Some("CW"), t(10, 0, 0));
        other_station.station_callsign = Some("K1ABC/P".into());
        let mut no_station = cand("W1AW", "20M", Some("CW"), t(10, 0, 0));
        no_station.station_callsign = None;
        let cands = vec![
            cand("W1AW", "40M", Some("CW"), t(10, 0, 0)),
            other_station,
            no_station,
        ];
        let r = rec("w1aw", "20m", "CW", t(10, 0, 0));
        assert_eq!(match_records(std::slice::from_ref(&r), &cands), vec![None]);

        // A record without a station call matches on the rest.
        let mut anon = r;
        anon.station_callsign = None;
        assert!(match_records(&[anon], &cands)[0].is_some_and(|i| i > 0));

        // Case doesn't matter for any of them.
        let cands = vec![cand("W1AW", "20M", Some("CW"), t(10, 0, 0))];
        let mut lower = rec("w1aw", "20m", "cw", t(10, 0, 0));
        lower.station_callsign = Some("k1abc".into());
        assert_eq!(match_records(&[lower], &cands), vec![Some(0)]);
    }

    fn plan(
        records: &[ServiceQso],
        cands: &[SyncCandidate],
        existing: &HashMap<QsoId, Vec<AwardCredit>>,
    ) -> (SyncBatch, SyncOutcome) {
        let matches = match_records(records, cands);
        plan_sync(
            "lotw",
            records,
            cands,
            &matches,
            existing,
            &HashMap::new(),
            None,
            t(23, 0, 0),
        )
    }

    #[test]
    fn plan_verifies_confirms_and_never_downgrades() {
        let mut confirmed = cand("W1AW", "20M", Some("CW"), t(9, 0, 0));
        confirmed.upload_verified = true;
        confirmed.confirmed = true;
        let mut verified = cand("W1AW", "40M", Some("CW"), t(9, 0, 0));
        verified.upload_verified = true;
        let fresh = cand("W1AW", "80M", Some("CW"), t(9, 0, 0));
        let cands = vec![confirmed.clone(), verified.clone(), fresh.clone()];

        // An unconfirmed record for a confirmed QSO changes nothing.
        let r = rec("W1AW", "20M", "CW", t(9, 0, 0));
        let (batch, out) = plan(&[r], &cands, &HashMap::new());
        assert!(batch.is_empty());
        assert_eq!(out, SyncOutcome::default());

        // Verified + now confirmed: one change, a new confirmation.
        let mut r = rec("W1AW", "40M", "CW", t(9, 0, 0));
        r.confirmed = true;
        r.confirmed_at = Some(t(12, 0, 0));
        // Fresh + unconfirmed record: verified only, at the receive time.
        let mut f = rec("W1AW", "80M", "CW", t(9, 0, 0));
        f.uploaded_at = Some(t(9, 30, 0));
        // Not in the log.
        let x = rec("K1XYZ", "80M", "CW", t(9, 0, 0));
        let (batch, out) = plan(&[r, f, x], &cands, &HashMap::new());
        assert_eq!(
            batch.states,
            vec![
                SyncStateChange {
                    qso_id: verified.qso_id,
                    uploaded_at: t(23, 0, 0),
                    confirmed_at: Some(t(12, 0, 0)),
                },
                SyncStateChange {
                    qso_id: fresh.qso_id,
                    uploaded_at: t(9, 30, 0),
                    confirmed_at: None,
                },
            ]
        );
        assert_eq!(out.newly_verified, 1);
        assert_eq!(out.newly_confirmed, 1);
        assert_eq!(out.not_in_log, vec![2]);

        // No confirmation date at all: now.
        let mut r = rec("W1AW", "80M", "CW", t(9, 0, 0));
        r.confirmed = true;
        let (batch, out) = plan(&[r], &cands, &HashMap::new());
        assert_eq!(batch.states[0].confirmed_at, Some(t(23, 0, 0)));
        assert_eq!((out.newly_verified, out.newly_confirmed), (1, 1));
    }

    #[test]
    fn plan_adds_credit_once_and_upgrades_submitted_to_granted() {
        let c = cand("W1AW", "20M", Some("CW"), t(9, 0, 0));
        let cands = vec![c.clone()];
        let mut r = rec("W1AW", "20M", "CW", t(9, 0, 0));
        r.confirmed = true;
        r.dxcc_id = Some(1);
        r.credits = parse_credit_list("DXCC:LOTW,DXCC_BAND:LOTW", CreditStatus::Granted);
        r.credits.extend(parse_credit_list(
            "WAS:LOTW,DXCC:LOTW",
            CreditStatus::Submitted,
        ));

        let (batch, out) = plan(std::slice::from_ref(&r), &cands, &HashMap::new());
        let got: Vec<(&str, CreditStatus, &str)> = batch
            .new_credits
            .iter()
            .map(|n| (n.credit.award.as_str(), n.credit.status, n.unit.as_str()))
            .collect();
        // DXCC is listed both ways: granted wins. The unit is LoTW's
        // entity, falling back to the local state for WAS.
        assert_eq!(
            got,
            vec![
                ("DXCC", CreditStatus::Granted, "1"),
                ("DXCC_BAND", CreditStatus::Granted, "1"),
                ("WAS", CreditStatus::Submitted, "CT"),
            ]
        );
        assert_eq!(batch.new_credits[0].qso_date, "2026-05-08");
        assert_eq!(out.credits_changed, 3);

        // Already held: granted stays, submitted DXCC_BAND is raised, a
        // submitted WAS stays submitted, nothing is added twice.
        let mut held = parse_credit_list("DXCC:LOTW,", CreditStatus::Granted);
        held.extend(parse_credit_list(
            "DXCC_BAND:LOTW,WAS:LOTW",
            CreditStatus::Submitted,
        ));
        let existing = HashMap::from([(c.qso_id, held)]);
        let (batch, out) = plan(std::slice::from_ref(&r), &cands, &existing);
        assert!(batch.new_credits.is_empty());
        assert_eq!(
            batch.credit_upgrades,
            vec![CreditUpgrade {
                qso_id: c.qso_id,
                award: "DXCC_BAND".into(),
                medium: Some("LOTW".into()),
            }]
        );
        assert_eq!(out.credits_changed, 1);

        // A granted credit is never lowered by a record saying submitted.
        let mut sub = r.clone();
        sub.credits = parse_credit_list("DXCC:LOTW", CreditStatus::Submitted);
        let held = parse_credit_list("DXCC:LOTW", CreditStatus::Granted);
        let (batch, _) = plan(&[sub], &cands, &HashMap::from([(c.qso_id, held)]));
        assert!(batch.new_credits.is_empty() && batch.credit_upgrades.is_empty());

        // Credit on a record that isn't a confirmation is ignored.
        let mut unconfirmed = r;
        unconfirmed.confirmed = false;
        let (batch, _) = plan(&[unconfirmed], &cands, &HashMap::new());
        assert!(batch.new_credits.is_empty());
    }

    #[test]
    fn plan_fills_detail_from_confirmations_only() {
        let c = cand("W1AW", "20M", Some("CW"), t(9, 0, 0));
        let other = cand("W1AW", "40M", Some("CW"), t(9, 0, 0));
        let cands = vec![c.clone(), other.clone()];
        let mut r = rec("W1AW", "20M", "CW", t(9, 0, 0));
        r.confirmed = true;
        r.detail = vec![
            (DetailField::State, "MA".into()),          // CT locally: differs
            (DetailField::Cnty, "CT,Hartford".into()),  // blank: filled
            (DetailField::Gridsquare, "FN31pr".into()), // refines FN31
        ];
        let (batch, out) = plan(std::slice::from_ref(&r), &cands, &HashMap::new());
        assert_eq!(batch.detail_fills.len(), 1);
        assert_eq!(batch.detail_fills[0].qso_id, c.qso_id);
        assert_eq!(batch.detail_fills[0].values.county.as_deref(), Some("CT,Hartford"));
        assert_eq!(batch.detail_fills[0].values.grid.as_deref(), Some("FN31pr"));
        assert_eq!(batch.detail_fills[0].values.state, None);
        let rows: Vec<(&str, FieldSource)> = batch
            .provenance
            .iter()
            .map(|w| (w.field_name.as_str(), w.row.clone().unwrap().0))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("GRIDSQUARE", FieldSource::ServiceSync("lotw".into())),
                ("STATE", FieldSource::ServiceDiffers("lotw".into())),
                ("CNTY", FieldSource::ServiceSync("lotw".into())),
            ]
        );
        assert_eq!((out.qsos_filled, out.qsos_differing), (1, 1));
        // The unmatched QSO is untouched.
        assert!(batch.provenance.iter().all(|w| w.qso_id == c.qso_id));

        // The same detail on a record that isn't a confirmation: ignored.
        let mut unconfirmed = r.clone();
        unconfirmed.confirmed = false;
        let (batch, out) = plan(&[unconfirmed], &cands, &HashMap::new());
        assert!(batch.detail_fills.is_empty() && batch.provenance.is_empty());
        assert_eq!((out.qsos_filled, out.qsos_differing), (0, 0));

        // A confirmation without detail (the uploads report's view of a
        // confirmed QSO) leaves recorded differences alone; one with
        // detail that no longer includes the field drops it.
        let rows = HashMap::from([(
            c.qso_id,
            vec![FieldProvenance {
                field_name: "STATE".into(),
                source: FieldSource::ServiceDiffers("lotw".into()),
                source_detail: Some("MA".into()),
                created_at: t(9, 0, 0),
            }],
        )]);
        let plan_with = |r: &ServiceQso| {
            plan_sync(
                "lotw",
                std::slice::from_ref(r),
                &cands,
                &match_records(std::slice::from_ref(r), &cands),
                &HashMap::new(),
                &rows,
                None,
                t(23, 0, 0),
            )
            .0
        };
        let mut bare = r.clone();
        bare.detail.clear();
        assert!(plan_with(&bare).provenance.is_empty());
        let mut without_state = r;
        without_state.detail.retain(|(f, _)| *f != DetailField::State);
        let gone = plan_with(&without_state);
        assert!(gone.provenance.iter().any(|w| w.field_name == "STATE" && w.row.is_none()));
    }
}
