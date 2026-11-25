#!/usr/bin/env python3
"""Check that everything in one ADIF file is also in another.

    python3 tools/adif_compare.py ORIGINAL.adi SLOGGER.adi

ORIGINAL is the file that was imported (a DXKeeper export, say); SLOGGER
is what slogger holds now: the .adi of a backup (Logbook tab -> Tools ->
Back up now). QSOs are matched on station callsign, call, date, time,
band and mode, the same key slogger's import uses to spot duplicates.

Reports QSOs in ORIGINAL that SLOGGER lacks, records ORIGINAL holds more
than once under that key (import keeps the first), and, field by field,
values that are missing or different. Numbers compare by value (7.03 =
7.030000) and lists ignore spacing after commas. QSOs only in SLOGGER
(logged since the import) are counted, not treated as a problem.
"""
import collections
import re
import sys

NUMERIC = {
    "FREQ", "FREQ_RX", "LAT", "LON", "DISTANCE", "ANT_AZ", "ANT_EL", "TX_PWR", "RX_PWR",
    "SFI", "A_INDEX", "K_INDEX", "AGE", "STX", "SRX", "CQZ", "ITUZ", "DXCC",
}
LISTS = {"CREDIT_GRANTED", "CREDIT_SUBMITTED"}
TAG = re.compile(rb"<([A-Za-z0-9_]+)(?::(\d+))?(?::[^>]*)?>")


def parse(path):
    data = open(path, "rb").read()
    header_end = data.lower().find(b"<eoh>")
    pos = header_end + 5 if header_end >= 0 else 0
    records, current = [], {}
    while match := TAG.search(data, pos):
        name = match.group(1).decode().upper()
        length = int(match.group(2) or 0)
        if name == "EOR":
            records.append(current)
            current = {}
        else:
            current[name] = data[match.end():match.end() + length].decode("utf-8", "replace")
        pos = match.end() + length
    return records


def key(record):
    time = (record.get("TIME_ON", "") + "00")[:6]
    return (
        record.get("STATION_CALLSIGN", "").upper(), record.get("CALL", "").upper(),
        record.get("QSO_DATE", ""), time, record.get("BAND", "").lower(),
        record.get("MODE", "").upper(),
    )


def normal(name, value):
    value = value.strip()
    if name in NUMERIC:
        try:
            return round(float(value), 6)
        except ValueError:
            pass
    if name in LISTS:
        return sorted(part.strip() for part in value.split(","))
    return value


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    original, held = parse(sys.argv[1]), parse(sys.argv[2])
    first, repeats = {}, 0
    for record in original:
        if key(record) in first:
            repeats += 1
        else:
            first[key(record)] = record
    have = {}
    for record in held:
        have.setdefault(key(record), record)

    absent = [k for k in first if k not in have]
    print(f"original: {len(original)} records, {len(first)} distinct QSOs, {repeats} repeats of a QSO already in the file")
    print(f"slogger:  {len(held)} records, {len(have) - (len(first) - len(absent))} not in the original")
    print(f"QSOs in the original that slogger lacks: {len(absent)}")
    for k in absent[:20]:
        print("   ", " ".join(k[1:]))

    same, missing, differ = collections.Counter(), collections.Counter(), collections.Counter()
    examples = collections.defaultdict(list)
    for k, a in first.items():
        b = have.get(k)
        if b is None:
            continue
        for name, value in a.items():
            if name not in b:
                if value.strip() == "":
                    same[name] += 1
                    continue
                missing[name] += 1
                examples[name].append((value, None))
            elif normal(name, value) != normal(name, b[name]):
                differ[name] += 1
                examples[name].append((value, b[name]))
            else:
                same[name] += 1

    names = sorted(set(same) | set(missing) | set(differ))
    bad = [n for n in names if missing[n] or differ[n]]
    print(f"fields in the original: {len(names)}; fully preserved: {len(names) - len(bad)}")
    if bad:
        print(f"{'field':36}{'same':>8}{'missing':>9}{'differs':>9}")
        for name in bad:
            print(f"{name:36}{same[name]:8}{missing[name]:9}{differ[name]:9}")
            for was, now in examples[name][:3]:
                print(f"      {was!r} -> {now!r}")
    sys.exit(1 if absent or bad else 0)


if __name__ == "__main__":
    main()
