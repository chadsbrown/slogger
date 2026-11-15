-- Columns for ADIF fields that a DXKeeper export carries and that used to
-- land in qso_exchange_fields, plus the indexes award-credit import needs.

-- My station: OPERATOR is who operated, distinct from owner_callsign.
ALTER TABLE qsos ADD COLUMN operator_callsign TEXT;

-- The WPX prefix (ADIF PFX). dxcc_prefix is the DXCC entity prefix.
ALTER TABLE qsos ADD COLUMN wpx_prefix TEXT;

-- The other station's position, decimal degrees (south / west negative).
ALTER TABLE qsos ADD COLUMN latitude REAL;
ALTER TABLE qsos ADD COLUMN longitude REAL;

-- The other operator.
ALTER TABLE qsos ADD COLUMN name TEXT;
ALTER TABLE qsos ADD COLUMN qth TEXT;
ALTER TABLE qsos ADD COLUMN address TEXT;
ALTER TABLE qsos ADD COLUMN email TEXT;
ALTER TABLE qsos ADD COLUMN age INTEGER;
ALTER TABLE qsos ADD COLUMN qsl_via TEXT;
ALTER TABLE qsos ADD COLUMN comment TEXT;

-- Contest exchange.
ALTER TABLE qsos ADD COLUMN contest_id TEXT;
ALTER TABLE qsos ADD COLUMN serial_sent INTEGER;
ALTER TABLE qsos ADD COLUMN serial_rcvd INTEGER;
ALTER TABLE qsos ADD COLUMN exchange_sent TEXT;
ALTER TABLE qsos ADD COLUMN exchange_rcvd TEXT;
ALTER TABLE qsos ADD COLUMN arrl_section TEXT;

-- Receive side of a split / cross-band QSO.
ALTER TABLE qsos ADD COLUMN band_rx TEXT;
ALTER TABLE qsos ADD COLUMN freq_rx_hz INTEGER;

-- Space weather.
ALTER TABLE qsos ADD COLUMN sfi INTEGER;
ALTER TABLE qsos ADD COLUMN a_index INTEGER;
ALTER TABLE qsos ADD COLUMN k_index INTEGER;

-- Import creates one award_definitions row per award name it meets
-- (INSERT ... ON CONFLICT(name) DO NOTHING needs the unique index).
CREATE UNIQUE INDEX idx_award_definitions_name ON award_definitions(name);

-- Credit is read per QSO (detail view, awards, export).
CREATE INDEX idx_award_credit_links_qso ON award_credit_links(qso_id);
