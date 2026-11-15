//! Award credit carried on a QSO — ADIF `CREDIT_GRANTED` and
//! `CREDIT_SUBMITTED`.
//!
//! Both fields are an ADIF CreditList: comma-separated entries, each an
//! award name optionally followed by `:` and the QSL media it was
//! credited through (`DXCC:LOTW`, `WAS:CARD&LOTW`, or just `WAS`).
//! DXKeeper writes them with a space after some commas and not others,
//! so whitespace around entries is not significant.

/// Whether the award sponsor has granted the credit or it has only been
/// submitted for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreditStatus {
    Granted,
    Submitted,
}

impl CreditStatus {
    /// The `award_credit_links.link_status` value, also used in
    /// `award_credit_imports.raw_json`.
    pub fn as_storage_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Submitted => "submitted",
        }
    }

    pub fn parse_storage(s: &str) -> Option<Self> {
        match s {
            "granted" => Some(Self::Granted),
            "submitted" => Some(Self::Submitted),
            _ => None,
        }
    }

    /// The ADIF field this status is read from and written back to.
    pub fn adif_field(self) -> &'static str {
        match self {
            Self::Granted => "CREDIT_GRANTED",
            Self::Submitted => "CREDIT_SUBMITTED",
        }
    }
}

/// One award a QSO counts towards, e.g. DXCC via LoTW, granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwardCredit {
    /// ADIF award name, uppercased (`DXCC`, `DXCC_BAND`, `WAS`, …).
    pub award: String,
    /// QSL media the credit came through (`LOTW`, `CARD`, `EQSL`, or a
    /// `&`-joined combination), as written; `None` when the entry named
    /// the award alone.
    pub medium: Option<String>,
    pub status: CreditStatus,
}

/// Parse an ADIF CreditList. Empty entries (a trailing comma, doubled
/// commas) are skipped rather than failing the whole list.
pub fn parse_credit_list(value: &str, status: CreditStatus) -> Vec<AwardCredit> {
    value
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            let (award, medium) = match entry.split_once(':') {
                Some((a, m)) => (a.trim(), Some(m.trim()).filter(|m| !m.is_empty())),
                None => (entry, None),
            };
            if award.is_empty() {
                return None;
            }
            Some(AwardCredit {
                award: award.to_ascii_uppercase(),
                medium: medium.map(str::to_ascii_uppercase),
                status,
            })
        })
        .collect()
}

/// Inverse of `parse_credit_list` for the credits with `status`; `None`
/// when there are none, so the caller can skip the field.
pub fn format_credit_list(credits: &[AwardCredit], status: CreditStatus) -> Option<String> {
    let parts: Vec<String> = credits
        .iter()
        .filter(|c| c.status == status)
        .map(|c| match &c.medium {
            Some(m) => format!("{}:{m}", c.award),
            None => c.award.clone(),
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credit(award: &str, medium: Option<&str>) -> AwardCredit {
        AwardCredit {
            award: award.into(),
            medium: medium.map(String::from),
            status: CreditStatus::Granted,
        }
    }

    #[test]
    fn parses_dxkeeper_spacing_and_bare_awards() {
        let got = parse_credit_list(
            "DXCC:LOTW, DXCC_BAND:LOTW, DXCC_MODE:LOTW,WAS",
            CreditStatus::Granted,
        );
        assert_eq!(
            got,
            vec![
                credit("DXCC", Some("LOTW")),
                credit("DXCC_BAND", Some("LOTW")),
                credit("DXCC_MODE", Some("LOTW")),
                credit("WAS", None),
            ]
        );
    }

    #[test]
    fn skips_empty_entries_and_normalises_case() {
        let got = parse_credit_list(" vucc_band ,, iota:card& lotw ,", CreditStatus::Submitted);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].award, "VUCC_BAND");
        assert_eq!(got[0].medium, None);
        assert_eq!(got[1].award, "IOTA");
        assert_eq!(got[1].medium.as_deref(), Some("CARD& LOTW"));
        assert!(got.iter().all(|c| c.status == CreditStatus::Submitted));
        assert!(parse_credit_list("", CreditStatus::Granted).is_empty());
    }

    #[test]
    fn format_round_trips_per_status() {
        let mut credits = parse_credit_list("DXCC:LOTW,WAS", CreditStatus::Granted);
        credits.extend(parse_credit_list("IOTA:CARD", CreditStatus::Submitted));
        assert_eq!(
            format_credit_list(&credits, CreditStatus::Granted).as_deref(),
            Some("DXCC:LOTW,WAS")
        );
        assert_eq!(
            format_credit_list(&credits, CreditStatus::Submitted).as_deref(),
            Some("IOTA:CARD")
        );
        assert_eq!(format_credit_list(&[], CreditStatus::Granted), None);
    }
}
