use std::path::Path;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("connection error: {0}")]
    Connection(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

#[derive(Debug, Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self, DatabaseError> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;

        let db = Self { pool };
        db.migrate().await?;
        Ok(db)
    }

    pub async fn open_in_memory() -> Result<Self, DatabaseError> {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;

        let db = Self { pool };
        db.migrate().await?;
        Ok(db)
    }

    async fn migrate(&self) -> Result<(), DatabaseError> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Writes a complete, consistent copy of the database to `dest`,
    /// which must not exist yet. Safe while the database is in use:
    /// SQLite takes the copy from a single read snapshot, so it never
    /// holds half of a write. The copy is one self-contained file (no
    /// `-wal` or `-shm` beside it).
    pub async fn backup_to(&self, dest: &Path) -> Result<(), DatabaseError> {
        sqlx::query("VACUUM INTO ?")
            .bind(dest.to_string_lossy().into_owned())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn count(db: &Database) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM station_locations")
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    async fn add_location(db: &Database, name: &str) {
        sqlx::query("INSERT INTO station_locations (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(name)
            .bind(name)
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00Z")
            .execute(db.pool())
            .await
            .unwrap();
    }

    /// The restore the docs describe: quit, set the live database and
    /// its `-wal` / `-shm` files aside, copy a snapshot into place.
    #[tokio::test]
    async fn a_snapshot_restores_the_log_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("slogger.sqlite");
        let snapshot = dir.path().join("snapshot.sqlite");

        let db = Database::open(&live).await.unwrap();
        add_location(&db, "before").await;
        db.backup_to(&snapshot).await.unwrap();
        add_location(&db, "after").await;
        assert_eq!(count(&db).await, 2);
        // A second snapshot never overwrites the first.
        assert!(db.backup_to(&snapshot).await.is_err());
        db.pool().close().await;

        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(dir.path().join(format!("slogger.sqlite{suffix}")));
        }
        std::fs::copy(&snapshot, &live).unwrap();

        let db = Database::open(&live).await.unwrap();
        assert_eq!(count(&db).await, 1);
        add_location(&db, "later").await;
        assert_eq!(count(&db).await, 2);
    }
}
