# Spot filter examples

The DX cluster feed can be filtered by a JSON file you write by hand.
This guide is a set of examples to copy and combine. Each one is also a
real file in [`docs/spot-filters/`](spot-filters/), and every "shows /
hides" statement below is checked by a test (`spot-feed/tests/filter_examples.rs`)
that runs sample spots through the same filter code slogger uses.

## Setting it up

Put your filter files in a folder named `spot-filters` beside
`config.toml`, one `.json` file per filter:

| | Filter folder |
| --- | --- |
| Windows | `%APPDATA%\slogger\spot-filters` |
| Linux | `~/.config/slogger/spot-filters` |

Create the folder if it is not there, and copy in whichever of the
example files you want. To keep them somewhere else, set `filter_dir` in
the `[dxcluster]` section of `config.toml`.

### Choosing a filter

The Spots panel has a **Filter** list at the top. It offers "No filter"
and every file in the folder, by file name without the `.json`. Pick one
and it takes effect straight away, with no restart:

- The cluster spots on screen are cleared, since they were chosen by the
  old filter, and slogger reconnects to the cluster with the new one. The
  panel refills as spots arrive. POTA spots stay.
- slogger remembers the choice and starts with it next time.
- The list is read from the folder each time you open it, so a file you
  have just copied in appears without a restart.

**Reload** re-reads the filter in force. Use it after editing that file.

If the file you pick is not a valid filter, the bottom bar says what is
wrong (with the line and column for a JSON mistake) and the filter you
had stays in force.

### The filter slogger starts with

The first time, before you have ever used the list, slogger starts with
the file named by `filter_file` in `config.toml`, or with no filter if
that is not set:

```toml
[dxcluster]
filter_file = 'C:\Users\you\AppData\Roaming\slogger\spot-filters\cw.json'
```

On Windows use single quotes, so the backslashes are taken literally.
Once you have picked from the list, that choice is used instead.

If the filter slogger starts with cannot be used (the file has gone, or
is not valid), slogger shows **no cluster spots** and a warning in the
bottom bar, until you pick another filter or fix the file and press
Reload. It never falls back to showing everything.

### Checking what is in force

- The Filter list shows the current filter.
- Hovering a cluster spot shows the mode rule in force at the end of the
  tooltip, such as `filter: CW only (C:\...\cw.json)`.
- A key slogger does not recognise, in `config.toml` or in the filter
  file, is reported as a warning in the bottom bar. A misspelt key is
  otherwise ignored, which looks exactly like a filter that does nothing.

The filter applies to cluster spots only. POTA spots have their own
`bands` and `modes` settings in the `[pota]` section of `config.toml`.

## How a filter file is read

JSON has no comments, so each example below is followed by a table that
explains its lines. Four rules cover everything:

1. **Leave out what you don't care about.** An empty file, `{}`, shows
   every spot. Each section you add can only take spots away.
2. **Sections combine with AND.** A spot is shown only if it passes
   `rf` and `dx` and `spotter` and so on.
3. **Entries in one list combine with OR.** `"band_allow": ["B40", "B20"]`
   means 40 m or 20 m.
4. **Deny beats allow.** A spot matching a `..._deny` list is hidden even
   if an `..._allow` list also matches it. An empty or absent allow list
   means "anything".

The sections that work in slogger:

| Section | Filters on |
| --- | --- |
| `rf` | band, mode, frequency |
| `dx` | the callsign that was spotted |
| `spotter` | who sent the spot: human or skimmer, their callsign, which cluster node |
| `content` | the spot's comment text |
| `correlation` | how many different stations have spotted it |
| `skimmer` | signal strength, speed and CQ, for skimmer spots only |
| `geo` | continent, CQ zone, ITU zone and entity, of the spotted station and of the spotter (needs `cty.dat`) |

Values you will need:

| | Allowed values |
| --- | --- |
| Bands | `"B160"` `"B80"` `"B60"` `"B40"` `"B30"` `"B20"` `"B17"` `"B15"` `"B12"` `"B10"` `"B6"` `"B2"` |
| Modes | `"CW"` `"SSB"` `"DIG"` `"AM"` `"FM"` `"Unknown"` |
| Continents | `"AF"` `"AN"` `"AS"` `"EU"` `"NA"` `"OC"` `"SA"` |
| Zones | plain numbers: `[14, 15]` |
| Frequencies | whole hertz, as a `[low, high]` pair, both ends included: `[14000000, 14030000]` |

Spelling and capitals matter: `"20m"` or `"cw"` is an error, and the
file is refused.

### Where a spot's mode comes from

This explains most surprises.

- **A skimmer spot** carries its mode in the comment (`CW 18 dB 25 WPM CQ`,
  `FT8 -5 dB`), and that is used. Every digital mode is `"DIG"`.
- **A spot from a person** carries no mode. slogger assigns one from the
  frequency, using a simple table: for example 14.000 to 14.100 MHz is CW,
  14.100 to 14.112 is DIG, and 14.112 up is SSB. The CW ranges are
  generous on purpose, so contest CW high in the band is not lost.

The consequence: someone spotting an FT8 station on 14.074 produces a
spot that counts as **CW**. A plain CW-only filter lets it through.
Example 4 deals with that.

## The examples

### 1. CW only

[`01-cw-only.json`](spot-filters/01-cw-only.json)

```json
{
  "rf": {
    "mode_allow": ["CW"]
  }
}
```

| Line | Meaning |
| --- | --- |
| `"rf"` | the band / mode / frequency section |
| `"mode_allow": ["CW"]` | show a spot only if its mode is CW |

Shows CW spots from people and from skimmers. Hides phone and digital
spots. Still shows a person's spot of a station on 14.074, for the
reason above.

### 2. CW, on the bands you have antennas for

[`02-cw-on-chosen-bands.json`](spot-filters/02-cw-on-chosen-bands.json)

```json
{
  "rf": {
    "mode_allow": ["CW"],
    "band_allow": ["B40", "B20", "B15", "B10"]
  }
}
```

| Line | Meaning |
| --- | --- |
| `"mode_allow": ["CW"]` | CW only |
| `"band_allow": [...]` | and only on 40, 20, 15 or 10 m |

Two rules in the same section still combine with AND: the spot must be
CW *and* on one of those bands. To name the bands you don't want
instead, use `"band_deny": ["B60", "B6", "B2"]`.

### 3. Everything except digital

[`03-no-digital.json`](spot-filters/03-no-digital.json)

```json
{
  "rf": {
    "mode_deny": ["DIG"],
    "freq_deny_hz": [
      [1840000, 1843000],
      [3567000, 3578000],
      [7047000, 7051000],
      [7074000, 7077000],
      [10136000, 10143000],
      [14074000, 14083000],
      [18100000, 18107000],
      [21074000, 21077000],
      [21140000, 21143000],
      [24915000, 24922000],
      [28074000, 28077000],
      [28180000, 28183000],
      [50313000, 50321000]
    ]
  }
}
```

| Line | Meaning |
| --- | --- |
| `"mode_deny": ["DIG"]` | hide anything whose mode is digital |
| `"freq_deny_hz": [...]` | also hide anything inside these frequency ranges, whatever its mode |

`mode_deny` alone catches skimmer FT8 spots and human spots in the
segments the table calls digital. The frequency list catches the rest:
human spots on the usual FT8 and FT4 frequencies that the table would
call CW or SSB. The list covers 160 m to 6 m; edit it to suit your
region.

Shows CW and phone on every band.

### 4. CW only, without the FT8 frequencies

[`04-cw-without-ft8-windows.json`](spot-filters/04-cw-without-ft8-windows.json)

The same frequency list as example 3, with `"mode_allow": ["CW"]` in
place of `"mode_deny": ["DIG"]`:

```json
{
  "rf": {
    "mode_allow": ["CW"],
    "freq_deny_hz": [
      [1840000, 1843000],
      [3567000, 3578000],
      ... the rest of the list from example 3 ...
    ]
  }
}
```

(The `...` line is not valid JSON; copy the complete file.)

This is example 1 with the hole closed. It is the one to use if you want
CW and nothing else.

### 5. Spots from people only

[`05-human-spots-only.json`](spot-filters/05-human-spots-only.json)

```json
{
  "spotter": {
    "allow_skimmer": false
  }
}
```

| Line | Meaning |
| --- | --- |
| `"spotter"` | the section about who sent the spot |
| `"allow_skimmer": false` | hide spots sent by skimmers |

A spotter is treated as a skimmer when its callsign ends in `-#` or in a
dash and one or two digits (`K3LR-#`, `W3LPL-2`). Everyone else is a
person. The opposite setting is `"allow_human": false`.

### 6. Skimmer spots only: calling CQ, strong, at a speed you can copy

[`06-skimmer-cq-strong.json`](spot-filters/06-skimmer-cq-strong.json)

```json
{
  "spotter": {
    "allow_human": false
  },
  "skimmer": {
    "require_cq": true,
    "snr_db": [10, 127],
    "wpm": [15, 35]
  }
}
```

| Line | Meaning |
| --- | --- |
| `"allow_human": false` | hide spots sent by people |
| `"skimmer"` | rules that apply to skimmer spots only |
| `"require_cq": true` | the skimmer heard the station calling CQ |
| `"snr_db": [10, 127]` | signal 10 dB or more at the skimmer (127 is simply "no upper limit") |
| `"wpm": [15, 35]` | sending between 15 and 35 words per minute |

The `skimmer` rules never hide a spot sent by a person. Without the
`spotter` part, this file would show every human spot plus the skimmer
spots that qualify. Example 13 uses that.

### 7. Only stations that two different spotters have reported

[`07-two-spotters.json`](spot-filters/07-two-spotters.json)

```json
{
  "correlation": {
    "min_unique_originators": 2
  }
}
```

| Line | Meaning |
| --- | --- |
| `"correlation"` | rules about how many reports a spot has |
| `"min_unique_originators": 2` | at least two different spotters must have reported it |

A spot appears when the second spotter reports the same call on the same
frequency. The same spotter repeating itself does not count. This
removes most busted calls and one-off mistakes, at the cost of seeing
rare stations a little later. A person and a skimmer count as two.

### 8. Hide particular callsigns and particular spotters

[`08-hide-calls-and-spotters.json`](spot-filters/08-hide-calls-and-spotters.json)

```json
{
  "dx": {
    "deny": {
      "globs": ["W1AW", "W1AW/*", "*/B"]
    }
  },
  "spotter": {
    "callsign": {
      "deny": {
        "globs": ["N0CALL", "EA5WU-#"]
      }
    }
  }
}
```

| Line | Meaning |
| --- | --- |
| `"dx"` | the section about the callsign that was spotted |
| `"deny": { "globs": [...] }` | hide spots of calls matching any of these patterns |
| `"W1AW"` | exactly W1AW |
| `"W1AW/*"` | W1AW/4, W1AW/KH6 and so on |
| `"*/B"` | any call ending in /B (beacons) |
| `"spotter": { "callsign": { "deny": ... } }` | hide everything sent by these spotters |

A glob is a pattern where `*` stands for any run of characters and `?`
for exactly one. It must match the whole callsign: `"W1AW"` does not
hide W1AWX. Put your own call here to stop seeing yourself.

### 9. A watch list: show only these

[`09-watch-list.json`](spot-filters/09-watch-list.json)

```json
{
  "dx": {
    "allow": {
      "globs": ["P5*", "3Y*", "VK0*", "FT5*", "BS7*"],
      "regexes": ["^K[HL]7"]
    }
  }
}
```

| Line | Meaning |
| --- | --- |
| `"allow": { "globs": [...] }` | show a spot only if its call matches one of these |
| `"P5*"` | any call starting P5 |
| `"regexes": ["^K[HL]7"]` | or matching this regular expression: starts with KH7 or KL7 |

`globs` and `regexes` in the same list combine with OR. A regular
expression matches anywhere in the call unless you anchor it with `^`
(start) or `$` (end). In JSON a backslash in a regular expression has to
be doubled: `"\\d"` for a digit.

An allow list hides everything else, so this is a filter for a second
slogger window or a DXpedition week, not for everyday use.

### 10. Hide spots by their comment

[`10-comments.json`](spot-filters/10-comments.json)

```json
{
  "content": {
    "comment_deny": {
      "globs": ["*pirate*", "*test*"],
      "regexes": ["\\bFT[48]\\b"],
      "case_insensitive": true
    }
  }
}
```

| Line | Meaning |
| --- | --- |
| `"content"` | the section about the comment text |
| `"comment_deny"` | hide a spot whose comment matches any of these |
| `"*pirate*"` | the word anywhere in the comment (a glob must match the whole comment, hence the `*` at both ends) |
| `"\\bFT[48]\\b"` | FT8 or FT4 as a whole word, so "FT891" is not caught |
| `"case_insensitive": true` | PIRATE, Pirate and pirate all match |

Spots with no comment are not affected. Callsign patterns need no
`case_insensitive`, because callsigns are upper-cased before matching;
comments are matched as sent.

Two related switches in the same section: `"cq_only": true` shows only
spots whose comment contains CQ (or that a skimmer heard calling CQ),
and `"suppress_split_qsy": true` hides spots whose comment contains UP,
QSX, SPLIT or QSY. The second one hides most DXpedition spots, so think
before using it.

### 11. Only certain parts of each band

[`11-frequency-windows.json`](spot-filters/11-frequency-windows.json)

```json
{
  "rf": {
    "freq_allow_hz": [
      [1800000, 1840000],
      [3500000, 3530000],
      [7000000, 7030000],
      [14000000, 14030000],
      [21000000, 21030000],
      [28000000, 28030000]
    ]
  }
}
```

| Line | Meaning |
| --- | --- |
| `"freq_allow_hz"` | show a spot only if its frequency is inside one of these ranges |
| `[14000000, 14030000]` | 14.000 to 14.030 MHz, both ends included |

This one shows the bottom of the CW bands, where the DX tends to be.
Bands with no range listed (30 m here) are hidden entirely. Use
`freq_deny_hz`, as in example 3, to cut pieces out instead.

### 12. Spots from one cluster node only

[`12-one-cluster-only.json`](spot-filters/12-one-cluster-only.json)

```json
{
  "spotter": {
    "source_allow": ["cluster-0"]
  }
}
```

| Line | Meaning |
| --- | --- |
| `"source_allow": ["cluster-0"]` | show only spots that arrived from this node |

slogger names the nodes in `[dxcluster] sources` by their position:
`cluster-0` is the first, `cluster-1` the second. `"source_deny"`
does the opposite. Useful when one of your nodes is a firehose and you
want to see what the other one alone gives you.

## By geography

These use slogger's country file. Put a current `cty.dat` (the Big CTY
version from country-files.com) in slogger's data folder, the one that
holds `slogger.sqlite`:

| | Data folder |
| --- | --- |
| Windows | `%LOCALAPPDATA%\slogger` |
| Linux | `~/.local/share/slogger` |

Without it these rules cannot work, and slogger warns at startup: an
allow list would hide every spot and a deny list none.

The `geo` section has two halves with the same settings: `dx` is about
the station that was spotted, `spotter` about whoever sent the spot.

### 15. Only spots from spotters on your continent

[`15-spotters-on-my-continent.json`](spot-filters/15-spotters-on-my-continent.json)

```json
{
  "geo": {
    "spotter": {
      "continent_allow": ["NA"]
    }
  }
}
```

| Line | Meaning |
| --- | --- |
| `"geo"` | the geography section |
| `"spotter"` | rules about where the spotter is |
| `"continent_allow": ["NA"]` | show a spot only if it was sent from North America |

The most useful geography filter: a station heard in Europe is often
inaudible in North America, and this hides those. It applies to skimmers
too, by their callsign.

### 16. Only DX outside your continent

[`16-dx-outside-my-continent.json`](spot-filters/16-dx-outside-my-continent.json)

```json
{
  "geo": {
    "dx": {
      "continent_deny": ["NA"]
    }
  }
}
```

| Line | Meaning |
| --- | --- |
| `"dx"` | rules about where the spotted station is |
| `"continent_deny": ["NA"]` | hide spots of North American stations |

A callsign the country file cannot place passes a deny list (it is not
known to be in North America) but fails an allow list (it is not known
to be anywhere you asked for).

### 17. Zones and entities

[`17-zones-and-entities.json`](spot-filters/17-zones-and-entities.json)

```json
{
  "geo": {
    "dx": {
      "cq_zone_allow": [14, 15, 25],
      "entity_deny": ["England"]
    }
  }
}
```

| Line | Meaning |
| --- | --- |
| `"cq_zone_allow": [14, 15, 25]` | show only stations in CQ zones 14, 15 or 25 |
| `"entity_deny": ["England"]` | but not England, although it is in zone 14 |

`itu_zone_allow` and `itu_zone_deny` work the same way. An entity name
must be spelt exactly as `cty.dat` spells it, capitals included: it is
the text before the first colon on the entity's line in that file. Some
are not what you might guess: `"United States"`, `"Fed. Rep. of Germany"`,
`"European Russia"`, `"Asiatic Russia"`.

## Mixing them

Combining filters is a matter of putting the sections side by side in
one file. Each section narrows what the others let through.

### 13. A CW contest weekend

[`13-contest-weekend.json`](spot-filters/13-contest-weekend.json)

Built from examples 4, 2, 8 and 6:

```json
{
  "rf": {
    "mode_allow": ["CW"],
    "band_deny": ["B60", "B30", "B17", "B12", "B6", "B2"],
    "freq_deny_hz": [ ... the list from example 3 ... ]
  },
  "dx": {
    "deny": {
      "globs": ["W1AW"]
    }
  },
  "skimmer": {
    "require_cq": true,
    "snr_db": [6, 127]
  }
}
```

| Part | From | Effect |
| --- | --- | --- |
| `mode_allow`, `freq_deny_hz` | example 4 | CW, without the FT8 frequencies |
| `band_deny` | example 2, inverted | no WARC bands, 60 m, 6 m or 2 m |
| `dx.deny` | example 8 | not your own call (replace W1AW) |
| `skimmer` | example 6, without the `spotter` part | skimmer spots must be a CQ at 6 dB or better; human spots are unaffected |

Shows: human CW spots on the contest bands, plus skimmer spots of
stations calling CQ with a usable signal. A skimmer spot at 4 dB, or one
that is not a CQ, is hidden. No speed limit, so fast stations stay.

### 14. A quiet evening looking for DX

[`14-dx-evening.json`](spot-filters/14-dx-evening.json)

```json
{
  "rf": {
    "mode_allow": ["CW", "SSB"],
    "band_allow": ["B40", "B30", "B20", "B17"],
    "freq_deny_hz": [ ... the list from example 3 ... ]
  },
  "spotter": {
    "allow_skimmer": false
  },
  "correlation": {
    "min_unique_originators": 2
  },
  "content": {
    "comment_deny": {
      "globs": ["*pirate*"],
      "case_insensitive": true
    }
  }
}
```

| Part | From | Effect |
| --- | --- | --- |
| `rf` | examples 2 and 3 | CW or phone, on four bands, no FT8 frequencies |
| `spotter` | example 5 | people only |
| `correlation` | example 7 | reported by two different people |
| `content` | example 10 | nothing anyone has called a pirate |

A spot has to pass all four sections. A CW station on 20 m spotted by
one person stays hidden until a second person spots it; a station on
80 m never appears, however many people spot it.

### 18. CW DX that someone near you can hear

[`18-cw-dx-heard-near-me.json`](spot-filters/18-cw-dx-heard-near-me.json)

```json
{
  "rf": {
    "mode_allow": ["CW"],
    "freq_deny_hz": [ ... the list from example 3 ... ]
  },
  "geo": {
    "dx": {
      "continent_deny": ["NA"]
    },
    "spotter": {
      "continent_allow": ["NA"]
    }
  }
}
```

| Part | From | Effect |
| --- | --- | --- |
| `rf` | example 4 | CW, without the FT8 frequencies |
| `geo.dx` | example 16 | the station is outside North America |
| `geo.spotter` | example 15 | and it was heard in North America |

A Japanese station spotted from the US is shown. The same station
spotted from Germany is not, and neither is a US station spotted from
the US. Change both `"NA"` to your own continent.

### A way to build your own

1. Start with the smallest file that does the main thing you want,
   usually example 1 or 4.
2. Choose it in the Filter list and watch the Spots panel for a while.
3. When something appears that you don't want, find which example hides
   that kind of spot and copy its section in. If your file already has
   that section (`"rf"`, say), add the new line inside the existing one;
   a section may appear only once.
4. If spots you expected go missing, remove sections one at a time. Allow
   lists and `correlation` are the usual cause.

After each edit, press Reload.

JSON is strict about punctuation. Every key and every text value is in
double quotes, items are separated by commas, and there is no comma
after the last item in a list or section. When slogger refuses a file,
the message gives the line and column.

## Parts of the filter format that do not work in slogger

The filter format comes from the spot library slogger uses, and has
settings slogger cannot act on, because it has no callbook, LoTW-user,
state or grid data to give the library. Avoid them:

| Setting | What happens in slogger |
| --- | --- |
| `geo` `country_allow`, `state_allow`, `grid_allow` | **hides every spot** (use `entity_allow` for countries) |
| `geo` `country_deny`, `state_deny`, `grid_deny` | no effect |
| `enrichment` (`lotw`, `in_callbook`, `membership`, ...) set to require something | **hides every spot** |
| `rf.mode_policy`, `dx.normalization`, `spotter.callsign.normalization` | no effect |

Two top-level settings are best left alone: `max_age_secs` (default 900)
and `freq_sanity_hz` (default 1.8 to 54 MHz). The second one means spots
above 54 MHz, including 2 m, are hidden unless you widen it:
`"freq_sanity_hz": [1800000, 148000000]`.
