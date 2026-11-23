# Library options review

Status: analysis only, nothing here is implemented. Written 2026-11-23.

## Question

slogger depends on several of our own libraries (riglib, winkey, otrsp,
dxfeed, station-data). Each offers more than slogger exposes. Is there a
reason not to support everything they have?

## Short answer

Mostly no, with one distinction. What the libraries offer falls into
three kinds, and only the first is really "options":

1. **Setup options** (how to connect to and configure a device). Cheap to
   expose as config keys. No good reason to leave them out.
2. **Operations and data** (set power, read SWR, callsign suggestions).
   These are features that need somewhere in the interface to live, so
   each is a design decision, not a config key.
3. **Plumbing** (test hooks, raw overrides, internal buffer sizes). A few
   should stay hidden because they overlap with something slogger already
   does, or would only let the operator break it.

## Revisions reviewed

| Library | Revision slogger builds against | Notes |
|---|---|---|
| riglib | `b8adc13` | same as local checkout |
| winkey | `6587b70` | same as local checkout |
| otrsp | `b8e150a` | same as local checkout |
| dxfeed | `313081e` | local checkout is one commit ahead (`ba55d75`, a plan-document update only) |
| station-data | `22890bf` | same as local checkout |

Re-check these before acting on this document; the libraries will move.

## 1. Setup options not yet exposed

Roughly 50 new config keys in total, most of them in winkey and dxfeed.

### winkey (`[keyer]`)

Exposed today: `serial_port`, `initial_wpm`, `sidetone`.
Wrapper: `keyer-control/src/lib.rs` (`build_keyer`). Library:
`winkey/src/builder.rs`.

| Builder method | What it sets | Library default |
|---|---|---|
| `paddle_mode` | iambic A, iambic B, ultimatic, bug | iambic B |
| `swap_paddles` | swap dit and dah paddles | off |
| `contest_spacing` | shorter word spacing | off |
| `auto_space` | automatic letter spacing | off |
| `sidetone(hz)` | sidetone pitch, 500 to 4000 Hz | 800 |
| `weight` | keying weight, 10 to 90 | 50 |
| `dit_dah_ratio` | 33 to 66 (50 is 3:1) | 50 |
| `farnsworth` | Farnsworth speed, 0 disables | 0 |
| `ptt_lead_in_ms` | PTT lead-in, 10 ms steps | 0 |
| `ptt_tail_ms` | PTT tail, 10 ms steps | 0 |
| `min_wpm` | bottom of the speed-pot range | 10 |
| `wpm_range` | width of the speed-pot range | 25 |
| `prefer_wk3` | use WK3 mode when the hardware has it | on |
| pin config: `PTT_ENABLE` | keyer drives PTT | on |
| pin config: `KEY_OUTPUT`, `KEY_OUTPUT_2` | which key output(s) are driven | output 1 |
| pin config: `HANG_TIME_0/1` | paddle hang time | 0 |

Note: slogger clamps `initial_wpm` to 5..50; the library accepts 5..99.

### riglib, serial rigs (`[[rig]]`)

Exposed today: `vendor`, `model`, `serial_port`, `baud_rate`, `label`,
`transceive`. Wrapper: `rig-control/src/lib.rs` (`build_rig`). Library:
`riglib/crates/riglib-<vendor>/src/builder.rs`.

| Builder method | Vendors | What it sets |
|---|---|---|
| `civ_address` | Icom | CI-V address when it differs from the model default |
| `collision_recovery` | Icom | CI-V bus collision handling (default on) |
| `command_timeout` | all | how long to wait for a reply |
| `auto_retry`, `max_retries` | all serial | retry failed commands |
| `ptt_method` | all serial | CAT, RTS or DTR PTT |
| `key_line` | all serial | which serial line keys CW |
| `set_command_mode` | Yaesu, Kenwood, Elecraft | how set commands are confirmed |
| `audio_device` | all serial | name of the rig's audio device |

### riglib, FlexRadio (`[[rig]]` with `vendor = "flex"`)

Exposed today: `model`, `host`.

| Builder method | What it sets |
|---|---|
| `tcp_port`, `udp_port` | non-default SmartSDR ports |
| `auto_create_slices` | create a slice if none exists |
| `command_timeout` | how long to wait for a reply |

### otrsp (`[so2r]`)

Exposed today: `serial_port`, `initial_tx`, `initial_rx_mode`.
Wrapper: `so2r-control/src/lib.rs`. Library: `otrsp/src/builder.rs`.

| Builder method | What it sets |
|---|---|
| `query_name` | whether to ask the device for its name at connect |

That is the only setup option the library has beyond the port.

### dxfeed (`[dxcluster]`)

Exposed today: `my_callsign`, `sources` (host and port), `filter_file`.
Wrapper: `spot-feed/src/lib.rs`. Library: `dxfeed/src/feed.rs`
(`DxFeedBuilder`), `dxfeed/src/source/cluster.rs`,
`dxfeed/src/aggregator/`, `dxfeed/src/skimmer/config.rs`.

| Area | Settings |
|---|---|
| Per cluster source | `password`, `login_timeout`, `read_timeout`, `originator_policy` |
| Reconnect | `set_backoff` (backoff timings) |
| Spot table | spot lifetime (`spot_ttl`), `max_spots`, `max_observations_per_spot` |
| Matching | match tolerance per mode (`cw_hz`, `ssb_hz`, `dig_hz`), frequency bucket per mode (`cw_bucket_hz`, `ssb_bucket_hz`, `dig_bucket_hz`) |
| Dedupe | `emit_updates`, `min_update_interval`, `detect_qsy` |
| Other aggregator | `mode_policy`, `callsign_norm` |
| Skimmer quality | `enabled`, `compute_valid`, `compute_busted`, `compute_qsy`, `gate_skimmer_output`, `allow_valid`, `allow_qsy`, `allow_unknown`, `allow_busted`, `valid_required_distinct_skimmers`, `valid_freq_window_hz`, `lookback_window` |

dxfeed has one source kind today (`SourceConfig::Cluster`).

### station-data

No setup options as such. slogger uses the CTY lookup (`CtyDb`) and the
TQSL station-data reader.

## 2. Features, not options

Each of these needs an interface decision before it is worth wiring.

- **riglib.** slogger only reads and sets frequency and mode
  (`get_frequency`, `set_frequency`, `get_mode`, `set_mode`,
  `subscribe`). The `Rig` trait also has: passband, PTT, power, S-meter,
  SWR, ALC, split, TX receiver, RIT and XIT, antenna, AGC, preamp,
  attenuator, VFO swap and A=B, CW key, CW speed, and sending and
  stopping CW messages through the rig. Events slogger ignores: PTT,
  split, S-meter, power and SWR readings.
- **otrsp.** Aux outputs (`set_aux`, `query_aux`) are in slogger's
  wrapper but nothing in the interface drives them. Also unused:
  `device_name`, `send_raw`, switch capabilities.
- **station-data.** Super check partial (callsign suggestions while
  typing, including N+1) and call history are unused.
- **dxfeed.** Changing the filter and the spot settings while running
  (`update_filter`, `update_aggregator_config`, the `set_*` methods on
  the running feed), and feed metrics.

## 3. What to keep hidden

- **Overlaps with slogger's own behaviour.** riglib's
  `reconnect_on_drop` (slogger has its own reconnect loop in
  `rig-control`), the Flex `client_name` (slogger sets "slogger"), and
  winkey's raw `pin_config` byte (better exposed as named keys, the way
  `sidetone` is).
- **Internals.** dxfeed's `source_channel_capacity`,
  `event_channel_capacity` and `tick_interval`; dxfeed's
  `entity_resolver` and `enrichment_resolver`; the builders' test-only
  constructors (`build_with_port`, `build_with_client`); Flex `radio()`
  (takes a discovery result, not a config value).

## Costs to weigh

- **Untested hardware paths.** Every key can be wired through and its
  parsing unit-tested, but only the operator can confirm each one does
  the right thing on the WinKeyer, the rig and the SO2R box.
- **Validation.** Each key needs a range check and a clear error, in the
  same style as the theme-file warnings (log plus status bar).
- **The example config grows.** The test
  `example_config_shows_every_option` in `app-config/src/lib.rs` forces
  every key to be listed in `docs/example-config.toml`. That is the
  quick reference we want, but the keyer and cluster sections will get
  long.

## Recommendation

Expose all the setup options in section 1, minus the items in section 3,
and treat section 2 as a separate backlog to pick from.

Open choice: one change, or library by library so each can be tested on
hardware as it lands. Library by library is the safer order; winkey
first, since it has the most keys an operator would actually want
(paddle mode, swap, weight, PTT lead-in and tail).

## Related finding: polling and front-panel presses

Found while investigating missed FILTER presses on an IC-7610 (see the
`transceive` notes in `docs/example-config.toml`):

- With `transceive = false`, slogger polls frequency and mode at 4 Hz.
- On Icoms with a sub receiver (IC-7610, 7800, 7850/51, 9700), riglib
  sends a "select MAIN receiver" command before every read
  (`select_receiver` in `riglib/crates/riglib-icom/src/rig.rs`). Each
  poll tick is therefore four exchanges, two of them writes: about 16 a
  second.
- Not determined: whether the missed presses come from bus load in
  general or from the select-MAIN write specifically. Telling them apart
  needs a riglib change (skip the select when the main receiver is
  already the target) and a test on the radio.
- `transceive = true` avoids the problem by not polling at all.
