---
title: pito-storekit
description: The SQLite store underneath PITO MD desktop apps, as a small Rust crate named storekit.
sidebar:
  order: 1
---

![The measure example: a new store opens and migrates in about 150 ms and reopens in 0.15 ms; a 4 million row job written in one transaction refuses the app's writes twice, while write_sliced keeps every app write under 232 ms; a 265.5 MiB store backs up to 18.2 MiB](images/measure.gif)

pito-storekit is the SQLite store underneath [PITO MD](https://pitomd.com)
desktop apps, as a small Rust crate named `storekit`. It is local SQLite
storage, not Apple's StoreKit: it has nothing to do with in-app purchases.

It opens a store with one set of tuned pragmas, runs ordered schema
migrations, writes long background jobs in short slices so the app's own
writes never wait long, keeps a gzip backup a day for 14 days, and names an
app's data and config folders where each OS expects them. It has no app logic
of its own.

Works on Linux and macOS (Windows through WSL).

## At a glance

Measured once on 2026-10-14 with `cargo run --release --example measure`
(storekit 0.1.2, Rust 1.97.1, AMD Ryzen 9 9950X, Samsung 990 PRO NVMe on
btrfs, Linux). The GIF above is a separate run of the same example; the
numbers move a little from run to run, and
[Measure it on your machine](guides/measure.md) shows how to run it yourself.

| | |
|---|---|
| Longest wait for one app write while a 4M-row job runs | **241 ms** with `write_sliced`, none refused; with one transaction two writes gave up after the 1 s busy wait |
| Reopen an already-migrated store | **0.13 ms** (median of 100) |
| Create a new store and migrate it | 74 ms |
| Daily backup of a 265.5 MiB store | **18.2 MiB**, 14.6× smaller, in 3.8 s |
| Code | 608 lines of Rust, tests and blank lines left out |
| Dependencies | 4 direct (`rusqlite` with SQLite built in, `flate2`, `dirs`, `tracing`), 29 crates in the whole build |
| Cold build | 70 s for `cargo build --release --examples`, SQLite's C source included |

## Where to go next

- [Install and first use](install.md): add the crate and open a first store.
- Guides, one task a page:
  - [Open a store](guides/open-a-store.md)
  - [Write migrations](guides/migrations.md)
  - [Run a long job without locking the app out](guides/long-jobs.md)
  - [Keep a backup a day](guides/backups.md)
  - [Find the app's folders](guides/app-folders.md)
  - [Settings, fixtures and the sandbox guard](guides/settings-and-fixtures.md)
  - [Measure it on your machine](guides/measure.md)
- [Formats](reference/formats.md): the files, tables, names and variables the
  crate reads and writes.
- [API reference](api/storekit/index.html): every public item, from
  `cargo doc`.
- [Changelog](changelog.md).

The code is MIT licensed, by Catalin Ilinca. The PITO MD name, every app and
game name and their logos are © Catalin Ilinca, all rights reserved, and are
not covered by the MIT licence.
