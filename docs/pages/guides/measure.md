---
title: Measure it on your machine
description: Run the measure example to time opening a store, a long job against the app's writes, and a daily backup on your own hardware.
sidebar:
  order: 16
---

![The measure example: a new store opens and migrates in about 150 ms and reopens in 0.15 ms; a 4 million row job written in one transaction refuses the app's writes twice, while write_sliced keeps every app write under 232 ms; a 265.5 MiB store backs up to 18.2 MiB](../images/measure.gif)

The repository's `measure` example times the three things storekit is for.
It works in a temporary folder and deletes it when it ends.

```sh
git clone https://github.com/gmrdad82/pito-storekit && cd pito-storekit
cargo run --release --example measure
```

A run on Linux (AMD Ryzen 9 9950X, NVMe on btrfs) printed:

```text
storekit 1.0.0

open a new store and migrate it     170 ms
reopen it, already migrated         0.11 ms (median of 100)

a job writes 4 million rows while the app writes one row every 5 ms
                         job   app p50   app p99   app max   refused
one transaction      2322 ms   0.10 ms     20 ms   1002 ms         1
write_sliced         2419 ms   0.11 ms    192 ms    229 ms         0

daily backup of a 265.5 MiB store   18.2 MiB in 3029 ms, 14.6x smaller
```

## Reading it

- **open** and **reopen**: what [`storekit::open`](open-a-store.md) costs the
  first time and every time after.
- **The job table**: the same 4M-row job written two ways while a second
  thread writes like an app. `app max` is the longest one app write waited,
  and `refused` counts app writes that gave up at the one-second busy wait.
  One transaction holds the lock for the whole job; `write_sliced` lets the
  app in between slices ([Run a long job](long-jobs.md)).
- **The backup**: one [`backup::keep`](backups.md) of the store the job
  built.

The numbers move from run to run and depend most on the disk; the refusals
under one transaction come and go with timing, while `write_sliced` keeps the
longest wait at a few hundred milliseconds.

The temporary folder is the system's (`/tmp` on Linux); set `TMPDIR` to put
it somewhere else, for example on the disk your app's data lives on:

```sh
TMPDIR=$HOME/scratch cargo run --release --example measure
```
