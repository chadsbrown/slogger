//! Logbook backups: a snapshot of the database plus an ADIF export,
//! written to a folder under timestamped names, with old ones pruned.
//!
//! The snapshot restores slogger exactly (see docs/backup.md); the ADIF
//! file is the copy any other logger can read. When the last backup was
//! made is read from the file names in the folder, so nothing else has
//! to be remembered.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_config::BackupConfig;
use app_persistence::Database;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use logbook_domain::{ExportOptions, LogbookService, QsoSearch, StationRepository};

const PREFIX: &str = "slogger-";
const STAMP: &str = "%Y%m%d-%H%M%SZ";
const SNAPSHOT_EXT: &str = "sqlite";
const ADIF_EXT: &str = "adi";

/// Everything a backup needs, cheap to clone into a task.
#[derive(Clone)]
pub struct Backups {
    db: Database,
    service: Arc<LogbookService>,
    stations: Arc<dyn StationRepository>,
    pub directory: PathBuf,
    interval: Duration,
    keep: usize,
}

impl std::fmt::Debug for Backups {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backups").field("directory", &self.directory).finish_non_exhaustive()
    }
}

/// A backup that was just written.
#[derive(Debug, Clone)]
pub struct BackupDone {
    pub at: DateTime<Utc>,
    pub snapshot: PathBuf,
    pub qsos: usize,
}

impl Backups {
    /// `None` when `[backup] enabled = false`. `data_dir` is where the
    /// logbook database lives; the default folder is `backups` in it.
    pub fn from_config(
        config: &BackupConfig,
        data_dir: &Path,
        db: Database,
        service: Arc<LogbookService>,
        stations: Arc<dyn StationRepository>,
    ) -> Option<Self> {
        config.enabled.then(|| Self {
            db,
            service,
            stations,
            directory: config.directory.clone().unwrap_or_else(|| data_dir.join("backups")),
            interval: Duration::days(i64::from(config.interval_days())),
            keep: config.keep(),
        })
    }

    /// When the newest backup in the folder was made, if there is one.
    pub fn latest(&self) -> Option<DateTime<Utc>> {
        snapshots(&self.directory).into_iter().map(|(at, _)| at).max()
    }

    /// Whether the newest backup is old enough (or missing) that one
    /// should be made now.
    pub fn due(&self, now: DateTime<Utc>) -> bool {
        is_due(self.latest(), now, self.interval)
    }

    /// Writes a backup now, then deletes all but the newest `keep`.
    /// Both files are written under temporary names and renamed when
    /// complete, so a failed backup never leaves a file that looks good.
    pub async fn run(&self) -> Result<BackupDone, String> {
        let dir = &self.directory;
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let at = Utc::now();
        let stem = format!("{PREFIX}{}", at.format(STAMP));
        let snapshot = dir.join(format!("{stem}.{SNAPSHOT_EXT}"));
        let adif = dir.join(format!("{stem}.{ADIF_EXT}"));
        if snapshot.exists() {
            // Two backups within the same second: the first one stands.
            return Ok(BackupDone { at, snapshot, qsos: 0 });
        }

        let part = dir.join(format!("{stem}.{SNAPSHOT_EXT}.part"));
        let _ = tokio::fs::remove_file(&part).await;
        self.db
            .backup_to(&part)
            .await
            .map_err(|e| format!("cannot write {}: {e}", part.display()))?;

        let locations = self.stations.list_locations().await.map_err(|e| e.to_string())?;
        let everything = QsoSearch { limit: Some(u32::MAX), ..QsoSearch::default() };
        let qsos = self.service.count_matching(everything.clone()).await.map_err(|e| e.to_string())?;
        let text = self
            .service
            .export_adif_by_search(everything, &locations, &ExportOptions::default())
            .await
            .map_err(|e| e.to_string())?;
        let adif_part = dir.join(format!("{stem}.{ADIF_EXT}.part"));
        tokio::fs::write(&adif_part, text)
            .await
            .map_err(|e| format!("cannot write {}: {e}", adif_part.display()))?;

        // The snapshot is renamed last: it is the file that marks a
        // backup as existing, so it appears only when its ADIF is there.
        tokio::fs::rename(&adif_part, &adif).await.map_err(|e| e.to_string())?;
        tokio::fs::rename(&part, &snapshot).await.map_err(|e| e.to_string())?;

        prune(dir, self.keep);
        Ok(BackupDone { at, snapshot, qsos: qsos as usize })
    }
}

fn is_due(latest: Option<DateTime<Utc>>, now: DateTime<Utc>, interval: Duration) -> bool {
    latest.is_none_or(|at| now - at >= interval)
}

/// The time in a backup's file name, for files this module wrote.
fn stamp_of(path: &Path) -> Option<DateTime<Utc>> {
    let stem = path.file_stem()?.to_str()?.strip_prefix(PREFIX)?;
    Some(NaiveDateTime::parse_from_str(stem, STAMP).ok()?.and_utc())
}

/// Completed snapshots in a folder, with the time each was made.
fn snapshots(dir: &Path) -> Vec<(DateTime<Utc>, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == SNAPSHOT_EXT))
        .filter_map(|path| Some((stamp_of(&path)?, path)))
        .collect()
}

/// Deletes all but the newest `keep` backups. Only files named the way
/// this module names them are touched; anything else in the folder is
/// left alone.
fn prune(dir: &Path, keep: usize) {
    let mut all = snapshots(dir);
    all.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    for (_, snapshot) in all.into_iter().skip(keep) {
        for path in [snapshot.with_extension(ADIF_EXT), snapshot] {
            if let Err(e) = std::fs::remove_file(&path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %path.display(), error = %e, "could not delete old backup");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_persistence::{SqliteQsoRepository, SqliteStationRepository};
    use chrono::TimeZone;
    use logbook_domain::QsoRepository;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_backup_is_due_when_there_is_none_or_the_newest_is_old() {
        let now = Utc.with_ymd_and_hms(2026, 11, 24, 12, 0, 0).unwrap();
        let week = Duration::days(7);
        assert!(is_due(None, now, week));
        assert!(!is_due(Some(now - Duration::days(6)), now, week));
        assert!(is_due(Some(now - Duration::days(7)), now, week));
        // interval_days = 0: every start.
        assert!(is_due(Some(now), now, Duration::zero()));
    }

    #[test]
    fn only_finished_backups_count_and_only_ours_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for day in ["20261101", "20261108", "20261115"] {
            touch(d, &format!("slogger-{day}-120000Z.sqlite"));
            touch(d, &format!("slogger-{day}-120000Z.adi"));
        }
        touch(d, "slogger-20261122-120000Z.sqlite.part"); // interrupted
        touch(d, "slogger.sqlite"); // someone's own copy
        touch(d, "notes.txt");

        let newest = snapshots(d).into_iter().map(|(at, _)| at).max().unwrap();
        assert_eq!(newest, Utc.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap());

        prune(d, 2);
        assert_eq!(
            names(d),
            [
                "notes.txt",
                "slogger-20261108-120000Z.adi",
                "slogger-20261108-120000Z.sqlite",
                "slogger-20261115-120000Z.adi",
                "slogger-20261115-120000Z.sqlite",
                "slogger-20261122-120000Z.sqlite.part",
                "slogger.sqlite",
            ]
        );
    }

    #[tokio::test]
    async fn a_backup_writes_a_snapshot_and_an_adif_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("slogger.sqlite")).await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let stations: Arc<dyn StationRepository> = Arc::new(SqliteStationRepository::new(&db));
        let service = Arc::new(LogbookService::new(repo));
        let backups = Backups::from_config(
            &BackupConfig::default(),
            dir.path(),
            db,
            service,
            stations,
        )
        .unwrap();
        assert_eq!(backups.directory, dir.path().join("backups"));
        assert!(backups.latest().is_none());
        assert!(backups.due(Utc::now()));

        let done = backups.run().await.unwrap();
        assert!(done.snapshot.is_file());
        assert!(done.snapshot.with_extension("adi").is_file());
        assert!(!backups.due(Utc::now()));
        // To the second, as the file name records it.
        assert_eq!(backups.latest().unwrap().timestamp(), done.at.timestamp());
        // The snapshot is a database slogger can open.
        Database::open(&done.snapshot).await.unwrap();

        let off = BackupConfig { enabled: false, ..BackupConfig::default() };
        let db = Database::open_in_memory().await.unwrap();
        let repo: Arc<dyn QsoRepository> = Arc::new(SqliteQsoRepository::new(&db));
        let stations: Arc<dyn StationRepository> = Arc::new(SqliteStationRepository::new(&db));
        assert!(
            Backups::from_config(&off, dir.path(), db, Arc::new(LogbookService::new(repo)), stations)
                .is_none()
        );
    }
}
