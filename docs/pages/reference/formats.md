---
title: Formats
description: The files, tables, names, environment variables and constants storekit reads and writes.
sidebar:
  order: 30
---

storekit has no command line and prints no JSON. These are the formats it
writes to disk and the names it reads, which other tools can rely on.

## The store

A store is one SQLite 3 file in WAL mode, so a live store has two sidecars
beside it, `<file>-wal` and `<file>-shm`. [`open`](../guides/open-a-store.md)
sets the pragmas each time it opens; nothing else is stored about them.

### `schema_meta`

```sql
CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
```

| key | value |
|---|---|
| `version` | the schema version the store is at, as decimal text (`"2"`) |

A missing table or row means version 0.
[Migrations](../guides/migrations.md) create it and keep it current.

### `settings`

```sql
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
```

The app creates it (`storekit::settings::SETTINGS_TABLE_SQL`). Keys and values
are the app's own text, except one storekit writes:

| key | value |
|---|---|
| `fixture_seeded` | the name of the fixture last seeded into this store |

## Backup files

[`backup::keep`](../guides/backups.md) writes into the folder it is given:

| Name | What it is |
|---|---|
| `<stem>-YYYYMMDD-HHMM.db.gz` | a finished backup: a gzip of a SQLite file made by `VACUUM INTO`, stamped with the UTC minute it started |
| `<stem>-YYYYMMDD-HHMM.db.gz.<pid>-<n>.plain` | the uncompressed copy while a backup is being made |
| `<stem>-YYYYMMDD-HHMM.db.gz.<pid>-<n>.part` | the gzip while it is being written, before its rename |

For example `notes-20261014-1901.db.gz`. `latest` and `due` read the stamp
from names that start with `<stem>-` and end in `.db.gz` or `.db`, so a plain
`<stem>-YYYYMMDD-HHMM.db` copy counts as a backup too, and a name without a
valid stamp never does. Finished backups older than 14 days (`KEEP_DAYS`)
are deleted after each new one, with the stem's `.plain` and `.part` scratch
files older than that (a killed run's leftovers, with or without the
`.<pid>-<n>` tag); any other file in the folder is left alone.

## Folders

From [`paths::App`](../guides/app-folders.md) with `folder` and `env_prefix`
(`-dev` is appended to `folder` in a debug build):

| Call | Linux and WSL | macOS |
|---|---|---|
| `config_dir()` | `$XDG_CONFIG_HOME/<folder>`, else `~/.config/<folder>` | `~/Library/Application Support/<folder>` |
| `data_dir()` | `$XDG_DATA_HOME/<folder>`, else `~/.local/share/<folder>` | `~/Library/Application Support/<folder>` |
| `db_path(file)` | `data_dir()/<file>` | `data_dir()/<file>` |
| `token_path()` | `config_dir()/token` | `config_dir()/token` |
| `mcp_token_path()` | `data_dir()/mcp-token` | `data_dir()/mcp-token` |
| `crash_spool_dir()` | `data_dir()/crash` | `data_dir()/crash` |
| `traces_dir()` | `data_dir()/traces` | `data_dir()/traces` |

## Environment variables

| Variable | Effect |
|---|---|
| `<PREFIX>_DATA` | replaces `data_dir()`; empty or blank means unset |
| `<PREFIX>_DB` | replaces `db_path(..)`; empty or blank means unset |
| `<PREFIX>_FIXTURES` | names the fixture `fixtures::seed_if_requested` seeds; empty or blank means none |

`<PREFIX>` is the `App`'s `env_prefix`.

## The sandbox stamp

`rig::guard` reads `/tmp/.rig` only when a fixture is named. It holds one
folder path (surrounding whitespace ignored). If the file is missing or empty,
or the data folder is not under that path, the process writes this line to
stderr and exits with code 2:

```text
rig: refusing to boot: a fixture is named but /tmp is not the tree's tmp/
```

## Constants

| Constant | Value | Used for |
|---|---|---|
| `DESK_PEN_WAIT` | 1 s | busy wait of `open` |
| `JOB_PEN_WAIT` | 30 s | busy wait of `open_companion` |
| `SLOW_SQL_ALARM` | 250 ms | a threshold for the `mark` profiler hook |
| `sliced::JOB_SLICE_BUDGET_QUIET` | 200 ms | a slice's work while the store is quiet |
| `sliced::JOB_SLICE_BUDGET_SHARED` | 25 ms | a slice's work after another writer was seen |
| `sliced::SHARED_SLICES` | 20 | slices kept short after another writer was seen |
| `sliced::JOB_SLICE_BREATH` | 10 ms | pause between slices |
| `sliced::PEN_CONTENDED` | 5 ms | a slice that waited this long to begin saw another writer |
| `backup::KEEP_DAYS` | 14 | days a backup is kept |
