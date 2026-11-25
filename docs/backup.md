# Logbook backups and restoring one

## What slogger backs up

Backups are on by default. Each one is a pair of files named with the
UTC time it was made:

| File | What it is |
| --- | --- |
| `slogger-20261124-153000Z.sqlite` | A complete copy of the logbook database: QSOs, station locations, confirmations, award credit. This is the file you restore from. |
| `slogger-20261124-153000Z.adi` | Every QSO as ADIF. For reading the log in another program, or as a last resort. |

A backup is made:

- at startup, when the newest backup is at least `interval_days` old
  (default 7) or there is none;
- before every ADIF import (if this backup fails, the import does not run);
- when you press **Back up now** in the Logbook tab's Tools panel, which
  also shows when the last backup was made and the folder it went to.

Only the newest `keep` backups (default 10) are kept. Files in the folder
that slogger did not name are never deleted.

Not backed up: `config.toml` (it holds your service passwords), themes,
the window layout, and `cty.dat`. Copy `config.toml` somewhere safe
yourself.

### Where

| | Logbook database | Default backup folder |
| --- | --- | --- |
| Windows | `%LOCALAPPDATA%\slogger\slogger.sqlite` | `%LOCALAPPDATA%\slogger\backups` |
| Linux | `~/.local/share/slogger/slogger.sqlite` | `~/.local/share/slogger/backups` |

The default folder is on the same disk as the logbook. That protects
against a bad import, a mistaken bulk edit or a damaged database, but not
against losing the disk. Set `directory` under `[backup]` in `config.toml`
to another drive or a synced folder (see `docs/example-config.toml`).

If a backup cannot be written (the drive is unplugged, the folder is
read-only), a warning appears in the bottom bar and stays there.

### Do not copy `slogger.sqlite` by hand as a backup

While slogger runs, recent changes live in two companion files,
`slogger.sqlite-wal` and `slogger.sqlite-shm`. Copying `slogger.sqlite`
alone can give you a file that is missing recent QSOs or is unreadable.
The backups slogger writes are single, self-contained files with no
companions.

## Restoring a backup

Restoring replaces the whole logbook with the one in the backup. Anything
logged after that backup was made is not in it; step 2 keeps the current
log so those QSOs can be recovered.

1. **Quit slogger.** It must not be running. Check that no `slogger`
   process remains.
2. **Set the current logbook aside.** In the logbook folder (table above),
   move these three files into a new folder, for example `before-restore`.
   The last two may not exist; move whichever are there.
   - `slogger.sqlite`
   - `slogger.sqlite-wal`
   - `slogger.sqlite-shm`

   Do not leave the `-wal` or `-shm` file behind. SQLite would apply
   their contents to the restored database and could damage it.
3. **Copy the backup into place.** Copy (do not move) the `.sqlite`
   backup you want into the logbook folder and rename the copy to
   `slogger.sqlite`.
4. **Start slogger** and check the Logbook tab: the QSO count and the
   most recent QSO should be those of the backup.

Windows, in PowerShell:

```powershell
cd $env:LOCALAPPDATA\slogger
mkdir before-restore
Move-Item slogger.sqlite* before-restore\
Copy-Item backups\slogger-20261124-153000Z.sqlite slogger.sqlite
```

Linux:

```sh
cd ~/.local/share/slogger
mkdir before-restore
mv slogger.sqlite* before-restore/
cp backups/slogger-20261124-153000Z.sqlite slogger.sqlite
```

### Getting back QSOs logged after the backup

Those QSOs are in the log you set aside in step 2, and in any backup made
after them.

- If a newer backup has them, import its `.adi` file: Logbook tab ->
  Tools -> ADIF import. Import skips QSOs the log already has, so only
  the missing ones are added.
- Otherwise they exist only in `before-restore`. slogger has no ADIF
  export button yet, so the way to get them out is to undo the restore
  (below), press **Back up now** to produce an `.adi` file, restore
  again, and import that file.

### Undoing a restore

Quit slogger, delete the `slogger.sqlite`, `-wal` and `-shm` files in the
logbook folder, and move the three files back out of `before-restore`.

### A backup from an older version of slogger

This works. When slogger opens a database from an earlier version it
upgrades it in place. The reverse does not: a backup made by a newer
slogger cannot be opened by an older one.

### If no `.sqlite` backup is usable

Start slogger with no `slogger.sqlite` in the logbook folder (it creates
an empty log), then import the newest `.adi` backup from the Tools panel.
QSOs, station locations and award credit come back. Upload and
confirmation status comes back as far as the ADIF fields record it.
