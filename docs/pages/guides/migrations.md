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

1. reads the stored version from the `schema_meta` table (0 for a new store
   or a missing table);
2. on an existing store (stored version above 0), calls `before`, if set;
3. runs, in the order listed, every step whose `to` is above the stored
   version: its SQL, then its backfill, then it records `to` as the stored
   version if that raises it. Each step reads the stored version again once
   it holds the write lock, and skips itself when another connection
   recorded that version in the meantime;
4. records `Migrations::version` if it is above the stored version.

`before`, and each step with its backfill and its version, run in their own
`BEGIN IMMEDIATE` transaction, committed only when all of it succeeds. Holding
the write lock from the start means a `before` or a step that reads and then
writes never meets another connection's commit in between: that writer waits
its busy timeout until the runner commits. If any part fails, `open` returns
its error and that transaction rolls back: the store keeps every step
before it, at the version of the last one that succeeded, and nothing of the
failed one, even when it failed on its third statement or the process died
halfway through its backfill. The next open resumes from there, so a step
that isn't safe to run twice, such as `ALTER TABLE … ADD COLUMN`, never runs
twice, even when two processes (an app and its command-line tool, say) open
the store together on the first start after an upgrade: the second waits for
the first's step, sees its version and moves on, and still runs any newer
step only its own list has. Steps run in the order you list them, not sorted by number, so append
new steps at the end: a step listed after a higher-numbered one is skipped for
good once that higher version is recorded.

You can call `migrate::run` inside a transaction of yours: the runner then
uses a `SAVEPOINT` for each part instead, and nothing is durable until you
commit. A step or `before` may use `SAVEPOINT`s of its own, but not `BEGIN`,
`COMMIT`, `VACUUM` or a `journal_mode` change.

A store already at `Migrations::version`, with no `before` set, opens without
taking the write lock: the runner reads the stored version and returns. With
`before` set, every open takes the write lock while `before` runs. An
older build that opens a store a newer build migrated runs no step and never
lowers the stored version (calling `before`, if set), so the newer build's
next open runs nothing again. The [formats page](../reference/formats.md)
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

The runner turns foreign keys off just before that step's transaction and back
on after it, whether the step succeeded or not, so its backfill runs with
them off too. `before` and every other step run with foreign keys as the
connection has them (on, after `open`), so their `ON DELETE` and `ON UPDATE`
actions fire as usual. SQLite ignores the `foreign_keys` pragma inside a
transaction, so when you call `migrate::run` inside one of yours, a rebuild
step runs with foreign keys as you left them.
Nothing checks the rebuilt table against its foreign keys; if you want the
step refused when it breaks one, run `PRAGMA foreign_key_check` in its
backfill and return an error when it finds a row.

## Run something before the steps

`Migrations::before` is an optional `fn(&Connection) -> rusqlite::Result<()>`
that runs on every open of an existing store, before any step and in its own
transaction, for example to repair rows an older build wrote. It never runs on a
new store.
