# Features and Configuration

Per-feature reference for slogger. For each capability: what it does, how
to turn it on, what config it needs, and what UI flows it enables (or
blocks if not configured).

For a copy-pasteable starter config, see `example-config.toml`.
For the high-level capability summary aimed at UI design, see
`backend-capability-summary.md`.

---

## File locations

| File | Path | Purpose |
|---|---|---|
| Config | `~/.config/slogger/config.toml` | All feature configuration. Created by hand or via `app_config::Config::write_template`. |
| Database | `~/.local/share/slogger/slogger.sqlite` | Logbook + service state + station_locations. Auto-created at first launch. |
| Country file | `~/.local/share/slogger/cty.dat` | DXCC entity database. Optional; resolver runs in NoOp mode if absent. |

Paths use `dirs::config_dir()` and `dirs::data_local_dir()`, so they
match XDG on Linux, `~/Library/Application Support/...` on macOS, and
`%APPDATA%\...` on Windows.

---

## Always-on capabilities

These run regardless of configuration.

### SQLite logbook

The `slogger.sqlite` database is auto-created at first launch with
schema migrations (`0001_initial.sql`, `0002_dxkeeper_fields.sql`)
applied. The schema covers QSOs,
exchange fields, notes, attachments, provenance, station_locations,
operators, service accounts + state + sync_jobs,
award definitions + progress + credit imports + links.

All writes go through `LogbookService` and the repository traits.
Writes are awaited synchronously from the UI; no batched commits.

### Active station location

At boot the first `station_location` (if any exist) becomes the active
location. Selecting a different location from the dropdown in the bottom
status bar updates it. The active location stamps `station_callsign` and
`station_location_id` onto every newly-logged QSO; if none exist, QSOs
are logged location-less until the operator creates one.

The bar's **Edit** and **New** buttons open the station editor (until the
first location exists the bar shows a single **Create station location**
button instead). It edits every field of a location — name, station and
owner callsign, grid, city, county, state, country, CQ/ITU zone, IOTA,
latitude/longitude — and its **TQSL location**, the TQSL station location
LoTW uploads of its QSOs are signed with:

- With TQSL's `station_data` readable, the TQSL field is a pick list of
  TQSL's names plus "(none)"; a stored name TQSL no longer has is shown
  flagged "not found in TQSL". Without it, the field is free text and the
  editor shows the path it tried.
- An empty TQSL field is preselected with the TQSL location of exactly the
  same name, if there is one — stored only when the operator saves.
- **Fill from TQSL** copies callsign, grid, county, state, zones and IOTA
  from the chosen TQSL location into the form's *empty* fields (how
  locations created by ADIF import, which have only a name and callsign,
  get their details).
- A warning (not a block) appears when the chosen TQSL location's
  callsign or grid disagrees with the form's.

> An earlier revision wrapped this in a per-boot `operating_session`
> entity (start/end/retarget, orphan cleanup, a Sessions pane). It was
> removed — nothing consumed the session id and the active location alone
> supplies the operating context QSOs need.

### ADIF I/O

- `parse_adif(text)` — read an ADIF string (file content, WSJT-X UDP
  payload, etc.) into `Vec<CreateQsoCommand>`. Maps the standard ADIF
  fields onto first-class QSO columns — including the other operator
  (`NAME`, `QTH`, `ADDRESS`, `EMAIL`, `AGE`, `QSL_VIA`, `COMMENT`), the
  contest exchange (`CONTEST_ID`, `STX`/`SRX`, `STX_STRING`/`SRX_STRING`,
  `ARRL_SECT`), the receive side (`BAND_RX`, `FREQ_RX`), `LAT`/`LON`,
  `DISTANCE`, `ANT_AZ`, `SFI`/`A_INDEX`/`K_INDEX`, `PFX` (the WPX prefix)
  and `OPERATOR` / `OWNER_CALLSIGN` (separately). DXKeeper's
  `APP_DXKEEPER_DXCCPREFIX` is the DXCC entity prefix. Unknown ADIF
  fields, and mapped ones whose value doesn't fit the column (a
  non-numeric `STX`), land in `qso_exchange_fields` with
  `FieldSource::ImportedAdif` so they survive round-trip. The QSL-service
  fields stay there too; `import_qsos` derives `qso_service_state` from
  them (`APP_DXKEEPER_QRZCOM_LOGID` → the QRZ row's `remote_id`).
- My station: `import_qsos` links each QSO to the `station_locations` row
  named by `APP_DXKEEPER_MY_QTHID` (or derived from `MY_*` fields),
  creating it on first sight. QSOs whose record names no location are
  left unassigned; the Logbook's Location = None filter plus "Assign to
  all" fixes them up.
- Award credit: `CREDIT_GRANTED` / `CREDIT_SUBMITTED` become
  `award_credit_imports` + `award_credit_links` rows; awards count a unit
  as credited when a QSO for it has granted credit.
- `export_adif(qsos, opts)` — write a `Vec<Qso>` to ADIF text: every QSO
  column (what the service uploads send). `export_adif_records` /
  `LogbookService::export_adif_by_search` also write the station location
  (`APP_DXKEEPER_MY_QTHID` + `MY_*`), credit and extra fields, so an
  export re-imports into an empty log unchanged.

### Awards (derived)

`logbook_domain::snapshot(qsos, year)` computes DXCC/WAS/WPX/IOTA/Marathon
on demand from the QSO log. No materialized table; recomputed every
refresh. Per-band DXCC breakdown also produced.

### Search / bulk

`LogbookService::search_qsos`, `search_full_qsos`, `count_matching`, and
`bulk_*_by_search` (including `bulk_assign_location_by_search`, which
applies to every match regardless of the result limit) are always
available. No config needed.

---

## Optional capabilities

Each gated by a config section. Most default to off so a fresh install
launches without trying to connect to hardware or remote services.

### `[station]` — operator default callsign

```toml
[station]
default_callsign = "W1ABC"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `default_callsign` | no | none | Stamped onto QSOs when no station_location is selected. Fallback only. |

The active station_location (if one exists) overrides this. Useful for
first-launch UX when the user hasn't created a station_location yet.

### `[[rig]]` — rig control (multi-rig supported)

```toml
[[rig]]
enabled = true
vendor = "icom"
model = "IC-7300"
serial_port = "/dev/ttyUSB0"
baud_rate = 115200
label = "Main"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `enabled` | yes | `false` | Off by default so a typoed config doesn't spam the log. |
| `vendor` | yes | none | `icom` / `yaesu` / `kenwood` / `elecraft` / `flex`. |
| `model` | yes | none | Case- and hyphen-insensitive against riglib's per-vendor model tables. Examples below. |
| `serial_port` | for serial vendors | none | e.g. `/dev/ttyUSB0`, `COM3`. Required for icom/yaesu/kenwood/elecraft. |
| `baud_rate` | no | vendor default | Vendor's default baud is used if unset. |
| `host` | for flex | none | Hostname/IP of the FlexRadio. Required when `vendor = "flex"`; ignored otherwise. |
| `label` | no | model | Friendly name (e.g. "Main", "Aux"). Helpful for SO2R. |
| `transceive` | no | `false` | How slogger follows the VFO knob. `false`: poll the rig at 4 Hz. `true`: no polling; listen for the rig's own broadcasts (auto-information on Yaesu/Kenwood/Elecraft, switched on at connect; CI-V Transceive on Icom, which must also be enabled in the rig's menu). |

**Multi-rig**: write multiple `[[rig]]` blocks. The first is `active_rig = 0`
at boot; UI can switch via `Message::ActiveRigChanged`.

**Supported models** (case-insensitive, hyphens optional):

- **Icom**: IC-7300, IC-7300mk2, IC-7610, IC-7600, IC-7700, IC-7800,
  IC-7850, IC-7851, IC-9700, IC-705, IC-7100, IC-9100, IC-7410, IC-905
- **Yaesu**: FT-DX10, FT-891, FT-991A, FT-DX101D, FT-DX101MP, FT-710
- **Kenwood**: TS-590S, TS-590SG, TS-990S, TS-890S
- **Elecraft**: K3, K3S, K4, KX2, KX3
- **FlexRadio**: 6400, 6400M, 6600, 6600M, 6700, 8400, 8600

**Auto-reconnect**: after the first successful connect, slogger reconnects
internally with exponential backoff (1→2→4→8→16→30s, capped at 30s) when
the rig disconnects. `set_*` commands during a disconnect window return
"rig not connected" cleanly. The initial connect fails fast — a typoed
serial port surfaces as `Rig: connect failed — ...` rather than spinning.

**Capabilities**: read freq/mode (live event-driven snapshots), set
freq/mode. PTT/split/audio not yet exposed. FlexRadio is treated as
single-VFO (multi-slice not yet wired).

### `[keyer]` — CW keyer (WinKeyer)

```toml
[keyer]
enabled = true
serial_port = "/dev/ttyUSB1"
initial_wpm = 25
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `enabled` | yes | `false` | |
| `serial_port` | yes when enabled | none | WinKeyer USB serial device. |
| `initial_wpm` | no | 25 | Clamped to 5..50 by the wrapper. |

**Capabilities**: `send_message(text)`, `abort`, `set_wpm(n)`,
`set_tune(bool)`. Snapshot stream surfaces current WPM (operator twiddling
the speed pot) and busy state (CW going out). Subscribes to
`KeyerEvent::CharacterSent` upstream — slogger doesn't surface character
echo to the UI yet.

**Auto-reconnect**: same backoff pattern as rig.

**Keyer pane and message buttons**: the Operating view's CW Keyer pane has
a free-text box (Enter sends), speed and abort controls, and a row of
message buttons. Buttons come from `[[keyer.macro]]` tables, each with a
`label` and `text`; the first twelve are bound to F1..F12. With none
configured a built-in set is used (CQ, His call, Report, TU, My call,
AGN). `text` may use `{MYCALL}`, `{CALL}` (entry-form callsign) and
`{RST}` (entry-form sent report, 9 keyed as N). See `example-config.toml`.

### `[so2r]` — OTRSP SO2R switch

```toml
[so2r]
enabled = true
serial_port = "/dev/ttyUSB2"
initial_tx = 1
initial_rx_mode = "stereo"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `enabled` | yes | `false` | |
| `serial_port` | yes when enabled | none | OTRSP-compatible device (microHAM MK2R, OTRSP Lite, etc.). |
| `initial_tx` | no | 1 | 1 or 2 — which radio gets TX (PTT/key/mic) at boot. |
| `initial_rx_mode` | no | `"mono"` | `"mono"` / `"stereo"` / `"reverse_stereo"` (also accepts `"reverse-stereo"` and `"rev_stereo"`). |

**Capabilities**: `set_tx_radio(1|2)`, `set_rx_audio(radio, mode)`,
`set_aux(port, value)` for BCD band-decoder outputs. Snapshot has TX
radio, RX radio, RX mode.

**Coupling with multi-rig**: not automatic. The operator's `active_rig`
index in slogger doesn't drive `set_tx_radio` automatically. UI redesign
should consider whether to couple them.

### `[wsjtx]` — WSJT-X UDP bridge

```toml
[wsjtx]
enabled = true
bind_addr = "127.0.0.1:2237"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `enabled` | no | `true` | Default-on because UDP binds are cheap and the bridge stays idle until WSJT-X actually sends. |
| `bind_addr` | no | `"127.0.0.1:2237"` | Match WSJT-X's "UDP server" target in Settings → Reporting. Use `0.0.0.0:2237` for cross-machine setups. |

**WSJT-X side**: File → Settings → Reporting → "UDP Server" =
`127.0.0.1`, port `2237`, ☑ Accept UDP requests, ☑ Notify on accepted UDP
request, ☑ Logged QSO ADIF.

**What flows**: on every "Log QSO" click in WSJT-X, slogger receives
message type 12 (Logged ADIF) over UDP, parses the ADIF, and runs it
through `import_qsos` with dedup. So if you bounce between WSJT-X and
ADIF-imported QSOs, duplicates don't pile up.

### `[dxcluster]` — DX cluster spot feed

```toml
[dxcluster]
my_callsign = "W1ABC"
sources = [
    { host = "dxc.kbx.org", port = 7300 },
    { host = "n1nr.org", port = 7300 },
]
filter_file = "/home/me/.config/slogger/spot-filter.json"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `my_callsign` | yes when enabled | none | Login callsign. Clusters reject anonymous logins. |
| `sources` | yes when enabled | empty | Array of `{ host, port }`. Each connects independently; dxfeed handles per-source reconnection. |
| `filter_file` | no | none | Path to a dxfeed `FilterConfigSerde` JSON. Filters spots before they reach the panel. Hand-authored — no UI yet. |

`is_configured()` requires both `my_callsign` AND at least one source.

The active filter file lets you restrict by band/mode/continent/spotter/
RBN-quality without flooding the UI with stuff you don't care about.
Format is dxfeed's own JSON schema; see `dxfeed::filter::config::FilterConfigSerde`.

### Sync services — `[lotw]` `[eqsl]` `[clublog]` `[qrz]` `[hrdlog]`

All five share the same shape: optional config section, `is_configured()`
check, single Update services button drives every configured service in
sequence. Each fails independently.

#### `[lotw]` — Logbook of the World

```toml
[lotw]
username = "W1ABC"
password = "your-lotw-website-password"
tqsl_path = "/usr/bin/tqsl"
tqsl_station_data = "/home/you/.tqsl/station_data"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `username` | for fetch | none | LotW website login (NOT the cert). |
| `password` | for fetch | none | Plaintext. |
| `tqsl_path` | no | `"tqsl"` (PATH) | Override only if tqsl isn't on PATH. |
| `tqsl_station_data` | no | `%APPDATA%\TrustedQSL\station_data` (Windows), `~/.tqsl/station_data` (elsewhere) | TQSL's list of station locations, read by the station editor and the upload. Unreadable = "TQSL's list unknown", not an error. The non-Windows default is unverified on real installs; set this if TQSL keeps the file elsewhere. |
| `station_location` | — | — | **Deprecated, ignored.** Was the one TQSL location every upload was signed with. Still parsed so old files load; a warning is logged at startup when it is set. Set each station location's TQSL location in the station editor instead. |

Upload needs no config beyond TQSL itself: it runs when at least one
station location has a TQSL location set (that, or `username`/`password`
for fetch, is what makes LoTW count as configured for Update services).

**Upload path**: pending QSOs are grouped by (station location, station
callsign), and each group is signed separately:
`tqsl -d -a compliant -l <location's TQSL location> -c <station callsign> -x -o out.tq8 in.adi`
→ HTTPS POST `out.tq8` to `lotw.arrl.org/lotw/upload`. `-c` picks the
certificate for the call the QSOs were made under (e.g. a `/P` from a
regular location); TQSL refuses if it has no such certificate.
`-a compliant` makes TQSL skip QSOs it knows were already uploaded or
fall outside the certificate's dates instead of re-signing them.

Strict mapping — a QSO is never sent to TQSL unless it maps cleanly. It
is **held back** (left pending, not marked failed) when it has no
station location, its location has no TQSL location, TQSL's list is
readable and lacks that name, or it has no station callsign. If TQSL's
list can't be read at all, a name the operator set is used as is (TQSL
refuses unknown names itself).

TQSL's result is read from the last `Final Status: Description (Code)`
line it prints in batch mode, falling back to its exit status: 0 and 9
(some skipped as already uploaded) → post the .tq8 and mark the group
uploaded; 8 and 14 with no output file → TQSL says they were already
uploaded, mark them uploaded without posting; anything else fails that
group only (its QSOs stay pending) and the description is shown. One
group failing doesn't stop the others.

Each run signs at most 500 QSOs, chosen *after* held-back QSOs are set
aside, so unmappable QSOs never crowd out sendable ones; any beyond 500
go on the next run. The one-line summary gives every count, e.g.
`upload: 120 uploaded · 14 held back (9 no station location, 5 "Versailles-House" has no TQSL location)`,
plus "already at LoTW", "more next run" and per-group failures
(`3 failed ("Home" as N9UNX/P: TQSL: Invalid passphrase (15))`) when
they apply.

**Sync path** (incremental): two GETs to
`lotw.arrl.org/lotwuser/lotwreport.adi`, both with
`login=...&password=...&qso_query=1&qso_withown=yes`:

- confirmations — `qso_qsl=yes&qso_qsldetail=yes&qso_qslsince=<watermark>`;
- accepted uploads — `qso_qsl=no&qso_qsorxsince=<watermark>`.

Each report's header carries the value to send next time
(`APP_LoTW_LASTQSL` / `APP_LoTW_LASTQSORX`); slogger stores it verbatim
per report in `service_sync_watermarks` and sends it back, so a routine
sync downloads only what changed. With no stored value (first sync) or on
a full resync it sends `1900-01-01` — never an empty `since`, which LoTW
would replace with a value it remembers server-side. A report's watermark
advances only in the same transaction that stores its results, and only
when it returned records; one report failing doesn't stop the other.
LoTW's comparison is inclusive, so the boundary record comes back each
time; applying a record twice changes nothing.

Matching: worked call + band + station callsign (from `qso_withown`), and
start times within ±5 minutes. Pairs are taken nearest in time first
(ties: same mode, then same mode group), one record per QSO, so two QSOs
with the same station on the same band and day each get their own
record. Mode is never required. Soft-deleted QSOs never match. Every
matched record marks the QSO upload-verified; `QSL_RCVD=Y` marks it
confirmed (`confirmed_at` from `QSLRDATE`, else `APP_LoTW_RXQSL`, else
now). Nothing is downgraded. `CREDIT_GRANTED` / `CREDIT_SUBMITTED` on a
confirmation become award credit on the QSO (added once; submitted raised
to granted later). All changes from one report are one transaction.

Other station's details: a confirmation (`qso_qsldetail=yes`) also says
what the QSLing station told LoTW about its own location. Nine of those
fields map to QSO columns — `DXCC`→`dxcc_id`, `CONT`→`continent`,
`CQZ`→`cq_zone`, `ITUZ`→`itu_zone`, `IOTA`→`iota`, `GRIDSQUARE`→`grid`,
`STATE`→`state`, `CNTY`→`county`, `PFX`→`wpx_prefix` — normalised as the
ADIF importer does (`CONT`/`PFX` upper-cased, numbers as integers, the
rest as given); `COUNTRY`, `VUCC_GRIDS`, `APP_LoTW_NPSUNIT` and the
`*_UserInvalid` zones are not used. Per field, on the matched QSO only:
a blank column is filled with LoTW's value; a grid that is a prefix of
LoTW's (`EM69` vs `EM69xv`) is replaced by the more precise one; an equal
value (case-insensitive, trimmed, numbers numerically, or LoTW's grid a
prefix of the log's) is left alone; a different value is **kept** — the
log may be right, or record what was heard — and the difference is
recorded. Unconfirmed records, and confirmed ones without detail (the
uploads report), change none of this. Filled values count toward awards
like any other, since awards are computed from the columns.

Both outcomes are rows in `qso_field_provenance`, at most one per QSO and
field: `field_name` is the ADIF name above, `source` is
`service_sync:lotw` (filled or refined from LoTW) or
`service_differs:lotw` (LoTW reports something else; log kept), and
`source_detail` is LoTW's value. A resync rewrites a field's row when
LoTW's value changes, drops a "differs" row once the log agrees (e.g. the
operator corrected it) or LoTW stops sending the field, and otherwise
writes nothing. Editing a QSO leaves the rows alone; the detail pane reads
them against the current values. The rows are internal bookkeeping:
ADIF export doesn't include them. The Logbook detail pane lists them under
**FROM LOTW** (`STATE  IN`, or `STATE  LoTW says IN, log has IL`), and the
Logbook search's LotW filter has **Differs from LoTW**.

An HTML page instead of ADIF (LoTW's way of reporting a failed query) is
shown as a short message; records without the closing `<APP_LoTW_EOF>`
are treated as a cut-off download and nothing from that report is
applied. Summary, e.g. `sync: 3 new confirmations · 12 uploads verified ·
1 not in log · 41 QSOs filled in from LoTW · 3 differ from LoTW` ("not in
log" = QSOs at LoTW with no match here, e.g. uploaded from another
logger; the last two count QSOs, not fields, and only new differences,
and appear only when non-zero).

Update services does upload-then-sync. **Full LoTW resync** (QSL tab)
runs the sync alone, ignoring the stored watermarks, for when the log and
LoTW have drifted apart (restored database, QSOs edited after upload).

#### `[eqsl]`

```toml
[eqsl]
username = "W1ABC"
password = "your-eqsl-password"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `username` | yes | none | |
| `password` | yes | none | |

**Upload**: multipart POST to `eqsl.cc/qslcard/ImportADIF.cfm` with
`EQSL_USER`/`EQSL_PSWD`/`ADIFData`.

**Fetch**: GET `DownloadInBox.cfm` → ADIF of confirmed QSOs → match.

#### `[clublog]`

```toml
[clublog]
email = "you@example.com"
password = "your-clublog-password"
callsign = "W1ABC"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `email` | yes | none | Account email (login). |
| `password` | yes | none | |
| `callsign` | yes | none | Whose log this is — Club Log accounts can host multiple callsigns. |

Upload-only. Club Log doesn't expose a public confirmation report.

#### `[qrz]`

```toml
[qrz]
api_key = "ABCD-1234-EFGH-5678"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `api_key` | yes | none | Per-account upload key from your QRZ logbook page. Distinct from QRZ XML subscription credentials. |

Upload-only. QRZ accepts ONE record per request, so the client splits
multi-record ADIF at `<EOR>` boundaries.

#### `[hrdlog]`

```toml
[hrdlog]
callsign = "W1ABC"
code = "your-hrdlog-upload-code"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `callsign` | yes | none | |
| `code` | yes | none | HRDLog "upload code" — distinct from website password. |

Upload-only.

---

## Data files

### cty.dat

Place at `~/.local/share/slogger/cty.dat`. Source: country-files.com (Big
CTY weekly version). Resolver loads at boot; if absent, all entity-related
fields are blank for new QSOs and awards count zero rare entities.

Replacement / improvement path for full coverage: parse ClubLog's
`cty.xml` instead of cty.dat. Currently slogger uses a hand-curated
prefix→entity-ID table covering ~70 prefixes; rare DXCC entities resolve
to `dxcc_id = None`.

### Spot filter file

Format: dxfeed's `FilterConfigSerde` JSON. Path is configurable via
`[dxcluster].filter_file`. No UI yet — hand-authored.

Example filter (drops everything outside NA + 20m/40m):
```json
{
  "geo": { "dx": { "continent_allow": ["NA"] } },
  "rf": { "band_allow": ["B20", "B40"] }
}
```

Unknown keys are ignored without an error, so a misspelt section silently
filters nothing. A ready-made CW-only filter is in
`docs/spot-filter-cw.json`.

---

## What boot does

In order:

1. Open SQLite at `~/.local/share/slogger/slogger.sqlite`. Apply
   migrations.
2. Build resolver: try `cty.dat`; fall back to `NoOpResolver` with a
   warning log.
3. Load `~/.config/slogger/config.toml`. Missing file = `Config::default()`
   (everything off).
4. Load station_locations; the first one (if any) becomes the active
   location.
5. If `[dxcluster]` configured: spawn dxfeed adapter, stash receiver.
6. If `[wsjtx].enabled`: bind UDP listener.
7. For each `[[rig]]`: connect (initial sync), spawn forwarder, store
   `RigEntry`.
8. If `[keyer].enabled`: connect, store handle.
9. If `[so2r].enabled`: connect, store handle.
10. Render iced window with current state.

Failures at any step that targets external hardware/network are logged
as warnings and the feature stays off. The app launches regardless.

---

## What's NOT yet wired (for context when planning UI)

These are real backend gaps. The UI redesign should know about them so it
doesn't promise things the backend can't deliver yet.

- **Operator entity** is in the schema but no service methods read or
  write it. No multi-op support yet.
- **DXCC entity coverage** is partial (~70 prefixes). Rare entities show
  as "?" in awards.
- **Service retry queue** (`service_sync_jobs` table) is empty — failures
  don't auto-retry between Update clicks.
- **PTT visibility** from the rig isn't surfaced (`RigEvent::PttChanged`
  is parsed but dropped).
- **FlexRadio multi-slice** — Flex rigs treated as single-VFO.
- **Audio routing** (sound card, headphones, mic) is OS responsibility,
  not slogger's. SO2R operators wire it externally.
- **No spot filter UI** — `[dxcluster].filter_file` is hand-authored JSON.
- **No bulk undo** — soft_delete is reversible at the DB level (the
  `deleted_at` column) but no `undo_delete` method or undo stack exists.
- **No live decoder** — slogger doesn't show FT8/RTTY/PSK decodes
  itself. WSJT-X handles decoding and slogger just imports the logged
  result.
- **No propagation prediction** — no PropView equivalent.
- **No map view** — no DXView equivalent.
