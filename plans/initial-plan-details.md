Below is a concrete starting point for the new persistence/domain layer. It deliberately supports the DXKeeper feature surface without copying DXKeeper’s wide-table design. DXKeeper’s schema shows why this matters: QSO facts, QTHs, QSL queues, award progress, WAS progress, DXCC credit links, and service status are all distinct concerns that should not live as one giant record. 

# 1. Crates

```text
crates/
  radio-core/
    src/
      ids.rs
      value.rs
      qso.rs
      station.rs
      service.rs
      provenance.rs
      events.rs

  logbook-domain/
    src/
      commands.rs
      queries.rs
      duplicate.rs
      validation.rs

  app-persistence/
    migrations/
      0001_initial.sql
    src/
      db.rs
      repositories/
        qso.rs
        station.rs
        service.rs
      sqlite/
        qso_repo.rs
        station_repo.rs
        service_repo.rs

  app-ui/
    iced shell
```

I would not start with `qsolog` as the center. At most, reuse pieces inside `logbook-domain` or `app-persistence`.

# 2. Initial SQLite schema

This is enough to start, without painting you into a corner.

```sql
PRAGMA foreign_keys = ON;

CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE operators (
    id TEXT PRIMARY KEY,
    callsign TEXT NOT NULL,
    name TEXT,
    email TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE station_locations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    station_callsign TEXT,
    owner_callsign TEXT,
    city TEXT,
    county TEXT,
    state TEXT,
    country TEXT,
    grid TEXT,
    latitude REAL,
    longitude REAL,
    cq_zone INTEGER,
    itu_zone INTEGER,
    iota TEXT,
    lotw_station_location TEXT,
    eqsl_account TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE operating_sessions (
    id TEXT PRIMARY KEY,
    operator_id TEXT,
    station_location_id TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    name TEXT,
    notes TEXT,
    FOREIGN KEY(operator_id) REFERENCES operators(id),
    FOREIGN KEY(station_location_id) REFERENCES station_locations(id)
);

CREATE TABLE qsos (
    id TEXT PRIMARY KEY,

    call TEXT NOT NULL,
    qso_begin TEXT NOT NULL,
    qso_end TEXT,

    band TEXT,
    freq_hz INTEGER,
    mode TEXT,
    submode TEXT,

    rst_sent TEXT,
    rst_rcvd TEXT,

    operator_id TEXT,
    station_location_id TEXT,
    operating_session_id TEXT,

    station_callsign TEXT,
    owner_callsign TEXT,

    dxcc_id INTEGER,
    dxcc_prefix TEXT,
    continent TEXT,
    cq_zone INTEGER,
    itu_zone INTEGER,
    grid TEXT,
    state TEXT,
    county TEXT,
    province TEXT,
    iota TEXT,

    tx_power_w REAL,
    rx_power_w REAL,

    propagation_mode TEXT,
    sat_name TEXT,
    sat_mode TEXT,

    distance_km REAL,
    bearing_deg REAL,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,

    FOREIGN KEY(operator_id) REFERENCES operators(id),
    FOREIGN KEY(station_location_id) REFERENCES station_locations(id),
    FOREIGN KEY(operating_session_id) REFERENCES operating_sessions(id)
);

CREATE INDEX idx_qsos_call ON qsos(call);
CREATE INDEX idx_qsos_time ON qsos(qso_begin);
CREATE INDEX idx_qsos_band_mode ON qsos(band, mode);
CREATE INDEX idx_qsos_dxcc ON qsos(dxcc_id);
CREATE INDEX idx_qsos_station_location ON qsos(station_location_id);
```

## Contest / structured exchange fields

```sql
CREATE TABLE qso_exchange_fields (
    id TEXT PRIMARY KEY,
    qso_id TEXT NOT NULL,
    field_name TEXT NOT NULL,
    raw_value TEXT NOT NULL,
    normalized_value TEXT,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL,

    FOREIGN KEY(qso_id) REFERENCES qsos(id) ON DELETE CASCADE
);

CREATE INDEX idx_qso_exchange_qso ON qso_exchange_fields(qso_id);
CREATE UNIQUE INDEX idx_qso_exchange_unique
    ON qso_exchange_fields(qso_id, field_name);
```

This avoids baking `SRX`, `STX`, `SRX_STRING`, `STX_STRING`, and every future contest field into the main QSO row.

## Notes and attachments

```sql
CREATE TABLE qso_notes (
    id TEXT PRIMARY KEY,
    qso_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY(qso_id) REFERENCES qsos(id) ON DELETE CASCADE
);

CREATE TABLE qso_attachments (
    id TEXT PRIMARY KEY,
    qso_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    path TEXT NOT NULL,
    description TEXT,
    created_at TEXT NOT NULL,

    FOREIGN KEY(qso_id) REFERENCES qsos(id) ON DELETE CASCADE
);
```

## Provenance

```sql
CREATE TABLE qso_field_provenance (
    id TEXT PRIMARY KEY,
    qso_id TEXT NOT NULL,
    field_name TEXT NOT NULL,
    source TEXT NOT NULL,
    source_detail TEXT,
    confidence REAL,
    created_at TEXT NOT NULL,

    FOREIGN KEY(qso_id) REFERENCES qsos(id) ON DELETE CASCADE
);

CREATE INDEX idx_qso_provenance_qso ON qso_field_provenance(qso_id);
```

This is one of the biggest improvements over legacy designs.

## Service accounts and per-QSO service state

```sql
CREATE TABLE service_accounts (
    id TEXT PRIMARY KEY,
    service TEXT NOT NULL,
    account_name TEXT NOT NULL,
    station_location_id TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY(station_location_id) REFERENCES station_locations(id)
);

CREATE TABLE qso_service_state (
    id TEXT PRIMARY KEY,
    qso_id TEXT NOT NULL,
    service TEXT NOT NULL,

    upload_state TEXT NOT NULL,
    confirmation_state TEXT NOT NULL,

    uploaded_at TEXT,
    confirmed_at TEXT,
    remote_id TEXT,

    last_sync_at TEXT,
    last_error TEXT,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY(qso_id) REFERENCES qsos(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_qso_service_unique
    ON qso_service_state(qso_id, service);
```

This replaces legacy service-specific QSO columns such as LoTW/eQSL/Club Log/QRZ fields with a clean extensible model.

## Sync jobs

```sql
CREATE TABLE service_sync_jobs (
    id TEXT PRIMARY KEY,
    service TEXT NOT NULL,
    job_type TEXT NOT NULL,
    status TEXT NOT NULL,
    qso_id TEXT,
    payload_json TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY(qso_id) REFERENCES qsos(id) ON DELETE CASCADE
);

CREATE INDEX idx_service_sync_status
    ON service_sync_jobs(service, status, next_attempt_at);
```

## Awards and credits

```sql
CREATE TABLE award_definitions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    definition_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE award_progress (
    id TEXT PRIMARY KEY,
    award_id TEXT NOT NULL,
    unit TEXT NOT NULL,
    band TEXT,
    mode TEXT,
    status TEXT NOT NULL,
    qso_id TEXT,
    updated_at TEXT NOT NULL,

    FOREIGN KEY(award_id) REFERENCES award_definitions(id),
    FOREIGN KEY(qso_id) REFERENCES qsos(id)
);

CREATE INDEX idx_award_progress_lookup
    ON award_progress(award_id, unit, band, mode);

CREATE TABLE award_credit_imports (
    id TEXT PRIMARY KEY,
    award_id TEXT NOT NULL,
    external_credit_id TEXT,
    call TEXT,
    qso_date TEXT,
    band TEXT,
    mode TEXT,
    unit TEXT NOT NULL,
    raw_json TEXT,
    imported_at TEXT NOT NULL,

    FOREIGN KEY(award_id) REFERENCES award_definitions(id)
);

CREATE TABLE award_credit_links (
    id TEXT PRIMARY KEY,
    credit_import_id TEXT NOT NULL,
    qso_id TEXT,
    link_status TEXT NOT NULL,
    reviewed_at TEXT,
    reviewed_by TEXT,

    FOREIGN KEY(credit_import_id) REFERENCES award_credit_imports(id),
    FOREIGN KEY(qso_id) REFERENCES qsos(id)
);
```

This is the modern equivalent of `DXCCCredit`, `Progress`, and `WASProgress` without copying their shape.

# 3. Rust domain structs

## IDs

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QsoId(pub uuid::Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OperatorId(pub uuid::Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StationLocationId(pub uuid::Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OperatingSessionId(pub uuid::Uuid);
```

## Value types

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callsign(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    M160,
    M80,
    M60,
    M40,
    M30,
    M20,
    M17,
    M15,
    M12,
    M10,
    M6,
    M2,
    Cm70,
    Cm23,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    CW,
    SSB,
    RTTY,
    AM,
    FM,
    FT8,
    FT4,
    Digital(String),
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropagationMode {
    Terrestrial,
    Satellite,
    Eme,
    MeteorScatter,
    Aurora,
    AircraftScatter,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldSource {
    OperatorEntered,
    RigDerived,
    ImportedAdif,
    StationDataResolved,
    ServiceSync(String),
    ManualOverride,
}
```

## QSO core

```rust
#[derive(Debug, Clone)]
pub struct Qso {
    pub id: QsoId,

    pub call: Callsign,
    pub qso_begin: chrono::DateTime<chrono::Utc>,
    pub qso_end: Option<chrono::DateTime<chrono::Utc>>,

    pub band: Option<Band>,
    pub freq_hz: Option<i64>,
    pub mode: Option<Mode>,
    pub submode: Option<String>,

    pub rst_sent: Option<String>,
    pub rst_rcvd: Option<String>,

    pub operator_id: Option<OperatorId>,
    pub station_location_id: Option<StationLocationId>,
    pub operating_session_id: Option<OperatingSessionId>,

    pub station_callsign: Option<Callsign>,
    pub owner_callsign: Option<Callsign>,

    pub dxcc_id: Option<u16>,
    pub dxcc_prefix: Option<String>,
    pub grid: Option<String>,
    pub state: Option<String>,
    pub county: Option<String>,
    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    pub iota: Option<String>,

    pub tx_power_w: Option<f32>,
    pub rx_power_w: Option<f32>,

    pub propagation_mode: Option<PropagationMode>,
    pub sat_name: Option<String>,
    pub sat_mode: Option<String>,

    pub distance_km: Option<f64>,
    pub bearing_deg: Option<f64>,

    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

## Exchange fields

```rust
#[derive(Debug, Clone)]
pub struct QsoExchangeField {
    pub name: String,
    pub raw_value: String,
    pub normalized_value: Option<String>,
    pub source: FieldSource,
}
```

## Station location

```rust
#[derive(Debug, Clone)]
pub struct StationLocation {
    pub id: StationLocationId,
    pub name: String,

    pub station_callsign: Option<Callsign>,
    pub owner_callsign: Option<Callsign>,

    pub city: Option<String>,
    pub county: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub grid: Option<String>,

    pub latitude: Option<f64>,
    pub longitude: Option<f64>,

    pub cq_zone: Option<u8>,
    pub itu_zone: Option<u8>,
    pub iota: Option<String>,

    pub lotw_station_location: Option<String>,
    pub eqsl_account: Option<String>,
}
```

# 4. Repository traits

Keep these traits in `logbook-domain` or `radio-core`, with SQLite implementations in `app-persistence`.

```rust
#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("not found")]
    NotFound,

    #[error("constraint violation: {0}")]
    Constraint(String),

    #[error("storage error: {0}")]
    Storage(String),
}

pub type RepoResult<T> = Result<T, RepositoryError>;
```

## QSO repository

```rust
#[async_trait::async_trait]
pub trait QsoRepository: Send + Sync {
    async fn insert_qso(&self, qso: &Qso) -> RepoResult<()>;

    async fn update_qso(&self, qso: &Qso) -> RepoResult<()>;

    async fn get_qso(&self, id: &QsoId) -> RepoResult<Option<Qso>>;

    async fn soft_delete_qso(&self, id: &QsoId) -> RepoResult<()>;

    async fn add_exchange_field(
        &self,
        qso_id: &QsoId,
        field: &QsoExchangeField,
    ) -> RepoResult<()>;

    async fn list_exchange_fields(
        &self,
        qso_id: &QsoId,
    ) -> RepoResult<Vec<QsoExchangeField>>;

    async fn search_qsos(
        &self,
        query: QsoSearch,
    ) -> RepoResult<Vec<QsoSummary>>;
}
```

## Search model

```rust
#[derive(Debug, Clone, Default)]
pub struct QsoSearch {
    pub call_prefix: Option<String>,
    pub exact_call: Option<Callsign>,
    pub band: Option<Band>,
    pub mode: Option<Mode>,
    pub dxcc_id: Option<u16>,
    pub station_location_id: Option<StationLocationId>,
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct QsoSummary {
    pub id: QsoId,
    pub call: Callsign,
    pub qso_begin: chrono::DateTime<chrono::Utc>,
    pub band: Option<Band>,
    pub mode: Option<Mode>,
    pub freq_hz: Option<i64>,
    pub dxcc_id: Option<u16>,
}
```

## Station repository

```rust
#[async_trait::async_trait]
pub trait StationRepository: Send + Sync {
    async fn insert_location(&self, location: &StationLocation) -> RepoResult<()>;

    async fn update_location(&self, location: &StationLocation) -> RepoResult<()>;

    async fn get_location(
        &self,
        id: &StationLocationId,
    ) -> RepoResult<Option<StationLocation>>;

    async fn list_locations(&self) -> RepoResult<Vec<StationLocation>>;
}
```

# 5. Commands for domain behavior

I would avoid letting the UI construct `Qso` directly. Use commands.

```rust
pub struct CreateQsoCommand {
    pub call: Callsign,
    pub qso_begin: chrono::DateTime<chrono::Utc>,

    pub band: Option<Band>,
    pub freq_hz: Option<i64>,
    pub mode: Option<Mode>,
    pub submode: Option<String>,

    pub rst_sent: Option<String>,
    pub rst_rcvd: Option<String>,

    pub operator_id: Option<OperatorId>,
    pub station_location_id: Option<StationLocationId>,
    pub operating_session_id: Option<OperatingSessionId>,

    pub exchange_fields: Vec<QsoExchangeField>,
}
```

```rust
pub struct LogbookService<R>
where
    R: QsoRepository,
{
    repo: R,
}

impl<R> LogbookService<R>
where
    R: QsoRepository,
{
    pub async fn create_qso(&self, command: CreateQsoCommand) -> RepoResult<QsoId> {
        let now = chrono::Utc::now();
        let id = QsoId(uuid::Uuid::new_v4());

        let qso = Qso {
            id: id.clone(),
            call: command.call,
            qso_begin: command.qso_begin,
            qso_end: None,
            band: command.band,
            freq_hz: command.freq_hz,
            mode: command.mode,
            submode: command.submode,
            rst_sent: command.rst_sent,
            rst_rcvd: command.rst_rcvd,
            operator_id: command.operator_id,
            station_location_id: command.station_location_id,
            operating_session_id: command.operating_session_id,
            station_callsign: None,
            owner_callsign: None,
            dxcc_id: None,
            dxcc_prefix: None,
            grid: None,
            state: None,
            county: None,
            cq_zone: None,
            itu_zone: None,
            iota: None,
            tx_power_w: None,
            rx_power_w: None,
            propagation_mode: None,
            sat_name: None,
            sat_mode: None,
            distance_km: None,
            bearing_deg: None,
            created_at: now,
            updated_at: now,
        };

        self.repo.insert_qso(&qso).await?;

        for field in command.exchange_fields {
            self.repo.add_exchange_field(&id, &field).await?;
        }

        Ok(id)
    }
}
```

# 6. iced integration shape

In the `iced` app, do not call SQLite directly from widgets.

Use messages like:

```rust
pub enum Message {
    QsoCallChanged(String),
    QsoBandChanged(Option<Band>),
    QsoModeChanged(Option<Mode>),
    LogQsoPressed,
    QsoCreated(Result<QsoId, String>),
}
```

The update layer issues a task/effect:

```rust
Message::LogQsoPressed => {
    let command = self.entry_state.to_create_qso_command();

    return Task::perform(
        self.services.logbook.create_qso(command),
        |result| Message::QsoCreated(result.map_err(|e| e.to_string())),
    );
}
```

The rule: **iced owns interaction state, not domain truth.**

# 7. First implementation phase

I would start with exactly this:

```text
Phase 1:
  radio-core:
    IDs
    Callsign
    Band
    Mode
    PropagationMode
    Qso
    StationLocation

  app-persistence:
    SQLite connection
    migrations
    QsoRepository
    StationRepository
    insert/get/search

  app-ui:
    minimal iced window
    QSO entry form
    QSO list
    log button
```

Do not implement awards, sync, or QSL queues yet. But the schema already leaves proper places for them.

# 8. The most important early decision

Use a **narrow QSO core** plus related tables.

Do **not** do this:

```rust
struct Qso {
    // 150 fields...
}
```

Do this:

```rust
Qso
QsoExchangeField
QsoServiceState
QsoAwardCredit
QsoNote
QsoAttachment
QsoProvenance
```

That is how you support DXKeeper/DXLab-level feature depth without recreating its legacy shape.

