use crate::Db;
use rusqlite::{Transaction, TransactionBehavior};
use std::time::{Duration, Instant};

pub const JOB_SLICE_BUDGET_QUIET: Duration = Duration::from_millis(200);
pub const JOB_SLICE_BUDGET_SHARED: Duration = Duration::from_millis(25);
pub const JOB_SLICE_BREATH: Duration = Duration::from_millis(10);
pub const SHARED_SLICES: usize = 20;
pub const PEN_CONTENDED: Duration = Duration::from_millis(5);

pub fn write_sliced<T>(
    db: &Db,
    items: impl IntoIterator<Item = T>,
    apply: impl FnMut(&Transaction<'_>, T) -> rusqlite::Result<()>,
) -> rusqlite::Result<usize> {
    write_sliced_with(
        db,
        items,
        apply,
        JOB_SLICE_BUDGET_QUIET,
        JOB_SLICE_BUDGET_SHARED,
        JOB_SLICE_BREATH,
    )
}

pub fn write_sliced_with<T>(
    db: &Db,
    items: impl IntoIterator<Item = T>,
    mut apply: impl FnMut(&Transaction<'_>, T) -> rusqlite::Result<()>,
    quiet: Duration,
    shared: Duration,
    breath: Duration,
) -> rusqlite::Result<usize> {
    let mut items = items.into_iter().peekable();
    let mut applied = 0;
    let mut shared_left = 0;
    while items.peek().is_some() {
        {
            let connection = db.lock().unwrap_or_else(|poison| poison.into_inner());
            let started = Instant::now();
            let transaction =
                Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)?;
            if started.elapsed() >= PEN_CONTENDED {
                shared_left = SHARED_SLICES;
            }
            let budget = if shared_left == 0 {
                quiet
            } else {
                shared_left -= 1;
                shared
            };
            let opened = Instant::now();
            for item in items.by_ref() {
                apply(&transaction, item)?;
                applied += 1;
                if opened.elapsed() >= budget {
                    break;
                }
            }
            transaction.commit()?;
        }
        if items.peek().is_some() && !breath.is_zero() {
            std::thread::sleep(breath);
        }
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[test]
    fn the_five_slice_constants_keep_the_pen_ruling() {
        assert_eq!(JOB_SLICE_BUDGET_QUIET, Duration::from_millis(200));
        assert_eq!(JOB_SLICE_BUDGET_SHARED, Duration::from_millis(25));
        assert_eq!(JOB_SLICE_BREATH, Duration::from_millis(10));
        assert_eq!(SHARED_SLICES, 20);
        assert_eq!(PEN_CONTENDED, Duration::from_millis(5));
    }

    #[test]
    fn write_sliced_lands_every_item_across_many_slices() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE values_seen(value INTEGER);")
            .unwrap();
        let db = Arc::new(Mutex::new(connection));
        let applied = write_sliced_with(
            &db,
            0..20,
            |transaction, value| {
                transaction.execute("INSERT INTO values_seen VALUES (?1)", [value])?;
                std::thread::sleep(Duration::from_millis(1));
                Ok(())
            },
            Duration::from_millis(2),
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(applied, 20);
        let connection = db.lock().unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM values_seen", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            20
        );
    }

    #[test]
    fn write_sliced_keeps_committed_slices_and_rolls_back_only_the_failing_one() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE values_seen(value INTEGER);")
            .unwrap();
        let db = Arc::new(Mutex::new(connection));
        let result = write_sliced_with(
            &db,
            0..10,
            |transaction, value| {
                if value == 5 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                transaction.execute("INSERT INTO values_seen VALUES (?1)", [value])?;
                std::thread::sleep(Duration::from_millis(1));
                Ok(())
            },
            Duration::from_millis(2),
            Duration::from_millis(1),
            Duration::ZERO,
        );
        assert!(result.is_err());
        let connection = db.lock().unwrap();
        let kept = connection
            .prepare("SELECT value FROM values_seen ORDER BY value")
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(!kept.is_empty());
        assert!(kept.iter().all(|value| *value < 5));
    }

    #[test]
    fn a_competing_writer_waits_no_longer_than_one_quiet_slice_and_breath() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
        std::fs::create_dir_all(&root).unwrap();
        let holder = tempfile::Builder::new()
            .prefix("sliced-competing-writer-")
            .tempdir_in(root)
            .unwrap();
        let path = holder.path().join("desk.db");
        let connection = Connection::open(&path).unwrap();
        connection.busy_timeout(Duration::from_secs(1)).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .execute_batch("CREATE TABLE values_seen(value INTEGER);")
            .unwrap();
        let db = Arc::new(Mutex::new(connection));
        let mut companion = crate::open_companion(&path, None).unwrap();
        companion
            .busy_handler(Some(|_| {
                std::thread::sleep(Duration::from_millis(1));
                true
            }))
            .unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let competitor = std::thread::spawn(move || {
            started_rx.recv().unwrap();
            let started = Instant::now();
            let transaction = companion
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            transaction
                .execute("INSERT INTO values_seen VALUES (-1)", [])
                .unwrap();
            transaction.commit().unwrap();
            started.elapsed()
        });
        let mut signalled = false;
        let applied = write_sliced(&db, 0..1_000, |transaction, value| {
            if !signalled {
                started_tx.send(()).unwrap();
                signalled = true;
            }
            transaction.execute("INSERT INTO values_seen VALUES (?1)", [value])?;
            std::thread::sleep(Duration::from_millis(1));
            Ok(())
        })
        .unwrap();
        let waited = competitor.join().unwrap();
        let ruled_budget = JOB_SLICE_BUDGET_QUIET + JOB_SLICE_BREATH;
        let generous_wall_clock_bound = ruled_budget + Duration::from_millis(100);
        eprintln!(
            "US2_WAIT_MS={} US2_RULE_MS={} US2_BOUND_MS={}",
            waited.as_millis(),
            ruled_budget.as_millis(),
            generous_wall_clock_bound.as_millis()
        );
        assert_eq!(applied, 1_000);
        assert!(waited <= generous_wall_clock_bound);
    }

    static CONTENDED_RELEASE: Mutex<Option<std::sync::mpsc::Sender<()>>> = Mutex::new(None);

    fn wait_past_contended_then_release(calls: i32) -> bool {
        if calls == 0 {
            std::thread::sleep(PEN_CONTENDED + Duration::from_millis(1));
            if let Some(release) = CONTENDED_RELEASE.lock().unwrap().take() {
                let _ = release.send(());
            }
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
        calls < 10_000
    }

    #[test]
    fn a_contended_pen_switches_the_next_slices_to_the_shared_budget() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
        std::fs::create_dir_all(&root).unwrap();
        let holder_dir = tempfile::Builder::new()
            .prefix("sliced-contended-")
            .tempdir_in(root)
            .unwrap();
        let path = holder_dir.path().join("desk.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .execute_batch("CREATE TABLE values_seen(value INTEGER);")
            .unwrap();
        connection
            .busy_handler(Some(wait_past_contended_then_release))
            .unwrap();
        let db = Arc::new(Mutex::new(connection));
        let reader = Connection::open(&path).unwrap();
        let pen = Connection::open(&path).unwrap();
        pen.execute_batch("BEGIN IMMEDIATE").unwrap();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *CONTENDED_RELEASE.lock().unwrap() = Some(release_tx);
        let holder = std::thread::spawn(move || {
            release_rx.recv().unwrap();
            pen.execute_batch("COMMIT").unwrap();
        });
        let mut seen = Vec::new();

        let applied = write_sliced_with(
            &db,
            0..60_i64,
            |transaction, value| {
                let committed: i64 =
                    reader.query_row("SELECT COUNT(*) FROM values_seen", [], |row| row.get(0))?;
                seen.push((value, committed));
                transaction.execute("INSERT INTO values_seen VALUES (?1)", [value])?;
                Ok(())
            },
            Duration::from_secs(3_600),
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap();
        holder.join().unwrap();

        assert_eq!(applied, 60);
        for &(value, committed) in &seen[..SHARED_SLICES] {
            assert_eq!(committed, value, "item {value} rode a shared slice of one");
        }
        assert!(
            seen.last().unwrap().1 < 59,
            "the quiet budget never came back"
        );
        assert_eq!(
            reader
                .query_row("SELECT COUNT(*) FROM values_seen", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            60
        );
    }
}
