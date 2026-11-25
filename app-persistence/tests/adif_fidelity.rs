//! Round-trips a real ADIF file through an empty logbook: import it,
//! export everything, and write the export next to a report, so the two
//! files can be compared field by field.
//!
//!   SLOGGER_ADIF=/path/log.adi SLOGGER_ADIF_OUT=/path/out.adi \
//!     [SLOGGER_CTY=/path/cty.dat] \
//!     cargo test -p app-persistence --test adif_fidelity -- --ignored --nocapture

use std::sync::Arc;

use app_persistence::{Database, SqliteQsoRepository, SqliteStationRepository};
use logbook_domain::{
    ExportOptions, LogbookService, QsoRepository, QsoSearch, StationRepository, parse_adif,
};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs SLOGGER_ADIF and SLOGGER_ADIF_OUT"]
async fn round_trip_a_real_adif_file() {
    let source = std::env::var("SLOGGER_ADIF").expect("SLOGGER_ADIF");
    let out = std::env::var("SLOGGER_ADIF_OUT").expect("SLOGGER_ADIF_OUT");
    let text = std::fs::read_to_string(&source).expect("read ADIF as UTF-8, as the app does");

    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path().join("slogger.sqlite")).await.unwrap();
    let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
    let stations: Arc<dyn StationRepository> = Arc::new(SqliteStationRepository::new(&db));
    let service = match std::env::var("SLOGGER_CTY") {
        Ok(cty) => LogbookService::with_resolver(
            repo,
            Arc::new(station_resolver::CtyDbResolver::from_path(std::path::Path::new(&cty)).unwrap()),
        ),
        Err(_) => LogbookService::new(repo),
    };

    let outcome = parse_adif(&text).unwrap();
    println!("parsed: {} commands, {} skipped by the parser", outcome.commands.len(), outcome.skipped.len());
    for s in outcome.skipped.iter().take(20) {
        println!("  parser skipped record {}: {}", s.index, s.reason);
    }
    let report = service.import_qsos(outcome.commands).await;
    println!(
        "import: created {} / duplicates {} / skipped {} / locations {} / errors {}",
        report.created,
        report.duplicates,
        report.skipped,
        report.locations_created,
        report.errors.len()
    );
    for e in report.errors.iter().take(20) {
        println!("  import error: {e}");
    }

    let locations = stations.list_locations().await.unwrap();
    let everything = QsoSearch { limit: Some(u32::MAX), ..QsoSearch::default() };
    let exported = service
        .export_adif_by_search(everything, &locations, &ExportOptions::default())
        .await
        .unwrap();
    std::fs::write(&out, exported).unwrap();
    println!("export written to {out}");
}
