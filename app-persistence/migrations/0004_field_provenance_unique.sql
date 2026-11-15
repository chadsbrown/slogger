-- qso_field_provenance comes into use with LoTW sync: a row per QSO field
-- a service filled in (source `service_sync:<service>`) or reports
-- differently from the log (`service_differs:<service>`), with the
-- service's value in source_detail. A sync replaces a field's row rather
-- than adding another, so one row per (QSO, field, source) is the most
-- there should ever be; the unique index enforces it, gives upserts a
-- conflict target, and serves the per-QSO lookups (detail pane, the
-- "differs from LoTW" search filter) the old index did.
-- The table was unused before this, so no duplicates can exist.
DROP INDEX IF EXISTS idx_qso_provenance_qso;
CREATE UNIQUE INDEX idx_qso_provenance_unique
    ON qso_field_provenance(qso_id, field_name, source);
