# Using Logbook of the World (LoTW) with slogger

slogger can upload your QSOs to LoTW, download confirmations, and fill in
details the other station gave LoTW. This guide covers setup, day-to-day
use, how to read the status messages, and what to do when something is
held back.

> **Status:** this was built from ARRL's and TQSL's documentation and
> tested against stand-ins. It has not yet been run against real TQSL or
> the real LoTW service. Expect rough edges on first use; the
> [troubleshooting](#troubleshooting) section lists the likeliest ones.

## The short version

1. Install TQSL and make sure it can sign logs on this computer.
2. Put your LoTW website login in slogger's config file.
3. In slogger, give each of your station locations a **TQSL location**.
4. Press **Update all services** in the QSL tab to upload and sync.

## How slogger and TQSL fit together

LoTW only accepts logs signed by ARRL's **TQSL** program. TQSL keeps its
own list of **Station Locations**: each has a callsign, grid, DXCC entity,
zones, state and county. When TQSL signs a QSO, it stamps it with one of
those locations. That stamp is what LoTW uses for awards, so signing with
the wrong location puts the wrong grid or county on the QSO.

slogger also has station locations: every QSO belongs to one (or to none).
The two lists are separate, so slogger needs to know, for each of its
locations, which TQSL location to sign with. You set that once per
location. slogger never guesses:

> **A QSO is only sent to TQSL when slogger knows exactly which TQSL
> location it belongs to.** Anything else is *held back*: it stays
> pending and nothing is sent.

## Before you start

- **TQSL is installed** and you have a valid callsign certificate loaded
  in it. If you can sign and upload a log from TQSL itself, you're ready.
- **Your TQSL Station Locations exist.** Create or check them in TQSL
  (*Station Location* menu). slogger reads TQSL's list but cannot create
  or change TQSL locations.
- **Your certificate has no passphrase.** slogger does not pass one to
  TQSL. If your certificate is passphrase-protected, signing will fail
  with `Invalid passphrase (15)`.
- **You know your LoTW website username and password**, the ones you use
  at lotw.arrl.org. These are for downloading confirmations.

## One-time setup

### 1. Config file

Add a `[lotw]` section to `config.toml` (`~/.config/slogger/` on Linux,
`%APPDATA%\slogger\` on Windows):

```toml
[lotw]
username = "your-lotw-login"
password = "your-lotw-website-password"
```

Optional settings:

| Setting | When you need it |
|---|---|
| `tqsl_path` | TQSL is not on your `PATH`. Give the full path to the program. On Windows use single quotes: `'C:\Program Files (x86)\TrustedQSL\tqsl.exe'`. |
| `tqsl_station_data` | slogger can't find TQSL's location list. Give the full path to TQSL's `station_data` file. |

The password is stored in plain text in this file. An old setting,
`station_location`, is ignored; you can delete it.

Uploading does not need the username and password. Downloading
confirmations does.

### 2. Give each station location a TQSL location

In the bottom bar, choose a station in the **STATION** selector and press
**Edit** (or **New** to create one). In the dialog:

- **TQSL location** lists the Station Locations slogger found in TQSL.
  Pick the one this slogger location should be signed with.
- If a TQSL location has exactly the same name as the slogger location,
  it is already selected for you. It is only stored when you press
  **Save**.
- **Fill from TQSL** copies the callsign, grid, county, state, zones and
  IOTA from the chosen TQSL location into any fields that are still
  empty. It never overwrites what you've typed. This is the quick way to
  complete locations created by an ADIF import, which arrive with only a
  name and callsign.
- A warning appears if the TQSL location's callsign or grid disagrees
  with the form. It doesn't stop you saving.
- Press **Save**. Clicking outside the dialog does not close it; use
  **Cancel**, the × button or Esc to leave without saving.

Repeat for every station location you want to upload from.

If the TQSL list can't be read, the field becomes a plain text box and
the dialog shows the path it tried. Type the TQSL location's name exactly
as it appears in TQSL, or set `tqsl_station_data` in the config.

### 3. QSOs with no station location

QSOs imported without a QTH have no station location and will never be
uploaded until you give them one:

1. In the **Logbook** tab's Search pane, set **Location** to
   **None (unassigned)** and search. Narrow it further (dates, band,
   call) if those QSOs came from different places.
2. In the Results pane, choose a location next to **Station location:**
   and press **Assign to all N**. This applies to every QSO matching the
   search, not just the rows shown.

slogger will not assign a location for you.

## Day-to-day use

### Uploading and syncing

In the **QSL** tab, press **Update all services**. For LoTW this does two
things in order:

1. **Upload.** Pending QSOs are grouped by station location and by the
   callsign you used, and each group is signed with its TQSL location and
   sent. Up to 500 QSOs go per run; the rest go next time.
2. **Sync.** slogger asks LoTW what has changed since the last sync: new
   confirmations, and uploads LoTW has accepted.

A QSO moves through these states:

| State | Meaning |
|---|---|
| pending | Not yet sent, or held back. |
| uploaded | Signed and sent; LoTW hasn't been seen to hold it yet. |
| verified | A later sync found the QSO in your LoTW account. |
| confirmed | The other station's log matched yours. |

The Logbook detail pane shows a QSO's state under **QSL**.

### Portable and prefixed callsigns

If you operate as, say, `N9UNX/P` from one of your usual locations, you
don't need a separate slogger location. slogger signs those QSOs with the
same TQSL location but asks TQSL to use the certificate for the callsign
on the QSO. You do need a certificate for that callsign in TQSL; without
one, that group fails and the others still go.

A call that puts you in a different DXCC entity (such as `I3/N9UNX`)
needs its own location, in both slogger and TQSL.

### Full LoTW resync

The first sync downloads your whole LoTW account. After that, each sync
is small. Press **Full LoTW resync** in the QSL tab to download
everything again. Use it:

- once after first setting up, so QSOs that were already confirmed get
  their details filled in (see below);
- after restoring or re-importing your database;
- when a QSO seems stuck at "uploaded" long after LoTW has processed it.

It only syncs; it doesn't upload.

## What a sync changes in your log

Besides QSL state, a confirmation from LoTW carries what the other
station told LoTW about its location. slogger uses that carefully:

- **An empty field is filled.** This covers DXCC entity, continent, CQ
  zone, ITU zone, IOTA, grid, state, county and WPX prefix.
- **A short grid is made more precise.** `EM69` becomes `EM69xv` if
  that's what LoTW has.
- **A value you already have is never changed.** If LoTW disagrees, your
  log keeps its value and slogger records the difference.
- **Award credit** that LoTW reports (DXCC, WAS and so on) is recorded
  against the QSO and shows in the Awards pane's credited counts.

Because awards are worked out from these fields, filled-in values start
counting straight away. A state filled in from LoTW counts toward WAS.

### Seeing what came from LoTW

Select a QSO in the Logbook tab. The detail pane's **FROM LOTW** section
lists:

- fields that were filled, for example `STATE  IN`;
- fields where LoTW disagrees, for example
  `STATE  LoTW says IN, log has IL`.

To list every QSO where LoTW disagrees, set the Logbook search's LoTW
filter to **Differs from LoTW**.

Zone differences are common and usually mean LoTW is right: a log's CQ
and ITU zones are often defaults for the callsign prefix, and the
station's real zone can differ.

There is no "accept LoTW's value" action yet, and only call, band, mode,
frequency and the two reports can be edited in slogger today. So most
differences can be seen but not yet corrected here.

## Reading the status messages

After **Update all services**, the status line and the QSL tab show a
line for the upload and one for the sync.

**Upload**, for example:

```
120 uploaded · 14 held back (9 no station location, 5 "Versailles-House" has no TQSL location)
```

| Part | Meaning |
|---|---|
| `N uploaded` | Signed and sent this run. |
| `N already at LoTW` | TQSL reported these as uploaded before; slogger marked them uploaded without sending. |
| `N more next run` | Ready to go but over the 500-per-run limit. |
| `N held back (…)` | Not sent. The reasons are listed; see below. |
| `N failed (…)` | A group TQSL refused, with TQSL's reason. Those QSOs stay pending. |

**Held-back reasons and fixes:**

| Reason | Fix |
|---|---|
| `no station location` | Assign one: see [QSOs with no station location](#3-qsos-with-no-station-location). |
| `"X" has no TQSL location` | Edit station location X and choose its TQSL location. |
| `"X": TQSL has no location "Y"` | X is mapped to a name TQSL doesn't have. Create Y in TQSL, or edit X and pick another. |
| `at "X" without a station callsign` | Those QSOs have no station callsign, so slogger can't tell TQSL which certificate to use. |

**Sync**, for example:

```
3 new confirmations · 12 uploads verified · 1 not in log · 41 QSOs filled in from LoTW · 3 differ from LoTW
```

| Part | Meaning |
|---|---|
| `full resync` | This was a full download, not an incremental one. |
| `N new confirmations` | QSOs newly confirmed by the other station. |
| `N uploads verified` | QSOs LoTW now holds in your account. |
| `N not in log` | LoTW has QSOs that slogger doesn't. Normal if you've uploaded from another program. |
| `N QSOs filled in from LoTW` | QSOs that had at least one empty field filled. |
| `N differ from LoTW` | QSOs with a newly found disagreement. Repeating a sync doesn't count them again; the search filter lists them all. |

## Troubleshooting

**"report looks incomplete"** on every sync. slogger checks that LoTW's
download ended with its end-of-file marker and discards the download if
not. ARRL describes that marker as deprecated, so if LoTW has stopped
sending it, this check needs relaxing in slogger. Please report it.

**A message about username or password.** LoTW rejected the login. Check
`username` and `password` in the config; they are your LoTW *website*
credentials, not anything to do with your certificate.

**Nothing uploads and the message says no station location has a TQSL
location set.** No location has been mapped yet. See
[step 2](#2-give-each-station-location-a-tqsl-location).

**The TQSL location list is empty or "could not be read".** slogger looks
for TQSL's `station_data` file in `%APPDATA%\TrustedQSL\` on Windows and
`~/.tqsl/` elsewhere. If yours is somewhere else, set `tqsl_station_data`
in the config.

**`Invalid passphrase (15)`.** Your certificate has a passphrase, which
slogger can't supply. Remove the passphrase in TQSL.

**A group fails with a certificate or DXCC error.** TQSL has no
certificate for the callsign on those QSOs, or that certificate's DXCC
entity doesn't match the TQSL location's. Fix it in TQSL; the QSOs stay
pending and will be retried.

**A QSO stays "uploaded" and never becomes "verified".** Run **Full LoTW
resync**. If it still doesn't verify, LoTW didn't accept it; check your
LoTW account's upload activity.

**"1 not in log" appears on every sync.** The newest QSO in your LoTW
account isn't in slogger, and each sync re-reads it. Harmless.

**A corrected QSO still shows under "Differs from LoTW".** The detail
pane updates as soon as you edit, but the search filter only catches up
when LoTW next sends that QSO's record. A Full LoTW resync refreshes it.

## Limits to be aware of

- QSOs that TQSL skips for being outside a certificate's date range are
  still marked uploaded along with the rest of their group, and won't
  verify.
- Uploads carry a fixed set of fields: the QSO itself and where the other
  station was. Names, addresses, email, comments and contest exchanges
  are never sent.
- In the station location dialog, the Tab key doesn't move between
  fields; click each one.
