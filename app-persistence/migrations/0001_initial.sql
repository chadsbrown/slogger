PRAGMA foreign_keys = ON;

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
