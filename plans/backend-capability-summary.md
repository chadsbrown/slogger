# Backend Capability Summary

Snapshot of what slogger's backend can do as of 2026-05-09, captured before
beginning the UI redesign. Source of truth for "what can the UI call?" — if
you're sketching flows, start here.

For per-section details on configuring features, see
`features-and-configuration.md`. For a copy-pasteable starting config, see
`example-config.toml`.

---

## Workspace shape

16 crates, ~9k LOC of slogger-authored code + tests. Pulls in shared
chadsbrown libs (`riglib`, `winkey`, `otrsp`, `dxfeed`, `station-data`,
`adif_parser`). 123 tests, no warnings, clean smoke launch.

```
radio-core           types: Qso, Callsign, Band, Mode, ids, wpx_prefix
logbook-domain       service + repository traits + commands + queries
                     + ADIF import/export + awards + bulk ops + search
app-persistence      SQLite impls (sqlx) of the repository traits
app-config           TOML config: [station] [lotw] [eqsl] [clublog] [qrz]
                     [hrdlog] [dxcluster] [wsjtx] [[rig]] [keyer] [so2r]
station-resolver     cty.dat → DXCC entity ID + zones + continent
spot-feed            DX cluster spots via dxfeed
wsjtx-bridge         WSJT-X UDP listener (auto-import logged QSOs)
rig-control          riglib wrapper, multi-rig, auto-reconnect, set/get
keyer-control        WinKeyer wrapper, send_message/set_wpm/abort/tune
so2r-control         OTRSP switch wrapper, set_tx/set_rx/set_aux
lotw-sync            TQSL shellout + report fetch (verify + confirm)
eqsl-sync            ImportADIF + DownloadInBox
clublog-sync         realtime.php upload
qrz-sync             logbook API per-record upload
hrdlog-sync          NewEntry.aspx upload
app-ui               iced 0.14 frontend (current dense single-window;
                     this is what gets redesigned)
```

---

## Capabilities the UI can call directly

### Logging (`LogbookService`)

- `create_qso(CreateQsoCommand)` — single-QSO insert with resolver enrichment
- `update_qso(id, command)` — edit, re-runs resolver
- `delete_qso(id)` — soft delete
- `import_qsos(commands)` — batch insert with dedup against existing log
- `parse_adif(text)` — text → `Vec<CreateQsoCommand>` (also handles WSJT-X
  UDP payloads)
- `export_adif(qsos, opts)` — round-trip ADIF
- `bulk_soft_delete(&[ids])`, `bulk_mark_uploaded(&[ids], svc, at)`,
  `bulk_mark_confirmed(&[ids], svc, at)`
- `bulk_*_by_search(QsoSearch, ...)` — search + apply in one call, returns
  `BulkReport { matched, succeeded }`

### Search / query

- `search_qsos(QsoSearch) -> Vec<QsoSummary>` — list view (light columns)
- `search_full_qsos(QsoSearch) -> Vec<Qso>` — edit/export (full records)
- `count_matching(QsoSearch) -> usize` — for "Delete N QSOs?" confirmations

`QsoSearch` filter dimensions:

| Field | Type | Notes |
|---|---|---|
| `call_prefix` | starts-with substring | Case-insensitive |
| `exact_call` | exact match | |
| `band` | enum | |
| `mode` | enum | |
| `dxcc_id` | numeric ARRL entity | |
| `station_location_id` | UUID | |
| `from` / `to` | datetime range | UTC |
| `state` | US state (2-letter) | |
| `iota` | "EU-005" etc. | |
| `continent` | "NA"/"EU"/... | |
| `lotw_confirmed` | `Option<bool>` | `Some(true)`/`Some(false)`/`None` |

`SortOrder`: `QsoBeginDesc` (default), `QsoBeginAsc`, `CallAsc`, `CallDesc`,
`BandAsc`, `BandDesc`.

### Awards (derived, not materialized)

`logbook_domain::snapshot(qsos, year)` returns:

- DXCC (worked / confirmed counts + per-band breakdown)
- WAS (US states, filtered to `dxcc_id == 291`)
- WPX (CQ WPX prefix derivation handling /digit, /M, /VE3 cases)
- IOTA (distinct islands)
- Marathon (per-year DXCC count)

Per-band/per-mode dimensions exist; UI can pivot.

### Live operating

- **Multi-rig**: `App.rigs: Vec<RigEntry>`, `active_rig: usize`. Each entry
  has handle + last snapshot + label. Auto-reconnect inside each handle's
  task. Snapshots flow via unified channel tagged with `rig_index`.
- **Per-rig commands**: `set_frequency_hz`, `set_mode_adif`. Returns clean
  errors during disconnect window.
- **CW keyer**: `send_message`, `abort`, `set_wpm`, `set_tune`. Snapshot has
  `wpm` + `keying` (TX in progress).
- **SO2R switch**: `set_tx_radio(1|2)`, `set_rx_audio(radio, mode)`,
  `set_aux(port, value)`. Snapshot has `tx_radio`, `rx_radio`, `rx_mode`.
- **DX cluster spots**: stream of `Spot { call, freq_hz, mode, comment,
  spotted_at }`. Configurable filter file. UI gets need-by-band annotation
  via `worked_by_band`.
- **WSJT-X bridge**: passive UDP listener; `WsjtxMessage::LoggedAdif` flows
  in. UI doesn't need to do anything — auto-imports run in `update()`.

### Sync services (5 of 5)

| Service | Operations | Auth |
|---|---|---|
| LotW | upload (via TQSL) + verify + confirm | website pwd + cert via tqsl |
| eQSL | upload + confirm | username + password |
| Club Log | upload | email + password + callsign |
| QRZ | upload (per-record) | api_key |
| HRDLog | upload | callsign + upload code |

All driven by `MultiUpdateSummary` from one `run_services_update(repo, cfg)`
call. Each service fails independently. Per-QSO upload state in
`qso_service_state` table (`pending`/`uploaded`/`verified`/`failed` +
confirmation axis).

### Station + session

- `StationRepository`: insert/update/list station_locations,
  start/end/get/retarget operating_sessions
- Boot closes orphan sessions (`ended_at IS NULL`) automatically
- Active station_location feeds `station_callsign` onto every QSO

### Resolver / DXCC

- `Resolver` trait. `CtyDbResolver` wraps `station_data::CtyDb`. Parses
  cty.dat at boot.
- Returns `Resolution { dxcc_id, dxcc_prefix, country, continent, cq_zone,
  itu_zone, lat, lon }`.
- `dxcc_id_for_prefix(s)`: ~70 hand-curated prefixes. **Limitation**: rare
  entities → `None`, so awards count is conservative.

---

## Backend gaps worth knowing about during UI design

- **DXCC entity ID coverage**: ~70 prefixes. Big DXers will hit "?" on rare
  entities. Replacement: parse ClubLog's cty.xml at startup → ~340 entities
  full coverage. Bounded work, ~200 LOC.
- **Operator entity**: schema has `operators` table. No backend wiring.
  Multi-op shared station UI would need this surfaced.
- **Service retry queue**: `service_sync_jobs` schema row exists. Nothing
  writes to it. If a sync fails, current behavior is "try again next time
  you press Update" — no per-QSO retry tracking.
- **ADIF import duplicates**: dedup is keyed on `station_callsign + call +
  date + band + mode`. QSOs without `station_callsign` skip dedup. Edge
  case worth knowing.
- **Spot filters via `[dxcluster].filter_file`**: filter format is dxfeed's
  JSON (continent/band/spotter rules etc.); user has to author it by hand.
  No UI.
- **No undo for bulk operations**: soft_delete is reversible at the schema
  level (`deleted_at`) but there's no `undo_delete` or undo stack.
- **Audio routing for SO2R**: OTRSP switches PTT/key/mic at the hardware
  level. Slogger doesn't manage soundcards. Operator wires audio via OS /
  device routing.
- **No FlexRadio multi-slice**: FlexRadio supports multiple "slices"
  (independent receivers); slogger treats Flex as one rig with one VFO.
- **No PTT visibility**: rig-control doesn't surface `RigEvent::PttChanged`
  in snapshots. UI can't show TX/RX indicator from rig events.
- **Operating-time vs after-the-fact**: WSJT-X imports happen passively but
  the operator can't *see* an FT8 decode in slogger — no decoder surface.

---

## What's worth thinking about for the UI redesign

These are questions, not prescriptions.

### Layout shape

Current single-column iced flow is becoming dense. Three plausible shapes:

- **Tabbed** (Log / Spots / Awards / Setup) — clean separation, one focus
  at a time. Easiest to navigate; loses context across views.
- **Multi-pane** (clogger-style) — entry + spots + log + awards + rig all
  visible, draggable panes. Most info-dense; needs careful default layout.
- **Two-mode** (Operating mode = real-time-driven, Logbook mode =
  retrospective) — different layouts per use case. Operator switches mode
  at session start.

### Operator-facing flows

A few high-leverage ones the backend already supports:

- **Click-spot-to-log**: spot → fills entry form + tunes rig. Already
  works backend-wise; UI should make this fast.
- **Find + bulk-action**: QsoSearch builder UI → result table with
  checkboxes → bulk action. The backend is ready (`count_matching`,
  `search_full_qsos`, `bulk_*_by_search`).
- **SO2R focus**: when there are 2 rigs, the entry form + rig set commands
  + OTRSP switch all need to align. Probably wants a "Radio 1 / Radio 2"
  toggle that drives all three.
- **Awards drill-down**: click a DXCC count → see which entities, which
  are unworked. The data is in `awards.dxcc.units` already.

### What's worth designing carefully

- The **operating-session** notion is the architectural centerpiece per
  the original plan. Most current UI doesn't surface it; you may want
  session-aware views (e.g., "QSOs from this session", "switch session").
- **Configuration UX** — every external service is gated by TOML editing.
  A settings panel could wire `app-config` write paths.
- **Setup vs running** — first-run experience (no cty.dat, no config) vs
  steady-state operating. The current "everything ungated runs silently"
  boot is a feature; UI should preserve that low-friction first-run.

---

## Tests + verification

123 tests at this writing. Per-crate counts:

- `app-config`: 18
- `app-persistence`: 19 (includes search/sort/bulk-by-search)
- `app-ui`: 4
- `clublog-sync`: 3
- `eqsl-sync`: 4
- `hrdlog-sync`: 3
- `keyer-control`: 1
- `logbook-domain`: 12
- `lotw-sync`: 7
- `qrz-sync`: 4
- `radio-core`: 20
- `rig-control`: 5
- `so2r-control`: 3
- `spot-feed`: 4
- `station-resolver`: 9
- `wsjtx-bridge`: 7

Hardware-dependent paths (rig connect, keyer connect, OTRSP connect, real
LotW upload, real cluster spots) are **not** integration-tested in CI. They
require real hardware / network and surface bugs at first-use.
