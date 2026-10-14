---
title: Write migrations
description: Describe the schema as ordered steps that storekit runs once each, with Rust backfills and table rebuilds.
sidebar:
  order: 11
---

`storekit::migrate` brings a store's schema up to date every time
[`open`](open-a-store.md) runs. You list the steps; it runs the ones the
store has not seen yet.

## List the steps

```rust
use storekit::migrate::{Migrations, Step};

const STEPS: &[Step] = &[
    Step::new(1, "CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL);", None),
    Step::new(2, "ALTER TABLE notes ADD COLUMN summary TEXT NOT NULL DEFAULT '';", None),
];

const MIGRATIONS: Migrations = Migrations { version: 2, steps: STEPS, before: None };
```

`Step::new(to, sql, backfill)` takes the version the step brings the store
to, the SQL to run (one or more statements), and an optional Rust function.
`Migrations::version` is the version the store ends at.

## What runs

On every open, the runner:

1. creates the `schema_meta` table if it is missing and reads the stored
   version (0 for a new store);
2. on an existing store (stored version above 0), calls `before`, if set;
3. runs, in the order listed, every step whose `to` is above the stored
   version, each followed by its backfill;
4. writes `Migrations::version` to `schema_meta` once, if it changed.

Steps run in the order you list them, not sorted by number, so append new
steps at the end. A step that fails returns the error from `open`, and the
stored version stays where it was. The [formats page](../reference/formats.md)
shows the `schema_meta` table.

## Fill a new column from Rust

When SQL alone can't fill a new column, give the step a backfill. It runs
right after the step's SQL, on the same connection:

```rust
use rusqlite::Connection;

fn fill_summaries(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute("UPDATE notes SET summary = substr(body, 1, 80)", [])?;
    Ok(())
}

const STEPS: &[Step] = &[
    Step::new(1, "CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL);", None),
    Step::new(2, "ALTER TABLE notes ADD COLUMN summary TEXT NOT NULL DEFAULT '';", Some(fill_summaries)),
];
```

## Rebuild a table

SQLite changes most column definitions only by building a new table and
copying the rows across. Run that step with foreign keys off, so dropping the
old table doesn't cascade:

```rust
Step::new(3, "CREATE TABLE notes_new(id INTEGER PRIMARY KEY, body TEXT NOT NULL, summary TEXT NOT NULL);
              INSERT INTO notes_new SELECT id, body, summary FROM notes;
              DROP TABLE notes;
              ALTER TABLE notes_new RENAME TO notes;", None)
    .with_foreign_keys_off(),
```

`with_foreign_keys_off()` turns foreign keys off before the step's SQL and
back on after it, even when the SQL fails.

## Run something before the steps

`Migrations::before` is an optional `fn(&Connection) -> rusqlite::Result<()>`
that runs on an existing store before any step, for example to repair rows an
older build wrote. It never runs on a new store.
