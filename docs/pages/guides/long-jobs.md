---
title: Run a long job without locking the app out
description: Write an import or rebuild of millions of rows in short slices with write_sliced, so the app's own writes keep getting in.
sidebar:
  order: 12
---

![Paired bars: the app's longest write wait is 236 ms with write_sliced against 1000 ms with one transaction, where two writes gave up; the 4M-row job takes 2778 ms sliced against 2636 ms in one transaction](../images/wait-paired.png)

SQLite lets one writer in at a time. A background job that imports or
rebuilds in one big transaction holds that lock for seconds, and every write
the app makes meanwhile waits, then fails once its busy wait runs out.
`storekit::sliced::write_sliced` writes the same job in short transactions so
the app gets in between them.

## Write the job in slices

Give the job its own connection from
[`open_companion`](open-a-store.md#a-connection-for-a-background-worker), on
its own thread, and hand `write_sliced` the items and what to do with each:

```rust
use std::sync::{Arc, Mutex};

let job: storekit::Db = Arc::new(Mutex::new(storekit::open_companion(&path, None)?));
let rows: Vec<String> = load_rows();
let written = storekit::sliced::write_sliced(&job, rows, |transaction, row| {
    transaction.execute("INSERT INTO notes(body) VALUES (?1)", [row]).map(|_| ())
})?;
```

It returns how many items it applied.

## How the slices are cut

Each slice is one `BEGIN IMMEDIATE` transaction:

- While the store is quiet, a slice applies items for up to 200 ms
  (`JOB_SLICE_BUDGET_QUIET`), then commits.
- When opening a slice waited 5 ms or more (`PEN_CONTENDED`), another writer
  was in, so the next 20 slices (`SHARED_SLICES`) stop at 25 ms
  (`JOB_SLICE_BUDGET_SHARED`).
- Between slices the job sleeps 10 ms (`JOB_SLICE_BREATH`), for the app to
  get in.

Every committed slice stays committed. If `apply` returns an error, only the
open slice rolls back, and `write_sliced` returns that error; the items
before it are in the store.

## What it measured

With the app writing one row every 5 ms during a 4M-row job, on a busy
machine (load average 46 to 48 on 32 threads):

| | Job | App write p50 | Longest app wait | App writes refused |
|---|---|---|---|---|
| One transaction | 2.6 s | 0.10 ms | 1 s, the busy wait | 2 |
| `write_sliced` | 2.8 s | 11 ms | 236 ms | 0 |

One transaction finishes the job a little sooner, but two of the app's writes
gave up; `write_sliced` refused none. [Measure it on your
machine](measure.md) runs the same comparison.

## Tune the budgets

`write_sliced_with` takes the quiet budget, the shared budget and the breath
as arguments:

```rust
use std::time::Duration;

storekit::sliced::write_sliced_with(
    &job,
    rows,
    |transaction, row| transaction.execute("INSERT INTO notes(body) VALUES (?1)", [row]).map(|_| ()),
    Duration::from_millis(100),
    Duration::from_millis(10),
    Duration::from_millis(5),
)?;
```

A zero breath sleeps not at all.
