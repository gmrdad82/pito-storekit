---
title: Find the app's folders
description: Name an app's config and data folders where Linux and macOS expect them, keep debug builds apart, and override them.
sidebar:
  order: 14
---

![The desk example opens a store, migrates it, seeds a fixture and keeps the day's gzip backup; a second run finds today's copy already kept](../images/desk.gif)

`storekit::paths::App` names an app's folders from two strings:

```rust
use storekit::paths::App;

const APP: App = App { folder: "notes", env_prefix: "NOTES" };
```

## Where they are

| | Linux and WSL | macOS |
|---|---|---|
| `APP.config_dir()` | `~/.config/notes` | `~/Library/Application Support/notes` |
| `APP.data_dir()` | `~/.local/share/notes` | `~/Library/Application Support/notes` |
| `APP.db_path("notes.db")` | `~/.local/share/notes/notes.db` | `~/Library/Application Support/notes/notes.db` |

On Linux they follow `XDG_CONFIG_HOME` and `XDG_DATA_HOME` when those are set.

A debug build appends `-dev` to the folder (`APP.folder()` returns
`notes-dev`), so trying a change never touches a release build's data.

## Override them

| Variable | Overrides |
|---|---|
| `NOTES_DATA` | `data_dir()`, and so every path under it |
| `NOTES_DB` | `db_path(..)`, the store's file |

The prefix is `env_prefix`. An empty or blank value counts as unset, so an
exported but empty variable never sends the store to a throwaway or relative
path. For example, to run against a scratch copy:

```sh
NOTES_DATA=/var/tmp/notes-scratch cargo run
```

## The files beside them

| Call | Path |
|---|---|
| `token_path()` | `config_dir()/token` |
| `mcp_token_path()` | `data_dir()/mcp-token` |
| `crash_spool_dir()` | `data_dir()/crash` |
| `traces_dir()` | `data_dir()/traces` |

They only name the paths; nothing creates them.
`storekit::paths::set_owner_only_permissions(path)` makes a file `0600`, for
a token. `storekit::paths::tilde(path)` shortens a path under the home folder
to `~/…` for display.

## See it run

The `desk` example in the repository prints its folders, opens and migrates
its store, sets a setting and keeps the day's backup. It writes to your real
data folder (`storekit-example-dev` in a debug build), which you can delete
after.

```sh
git clone https://github.com/gmrdad82/pito-storekit && cd pito-storekit
cargo run --example desk
```

```text
folder=storekit-example-dev
config=~/.config/storekit-example-dev
data=~/.local/share/storekit-example-dev
db=~/.local/share/storekit-example-dev/desk.db
wal=true
shm=true
schema_version=2
setting=written
fixture=none
rows=0
backup=~/.local/share/storekit-example-dev/backups/storekit-example-20261014-1901.db.gz
backup_count=1
latest=1792004460
```

Run it again the same UTC day and it prints `backup=already-kept-today`.
