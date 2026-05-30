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
| Database | `~/.local/share/slogger/slogger.sqlite` | Logbook + service state + sessions + station_locations. Auto-created at first launch. |
| Country file | `~/.local/share/slogger/cty.dat` | DXCC entity database. Optional; resolver runs in NoOp mode if absent. |

Paths use `dirs::config_dir()` and `dirs::data_local_dir()`, so they
match XDG on Linux, `~/Library/Application Support/...` on macOS, and
`%APPDATA%\...` on Windows.

---

## Always-on capabilities

These run regardless of configuration.

### SQLite logbook

The `slogger.sqlite` database is auto-created at first launch with
schema migration `0001_initial.sql` applied. The schema covers QSOs,
exchange fields, notes, attachments, provenance, station_locations,
operators, operating_sessions, service accounts + state + sync_jobs,
award definitions + progress + credit imports + links.

All writes go through `LogbookService` and the repository traits.
Writes are awaited synchronously from the UI; no batched commits.

### Operating session

A new session is started at every boot:
- Closes any leftover sessions with `ended_at IS NULL` from prior runs
- Starts a fresh session named "slogger session"
- Stamps every newly-logged QSO with the session's id

If station_locations exist, the first one is the active location and
gets stamped on the session. If none exist, the session is location-less
until the operator creates one.

### ADIF I/O

- `parse_adif(text)` — read an ADIF string (file content, WSJT-X UDP
  payload, etc.) into `Vec<CreateQsoCommand>`. Maps the standard ADIF
  fields onto first-class QSO columns; unknown ADIF fields land in
  `qso_exchange_fields` with `FieldSource::ImportedAdif` so they survive
  round-trip.
- `export_adif(qsos, opts)` — write a `Vec<Qso>` to ADIF text. Handles
  band/mode/freq/RST/DXCC/zones/grid/state/county/IOTA/power/propagation.

### Awards (derived)

`logbook_domain::snapshot(qsos, year)` computes DXCC/WAS/WPX/IOTA/Marathon
on demand from the QSO log. No materialized table; recomputed every
refresh. Per-band DXCC breakdown also produced.

### Search / bulk

`LogbookService::search_qsos`, `search_full_qsos`, `count_matching`, and
`bulk_*_by_search` are always available. No config needed.

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
station_location = "Home"
tqsl_path = "/usr/bin/tqsl"
```

| Field | Required | Default | Notes |
|---|---|---|---|
| `username` | for fetch | none | LotW website login (NOT the cert). |
| `password` | for fetch | none | Plaintext. |
| `station_location` | for upload | none | TQSL station-location *name*, must exist in your tqsl install. |
| `tqsl_path` | no | `"tqsl"` (PATH) | Override only if tqsl isn't on PATH. |

**Upload path**: ADIF → `tqsl -d -a all -l <name> -x -o out.tq8 in.adi` →
HTTPS POST `out.tq8` to `lotw.arrl.org/lotw/upload`.

**Fetch path**: GET `lotw.arrl.org/lotwuser/lotwreport.adi` with
`login=...&password=...&qso_query=1&qso_qsldetail=yes` → ADIF response →
match against local QSOs by `station_callsign + call + date + band + mode`
→ `mark_upload_verified` for matches; `mark_confirmed` if `QSL_RCVD=Y`.

Single Update click does upload-then-fetch. The fetch endpoint reflects
both verification (LotW has the QSO) and confirmation (other station
matched) in one round-trip.

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
  "band": { "allow": ["20m", "40m"] }
}
```

---

## What boot does

In order:

1. Open SQLite at `~/.local/share/slogger/slogger.sqlite`. Apply
   migrations.
2. Build resolver: try `cty.dat`; fall back to `NoOpResolver` with a
   warning log.
3. Load `~/.config/slogger/config.toml`. Missing file = `Config::default()`
   (everything off).
4. Close orphan operating_sessions (`ended_at IS NULL` from prior runs).
5. Start a new operating_session.
6. If `[dxcluster]` configured: spawn dxfeed adapter, stash receiver.
7. If `[wsjtx].enabled`: bind UDP listener.
8. For each `[[rig]]`: connect (initial sync), spawn forwarder, store
   `RigEntry`.
9. If `[keyer].enabled`: connect, store handle.
10. If `[so2r].enabled`: connect, store handle.
11. Render iced window with current state.

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
