use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use logbook_domain::{RepoResult, RepositoryError, StationRepository};
use radio_core::{
    Callsign, OperatingSession, OperatingSessionId, OperatorId, StationLocation, StationLocationId,
};

use crate::db::Database;

#[derive(Debug)]
pub struct SqliteStationRepository {
    pool: SqlitePool,
}

impl SqliteStationRepository {
    pub fn new(db: &Database) -> Self {
        Self { pool: db.pool().clone() }
    }
}

fn map_err(e: sqlx::Error) -> RepositoryError {
    match e {
        sqlx::Error::RowNotFound => RepositoryError::NotFound,
        sqlx::Error::Database(err) if err.is_unique_violation() || err.is_foreign_key_violation() => {
            RepositoryError::Constraint(err.to_string())
        }
        other => RepositoryError::Storage(other.to_string()),
    }
}

fn parse_callsign_opt(s: Option<String>) -> RepoResult<Option<Callsign>> {
    s.as_deref()
        .map(|c| Callsign::parse(c).map_err(|e| RepositoryError::Storage(format!("bad callsign: {e}"))))
        .transpose()
}

fn parse_dt(s: &str) -> RepoResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| RepositoryError::Storage(format!("bad timestamp: {e}")))
}

fn fmt_dt(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

fn row_to_location(row: &sqlx::sqlite::SqliteRow) -> RepoResult<StationLocation> {
    let id: String = row.try_get("id").map_err(map_err)?;
    let name: String = row.try_get("name").map_err(map_err)?;
    let station_callsign: Option<String> = row.try_get("station_callsign").map_err(map_err)?;
    let owner_callsign: Option<String> = row.try_get("owner_callsign").map_err(map_err)?;
    let city: Option<String> = row.try_get("city").map_err(map_err)?;
    let county: Option<String> = row.try_get("county").map_err(map_err)?;
    let state: Option<String> = row.try_get("state").map_err(map_err)?;
    let country: Option<String> = row.try_get("country").map_err(map_err)?;
    let grid: Option<String> = row.try_get("grid").map_err(map_err)?;
    let latitude: Option<f64> = row.try_get("latitude").map_err(map_err)?;
    let longitude: Option<f64> = row.try_get("longitude").map_err(map_err)?;
    let cq_zone: Option<i64> = row.try_get("cq_zone").map_err(map_err)?;
    let itu_zone: Option<i64> = row.try_get("itu_zone").map_err(map_err)?;
    let iota: Option<String> = row.try_get("iota").map_err(map_err)?;
    let lotw_station_location: Option<String> =
        row.try_get("lotw_station_location").map_err(map_err)?;
    let eqsl_account: Option<String> = row.try_get("eqsl_account").map_err(map_err)?;
    let created_at: String = row.try_get("created_at").map_err(map_err)?;
    let updated_at: String = row.try_get("updated_at").map_err(map_err)?;

    let uuid = Uuid::parse_str(&id)
        .map_err(|e| RepositoryError::Storage(format!("bad uuid: {e}")))?;

    Ok(StationLocation {
        id: StationLocationId::from_uuid(uuid),
        name,
        station_callsign: parse_callsign_opt(station_callsign)?,
        owner_callsign: parse_callsign_opt(owner_callsign)?,
        city,
        county,
        state,
        country,
        grid,
        latitude,
        longitude,
        cq_zone: cq_zone.map(|v| v as u8),
        itu_zone: itu_zone.map(|v| v as u8),
        iota,
        lotw_station_location,
        eqsl_account,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

#[async_trait]
impl StationRepository for SqliteStationRepository {
    async fn insert_location(&self, location: &StationLocation) -> RepoResult<()> {
        sqlx::query(
            r#"
            INSERT INTO station_locations (
                id, name,
                station_callsign, owner_callsign,
                city, county, state, country, grid,
                latitude, longitude,
                cq_zone, itu_zone, iota,
                lotw_station_location, eqsl_account,
                created_at, updated_at
            ) VALUES (
                ?, ?,
                ?, ?,
                ?, ?, ?, ?, ?,
                ?, ?,
                ?, ?, ?,
                ?, ?,
                ?, ?
            )
            "#,
        )
        .bind(location.id.as_uuid().to_string())
        .bind(&location.name)
        .bind(location.station_callsign.as_ref().map(|c| c.as_str().to_string()))
        .bind(location.owner_callsign.as_ref().map(|c| c.as_str().to_string()))
        .bind(&location.city)
        .bind(&location.county)
        .bind(&location.state)
        .bind(&location.country)
        .bind(&location.grid)
        .bind(location.latitude)
        .bind(location.longitude)
        .bind(location.cq_zone.map(|v| v as i64))
        .bind(location.itu_zone.map(|v| v as i64))
        .bind(&location.iota)
        .bind(&location.lotw_station_location)
        .bind(&location.eqsl_account)
        .bind(fmt_dt(&location.created_at))
        .bind(fmt_dt(&location.updated_at))
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn update_location(&self, location: &StationLocation) -> RepoResult<()> {
        sqlx::query(
            r#"
            UPDATE station_locations SET
                name = ?,
                station_callsign = ?, owner_callsign = ?,
                city = ?, county = ?, state = ?, country = ?, grid = ?,
                latitude = ?, longitude = ?,
                cq_zone = ?, itu_zone = ?, iota = ?,
                lotw_station_location = ?, eqsl_account = ?,
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&location.name)
        .bind(location.station_callsign.as_ref().map(|c| c.as_str().to_string()))
        .bind(location.owner_callsign.as_ref().map(|c| c.as_str().to_string()))
        .bind(&location.city)
        .bind(&location.county)
        .bind(&location.state)
        .bind(&location.country)
        .bind(&location.grid)
        .bind(location.latitude)
        .bind(location.longitude)
        .bind(location.cq_zone.map(|v| v as i64))
        .bind(location.itu_zone.map(|v| v as i64))
        .bind(&location.iota)
        .bind(&location.lotw_station_location)
        .bind(&location.eqsl_account)
        .bind(fmt_dt(&location.updated_at))
        .bind(location.id.as_uuid().to_string())
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn get_location(
        &self,
        id: &StationLocationId,
    ) -> RepoResult<Option<StationLocation>> {
        let row = sqlx::query("SELECT * FROM station_locations WHERE id = ?")
            .bind(id.as_uuid().to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        row.as_ref().map(row_to_location).transpose()
    }

    async fn list_locations(&self) -> RepoResult<Vec<StationLocation>> {
        let rows = sqlx::query("SELECT * FROM station_locations ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        rows.iter().map(row_to_location).collect()
    }

    async fn start_session(
        &self,
        operator_id: Option<&OperatorId>,
        station_location_id: Option<&StationLocationId>,
        name: Option<&str>,
    ) -> RepoResult<OperatingSessionId> {
        let id = OperatingSessionId::new();
        let now = fmt_dt(&Utc::now());
        sqlx::query(
            r#"
            INSERT INTO operating_sessions
                (id, operator_id, station_location_id, started_at, ended_at, name, notes)
            VALUES (?, ?, ?, ?, NULL, ?, NULL)
            "#,
        )
        .bind(id.as_uuid().to_string())
        .bind(operator_id.map(|i| i.as_uuid().to_string()))
        .bind(station_location_id.map(|i| i.as_uuid().to_string()))
        .bind(&now)
        .bind(name)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(id)
    }

    async fn end_session(&self, id: &OperatingSessionId) -> RepoResult<()> {
        let now = fmt_dt(&Utc::now());
        sqlx::query("UPDATE operating_sessions SET ended_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id.as_uuid().to_string())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn get_session(
        &self,
        id: &OperatingSessionId,
    ) -> RepoResult<Option<OperatingSession>> {
        let row = sqlx::query("SELECT * FROM operating_sessions WHERE id = ?")
            .bind(id.as_uuid().to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        let Some(row) = row else { return Ok(None) };

        let id_s: String = row.try_get("id").map_err(map_err)?;
        let operator_id: Option<String> = row.try_get("operator_id").map_err(map_err)?;
        let station_location_id: Option<String> =
            row.try_get("station_location_id").map_err(map_err)?;
        let started_at: String = row.try_get("started_at").map_err(map_err)?;
        let ended_at: Option<String> = row.try_get("ended_at").map_err(map_err)?;
        let name: Option<String> = row.try_get("name").map_err(map_err)?;
        let notes: Option<String> = row.try_get("notes").map_err(map_err)?;

        let parse_uuid = |s: &str| {
            Uuid::parse_str(s).map_err(|e| RepositoryError::Storage(format!("bad uuid: {e}")))
        };
        let id_uuid = parse_uuid(&id_s)?;

        Ok(Some(OperatingSession {
            id: OperatingSessionId::from_uuid(id_uuid),
            operator_id: operator_id
                .as_deref()
                .map(parse_uuid)
                .transpose()?
                .map(OperatorId::from_uuid),
            station_location_id: station_location_id
                .as_deref()
                .map(parse_uuid)
                .transpose()?
                .map(StationLocationId::from_uuid),
            started_at: parse_dt(&started_at)?,
            ended_at: ended_at.as_deref().map(parse_dt).transpose()?,
            name,
            notes,
        }))
    }

    async fn set_session_station_location(
        &self,
        id: &OperatingSessionId,
        station_location_id: Option<&StationLocationId>,
    ) -> RepoResult<()> {
        sqlx::query(
            "UPDATE operating_sessions SET station_location_id = ? WHERE id = ?",
        )
        .bind(station_location_id.map(|i| i.as_uuid().to_string()))
        .bind(id.as_uuid().to_string())
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    async fn close_open_sessions(&self) -> RepoResult<usize> {
        let now = fmt_dt(&Utc::now());
        let result = sqlx::query(
            "UPDATE operating_sessions SET ended_at = ? WHERE ended_at IS NULL",
        )
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(result.rows_affected() as usize)
    }

    async fn list_sessions(
        &self,
        limit: Option<u32>,
    ) -> RepoResult<Vec<OperatingSession>> {
        // Bind the limit as a SQLite LIMIT clause; `None` means no cap,
        // which we encode as -1 since SQLite's `LIMIT -1` is "no limit".
        let cap = limit.map(|n| n as i64).unwrap_or(-1);
        let rows = sqlx::query(
            r#"
            SELECT id, operator_id, station_location_id, started_at,
                   ended_at, name, notes
            FROM operating_sessions
            ORDER BY started_at DESC
            LIMIT ?
            "#,
        )
        .bind(cap)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        let parse_uuid = |s: &str| {
            Uuid::parse_str(s).map_err(|e| RepositoryError::Storage(format!("bad uuid: {e}")))
        };

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id_s: String = row.try_get("id").map_err(map_err)?;
            let operator_id: Option<String> =
                row.try_get("operator_id").map_err(map_err)?;
            let station_location_id: Option<String> =
                row.try_get("station_location_id").map_err(map_err)?;
            let started_at: String = row.try_get("started_at").map_err(map_err)?;
            let ended_at: Option<String> = row.try_get("ended_at").map_err(map_err)?;
            let name: Option<String> = row.try_get("name").map_err(map_err)?;
            let notes: Option<String> = row.try_get("notes").map_err(map_err)?;

            let id_uuid = parse_uuid(&id_s)?;
            out.push(OperatingSession {
                id: OperatingSessionId::from_uuid(id_uuid),
                operator_id: operator_id
                    .as_deref()
                    .map(parse_uuid)
                    .transpose()?
                    .map(OperatorId::from_uuid),
                station_location_id: station_location_id
                    .as_deref()
                    .map(parse_uuid)
                    .transpose()?
                    .map(StationLocationId::from_uuid),
                started_at: parse_dt(&started_at)?,
                ended_at: ended_at.as_deref().map(parse_dt).transpose()?,
                name,
                notes,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logbook_domain::StationRepository as _;

    async fn fresh_repo() -> SqliteStationRepository {
        let db = Database::open_in_memory().await.unwrap();
        SqliteStationRepository::new(&db)
    }

    fn sample_location(name: &str) -> StationLocation {
        StationLocation {
            id: StationLocationId::new(),
            name: name.into(),
            station_callsign: Some(Callsign::parse("W1ABC").unwrap()),
            owner_callsign: None,
            city: None,
            county: None,
            state: Some("CT".into()),
            country: None,
            grid: Some("FN31".into()),
            latitude: None,
            longitude: None,
            cq_zone: Some(5),
            itu_zone: Some(8),
            iota: None,
            lotw_station_location: None,
            eqsl_account: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn locations_round_trip() {
        let repo = fresh_repo().await;
        let loc = sample_location("Home");
        repo.insert_location(&loc).await.unwrap();
        let fetched = repo.get_location(&loc.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "Home");
        assert_eq!(fetched.station_callsign, loc.station_callsign);
        assert_eq!(repo.list_locations().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn close_open_sessions_handles_orphans_at_boot() {
        let repo = fresh_repo().await;
        let loc = sample_location("Home");
        repo.insert_location(&loc).await.unwrap();

        // Two open sessions (simulating prior crashed-out runs) plus one
        // already-closed session.
        let s1 = repo.start_session(None, Some(&loc.id), Some("first")).await.unwrap();
        let s2 = repo.start_session(None, Some(&loc.id), Some("second")).await.unwrap();
        let s3 = repo.start_session(None, Some(&loc.id), Some("already closed")).await.unwrap();
        repo.end_session(&s3).await.unwrap();

        let closed = repo.close_open_sessions().await.unwrap();
        assert_eq!(closed, 2, "expected to close two open sessions");

        for id in [s1, s2, s3] {
            let s = repo.get_session(&id).await.unwrap().unwrap();
            assert!(s.ended_at.is_some(), "session {} should be closed", id);
        }
    }

    #[tokio::test]
    async fn session_lifecycle() {
        let repo = fresh_repo().await;
        let loc = sample_location("Home");
        repo.insert_location(&loc).await.unwrap();

        let session_id = repo
            .start_session(None, Some(&loc.id), Some("evening run"))
            .await
            .unwrap();
        let s = repo.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(s.name.as_deref(), Some("evening run"));
        assert_eq!(s.station_location_id, Some(loc.id));
        assert!(s.ended_at.is_none());

        // Mid-session re-target.
        let other = sample_location("Field");
        repo.insert_location(&other).await.unwrap();
        repo.set_session_station_location(&session_id, Some(&other.id))
            .await
            .unwrap();
        let s2 = repo.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(s2.station_location_id, Some(other.id));

        repo.end_session(&session_id).await.unwrap();
        let s3 = repo.get_session(&session_id).await.unwrap().unwrap();
        assert!(s3.ended_at.is_some());
    }
}
