use rusqlite::Connection;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use storekit::migrate::{Migrations, Step};

const ROWS: usize = 4_000_000;
const DESK_EVERY: Duration = Duration::from_millis(5);
const REOPENS: usize = 100;

const STEPS: &[Step] = &[
    Step::new(
        1,
        "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL);
         CREATE TABLE edits(id INTEGER PRIMARY KEY, at INTEGER NOT NULL);",
        None,
    ),
    Step::new(
        2,
        "ALTER TABLE notes ADD COLUMN summary TEXT NOT NULL DEFAULT '';",
        None,
    ),
];

const MIGRATIONS: Migrations = Migrations {
    version: 2,
    steps: STEPS,
    before: None,
};

struct Desk {
    waits: Vec<Duration>,
    refused: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("storekit-measure-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let result = run(&root);
    let _ = std::fs::remove_dir_all(&root);
    result
}

fn run(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("storekit {}", env!("CARGO_PKG_VERSION"));
    println!();

    let fresh = root.join("fresh.db");
    let started = Instant::now();
    drop(storekit::open(&fresh, &MIGRATIONS, None)?);
    let created = started.elapsed();
    let mut reopens = Vec::with_capacity(REOPENS);
    for _ in 0..REOPENS {
        let started = Instant::now();
        drop(storekit::open(&fresh, &MIGRATIONS, None)?);
        reopens.push(started.elapsed());
    }
    reopens.sort();
    println!("{:<36}{}", "open a new store and migrate it", ms(created));
    println!(
        "{:<36}{} (median of {REOPENS})",
        "reopen it, already migrated",
        ms(reopens[REOPENS / 2])
    );
    println!();

    println!(
        "a job writes {} million rows while the app writes one row every 5 ms",
        ROWS / 1_000_000
    );
    println!(
        "{:<18}{:>10}{:>10}{:>10}{:>10}{:>10}",
        "", "job", "app p50", "app p99", "app max", "refused"
    );
    let (whole, whole_desk) = contend(&root.join("whole.db"), false)?;
    report("one transaction", whole, &whole_desk);
    let (sliced, sliced_desk) = contend(&root.join("sliced.db"), true)?;
    report("write_sliced", sliced, &sliced_desk);
    println!();

    let store = root.join("sliced.db");
    let connection = storekit::open(&store, &MIGRATIONS, None)?;
    connection.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
    let store_bytes = std::fs::metadata(&store)?.len();
    let started = Instant::now();
    let kept = storekit::backup::keep(&connection, &root.join("backups"), "measure")?;
    let took = started.elapsed();
    println!(
        "{:<36}{} in {}, {:.1}x smaller",
        format!("daily backup of a {} store", mib(store_bytes)),
        mib(kept.bytes),
        ms(took),
        store_bytes as f64 / kept.bytes as f64
    );
    Ok(())
}

fn contend(path: &Path, sliced: bool) -> rusqlite::Result<(Duration, Desk)> {
    let desk = storekit::open(path, &MIGRATIONS, None)?;
    let job: storekit::Db = Arc::new(Mutex::new(storekit::open_companion(path, None)?));
    let done = Arc::new(AtomicBool::new(false));

    let watcher = {
        let done = Arc::clone(&done);
        std::thread::spawn(move || app_writes(desk, &done))
    };
    std::thread::sleep(Duration::from_millis(50));

    let started = Instant::now();
    let insert = |transaction: &Connection, n: usize| {
        transaction
            .prepare_cached("INSERT INTO notes(body) VALUES (?1)")?
            .execute([format!(
                "note {n}: a line of text the size of a short paragraph"
            )])
            .map(|_| ())
    };
    if sliced {
        storekit::sliced::write_sliced(&job, 0..ROWS, |transaction, n| insert(transaction, n))?;
    } else {
        let mut connection = job.lock().unwrap_or_else(|poison| poison.into_inner());
        let transaction = connection.transaction()?;
        for n in 0..ROWS {
            insert(&transaction, n)?;
        }
        transaction.commit()?;
    }
    let took = started.elapsed();
    done.store(true, Ordering::Relaxed);
    Ok((took, watcher.join().expect("the app thread panicked")))
}

fn app_writes(connection: Connection, done: &AtomicBool) -> Desk {
    let mut desk = Desk {
        waits: Vec::new(),
        refused: 0,
    };
    while !done.load(Ordering::Relaxed) {
        let started = Instant::now();
        match connection.execute("INSERT INTO edits(at) VALUES (unixepoch())", []) {
            Ok(_) => desk.waits.push(started.elapsed()),
            Err(_) => desk.refused += 1,
        }
        std::thread::sleep(DESK_EVERY);
    }
    desk.waits.sort();
    desk
}

fn report(name: &str, job: Duration, desk: &Desk) {
    let at = |q: f64| {
        desk.waits
            .get(((desk.waits.len() as f64 - 1.0) * q).round() as usize)
            .map(|wait| ms(*wait))
            .unwrap_or_else(|| "-".to_owned())
    };
    println!(
        "{name:<18}{:>10}{:>10}{:>10}{:>10}{:>10}",
        ms(job),
        at(0.5),
        at(0.99),
        at(1.0),
        desk.refused
    );
}

fn ms(duration: Duration) -> String {
    let ms = duration.as_secs_f64() * 1000.0;
    if ms < 10.0 {
        format!("{ms:.2} ms")
    } else {
        format!("{ms:.0} ms")
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}
