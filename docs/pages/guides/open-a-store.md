---
title: Open a store
description: Open the app's SQLite store with storekit's pragmas and migrations, and a second connection for a background worker.
sidebar:
  order: 10
---

A storekit app opens its store one way, with `storekit::open`, and gives a
background worker its own connection with `storekit::open_companion`.

## The app's connection

```rust
let connection = storekit::open(&APP.db_path("notes.db"), &MIGRATIONS, None)?;
```

`open(path, migrations, mark)` returns a `rusqlite::Connection` with:

| Setting | Value |
|---|---|
| `journal_mode` | `WAL`, so readers never block the writer |
| `synchronous` | `NORMAL` |
| `foreign_keys` | `ON` |
| `cache_size` | `-32768`, a 32 MiB page cache |
| `mmap_size` | `268435456`, 256 MiB of memory-mapped I/O |
| busy wait | `DESK_PEN_WAIT`, one second |

Then it runs the migrations ([Write migrations](migrations.md)). The store
file is created if it is missing, but its folder is not: create the folder
first, as the [quick start](../install.md) does with
`std::fs::create_dir_all(APP.data_dir())`.

In the measured run, reopening a store that was already migrated took
0.15 ms (median of 100), and creating and migrating a new one 45 ms.

## A connection for a background worker

```rust
use std::sync::{Arc, Mutex};

let job: storekit::Db = Arc::new(Mutex::new(storekit::open_companion(&path, None)?));
```

`open_companion(path, mark)` opens a second connection to the same store with
WAL, `synchronous = NORMAL` and foreign keys on, and a 30-second busy wait
(`JOB_PEN_WAIT`): a worker can wait for the app, the app should not wait for
the worker. It never migrates, so open the store with `open` first.
`storekit::Db` is `Arc<Mutex<Connection>>`, the shape
[`write_sliced`](long-jobs.md) and [`backup::spawn`](backups.md) take.

## Timing every statement

`mark` is an optional `fn(&str, Duration)` that SQLite calls after each
statement with its SQL and how long it took (rusqlite's `profile` hook).
`SLOW_SQL_ALARM` is 250 ms, the threshold the PITO MD apps use with it:

```rust
use std::time::Duration;

fn mark(sql: &str, took: Duration) {
    if took >= storekit::SLOW_SQL_ALARM {
        eprintln!("slow SQL ({} ms): {sql}", took.as_millis());
    }
}

let connection = storekit::open(&path, &MIGRATIONS, Some(mark))?;
```

Pass `None` to time nothing.
