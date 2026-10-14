---
title: Keep a backup a day
description: Keep one gzip copy of the store per UTC day for 14 days, by hand or from a background thread, and restore one.
sidebar:
  order: 13
---

![Bars: the 265.5 MiB store and its 18.2 MiB gzip backup; VACUUM INTO, gzip, fsync and rename took 3.8 s](../images/backup-bars.png)

`storekit::backup` keeps one compressed copy of the store a day and deletes
copies older than 14 days (`KEEP_DAYS`). In the measured run a 265.5 MiB store
became an 18.2 MiB backup, 14.6× smaller, in 3.8 s.

## Keep one now

```rust
let backups = APP.data_dir().join("backups");
if storekit::backup::due(&backups, APP.folder, now()) {
    let kept = storekit::backup::keep(&connection, &backups, APP.folder)?;
    println!("{} ({} bytes)", kept.path.display(), kept.bytes);
}
```

`keep(connection, dir, stem)` creates `dir` if needed, then:

1. `VACUUM INTO` a plain copy, `<name>.plain`;
2. gzips it into `<name>.part` and fsyncs it;
3. renames it to `<stem>-YYYYMMDD-HHMM.db.gz` (UTC);
4. deletes this stem's copies older than 14 days.

A half-made backup never takes a finished name, and files it didn't name are
left alone. `keep` returns a `Kept` with the path and its size in bytes, or
the error as a string.

`due(dir, stem, now)` is true when the newest copy is from an earlier UTC day
than `now` (Unix seconds), or there is none. `latest(dir, stem)` returns the
newest copy's time in Unix seconds, read from its name. The
[formats page](../reference/formats.md#backup-files) spells out the name.

## Keep them from a background thread

```rust
use rusqlite::Connection;
use std::path::PathBuf;

fn backup_dir(_connection: &Connection) -> Option<PathBuf> {
    Some(APP.data_dir().join("backups"))
}

let handle = storekit::backup::spawn(db.clone(), backup_dir, "notes");
```

`spawn(db, dir_of, stem)` starts a thread named `backup` that checks every
minute and keeps a copy when one is due. `dir_of` is asked for the folder each
time, so it can read it from the store's settings; returning `None` skips the
run and marks the status `unconfigured`.

The returned `Handle` answers:

- `handle.status()`: a `Status` with `running`, `last_run_unix`,
  `next_run_unix`, `last_elapsed`, `last_bytes`, `last_error` and
  `unconfigured`;
- `handle.backup_now()`: keeps a copy at once, due or not.

## Restore one

A backup is a gzipped SQLite file. Close the app, then:

```sh
gunzip -c ~/.local/share/notes/backups/notes-20261014-1901.db.gz > ~/.local/share/notes/notes.db
rm -f ~/.local/share/notes/notes.db-wal ~/.local/share/notes/notes.db-shm
```

## Gzip any file the same way

`storekit::archive::gzip_file(from, into)` gzips one file and fsyncs the
result, as the backups do.
