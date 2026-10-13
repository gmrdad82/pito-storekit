<p align="center"><img src="docs/images/desk.gif" alt="The desk example opens a store, migrates it, seeds a fixture and keeps the day's gzip backup; a second run finds today's copy already kept"></p>

# pito-storekit

[![CI](https://github.com/gmrdad82/pito-storekit/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/gmrdad82/pito-storekit/actions/workflows/ci.yml)
[![Version](https://img.shields.io/github/v/tag/gmrdad82/pito-storekit)](https://github.com/gmrdad82/pito-storekit/tags)

Works on Linux and macOS (Windows through WSL).

The SQLite store underneath [PITO](https://pitomd.com) desktop apps, as a
small Rust crate named `storekit`. It is local SQLite storage, not Apple's
StoreKit: it has nothing to do with in-app purchases.

It opens a store with one set of tuned pragmas, runs ordered schema
migrations, writes long background jobs in short slices so the app's own
writes never wait long, keeps a gzip backup a day for 14 days, and names an
app's data and config folders in the places each OS expects. It has no app
logic of its own.

## Install

It is not on crates.io: add it to your `Cargo.toml` from git, pinned to a
release tag.

```toml
storekit = { git = "https://github.com/gmrdad82/pito-storekit", tag = "v0.1.2" }
```

It needs Rust 1.97 or newer and a C compiler, since SQLite is built in
through rusqlite's `bundled` feature (Xcode's command line tools on macOS,
`build-essential` or its like on Linux and WSL). Nothing else is installed or
downloaded.

## Use

```rust
use storekit::migrate::{Migrations, Step};
use storekit::paths::App;

const APP: App = App { folder: "notes", env_prefix: "NOTES" };

const STEPS: &[Step] = &[
    Step::new(1, "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                  CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL);", None),
    Step::new(2, "ALTER TABLE notes ADD COLUMN summary TEXT NOT NULL DEFAULT '';", None),
];

const MIGRATIONS: Migrations = Migrations { version: 2, steps: STEPS, before: None };

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(APP.data_dir())?;
    let connection = storekit::open(&APP.db_path("notes.db"), &MIGRATIONS, None)?;
    storekit::settings::set(&connection, "theme", "dark")?;
    let backups = APP.data_dir().join("backups");
    if storekit::backup::due(&backups, APP.folder, now()) {
        storekit::backup::keep(&connection, &backups, APP.folder)?;
    }
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}
```

`cargo run --example desk` runs the whole tour shown above the title and
prints what it did. It writes to your real data folder (in a debug build,
`~/.local/share/storekit-example-dev` on Linux), which you can delete after.

## What it does

- **`storekit::open(path, migrations, mark)`** opens a store in WAL mode with
  `synchronous = NORMAL`, foreign keys on, a 32 MiB page cache, 256 MiB of
  memory-mapped I/O and a one-second busy wait (`DESK_PEN_WAIT`), then runs
  the migrations. `mark` is an optional SQL profiler hook; `SLOW_SQL_ALARM`
  (250 ms) is the threshold the PITO MD apps use with it.
- **`storekit::open_companion(path, mark)`** opens a second connection for a
  background worker with the same pragmas and a 30-second busy wait
  (`JOB_PEN_WAIT`), and never migrates.
- **`storekit::migrate`** runs `Step`s in the order listed, skipping those at
  or below the stored version, and records the version in a `schema_meta`
  table once. A step can carry a Rust backfill, and
  `Step::with_foreign_keys_off()` runs a table rebuild with foreign keys off
  and turns them back on even when it fails. `Migrations::before` runs on an
  existing store before any step.
- **`storekit::sliced::write_sliced(db, items, apply)`** writes a long job in
  short immediate transactions: 200 ms of work per slice while the store is
  quiet, 25 ms for the next 20 slices once another writer is seen waiting,
  and a 10 ms breath between slices. Each committed slice stays committed; a
  failing item rolls back only its own slice. `write_sliced_with` takes the
  budgets as arguments.
- **`storekit::backup`** keeps one copy a day (by UTC day) as
  `<stem>-YYYYMMDD-HHMM.db.gz`: `VACUUM INTO` a plain copy, gzip it, fsync
  it, then rename it into place, and delete copies older than 14 days. Files
  it didn't name are left alone. `keep` makes one now, `due` and `latest`
  read the folder, and `spawn` runs a thread that checks every minute and
  answers `Handle::status()` and `Handle::backup_now()`.
- **`storekit::archive::gzip_file(from, into)`** gzips one file and fsyncs
  the result, as the backups do.
- **`storekit::paths::App`** names an app's folders: `config_dir()` and
  `data_dir()` follow the XDG base directories on Linux and WSL
  (`~/.config/<folder>`, `~/.local/share/<folder>`) and use
  `~/Library/Application Support/<folder>` on macOS. A debug build appends
  `-dev` to the folder, so it never touches a release build's data.
  `<PREFIX>_DATA` and `<PREFIX>_DB` override the data folder and the store's
  path. `token_path`, `mcp_token_path`, `crash_spool_dir` and `traces_dir`
  name the usual files beside them, `set_owner_only_permissions` makes a file
  `0600`, and `tilde` shortens a path under the home folder for display.
- **`storekit::settings`** reads, writes and clears a `settings` key-value
  table (`SETTINGS_TABLE_SQL` creates it).
- **`storekit::fixtures`** seeds a named fixture once when
  `<PREFIX>_FIXTURES` names one, remembering it in `settings`.
- **`storekit::rig::guard(app, data_dir)`** refuses to boot (exit 2) when a
  fixture is named but the data folder is not under the folder written in
  `/tmp/.rig`, so test data is only seeded into a sandbox made for it.

## Development

`bin/gate` runs `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings` and `cargo test`. The tests keep their scratch files in the tree's
`tmp/`. CI runs `bin/gate` on Linux and macOS on every push, every pull
request from a fork and nightly.

The GIF at the top is a prec shot of the example (`render/terminal.toml` and
`render/terminal/desk.tape`), made a GIF by pconv; see [NOTICE](NOTICE).

## Contributing

Issues and pull requests are welcome. Please read the
[code of conduct](CODE_OF_CONDUCT.md) first. A change keeps `bin/gate` green
with no warnings. Report a security issue privately, as
[SECURITY.md](SECURITY.md) says, not in a public issue.

## Licence

The code is MIT licensed: see [LICENSE](LICENSE), by Catalin Ilinca. The PITO MD
name, every app and game name and their logos are © Catalin Ilinca, all
rights reserved, and are not covered by the MIT licence: see
[TRADEMARKS.md](TRADEMARKS.md). [NOTICE](NOTICE) says where the GIF comes from.
