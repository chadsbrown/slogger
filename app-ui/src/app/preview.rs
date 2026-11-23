//! Draws the app to PNG files without a window, so a theme can be judged
//! by eye. Not a test of anything: run it on purpose with
//!
//! ```text
//! cargo test -p app-ui render_theme_previews -- --ignored
//! ```
//!
//! `SLOGGER_PREVIEW_DIR` says where the images go (default
//! `target/theme-previews`), `SLOGGER_PREVIEW_THEMES` which themes to
//! draw as a comma-separated list of names (default: the slogger-designed
//! ones), and `SLOGGER_PREVIEW_VIEWS` which of `operating`, `qsl`,
//! `logbook` and `picker` (default: `operating`).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use app_persistence::{Database, SqliteQsoRepository};
use chrono::{Duration, Utc};
use condx_feed::{BandCondition, CondXSnapshot};
use iced::{Settings, Size};
use iced_test::Simulator;
use logbook_domain::{LogbookService, QsoSummary};
use radio_core::{Band, Callsign, Mode, QsoId};
use spot_feed::Spot;
use station_resolver::{Resolution, Resolver};

use super::message::DupeMatch;
use super::modals::Modal;
use super::state::App;
use super::theme::{ThemeSource, all_themes};
use super::types::RigEntry;
use super::views::ViewKind;

const WINDOW: Size = Size::new(1240.0, 800.0);

/// Knows a handful of prefixes, enough to give the sample spots an
/// entity and a needed-or-worked status.
#[derive(Debug)]
struct SampleResolver;

const ENTITIES: &[(&str, u16, &str, &str, u8)] = &[
    ("JA", 339, "Japan", "AS", 25),
    ("DL", 230, "Germany", "EU", 14),
    ("VK", 150, "Australia", "OC", 30),
    ("ZS", 462, "South Africa", "AF", 38),
    ("PY", 108, "Brazil", "SA", 11),
    ("G", 223, "England", "EU", 14),
    ("K", 291, "United States", "NA", 5),
    ("W", 291, "United States", "NA", 5),
];

impl Resolver for SampleResolver {
    fn resolve(&self, call: &Callsign) -> Option<Resolution> {
        let call = call.as_str();
        let &(prefix, id, country, continent, zone) =
            ENTITIES.iter().find(|(prefix, ..)| call.starts_with(prefix))?;
        Some(Resolution {
            dxcc_id: Some(id),
            dxcc_prefix: Some(prefix.into()),
            country: Some(country.into()),
            continent: Some(continent.into()),
            cq_zone: Some(zone),
            ..Resolution::empty()
        })
    }
}

/// An app in the middle of an evening's operating: spots coming in, a
/// call half entered that turns out to be a dupe, a rig on 20 m.
fn sample_app() -> App {
    let (mut app, _) = App::init();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let db = runtime.block_on(Database::open_in_memory()).expect("database");
    let repo = Arc::new(SqliteQsoRepository::new(&db));
    app.service = Some(Arc::new(LogbookService::new(repo.clone())));
    app.repo = Some(repo);
    app.resolver = Some(Arc::new(SampleResolver));

    let now = Utc::now();
    let spot = |call: &str, khz: f64, mode: &str, comment: &str, minutes: i64| Spot {
        call: call.into(),
        freq_hz: (khz * 1000.0) as u64,
        mode: Some(mode.into()),
        comment: (!comment.is_empty()).then(|| comment.into()),
        spotted_at: now - Duration::minutes(minutes),
        first_seen: now - Duration::minutes(minutes),
    };
    app.spots_active = true;
    app.spots = [
        spot("JA1NUT", 14025.0, "CW", "599 TU", 1),
        spot("ZS6CCY", 21074.0, "FT8", "-12 dB", 2),
        spot("DL3ABC", 7185.0, "SSB", "loud in NA", 3),
        spot("VK2XYZ", 28074.0, "FT8", "", 5),
        spot("PY2ZZ", 14195.0, "SSB", "CQ DX", 6),
        spot("G4ABC", 3525.5, "CW", "up 1", 9),
        spot("K5MM", 50313.0, "FT8", "EM10", 12),
        spot("JA7QVI", 18100.0, "FT8", "", 14),
    ]
    .into_iter()
    .collect();
    // POTA switched on: one activator the cluster also has (DL3ABC,
    // merged into its row) and two it doesn't.
    let park = |call: &str, khz: f64, mode: &str, reference: &str, name: &str, region: &str,
                comment: &str, minutes: i64| super::pota::Activation {
        spot: pota_feed::PotaSpot {
            call: call.into(),
            freq_hz: (khz * 1000.0) as u64,
            mode: Some(mode.into()),
            reference: reference.into(),
            park_name: Some(name.into()),
            location: Some(region.into()),
            comment: (!comment.is_empty()).then(|| comment.into()),
            spotted_at: now - Duration::minutes(minutes),
            qrt: false,
        },
        first_seen: now - Duration::minutes(minutes),
    };
    app.pota_on = true;
    app.pota = vec![
        park("DL3ABC", 7185.0, "SSB", "DE-0042", "Eifel National Park", "DE-NW", "", 3),
        park("WC1D", 18072.0, "CW", "US-8390", "Mystic Lakes State Park", "US-MA", "", 4),
        park("K4SWL", 14061.0, "CW", "US-6856", "Lake James State Park", "US-NC", "QRP 5w", 7),
    ];
    // Germany and the US are already worked on these bands.
    app.worked_by_band.insert(Band::M40, HashSet::from([230]));
    app.worked_by_band.insert(Band::M6, HashSet::from([291]));
    app.worked_by_band.insert(Band::M20, HashSet::from([291]));

    let qso = |call: &str, khz: i64, band: Band, mode: Mode, prefix: &str, cont: &str, zone: u8, min: i64| {
        QsoSummary {
            id: QsoId::new(),
            call: Callsign::parse(call).expect("callsign"),
            qso_begin: now - Duration::minutes(min),
            band: Some(band),
            mode: Some(mode),
            freq_hz: Some(khz * 1000),
            dxcc_id: None,
            dxcc_prefix: Some(prefix.into()),
            continent: Some(cont.into()),
            cq_zone: Some(zone),
        }
    };
    app.recent = vec![
        qso("W1AW", 14074, Band::M20, Mode::FT8, "K", "NA", 5, 4),
        qso("EA5XYZ", 14030, Band::M20, Mode::CW, "EA", "EU", 14, 11),
        qso("JA3ABC", 21260, Band::M15, Mode::SSB, "JA", "AS", 25, 25),
        qso("VE3KP", 7030, Band::M40, Mode::CW, "VE", "NA", 4, 48),
        qso("F5XYZ", 14200, Band::M20, Mode::SSB, "F", "EU", 14, 70),
        qso("ZL2AB", 28074, Band::M10, Mode::FT8, "ZL", "OC", 32, 95),
    ];
    app.editing_qso = Some(app.recent[1].id);

    app.call_input = "JA1NUT".into();
    app.band = Some(Band::M20);
    app.mode = Some(Mode::CW);
    app.freq_input = "14025.0".into();
    app.dupe_match = Some(DupeMatch {
        qso_begin: now - Duration::days(40),
    });
    app.rigs.push(RigEntry {
        label: "Main".into(),
        config: Default::default(),
        handle: None,
        snapshot: Some(rig_control::RigSnapshot {
            vendor: "icom".into(),
            model: "IC-7300".into(),
            freq_hz: Some(14_025_000),
            mode: Some("CW".into()),
            at: now,
        }),
        status: "connected".into(),
    });
    // SLOGGER_PREVIEW_RIGS=2 adds a second radio, which turns the rig
    // blocks into buttons with one marked active.
    if std::env::var("SLOGGER_PREVIEW_RIGS").is_ok_and(|n| n == "2") {
        app.rigs.push(RigEntry {
            label: "Second".into(),
            config: Default::default(),
            handle: None,
            snapshot: Some(rig_control::RigSnapshot {
                vendor: "yaesu".into(),
                model: "FT-710".into(),
                freq_hz: Some(7_030_000),
                mode: Some("CW".into()),
                at: now,
            }),
            status: "connected".into(),
        });
    }

    let cond = |band: &str, time: &str, rating: &str| BandCondition {
        band: band.into(),
        time: time.into(),
        rating: rating.into(),
    };
    app.condx_active = true;
    app.condx = Some(CondXSnapshot {
        updated: "22 Nov 2026 1800 GMT".into(),
        sfi: Some(142),
        sunspots: Some(96),
        a_index: Some(8),
        k_index: Some(2),
        xray: "B7.4".into(),
        geomag_field: "QUIET".into(),
        signal_noise: "S1-S2".into(),
        conditions: vec![
            cond("80m-40m", "day", "Poor"),
            cond("30m-20m", "day", "Good"),
            cond("17m-15m", "day", "Good"),
            cond("12m-10m", "day", "Fair"),
            cond("80m-40m", "night", "Good"),
            cond("30m-20m", "night", "Fair"),
            cond("17m-15m", "night", "Poor"),
            cond("12m-10m", "night", "Poor"),
        ],
    });
    app.status = Some("Logged EA5XYZ on 20M CW".into());
    // SLOGGER_PREVIEW_NO_CONDX=1 leaves the CondX panel closed; open, it
    // covers the right-hand end of the Spots table.
    let no_condx = std::env::var("SLOGGER_PREVIEW_NO_CONDX").is_ok();
    for kind in super::panels::PanelKind::OPERATING {
        let wanted = !(no_condx && *kind == super::panels::PanelKind::CondX);
        if app.panels.is_open(*kind) != wanted {
            app.panels.toggle(*kind);
        }
    }
    app
}

fn env_list(name: &str) -> Option<Vec<String>> {
    let value = std::env::var(name).ok()?;
    Some(value.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect())
}

fn file_stem(theme: &str, view: &str) -> String {
    let slug: String = theme
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    format!("{slug}--{view}")
}

#[test]
#[ignore = "writes images; run on purpose to look at themes"]
fn render_theme_previews() {
    let dir = std::env::var("SLOGGER_PREVIEW_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("../target/theme-previews"));
    let views = env_list("SLOGGER_PREVIEW_VIEWS").unwrap_or_else(|| vec!["operating".into()]);
    let wanted = env_list("SLOGGER_PREVIEW_THEMES");

    let mut app = sample_app();
    let themes: Vec<iced::Theme> = all_themes()
        .into_iter()
        .filter(|entry| match &wanted {
            Some(names) => names.iter().any(|n| n.eq_ignore_ascii_case(&entry.theme.to_string())),
            None => entry.source == ThemeSource::Slogger,
        })
        .map(|entry| entry.theme)
        .collect();
    assert!(!themes.is_empty(), "no theme matched SLOGGER_PREVIEW_THEMES");

    for theme in themes {
        app.theme = theme.clone();
        for view in &views {
            app.modal = None;
            app.current_view = match view.as_str() {
                "qsl" => ViewKind::Qsl,
                "logbook" => ViewKind::Logbook,
                _ => ViewKind::Operating,
            };
            if view == "picker" {
                app.modal = Some(Modal::ThemePicker);
            }
            if view == "editor" {
                use super::editor::EditorMessage;
                let _ = app.edit_theme(true);
                let _ = app.update_theme_editor(EditorMessage::Toggle(super::theme::Role::Pane));
            }
            let drawn = app.theme.clone();
            let mut ui = Simulator::with_size(Settings::default(), WINDOW, app.view());
            let shot = ui.snapshot(&drawn).expect("snapshot");
            drop(ui);
            if view == "editor" {
                let _ = app.update_theme_editor(super::editor::EditorMessage::Cancel);
            }
            let path = dir.join(file_stem(&theme.to_string(), view));
            // `matches_image` only writes when the file is missing.
            for old in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let name = old.file_name().to_string_lossy().into_owned();
                if name.starts_with(&format!("{}-", file_stem(&theme.to_string(), view))) {
                    let _ = std::fs::remove_file(old.path());
                }
            }
            shot.matches_image(&path).expect("write image");
        }
    }
    println!("theme previews written to {}", dir.display());
}
