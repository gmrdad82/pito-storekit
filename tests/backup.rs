use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use storekit::backup::{self, Status};
use storekit::settings::{self, SETTINGS_TABLE_SQL};

fn scratch(tag: &str) -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("backup-it-{tag}-"))
        .tempdir_in(root)
        .unwrap()
}

fn store_with_notes(rows: &[&str]) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(&format!(
            "{SETTINGS_TABLE_SQL} CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL);"
        ))
        .unwrap();
    for row in rows {
        connection
            .execute("INSERT INTO notes(body) VALUES (?1)", [row])
            .unwrap();
    }
    connection
}

fn restore(gz: &Path, into: &Path) -> Connection {
    let mut decoder = flate2::read::GzDecoder::new(std::fs::File::open(gz).unwrap());
    let mut plain = std::fs::File::create(into).unwrap();
    std::io::copy(&mut decoder, &mut plain).unwrap();
    Connection::open(into).unwrap()
}

fn bodies(connection: &Connection) -> Vec<String> {
    connection
        .prepare("SELECT body FROM notes ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn backup_dir(connection: &Connection) -> Option<PathBuf> {
    settings::get(connection, "backup_dir")
        .ok()
        .flatten()
        .map(PathBuf::from)
}

fn wait_for(handle: &backup::Handle, done: impl Fn(&Status) -> bool) -> Status {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = handle.status();
        if done(&status) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "status never settled: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn keep_writes_a_gzip_that_restores_to_the_source_rows() {
    let holder = scratch("restore");
    let dir = holder.path().join("nested").join("backups");
    let connection = store_with_notes(&["first", "second", "third"]);

    let kept = backup::keep(&connection, &dir, "notes").unwrap();

    assert_eq!(kept.path.parent(), Some(dir.as_path()));
    let name = kept.path.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("notes-") && name.ends_with(".db.gz"));
    assert_eq!(name.len(), "notes-YYYYMMDD-HHMM.db.gz".len());
    assert_eq!(names(&dir), vec![name.to_owned()]);
    assert_eq!(std::fs::metadata(&kept.path).unwrap().len(), kept.bytes);
    assert!(kept.bytes > 0);
    let restored = restore(&kept.path, &holder.path().join("restored.db"));
    assert_eq!(bodies(&restored), vec!["first", "second", "third"]);
    assert_eq!(backup::latest(&dir, "notes").map(|at| at % 60), Some(0));
    assert!(!backup::due(
        &dir,
        "notes",
        backup::latest(&dir, "notes").unwrap()
    ));
}

#[test]
fn keep_prunes_only_its_own_copies_past_the_window() {
    let holder = scratch("prune");
    let dir = holder.path();
    let strangers = [
        "notes.txt",
        "notes-garbled.db",
        "other-20200101-0000.db.gz",
        "notes-20200101-0000.sqlite",
        "notes-29991231-2359.db.gz",
        "other-20200101-0000.db.gz.plain",
        "other-20200101-0000.db.gz.12-0.part",
        "notes-20200101-0000.db.gz.tmp.plain",
        "notes-20200101-0000.db.plain",
    ];
    for name in strangers {
        std::fs::write(dir.join(name), b"keep me").unwrap();
    }
    let old = [
        "notes-20200101-0000.db.gz",
        "notes-20200102-1200.db",
        "notes-20200101-0000.db.gz.plain",
        "notes-20200101-0000.db.gz.part",
        "notes-20200103-0900.db.gz.31337-4.plain",
        "notes-20200103-0900.db.gz.31337-4.part",
    ];
    for name in old {
        std::fs::write(dir.join(name), b"old").unwrap();
    }
    let connection = store_with_notes(&["only"]);

    let kept = backup::keep(&connection, dir, "notes").unwrap();

    let left = names(dir);
    for name in old {
        assert!(!left.contains(&name.to_owned()), "{name} was kept");
    }
    for name in strangers {
        assert!(left.contains(&name.to_owned()), "{name} was removed");
        assert_eq!(std::fs::read(dir.join(name)).unwrap(), b"keep me");
    }
    assert!(kept.path.exists());
    assert_eq!(left.len(), strangers.len() + 1);
}

#[test]
fn keep_refuses_a_folder_it_cannot_create() {
    let holder = scratch("no-folder");
    let blocker = holder.path().join("a-file");
    std::fs::write(&blocker, b"x").unwrap();
    let connection = store_with_notes(&["row"]);

    assert!(backup::keep(&connection, &blocker.join("backups"), "notes").is_err());
    assert_eq!(std::fs::read(&blocker).unwrap(), b"x");
}

#[cfg(unix)]
#[test]
fn keep_in_a_folder_it_can_write_but_not_list_keeps_the_copy_and_prunes_nothing() {
    use std::os::unix::fs::PermissionsExt;

    let holder = scratch("write-only");
    let dir = holder.path().join("drop-box");
    std::fs::create_dir(&dir).unwrap();
    let old = "notes-20200101-0000.db.gz";
    std::fs::write(dir.join(old), b"old").unwrap();
    let connection = store_with_notes(&["first", "second"]);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();
    if std::fs::read_dir(&dir).is_ok() {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }

    let kept = backup::keep(&connection, &dir, "notes");

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let kept = kept.unwrap();
    assert_eq!(kept.bytes, std::fs::metadata(&kept.path).unwrap().len());
    assert!(kept.bytes > 0);
    let kept_name = kept.path.file_name().unwrap().to_str().unwrap().to_owned();
    assert_eq!(names(&dir), vec![old.to_owned(), kept_name]);
    assert_eq!(std::fs::read(dir.join(old)).unwrap(), b"old");
    let restored = restore(&kept.path, &holder.path().join("restored.db"));
    assert_eq!(bodies(&restored), vec!["first", "second"]);
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_that_is_not_utf8_returns_an_error_instead_of_panicking() {
    use std::os::unix::ffi::OsStrExt;

    let holder = scratch("not-utf8");
    let dir = holder
        .path()
        .join(std::ffi::OsStr::from_bytes(b"backups-\xff"));
    let connection = store_with_notes(&["row"]);

    assert!(backup::keep(&connection, &dir, "notes").is_err());
    assert!(backup::backup_into(&connection, &dir.join("copy.db")).is_err());
    assert!(names(&dir).is_empty());
}

#[test]
fn backup_into_writes_a_plain_store_and_refuses_to_overwrite_one() {
    let holder = scratch("into");
    let connection = store_with_notes(&["a", "b"]);
    let copy = holder.path().join("copy.db");

    backup::backup_into(&connection, &copy).unwrap();

    assert_eq!(bodies(&Connection::open(&copy).unwrap()), vec!["a", "b"]);
    connection
        .execute("INSERT INTO notes(body) VALUES ('c')", [])
        .unwrap();
    assert!(backup::backup_into(&connection, &copy).is_err());
    assert_eq!(bodies(&Connection::open(&copy).unwrap()), vec!["a", "b"]);
}

#[test]
fn the_backup_thread_waits_for_a_folder_then_keeps_a_copy_on_demand() {
    let holder = scratch("spawn");
    let dir = holder.path().join("backups");
    let db = Arc::new(Mutex::new(store_with_notes(&["spawned"])));
    let handle = backup::spawn(Arc::clone(&db), backup_dir, "notes");

    let idle = wait_for(&handle, |status| status.unconfigured);
    assert_eq!(idle.next_run_unix, None);
    assert_eq!(idle.last_run_unix, None);

    settings::set(&db.lock().unwrap(), "backup_dir", &dir.to_string_lossy()).unwrap();
    handle.backup_now();
    let ran = wait_for(&handle, |status| status.last_run_unix.is_some());

    assert!(!ran.unconfigured);
    assert!(!ran.running);
    assert_eq!(ran.last_error, None);
    let last = ran.last_run_unix.unwrap();
    assert_eq!(ran.next_run_unix, Some(backup::next_due_unix(last)));
    assert!(ran.next_run_unix.unwrap() > last);
    let copies = names(&dir);
    assert_eq!(copies.len(), 1);
    let copy = dir.join(&copies[0]);
    assert_eq!(
        ran.last_bytes,
        Some(std::fs::metadata(&copy).unwrap().len())
    );
    let restored = restore(&copy, &holder.path().join("restored.db"));
    assert_eq!(bodies(&restored), vec!["spawned"]);

    settings::clear(&db.lock().unwrap(), "backup_dir").unwrap();
    handle.backup_now();
    wait_for(&handle, |status| status.unconfigured);
}

#[test]
fn the_backup_thread_reports_a_folder_it_cannot_write() {
    let holder = scratch("spawn-error");
    let blocker = holder.path().join("a-file");
    std::fs::write(&blocker, b"x").unwrap();
    let db = Arc::new(Mutex::new(store_with_notes(&["row"])));
    settings::set(
        &db.lock().unwrap(),
        "backup_dir",
        &blocker.join("backups").to_string_lossy(),
    )
    .unwrap();

    let handle = backup::spawn(Arc::clone(&db), backup_dir, "notes");
    let failed = wait_for(&handle, |status| status.last_error.is_some());

    assert!(!failed.running);
    assert_eq!(failed.last_bytes, None);
    assert!(failed.last_run_unix.is_some());
    assert_eq!(std::fs::read(&blocker).unwrap(), b"x");

    settings::clear(&db.lock().unwrap(), "backup_dir").unwrap();
    handle.backup_now();
    wait_for(&handle, |status| status.unconfigured);
}

#[test]
fn scratch_files_never_count_as_a_copy() {
    let holder = scratch("scratch-not-copy");
    let dir = holder.path();
    for name in [
        "notes-29991231-2359.db.gz.plain",
        "notes-29991231-2359.db.gz.part",
        "notes-29991231-2359.db.gz.5-0.plain",
    ] {
        std::fs::write(dir.join(name), b"half").unwrap();
    }

    assert_eq!(backup::latest(dir, "notes"), None);
    assert!(backup::due(dir, "notes", 32_503_679_000));
}

#[test]
fn the_backup_thread_still_runs_when_only_scratch_files_are_there() {
    let holder = scratch("spawn-scratch");
    let dir = holder.path().join("backups");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes-29991231-2359.db.gz.7-0.plain"), b"half").unwrap();
    let db = Arc::new(Mutex::new(store_with_notes(&["row"])));
    settings::set(&db.lock().unwrap(), "backup_dir", &dir.to_string_lossy()).unwrap();

    let handle = backup::spawn(Arc::clone(&db), backup_dir, "notes");
    let ran = wait_for(&handle, |status| status.last_run_unix.is_some());

    assert_eq!(ran.last_error, None);
    assert_eq!(
        ran.next_run_unix,
        Some(backup::next_due_unix(ran.last_run_unix.unwrap()))
    );
    assert!(backup::latest(&dir, "notes").is_some());

    settings::clear(&db.lock().unwrap(), "backup_dir").unwrap();
    handle.backup_now();
    wait_for(&handle, |status| status.unconfigured);
}

#[test]
fn latest_reads_the_newest_stamp_past_a_stranger_that_sorts_last() {
    let holder = scratch("latest-stranger");
    let dir = holder.path();
    for name in [
        "notes-20261001-0800.db.gz",
        "notes-20261014-1901.db.gz",
        "notes-before-upgrade.db",
    ] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    assert_eq!(backup::latest(dir, "notes"), Some(1_792_004_460));
    assert!(!backup::due(dir, "notes", 1_792_020_600));
    assert!(backup::due(dir, "notes", 1_792_020_600 + 86_400));
}

#[test]
fn the_backup_thread_keeps_no_second_copy_beside_a_stranger_that_sorts_last() {
    let holder = scratch("spawn-stranger");
    let dir = holder.path().join("backups");
    let db = Arc::new(Mutex::new(store_with_notes(&["row"])));
    backup::keep(&db.lock().unwrap(), &dir, "notes").unwrap();
    std::fs::write(dir.join("notes-before-upgrade.db"), b"mine").unwrap();
    let before = names(&dir);
    settings::set(&db.lock().unwrap(), "backup_dir", &dir.to_string_lossy()).unwrap();

    let handle = backup::spawn(Arc::clone(&db), backup_dir, "notes");
    let polled = wait_for(&handle, |status| status.next_run_unix.is_some());

    assert_eq!(polled.last_run_unix, None);
    assert!(!polled.unconfigured);
    assert_eq!(
        polled.next_run_unix,
        backup::latest(&dir, "notes").map(backup::next_due_unix)
    );
    assert_eq!(names(&dir), before);
    assert_eq!(
        std::fs::read(dir.join("notes-before-upgrade.db")).unwrap(),
        b"mine"
    );

    settings::clear(&db.lock().unwrap(), "backup_dir").unwrap();
    handle.backup_now();
    wait_for(&handle, |status| status.unconfigured);
}
