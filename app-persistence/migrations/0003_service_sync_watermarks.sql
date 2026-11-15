-- Where each service's incremental report left off. LoTW hands back a
-- value in each report's header (APP_LoTW_LASTQSL, APP_LoTW_LASTQSORX)
-- to send as `since` on the next identical query; it is stored verbatim.
-- service_accounts and service_sync_jobs don't fit: one describes an
-- account, the other a retry queue.
CREATE TABLE service_sync_watermarks (
    service TEXT NOT NULL,
    report TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (service, report)
);
