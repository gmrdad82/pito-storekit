use crate::Db;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const KEEP_DAYS: i64 = 14;

const POLL_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub path: PathBuf,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub queue_depth: usize,
    pub last_error: Option<String>,
    pub last_run_unix: Option<i64>,
    pub next_run_unix: Option<i64>,
    pub running: bool,
    pub progress: f32,
    pub last_elapsed: Option<Duration>,
    pub last_changed: Option<usize>,
    pub last_bytes: Option<u64>,
    pub unconfigured: bool,
    pub detail: Option<String>,
    pub rate_per_s: Option<f32>,
}

#[derive(Clone)]
pub struct Handle {
    status: Arc<Mutex<Status>>,
    nudge: std::sync::mpsc::Sender<()>,
}

impl Handle {
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub fn backup_now(&self) {
        let _ = self.nudge.send(());
    }
}

pub fn keep(connection: &Connection, dir: &Path, stem: &str) -> Result<Kept, String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let stamp = format_stamp(now_unix());
    let final_name = format!("{stem}-{stamp}.db.gz");
    let plain_path = dir.join(format!("{final_name}.plain"));
    let part_path = dir.join(format!("{final_name}.part"));
    let final_path = dir.join(&final_name);

    backup_into(connection, &plain_path).map_err(|error| error.to_string())?;
    crate::archive::gzip_file(&plain_path, &part_path).map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(&plain_path);
    std::fs::rename(&part_path, &final_path).map_err(|error| error.to_string())?;

    if let Ok(existing) = list_backup_names(dir, stem) {
        for name in backups_to_delete(&existing, stem, now_unix()) {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
    let bytes = std::fs::metadata(&final_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    Ok(Kept {
        path: final_path,
        bytes,
    })
}

pub fn due(dir: &Path, stem: &str, now: i64) -> bool {
    match latest(dir, stem) {
        Some(at) => at.div_euclid(86_400) != now.div_euclid(86_400),
        None => true,
    }
}

pub fn next_due_unix(after: i64) -> i64 {
    (after.div_euclid(86_400) + 1) * 86_400
}

pub fn latest(dir: &Path, stem: &str) -> Option<i64> {
    list_backup_names(dir, stem)
        .ok()?
        .last()
        .and_then(|name| stamp_to_unix(name, stem))
}

pub fn backup_into(connection: &Connection, destination: &Path) -> rusqlite::Result<()> {
    let destination = destination
        .to_str()
        .expect("backup destination must be valid utf-8");
    connection.execute("VACUUM INTO ?1", [destination])?;
    Ok(())
}

pub fn spawn(db: Db, dir_of: fn(&Connection) -> Option<PathBuf>, stem: &'static str) -> Handle {
    let status = Arc::new(Mutex::new(Status::default()));
    let (nudge, nudged) = std::sync::mpsc::channel();
    let handle = Handle {
        status: Arc::clone(&status),
        nudge,
    };
    std::thread::Builder::new()
        .name("backup".into())
        .spawn(move || run_scheduled(db, status, nudged, dir_of, stem))
        .expect("the backup thread must start");
    handle
}

fn run_scheduled(
    db: Db,
    status: Arc<Mutex<Status>>,
    nudged: std::sync::mpsc::Receiver<()>,
    dir_of: fn(&Connection) -> Option<PathBuf>,
    stem: &'static str,
) {
    let mut forced = false;
    loop {
        run_once(&db, &status, forced, dir_of, stem);
        forced = match nudged.recv_timeout(Duration::from_secs(POLL_SECS)) {
            Ok(()) => true,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                std::thread::sleep(Duration::from_secs(POLL_SECS));
                false
            }
        };
    }
}

fn run_once(
    db: &Db,
    status: &Mutex<Status>,
    forced: bool,
    dir_of: fn(&Connection) -> Option<PathBuf>,
    stem: &str,
) {
    let dir = {
        let connection = db.lock().unwrap_or_else(|poison| poison.into_inner());
        dir_of(&connection)
    };
    let Some(dir) = dir else {
        let mut status = status.lock().unwrap_or_else(|poison| poison.into_inner());
        status.unconfigured = true;
        status.next_run_unix = None;
        return;
    };

    let now = now_unix();
    if forced || due(&dir, stem, now) {
        {
            let mut status = status.lock().unwrap_or_else(|poison| poison.into_inner());
            status.unconfigured = false;
            status.running = true;
            status.progress = 0.0;
            status.queue_depth = 0;
        }
        let started = Instant::now();
        let outcome = {
            let connection = db.lock().unwrap_or_else(|poison| poison.into_inner());
            keep(&connection, &dir, stem)
        };
        let ran_at = now_unix();
        let mut status = status.lock().unwrap_or_else(|poison| poison.into_inner());
        status.running = false;
        status.progress = 0.0;
        status.queue_depth = 0;
        status.last_run_unix = Some(ran_at);
        status.next_run_unix = Some(next_due_unix(ran_at));
        status.last_elapsed = Some(started.elapsed());
        match outcome {
            Ok(kept) => {
                status.last_bytes = Some(kept.bytes);
                status.last_error = None;
            }
            Err(error) => status.last_error = Some(error),
        }
    } else {
        let mut status = status.lock().unwrap_or_else(|poison| poison.into_inner());
        status.unconfigured = false;
        status.next_run_unix = Some(latest(&dir, stem).map(next_due_unix).unwrap_or(now));
    }
}

fn list_backup_names(dir: &Path, stem: &str) -> std::io::Result<Vec<String>> {
    let prefix = format!("{stem}-");
    let mut names: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| {
            name.starts_with(&prefix) && (name.ends_with(".db") || name.ends_with(".db.gz"))
        })
        .collect();
    names.sort();
    Ok(names)
}

fn backups_to_delete(names: &[String], stem: &str, now: i64) -> Vec<String> {
    let cutoff = now - KEEP_DAYS * 86_400;
    names
        .iter()
        .filter(|name| stamp_to_unix(name, stem).is_some_and(|at| at < cutoff))
        .cloned()
        .collect()
}

fn stamp_to_unix(name: &str, stem: &str) -> Option<i64> {
    let core = name.strip_prefix(&format!("{stem}-"))?;
    let core = core
        .strip_suffix(".gz")
        .unwrap_or(core)
        .strip_suffix(".db")?;
    let (date, time) = core.split_once('-')?;
    if date.len() != 8 || time.len() != 4 {
        return None;
    }
    let year = date[0..4].parse().ok()?;
    let month = date[4..6].parse().ok()?;
    let day = date[6..8].parse().ok()?;
    let hour: i64 = time[0..2].parse().ok()?;
    let minute: i64 = time[2..4].parse().ok()?;
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn format_stamp(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3_600;
    let minute = seconds % 3_600 / 60;
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{self, SETTINGS_TABLE_SQL};

    fn scratch(tag: &str) -> tempfile::TempDir {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix(&format!("backup-{tag}-"))
            .tempdir_in(root)
            .unwrap()
    }

    fn backup_dir(connection: &Connection) -> Option<PathBuf> {
        settings::get(connection, "backup_dir")
            .ok()
            .flatten()
            .map(PathBuf::from)
    }

    fn db_with_backup_dir(dir: Option<&Path>) -> Db {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SETTINGS_TABLE_SQL).unwrap();
        if let Some(dir) = dir {
            settings::set(&connection, "backup_dir", &dir.to_string_lossy()).unwrap();
        }
        Arc::new(Mutex::new(connection))
    }

    #[test]
    fn keep_days_keeps_the_fourteen_day_ruling() {
        assert_eq!(KEEP_DAYS, 14);
    }

    #[test]
    fn keeps_everything_inside_fourteen_days() {
        let now = stamp_to_unix("desk-20260818-1200.db.gz", "desk").unwrap();
        let names = vec![
            "desk-20260805-1200.db.gz".to_owned(),
            "desk-20260812-0400.db.gz".to_owned(),
            "desk-20260818-1100.db.gz".to_owned(),
        ];
        assert!(backups_to_delete(&names, "desk", now).is_empty());
    }

    #[test]
    fn deletes_only_what_is_older_than_fourteen_days() {
        let now = stamp_to_unix("desk-20260818-1200.db.gz", "desk").unwrap();
        let names = vec![
            "desk-20260720-1200.db".to_owned(),
            "desk-20260804-1100.db.gz".to_owned(),
            "desk-20260810-0400.db.gz".to_owned(),
        ];
        assert_eq!(
            backups_to_delete(&names, "desk", now),
            vec![
                "desk-20260720-1200.db".to_owned(),
                "desk-20260804-1100.db.gz".to_owned()
            ]
        );
    }

    #[test]
    fn a_name_without_a_readable_stamp_is_never_deleted() {
        let now = stamp_to_unix("desk-20260818-1200.db.gz", "desk").unwrap();
        assert!(backups_to_delete(&["desk-garbled.db".to_owned()], "desk", now).is_empty());
    }

    #[test]
    fn backup_names_round_trip_through_their_stamp() {
        let stamp = format_stamp(1_700_000_000);
        assert_eq!(
            stamp_to_unix(&format!("desk-{stamp}.db"), "desk"),
            Some(1_700_000_000 - 20)
        );
    }

    #[test]
    fn a_gz_stamp_reads_the_same_as_a_bare_one() {
        assert_eq!(
            stamp_to_unix("desk-20260812-0400.db.gz", "desk"),
            stamp_to_unix("desk-20260812-0400.db", "desk")
        );
        assert!(stamp_to_unix("desk-assets.tar", "desk").is_none());
    }

    #[test]
    fn a_folder_with_no_copies_is_always_due() {
        let holder = scratch("due-empty");
        assert!(due(holder.path(), "desk", 1_786_000_000));
    }

    #[test]
    fn a_missing_folder_is_due_rather_than_an_error() {
        let holder = scratch("due-missing");
        assert!(due(&holder.path().join("not-there"), "desk", 1_786_000_000));
    }

    #[test]
    fn one_copy_a_day_holds_every_later_boot_off() {
        let holder = scratch("due-fresh");
        std::fs::write(holder.path().join("desk-20260812-0400.db.gz"), b"x").unwrap();
        let at = latest(holder.path(), "desk").unwrap();
        assert!(!due(holder.path(), "desk", at + 60));
        assert!(!due(holder.path(), "desk", at + 12 * 3_600));
        assert!(due(holder.path(), "desk", at + 86_400));
    }

    #[test]
    fn a_late_night_copy_is_stale_the_next_morning() {
        let holder = scratch("due-midnight");
        std::fs::write(holder.path().join("desk-20260812-2350.db.gz"), b"x").unwrap();
        let at = latest(holder.path(), "desk").unwrap();
        assert!(due(holder.path(), "desk", at + 20 * 60));
    }

    #[test]
    fn next_due_is_the_coming_midnight() {
        let at = stamp_to_unix("desk-20260812-0400.db.gz", "desk").unwrap();
        assert_eq!(
            next_due_unix(at),
            stamp_to_unix("desk-20260813-0000.db.gz", "desk").unwrap()
        );
    }

    #[test]
    fn strangers_in_the_folder_are_ignored() {
        let holder = scratch("strangers");
        for name in ["notes.txt", "desk-nonsense.db", "backup.db"] {
            std::fs::write(holder.path().join(name), b"x").unwrap();
        }
        assert_eq!(latest(holder.path(), "desk"), None);
        assert!(due(holder.path(), "desk", 1_786_000_000));
    }

    #[test]
    fn a_malformed_stamp_reads_as_nothing() {
        assert_eq!(stamp_to_unix("desk-2026081-0400.db", "desk"), None);
        assert_eq!(stamp_to_unix("desk-20260812-040.db", "desk"), None);
        assert_eq!(stamp_to_unix("desk-abcdefgh-0400.db", "desk"), None);
        assert_eq!(stamp_to_unix("other-20260812-0400.db", "desk"), None);
        assert_eq!(stamp_to_unix("desk-20260812-0400.sqlite", "desk"), None);
    }

    #[test]
    fn stamps_advance_with_the_clock() {
        let earlier = stamp_to_unix("desk-20260812-0400.db", "desk").unwrap();
        let later = stamp_to_unix("desk-20260812-0500.db", "desk").unwrap();
        assert_eq!(later - earlier, 3_600);
        let tomorrow = stamp_to_unix("desk-20260813-0400.db", "desk").unwrap();
        assert_eq!(tomorrow - earlier, 86_400);
    }

    #[test]
    fn run_once_backs_up_when_due_and_reports_a_fresh_next_due() {
        let holder = scratch("run-once-due");
        let db = db_with_backup_dir(Some(holder.path()));
        let status = Mutex::new(Status::default());

        run_once(&db, &status, false, backup_dir, "desk");

        let status = status.into_inner().unwrap();
        assert!(!status.running);
        assert_eq!(status.progress, 0.0);
        assert_eq!(status.queue_depth, 0);
        let last = status.last_run_unix.unwrap();
        assert_eq!(status.next_run_unix, Some(next_due_unix(last)));
        assert!(status.last_error.is_none());
        assert!(status.last_elapsed.is_some());
        assert_eq!(list_backup_names(holder.path(), "desk").unwrap().len(), 1);
    }

    #[test]
    fn run_once_leaves_a_fresh_backup_alone_but_still_reports_next_due() {
        let holder = scratch("not-due");
        let stamp = format_stamp(now_unix());
        std::fs::write(holder.path().join(format!("desk-{stamp}.db.gz")), b"x").unwrap();
        let db = db_with_backup_dir(Some(holder.path()));
        let status = Mutex::new(Status::default());

        run_once(&db, &status, false, backup_dir, "desk");

        let status = status.into_inner().unwrap();
        assert_eq!(status.last_run_unix, None);
        assert_eq!(list_backup_names(holder.path(), "desk").unwrap().len(), 1);
        assert!(status.next_run_unix.is_some());
    }

    #[test]
    fn run_once_on_an_unset_location_does_nothing_and_never_errors() {
        let db = db_with_backup_dir(None);
        let status = Mutex::new(Status::default());
        run_once(&db, &status, false, backup_dir, "desk");
        let status = status.into_inner().unwrap();
        assert!(status.unconfigured);
        assert!(!status.running);
        assert_eq!(status.last_error, None);
        assert_eq!(status.last_run_unix, None);
        assert_eq!(status.next_run_unix, None);
    }

    #[test]
    fn backup_dir_reads_back_exactly_what_settings_wrote_with_no_fallback() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SETTINGS_TABLE_SQL).unwrap();
        assert_eq!(backup_dir(&connection), None);
        settings::set(&connection, "backup_dir", "/picked/location").unwrap();
        assert_eq!(
            backup_dir(&connection),
            Some(PathBuf::from("/picked/location"))
        );
    }

    #[test]
    fn latest_reads_the_newest_stamp() {
        let holder = scratch("latest");
        for name in [
            "desk-20260810-0100.db.gz",
            "desk-20260812-2300.db.gz",
            "desk-20260811-1200.db.gz",
        ] {
            std::fs::write(holder.path().join(name), b"x").unwrap();
        }
        assert_eq!(
            latest(holder.path(), "desk"),
            stamp_to_unix("desk-20260812-2300.db.gz", "desk")
        );
    }
}
