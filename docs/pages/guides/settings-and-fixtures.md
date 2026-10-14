---
title: Settings, fixtures and the sandbox guard
description: Keep key-value settings in the store, seed a named test fixture once, and refuse to seed outside a sandbox.
sidebar:
  order: 15
---

## Settings

`storekit::settings` reads and writes a `settings` key-value table. Create it
in your first migration with `storekit::settings::SETTINGS_TABLE_SQL`, or with
the same SQL written out:

```sql
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
```

Then:

```rust
storekit::settings::set(&connection, "theme", "dark")?;
let theme: Option<String> = storekit::settings::get(&connection, "theme")?;
storekit::settings::clear(&connection, "theme")?;
```

`set` inserts or replaces, `get` returns `None` for a missing key, and `clear`
deletes the key.

## Fixtures

A fixture fills a store with known data for a demo or a test. Implement
`storekit::fixtures::Fixture`:

```rust
use rusqlite::Connection;
use storekit::fixtures::Fixture;

struct Demo;

impl Fixture for Demo {
    fn name(&self) -> &str {
        "demo"
    }

    fn seed(&self, connection: &Connection) -> rusqlite::Result<()> {
        for body in ["first", "second", "third"] {
            connection.execute("INSERT INTO notes(body) VALUES (?1)", [body])?;
        }
        Ok(())
    }
}
```

and call it after `open`:

```rust
let seeded = storekit::fixtures::seed_if_requested(&connection, &APP, &[&Demo])?;
```

It does nothing unless `<PREFIX>_FIXTURES` (here `NOTES_FIXTURES`) names a
fixture. Then it seeds that fixture once and records its name under the
`fixture_seeded` setting, so the next start doesn't seed it again. It returns
the name, or `None`. A name no fixture answers to is an error.

The `desk` example has a `demo` fixture:

```sh
PITO_EXAMPLE_FIXTURES=demo cargo run --example desk
```

```text
fixture=demo
rows=3
```

## The sandbox guard

Seeding test data into a real store would be hard to undo. Call
`storekit::rig::guard(&APP, &data_dir)` at start-up, before opening the
store:

```rust
storekit::rig::guard(&APP, &APP.data_dir());
```

When no fixture is named it does nothing. When one is, the file `/tmp/.rig`
must hold a folder path, and the data folder must be under it; otherwise the
process prints this to stderr and exits with code 2:

```text
rig: refusing to boot: a fixture is named but /tmp is not the tree's tmp/
```

A test harness that makes a sandbox writes its path to `/tmp/.rig` and points
`<PREFIX>_DATA` inside it. `storekit::rig::guard_at(stamp, requested,
data_dir)` makes the same check against any stamp file and returns the
refusal as an `Err` instead of exiting.
