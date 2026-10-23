use crate::Db;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    let (plain_path, part_path) = scratch_paths(dir, &final_name);
    let final_path = dir.join(&final_name);

    let made = backup_into(connection, &plain_path)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            crate::archive::gzip_file(&plain_path, &part_path).map_err(|error| error.to_string())
        })
        .and_then(|()| {
            let _ = std::fs::remove_file(&plain_path);
            std::fs::rename(&part_path, &final_path).map_err(|error| error.to_string())
        });
    if let Err(error) = made {
        let _ = std::fs::remove_file(&plain_path);
        let _ = std::fs::remove_file(&part_path);
        return Err(error);
    }

    let now = now_unix();
    if let Ok(existing) = list_backup_names(dir, stem) {
        for name in backups_to_delete(&existing, stem, now) {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
    if let Ok(leftovers) = list_scratch_names(dir, stem) {
        for name in scratch_to_delete(&leftovers, stem, now) {
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

fn scratch_paths(dir: &Path, final_name: &str) -> (PathBuf, PathBuf) {
    static RUNS: AtomicU64 = AtomicU64::new(0);
    let tag = format!(
        "{}-{}",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    );
    (
        dir.join(format!("{final_name}.{tag}.plain")),
        dir.join(format!("{final_name}.{tag}.part")),
    )
}

pub fn due(dir: &Path, stem: &str, now: i64) -> bool {
    !copy_days(dir, stem).contains(&now.div_euclid(86_400))
}

fn copy_days(dir: &Path, stem: &str) -> Vec<i64> {
    list_backup_names(dir, stem)
        .unwrap_or_default()
        .iter()
        .filter_map(|name| stamp_to_unix(name, stem))
        .map(|at| at.div_euclid(86_400))
        .collect()
}

fn next_open_day_unix(dir: &Path, stem: &str, after: i64) -> i64 {
    let days = copy_days(dir, stem);
    let mut day = after.div_euclid(86_400) + 1;
    while days.contains(&day) {
        day += 1;
    }
    day * 86_400
}

pub fn next_due_unix(after: i64) -> i64 {
    (after.div_euclid(86_400) + 1) * 86_400
}

pub fn latest(dir: &Path, stem: &str) -> Option<i64> {
    list_backup_names(dir, stem)
        .ok()?
        .iter()
        .filter_map(|name| stamp_to_unix(name, stem))
        .max()
}

pub fn backup_into(connection: &Connection, destination: &Path) -> rusqlite::Result<()> {
    let destination = destination
        .to_str()
        .ok_or_else(|| rusqlite::Error::InvalidPath(destination.to_path_buf()))?;
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
        status.next_run_unix = Some(next_open_day_unix(&dir, stem, ran_at));
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
        status.next_run_unix = Some(next_open_day_unix(&dir, stem, now));
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

fn list_scratch_names(dir: &Path, stem: &str) -> std::io::Result<Vec<String>> {
    Ok(std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| scratch_stamp(name, stem).is_some())
        .collect())
}

fn scratch_to_delete(names: &[String], stem: &str, now: i64) -> Vec<String> {
    let cutoff = now - KEEP_DAYS * 86_400;
    names
        .iter()
        .filter(|name| scratch_stamp(name, stem).is_some_and(|at| at < cutoff))
        .cloned()
        .collect()
}

fn scratch_stamp(name: &str, stem: &str) -> Option<i64> {
    let core = name
        .strip_suffix(".plain")
        .or_else(|| name.strip_suffix(".part"))?;
    let finished = match core.rsplit_once('.') {
        Some((head, tag)) if is_run_tag(tag) => head,
        _ => core,
    };
    if !finished.ends_with(".db.gz") {
        return None;
    }
    stamp_to_unix(finished, stem)
}

fn is_run_tag(tag: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    tag.split_once('-')
        .is_some_and(|(pid, run)| digits(pid) && digits(run))
}

fn stamp_to_unix(name: &str, stem: &str) -> Option<i64> {
    let core = name.strip_prefix(&format!("{stem}-"))?;
    let core = core
        .strip_suffix(".gz")
        .unwrap_or(core)
        .strip_suffix(".db")?;
    let (date, time) = core.split_once('-')?;
    let digits = |part: &str, len: usize| {
        part.len() == len && part.bytes().all(|byte| byte.is_ascii_digit())
    };
    if !digits(date, 8) || !digits(time, 4) {
        return None;
    }
    let year = date[0..4].parse().ok()?;
    let month = date[4..6].parse().ok()?;
    let day = date[6..8].parse().ok()?;
    let hour: i64 = time[0..2].parse().ok()?;
    let minute: i64 = time[2..4].parse().ok()?;
    let unix = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60;
    (format_stamp(unix) == core).then_some(unix)
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
    fn a_stamp_outside_the_calendar_or_the_clock_reads_as_nothing() {
        for name in [
            "desk-20261300-0400.db.gz",
            "desk-20260012-0400.db.gz",
            "desk-20261000-0400.db.gz",
            "desk-20261032-0400.db.gz",
            "desk-20260230-0400.db.gz",
            "desk-20260229-0400.db.gz",
            "desk-20260812-2400.db.gz",
            "desk-20260812-0460.db.gz",
            "desk-20261399-9999.db",
            "desk-2026+812-0400.db",
            "desk-20260812-+400.db",
            "desk-202\u{e9}081-0400.db",
            "desk-20260812-0\u{e9}0.db",
        ] {
            assert_eq!(stamp_to_unix(name, "desk"), None, "{name}");
        }
        assert_eq!(
            stamp_to_unix("desk-20280229-2359.db.gz", "desk"),
            Some(1_835_481_540)
        );
        assert_eq!(
            stamp_to_unix("desk-20261231-0000.db", "desk"),
            Some(1_798_675_200)
        );
    }

    #[test]
    fn a_stranger_with_an_impossible_stamp_never_holds_the_next_backup_off() {
        let holder = scratch("impossible-stamp");
        let now = stamp_to_unix("desk-20260818-1200.db.gz", "desk").unwrap();
        std::fs::write(holder.path().join("desk-20261399-9999.db"), b"x").unwrap();
        std::fs::write(holder.path().join("desk-202\u{e9}081-0400.db"), b"x").unwrap();

        assert_eq!(latest(holder.path(), "desk"), None);
        assert!(due(holder.path(), "desk", now));
        assert!(
            backups_to_delete(
                &list_backup_names(holder.path(), "desk").unwrap(),
                "desk",
                now + 30 * 86_400
            )
            .is_empty()
        );
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

    #[test]
    fn stamps_before_1970_read_the_right_calendar_day() {
        assert_eq!(format_stamp(-60), "19691231-2359");
        assert_eq!(format_stamp(-2_208_988_800), "19000101-0000");
        assert_eq!(format_stamp(-310_500_000), "19600229-0600");
        assert_eq!(
            stamp_to_unix("desk-19600229-0600.db.gz", "desk"),
            Some(-310_500_000)
        );
        assert_eq!(stamp_to_unix("desk-19691231-2359.db", "desk"), Some(-60));
    }

    #[test]
    fn leap_days_land_on_the_twenty_ninth_and_centuries_skip_them() {
        assert_eq!(format_stamp(951_782_400), "20000229-0000");
        assert_eq!(format_stamp(1_709_210_040), "20240229-1234");
        assert_eq!(format_stamp(4_107_456_000), "21000228-0000");
        assert_eq!(format_stamp(4_107_456_000 + 86_400), "21000301-0000");
        assert_eq!(
            stamp_to_unix("desk-21000301-0000.db", "desk"),
            Some(4_107_542_400)
        );
    }

    #[test]
    fn the_calendar_holds_before_the_first_era() {
        assert_eq!(civil_from_days(-719_468), (0, 3, 1));
        assert_eq!(civil_from_days(-719_469), (0, 2, 29));
        assert_eq!(days_from_civil(0, 2, 29), -719_469);
        assert_eq!(civil_from_days(-719_834), (-1, 3, 1));
        assert_eq!(days_from_civil(-1, 3, 1), -719_834);
    }

    fn notes_store() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE notes(body TEXT); INSERT INTO notes VALUES ('kept');")
            .unwrap();
        connection
    }

    fn scratch_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".plain") || name.ends_with(".part"))
            .collect()
    }

    #[test]
    fn another_runs_scratch_files_in_the_same_minute_are_left_alone() {
        let holder = scratch("leftover");
        let now = now_unix();
        let mut planted = Vec::new();
        for at in [now, now + 60] {
            let name = format!("desk-{}.db.gz", format_stamp(at));
            for suffix in [".plain", ".part", ".1-0.plain", ".1-0.part"] {
                let path = holder.path().join(format!("{name}{suffix}"));
                std::fs::write(&path, b"in flight").unwrap();
                planted.push(path);
            }
        }

        let kept = keep(&notes_store(), holder.path(), "desk").unwrap();

        for path in &planted {
            assert_eq!(std::fs::read(path).unwrap(), b"in flight");
        }
        assert_eq!(scratch_files(holder.path()).len(), planted.len());
        assert_eq!(std::fs::metadata(&kept.path).unwrap().len(), kept.bytes);
        let mut header = [0u8; 2];
        std::io::Read::read_exact(&mut std::fs::File::open(&kept.path).unwrap(), &mut header)
            .unwrap();
        assert_eq!(header, [0x1f, 0x8b]);
    }

    #[test]
    fn every_run_gets_its_own_scratch_names_that_the_prune_recognises() {
        let dir = Path::new("backups");
        let (first_plain, first_part) = scratch_paths(dir, "desk-20260812-0400.db.gz");
        let (second_plain, second_part) = scratch_paths(dir, "desk-20260812-0400.db.gz");

        assert_ne!(first_plain, second_plain);
        assert_ne!(first_part, second_part);
        let at = stamp_to_unix("desk-20260812-0400.db.gz", "desk");
        for path in [&first_plain, &first_part, &second_plain, &second_part] {
            let name = path.file_name().unwrap().to_str().unwrap();
            assert!(name.contains(&std::process::id().to_string()));
            assert_eq!(scratch_stamp(name, "desk"), at);
        }
    }

    #[test]
    fn only_this_stems_scratch_names_carry_a_stamp() {
        let at = stamp_to_unix("desk-20260812-0400.db.gz", "desk");
        assert_eq!(scratch_stamp("desk-20260812-0400.db.gz.plain", "desk"), at);
        assert_eq!(scratch_stamp("desk-20260812-0400.db.gz.part", "desk"), at);
        assert_eq!(
            scratch_stamp("desk-20260812-0400.db.gz.77-3.part", "desk"),
            at
        );
        for stranger in [
            "desk-20260812-0400.db.gz",
            "desk-20260812-0400.db.gz.abc.plain",
            "desk-20260812-0400.db.gz..plain",
            "desk-20260812-0400.db.gz.-.plain",
            "desk-20260812-0400.db.gz.---.part",
            "desk-20260812-0400.db.gz.77.part",
            "desk-20260812-0400.db.gz.77-3-1.part",
            "desk-20260812-0400.db.plain",
            "other-20260812-0400.db.gz.plain",
            "desk-garbled.db.gz.part",
        ] {
            assert_eq!(scratch_stamp(stranger, "desk"), None, "{stranger}");
        }
    }

    #[test]
    fn keep_keeps_a_copy_and_a_leftover_inside_the_window_and_prunes_both_past_it() {
        let holder = scratch("window");
        let now = now_unix();
        let recent = format!("desk-{}.db.gz", format_stamp(now - 13 * 86_400));
        let old = format!("desk-{}.db.gz", format_stamp(now - 15 * 86_400));
        for name in [
            recent.clone(),
            format!("{recent}.9-0.plain"),
            format!("{recent}.part"),
            old.clone(),
            format!("{old}.9-0.plain"),
            format!("{old}.part"),
        ] {
            std::fs::write(holder.path().join(name), b"x").unwrap();
        }

        keep(&notes_store(), holder.path(), "desk").unwrap();

        assert!(holder.path().join(&recent).exists());
        assert!(holder.path().join(format!("{recent}.9-0.plain")).exists());
        assert!(holder.path().join(format!("{recent}.part")).exists());
        assert!(!holder.path().join(&old).exists());
        assert!(!holder.path().join(format!("{old}.9-0.plain")).exists());
        assert!(!holder.path().join(format!("{old}.part")).exists());
    }

    #[test]
    fn a_failed_keep_leaves_no_scratch_file_behind() {
        let holder = scratch("rename-fails");
        let now = now_unix();
        let mut blockers = Vec::new();
        for at in [now, now + 60] {
            let blocker = holder
                .path()
                .join(format!("desk-{}.db.gz", format_stamp(at)));
            std::fs::create_dir(&blocker).unwrap();
            std::fs::write(blocker.join("inside"), b"x").unwrap();
            blockers.push(blocker);
        }

        assert!(keep(&notes_store(), holder.path(), "desk").is_err());

        assert!(scratch_files(holder.path()).is_empty());
        for blocker in blockers {
            assert_eq!(std::fs::read(blocker.join("inside")).unwrap(), b"x");
        }
    }

    #[test]
    fn run_once_records_the_error_when_the_folder_cannot_be_made() {
        let holder = scratch("run-once-error");
        let blocker = holder.path().join("a-file");
        std::fs::write(&blocker, b"x").unwrap();
        let db = db_with_backup_dir(Some(&blocker.join("backups")));
        let status = Mutex::new(Status::default());

        run_once(&db, &status, true, backup_dir, "desk");

        let status = status.into_inner().unwrap();
        assert!(status.last_error.is_some());
        assert!(status.last_bytes.is_none());
        assert!(!status.running);
        assert!(status.last_run_unix.is_some());
    }

    #[test]
    fn a_copy_stamped_on_a_later_day_never_holds_that_days_backup_off() {
        let holder = scratch("due-future");
        std::fs::write(holder.path().join("desk-20261015-0900.db.gz"), b"x").unwrap();
        let noon = stamp_to_unix("desk-20261014-1200.db", "desk").unwrap();
        assert!(due(holder.path(), "desk", noon));
        std::fs::write(holder.path().join("desk-20261014-0100.db.gz"), b"x").unwrap();
        assert!(!due(holder.path(), "desk", noon));
        assert!(!due(holder.path(), "desk", noon + 86_400));
        assert!(due(holder.path(), "desk", noon + 2 * 86_400));
    }

    #[test]
    fn the_next_run_skips_days_that_already_hold_a_copy() {
        let holder = scratch("next-open-day");
        for name in ["desk-20261015-0900.db.gz", "desk-20261016-0100.db"] {
            std::fs::write(holder.path().join(name), b"x").unwrap();
        }
        let noon = stamp_to_unix("desk-20261014-1200.db", "desk").unwrap();
        assert_eq!(
            next_open_day_unix(holder.path(), "desk", noon),
            stamp_to_unix("desk-20261017-0000.db", "desk").unwrap()
        );
        assert_eq!(
            next_open_day_unix(holder.path(), "desk", noon + 3 * 86_400),
            stamp_to_unix("desk-20261018-0000.db", "desk").unwrap()
        );
    }

    #[test]
    fn a_copy_from_a_clock_that_ran_ahead_gets_one_backup_today_and_no_more() {
        let holder = scratch("future-thread");
        let today = now_unix().div_euclid(86_400);
        let ahead = format!(
            "desk-{}.db.gz",
            format_stamp((today + 1) * 86_400 + 9 * 3_600)
        );
        std::fs::write(holder.path().join(&ahead), b"x").unwrap();
        let db = db_with_backup_dir(Some(holder.path()));

        let handle = spawn(Arc::clone(&db), backup_dir, "desk");
        let deadline = Instant::now() + Duration::from_secs(20);
        let ran = loop {
            let status = handle.status();
            if status.last_run_unix.is_some() {
                break status;
            }
            assert!(Instant::now() < deadline, "no copy was kept: {status:?}");
            std::thread::sleep(Duration::from_millis(2));
        };

        assert_eq!(ran.last_error, None);
        assert_eq!(ran.next_run_unix, Some((today + 2) * 86_400));
        assert_eq!(list_backup_names(holder.path(), "desk").unwrap().len(), 2);
        let again = Mutex::new(Status::default());
        run_once(&db, &again, false, backup_dir, "desk");
        let again = again.into_inner().unwrap();
        assert_eq!(again.last_run_unix, None);
        assert_eq!(again.next_run_unix, Some((today + 2) * 86_400));
        assert_eq!(list_backup_names(holder.path(), "desk").unwrap().len(), 2);
    }
}
