//! Every example filter in docs/spot-filters/ is loaded the way slogger
//! loads a filter file and fed sample cluster lines through dxfeed's own
//! aggregator, set up as slogger sets it up (no entity or enrichment
//! resolver, default aggregator settings). docs/spot-filters.md says
//! what each example shows and hides; these are those claims.

use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::Utc;
use dxfeed::aggregator::core::{Aggregator, AggregatorConfig, IncomingObservation};
use dxfeed::callsign::is_likely_skimmer;
use dxfeed::domain::OriginatorKind;
use dxfeed::model::{DxEvent, SourceId, SpotEventKind};
use dxfeed::parser::spot::{ParsedLine, parse_line, parse_skimmer_comment};

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/spot-filters")
}

/// A spot line as a cluster node sends it.
fn line(spotter: &str, khz: &str, dx: &str, comment: &str) -> String {
    format!("DX de {:<10}{khz:>8}  {dx:<13}{comment:<30} 1234Z", format!("{spotter}:"))
}

/// The DX calls the Spots panel would show after these lines arrive,
/// each from cluster node `source` (0 is the first in config.toml).
fn shown(example: &str, lines: &[(usize, String)]) -> BTreeSet<String> {
    shown_from(&examples_dir().join(format!("{example}.json")), lines)
}

/// The same, for filter JSON given inline.
fn shown_by(json: &str, lines: &[(usize, String)]) -> BTreeSet<String> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filter.json");
    std::fs::write(&path, json).unwrap();
    shown_from(&path, lines)
}

fn shown_from(path: &std::path::Path, lines: &[(usize, String)]) -> BTreeSet<String> {
    let example = path.display();
    assert_eq!(spot_feed::unknown_filter_keys(path), Vec::<String>::new(), "{example}");
    let filter = spot_feed::load_filter_file(path)
        .unwrap_or_else(|e| panic!("{example}: {e}"))
        .validate_and_compile()
        .unwrap_or_else(|e| panic!("{example}: {e:?}"));
    let mut aggregator = Aggregator::new(filter, None, AggregatorConfig::default(), None, None);

    let mut calls = BTreeSet::new();
    for (source, text) in lines {
        let ParsedLine::Spot(parsed) = parse_line(text) else {
            panic!("not a spot line: {text}");
        };
        let originator_kind = if is_likely_skimmer(&parsed.spotter_call) {
            OriginatorKind::Skimmer
        } else {
            OriginatorKind::Human
        };
        let skimmer_fields = parsed.comment.as_deref().map(parse_skimmer_comment);
        let events = aggregator.process_observation(IncomingObservation {
            parsed,
            source_id: SourceId(format!("cluster-{source}")),
            originator_kind,
            received_at: Utc::now(),
            skimmer_fields,
        });
        for event in events {
            if let DxEvent::Spot(spot) = event {
                match spot.kind {
                    SpotEventKind::New | SpotEventKind::Update => {
                        calls.insert(spot.spot.dx_call.clone());
                    }
                    SpotEventKind::Withdraw => {
                        calls.remove(&spot.spot.dx_call);
                    }
                }
            }
        }
    }
    calls
}

fn set(calls: &[&str]) -> BTreeSet<String> {
    calls.iter().map(|c| c.to_string()).collect()
}

/// One of each kind of spot, on node 0. The DX call says what it is.
fn mixed() -> Vec<(usize, String)> {
    [
        line("W3LPL", "14025.0", "CW20", "599 up 1"),
        line("W3LPL", "7010.0", "CW40", ""),
        line("W3LPL", "10105.0", "CW30", ""),
        line("W3LPL", "3520.0", "CW80", ""),
        line("W3LPL", "14200.0", "SSB20", "5/9"),
        line("W3LPL", "7180.0", "SSB40", ""),
        line("W3LPL", "14074.0", "FT8ON20", "FT8 -10"),
        line("W3LPL", "14105.0", "DIG20", "RTTY"),
        line("W3LPL", "50313.0", "FT8ON6", ""),
        line("K3LR-#", "21030.0", "SKIMCQ", "CW 18 dB 25 WPM CQ"),
        line("K3LR-#", "21032.0", "SKIMWEAK", "CW 4 dB 25 WPM CQ"),
        line("K3LR-#", "21034.0", "SKIMNOCQ", "CW 20 dB 25 WPM"),
        line("K3LR-#", "21036.0", "SKIMFAST", "CW 20 dB 42 WPM CQ"),
        line("K3LR-#", "14076.0", "SKIMFT8", "FT8 -5 dB CQ"),
    ]
    .into_iter()
    .map(|l| (0, l))
    .collect()
}

const HUMAN: [&str; 9] =
    ["CW20", "CW40", "CW30", "CW80", "SSB20", "SSB40", "FT8ON20", "DIG20", "FT8ON6"];
const SKIMMER: [&str; 5] = ["SKIMCQ", "SKIMWEAK", "SKIMNOCQ", "SKIMFAST", "SKIMFT8"];

#[test]
fn every_example_file_is_covered_here() {
    let mut found: Vec<String> = std::fs::read_dir(examples_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            "01-cw-only",
            "02-cw-on-chosen-bands",
            "03-no-digital",
            "04-cw-without-ft8-windows",
            "05-human-spots-only",
            "06-skimmer-cq-strong",
            "07-two-spotters",
            "08-hide-calls-and-spotters",
            "09-watch-list",
            "10-comments",
            "11-frequency-windows",
            "12-one-cluster-only",
            "13-contest-weekend",
            "14-dx-evening",
        ]
    );
}

/// What the guide says about the parts of a filter file that slogger
/// cannot act on, because it gives dxfeed no country or callbook data.
#[test]
fn rules_slogger_cannot_evaluate() {
    let everything: BTreeSet<String> = set(&HUMAN).union(&set(&SKIMMER)).cloned().collect();
    assert_eq!(shown_by("{}", &mixed()), everything);
    // An allow list on something unknown hides every spot...
    for hides_all in [
        r#"{"geo": {"dx": {"continent_allow": ["EU"]}}}"#,
        r#"{"geo": {"spotter": {"cq_zone_allow": [5]}}}"#,
        r#"{"enrichment": {"lotw": "RequireTrue"}}"#,
    ] {
        assert_eq!(shown_by(hides_all, &mixed()), set(&[]), "{hides_all}");
    }
    // ...and a deny list on it hides nothing.
    for no_effect in [
        r#"{"geo": {"dx": {"continent_deny": ["NA"]}}}"#,
        r#"{"rf": {"mode_policy": "TrustUpstream"}}"#,
        r#"{"dx": {"normalization": {"portable_policy": "StripAllAfterSlash"}}}"#,
    ] {
        assert_eq!(shown_by(no_effect, &mixed()), everything, "{no_effect}");
    }
}

#[test]
fn cw_only() {
    // A human spot has no mode of its own: it is CW, SSB or DIG by where
    // it is in the band. 14074 falls in the CW part of that table, so a
    // human spot of an FT8 station there counts as CW.
    assert_eq!(
        shown("01-cw-only", &mixed()),
        set(&["CW20", "CW40", "CW30", "CW80", "FT8ON20", "SKIMCQ", "SKIMWEAK", "SKIMNOCQ", "SKIMFAST"])
    );
}

#[test]
fn cw_on_chosen_bands() {
    assert_eq!(
        shown("02-cw-on-chosen-bands", &mixed()),
        set(&["CW20", "CW40", "FT8ON20", "SKIMCQ", "SKIMWEAK", "SKIMNOCQ", "SKIMFAST"])
    );
}

#[test]
fn no_digital() {
    assert_eq!(
        shown("03-no-digital", &mixed()),
        set(&["CW20", "CW40", "CW30", "CW80", "SSB20", "SSB40", "SKIMCQ", "SKIMWEAK", "SKIMNOCQ", "SKIMFAST"])
    );
}

#[test]
fn cw_without_the_ft8_windows() {
    assert_eq!(
        shown("04-cw-without-ft8-windows", &mixed()),
        set(&["CW20", "CW40", "CW30", "CW80", "SKIMCQ", "SKIMWEAK", "SKIMNOCQ", "SKIMFAST"])
    );
}

#[test]
fn human_spots_only() {
    assert_eq!(shown("05-human-spots-only", &mixed()), set(&HUMAN));
}

#[test]
fn skimmer_cq_strong() {
    assert_eq!(shown("06-skimmer-cq-strong", &mixed()), set(&["SKIMCQ"]));
}

#[test]
fn two_spotters() {
    let mut lines = mixed();
    assert_eq!(shown("07-two-spotters", &lines), set(&[]));
    // The same spotter again does not count; a second one does.
    lines.push((0, line("W3LPL", "14025.0", "CW20", "still there")));
    assert_eq!(shown("07-two-spotters", &lines), set(&[]));
    lines.push((0, line("N2IC", "14025.1", "CW20", "")));
    lines.push((0, line("W3LPL-#", "21030.0", "SKIMCQ", "CW 9 dB 25 WPM CQ")));
    assert_eq!(shown("07-two-spotters", &lines), set(&["CW20", "SKIMCQ"]));
}

#[test]
fn hide_calls_and_spotters() {
    let lines: Vec<(usize, String)> = [
        line("W3LPL", "14025.0", "W1AW", ""),
        line("W3LPL", "7010.0", "W1AW/4", ""),
        line("W3LPL", "7012.0", "W1AWX", ""),
        line("W3LPL", "28200.0", "4U1UN/B", "beacon"),
        line("W3LPL", "14030.0", "JA1ABC", ""),
        line("N0CALL", "14040.0", "BYNOCALL", ""),
        line("EA5WU-#", "14050.0", "BYSKIM", "CW 20 dB 25 WPM CQ"),
        line("K3LR-#", "14060.0", "BYOTHER", "CW 20 dB 25 WPM CQ"),
    ]
    .into_iter()
    .map(|l| (0, l))
    .collect();
    assert_eq!(shown("08-hide-calls-and-spotters", &lines), set(&["W1AWX", "JA1ABC", "BYOTHER"]));
}

#[test]
fn watch_list() {
    let lines: Vec<(usize, String)> = [
        line("W3LPL", "14025.0", "P5DX", ""),
        line("W3LPL", "14026.0", "3Y0J", ""),
        line("W3LPL", "14027.0", "VK0AI", ""),
        line("W3LPL", "14028.0", "KH7XX", ""),
        line("W3LPL", "14029.0", "KL7AA", ""),
        line("W3LPL", "14030.0", "VK2ABC", ""),
        line("W3LPL", "14031.0", "W1KH7", ""),
        line("K3LR-#", "14032.0", "FT5XT", "CW 20 dB 25 WPM CQ"),
    ]
    .into_iter()
    .map(|l| (0, l))
    .collect();
    assert_eq!(
        shown("09-watch-list", &lines),
        set(&["P5DX", "3Y0J", "VK0AI", "KH7XX", "KL7AA", "FT5XT"])
    );
}

#[test]
fn comments() {
    let lines: Vec<(usize, String)> = [
        line("W3LPL", "14025.0", "A1", "PIRATE!!"),
        line("W3LPL", "14026.0", "A2", "just a Test"),
        line("W3LPL", "14027.0", "A3", "FT8 -12"),
        line("W3LPL", "14028.0", "A4", "ft4"),
        line("W3LPL", "14029.0", "A5", "FT891 great rig"),
        line("W3LPL", "14030.0", "A6", "loud in FN31"),
        line("W3LPL", "14031.0", "A7", ""),
    ]
    .into_iter()
    .map(|l| (0, l))
    .collect();
    assert_eq!(shown("10-comments", &lines), set(&["A5", "A6", "A7"]));
}

#[test]
fn frequency_windows() {
    let lines: Vec<(usize, String)> = [
        line("W3LPL", "14005.0", "IN20", ""),
        line("W3LPL", "14030.0", "EDGE20", ""),
        line("W3LPL", "14030.1", "OUT20", ""),
        line("W3LPL", "7025.0", "IN40", ""),
        line("W3LPL", "10105.0", "OUT30", ""),
        line("K3LR-#", "21010.0", "IN15", "CW 20 dB 25 WPM CQ"),
    ]
    .into_iter()
    .map(|l| (0, l))
    .collect();
    assert_eq!(shown("11-frequency-windows", &lines), set(&["IN20", "EDGE20", "IN40", "IN15"]));
}

#[test]
fn one_cluster_only() {
    let lines = vec![
        (0, line("W3LPL", "14025.0", "FIRST", "")),
        (1, line("W3LPL", "14035.0", "SECOND", "")),
    ];
    assert_eq!(shown("12-one-cluster-only", &lines), set(&["FIRST"]));
}

#[test]
fn contest_weekend() {
    let mut lines = mixed();
    lines.push((0, line("W3LPL", "14040.0", "W1AW", "")));
    // Human CW spots on contest bands, and skimmer spots that are a CQ
    // at 6 dB or better. No WARC bands, no FT8 windows, not W1AW.
    assert_eq!(
        shown("13-contest-weekend", &lines),
        set(&["CW20", "CW40", "CW80", "SKIMCQ", "SKIMFAST"])
    );
}

#[test]
fn dx_evening() {
    let mut lines = mixed();
    lines.push((0, line("W3LPL", "18075.0", "ONCE17", "")));
    lines.push((0, line("W3LPL", "18080.0", "BAD17", "")));
    lines.push((0, line("N2IC", "18080.0", "BAD17", "Pirate")));
    for spotter in ["N2IC", "K1TTT"] {
        lines.push((0, line(spotter, "14025.0", "CW20", "")));
        lines.push((0, line(spotter, "14200.0", "SSB20", "")));
        lines.push((0, line(spotter, "14074.0", "FT8ON20", "")));
        lines.push((0, line(spotter, "3520.0", "CW80", "")));
    }
    assert_eq!(shown("14-dx-evening", &lines), set(&["CW20", "SSB20"]));
}

/// Smaller statements the guide makes in passing.
#[test]
fn asides_in_the_guide() {
    let lines: Vec<(usize, String)> = [
        line("W3LPL", "14025.0", "PLAIN", "599"),
        line("W3LPL", "14026.0", "CALLING", "cq dx"),
        line("W3LPL", "14027.0", "SPLIT", "up 2"),
        line("W3LPL", "14028.0", "NOTE", ""),
        line("K3LR-#", "14029.0", "SKCQ", "CW 20 dB 25 WPM CQ"),
        line("K3LR-#", "14031.0", "SKNOCQ", "CW 20 dB 25 WPM"),
        line("W3LPL", "144050.0", "ON2M", ""),
    ]
    .into_iter()
    .map(|l| (0, l))
    .collect();
    // 2 m is outside the default frequency sanity range.
    assert_eq!(shown_by("{}", &lines), set(&["PLAIN", "CALLING", "SPLIT", "NOTE", "SKCQ", "SKNOCQ"]));
    assert!(shown_by(r#"{"freq_sanity_hz": [1800000, 148000000]}"#, &lines).contains("ON2M"));
    assert_eq!(shown_by(r#"{"content": {"cq_only": true}}"#, &lines), set(&["CALLING", "SKCQ"]));
    assert_eq!(
        shown_by(r#"{"content": {"suppress_split_qsy": true}}"#, &lines),
        set(&["PLAIN", "CALLING", "NOTE", "SKCQ", "SKNOCQ"])
    );
    // A spotter whose call ends in a dash and digits is a skimmer.
    let numbered = vec![(0, line("W3LPL-2", "14025.0", "BYNUMBERED", "CW 20 dB 25 WPM CQ"))];
    assert_eq!(shown("05-human-spots-only", &numbered), set(&[]));
    // A person and a skimmer are two spotters.
    let pair = vec![
        (0, line("W3LPL", "14025.0", "BOTH", "")),
        (0, line("K3LR-#", "14025.0", "BOTH", "CW 20 dB 25 WPM CQ")),
    ];
    assert_eq!(shown("07-two-spotters", &pair), set(&["BOTH"]));
    // A word that is not a band or mode name stops the file loading.
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, r#"{"rf": {"band_allow": ["20m"]}}"#).unwrap();
    let err = spot_feed::load_filter_file(&bad).unwrap_err().to_string();
    assert!(err.contains("line 1 column"), "{err}");
}
