---
title: Install and first use
description: Add storekit to a Rust project on Linux, WSL or macOS, then open, migrate and back up a first store.
sidebar:
  order: 2
---

storekit is not on crates.io: add it from git, pinned to a release tag. You
need Rust 1.97 or newer and a C compiler, since SQLite is compiled into the
crate. It works on Linux and macOS; on Windows, use it inside WSL.

## Linux and WSL

On Debian or Ubuntu (any distribution's C toolchain works):

```sh
sudo apt install build-essential
cargo new notes && cd notes
cargo add storekit --git https://github.com/gmrdad82/pito-storekit --tag v1.0.1
```

## macOS

```sh
xcode-select --install
cargo new notes && cd notes
cargo add storekit --git https://github.com/gmrdad82/pito-storekit --tag v1.0.1
```

## What it adds

`cargo add` writes this line to your `Cargo.toml`:

```toml
storekit = { git = "https://github.com/gmrdad82/pito-storekit", tag = "v1.0.1" }
```

Any release tag in the [changelog](changelog.md) works in its place.

## Quick start

Put this in `src/main.rs`:

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
    println!("{}", storekit::paths::tilde(&APP.db_path("notes.db").display().to_string()));
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}
```

Then run it:

```sh
cargo run
```

```text
~/.local/share/notes-dev/notes.db
```

That is the store's path on Linux in a debug build; on macOS it is
`~/Library/Application Support/notes-dev/notes.db`, and a release build
(`cargo run --release`) drops the `-dev`. The first run also leaves the day's
backup in the `backups/` folder beside the store, named like
`notes-20261014-1901.db.gz`; another run the same UTC day keeps no second one.

Next, [Open a store](guides/open-a-store.md) says what `open` set up, and
[Find the app's folders](guides/app-folders.md) where the paths come from.
