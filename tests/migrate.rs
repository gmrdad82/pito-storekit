use rusqlite::Connection;
use std::path::PathBuf;
use storekit::fixtures::{self, Fixture};
use storekit::migrate::{self, Migrations, Step};
use storekit::paths::App;
use storekit::settings::{self, SETTINGS_TABLE_SQL};

fn scratch(tag: &str) -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("migrate-it-{tag}-"))
        .tempdir_in(root)
        .unwrap()
}

fn fill_titles(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute("UPDATE notes SET title = 'note ' || id", [])?;
    connection.execute("INSERT INTO log(event) VALUES ('backfill 2')", [])?;
    Ok(())
}

fn count_before(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute("INSERT INTO log(event) VALUES ('before')", [])?;
    Ok(())
}

fn exit_halfway(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute("INSERT INTO log(event) VALUES ('halfway')", [])?;
    std::process::exit(3)
}

fn drop_the_first_note(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute("DELETE FROM notes WHERE id = 1", [])?;
    Ok(())
}

fn read_then_write_beside_another_writer(connection: &Connection) -> rusqlite::Result<()> {
    let seen: i64 = connection.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
    let other = Connection::open(connection.path().unwrap_or_default())?;
    other.busy_timeout(std::time::Duration::ZERO)?;
    let other_wrote = other
        .execute("INSERT INTO log(event) VALUES ('other inside')", [])
        .is_ok();
    connection.execute(
        "INSERT INTO log(event) VALUES (?1)",
        [format!("before saw {seen}, other wrote: {other_wrote}")],
    )?;
    Ok(())
}

fn failing_backfill(_connection: &Connection) -> rusqlite::Result<()> {
    Err(rusqlite::Error::InvalidQuery)
}

const V1: &str = "CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL);
     CREATE TABLE log(id INTEGER PRIMARY KEY, event TEXT NOT NULL);
     INSERT INTO notes(body) VALUES ('one'), ('two');
     INSERT INTO log(event) VALUES ('step 1');";
const V2: &str = "ALTER TABLE notes ADD COLUMN title TEXT;
     INSERT INTO log(event) VALUES ('step 2');";
const V3: &str =
    "CREATE TABLE tags(id INTEGER PRIMARY KEY, note INTEGER NOT NULL REFERENCES notes(id));
     INSERT INTO log(event) VALUES ('step 3');";

static ONE: &[Step] = &[Step::new(1, V1, None)];
static THREE: &[Step] = &[
    Step::new(1, V1, None),
    Step::new(2, V2, Some(fill_titles)),
    Step::new(3, V3, None),
];
static BREAKS_AT_THREE: &[Step] = &[
    Step::new(1, V1, None),
    Step::new(2, V2, Some(fill_titles)),
    Step::new(
        3,
        "INSERT INTO log(event) VALUES ('step 3'); INSERT INTO missing_table VALUES (1);",
        None,
    ),
];
static BACKFILL_FAILS: &[Step] = &[
    Step::new(1, V1, None),
    Step::new(
        2,
        "INSERT INTO log(event) VALUES ('step 2');",
        Some(failing_backfill),
    ),
];
static REBUILD_FAILS: &[Step] = &[
    Step::new(1, V1, None),
    Step::new(
        2,
        "CREATE TABLE half(id INTEGER); SELECT * FROM missing_table;",
        None,
    )
    .with_foreign_keys_off(),
];
const V1_TAGGED: &str = "CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL);
     CREATE TABLE log(id INTEGER PRIMARY KEY, event TEXT NOT NULL);
     CREATE TABLE tags(id INTEGER PRIMARY KEY, note INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE, label TEXT NOT NULL);
     INSERT INTO notes(body) VALUES ('one'), ('two');
     INSERT INTO tags(note, label) VALUES (1, 'red'), (2, 'blue');
     INSERT INTO log(event) VALUES ('step 1');";

static TAGGED: &[Step] = &[Step::new(1, V1_TAGGED, None)];
const REBUILD_NOTES: &str = "CREATE TABLE notes_new(id INTEGER PRIMARY KEY, body TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0);
     INSERT INTO notes_new(id, body) SELECT id, body FROM notes;
     DROP TABLE notes;
     ALTER TABLE notes_new RENAME TO notes;
     INSERT INTO log(event) VALUES ('rebuild');";
static REBUILD: &[Step] = &[
    Step::new(1, V1_TAGGED, None),
    Step::new(2, REBUILD_NOTES, None).with_foreign_keys_off(),
];
static CASCADES_BESIDE_A_REBUILD: &[Step] = &[
    Step::new(1, V1_TAGGED, None),
    Step::new(2, REBUILD_NOTES, None).with_foreign_keys_off(),
    Step::new(
        3,
        "INSERT INTO notes(id, body) VALUES (3, 'three');
         INSERT INTO tags(note, label) VALUES (3, 'green'), (2, 'teal');
         DELETE FROM notes WHERE id = 3;",
        None,
    ),
];
static TWO_STATEMENTS_FAIL_ON_THE_SECOND: &[Step] = &[
    Step::new(1, V1, None),
    Step::new(
        2,
        "CREATE TABLE first(id INTEGER); INSERT INTO log(event) VALUES ('step 2'); INSERT INTO missing_table VALUES (1);",
        None,
    ),
];
static CRASHES_IN_THREE: &[Step] = &[
    Step::new(1, V1, None),
    Step::new(2, V2, Some(fill_titles)),
    Step::new(3, V3, Some(exit_halfway)),
];

fn migrations(version: i64, steps: &'static [Step]) -> Migrations {
    Migrations {
        version,
        steps,
        before: Some(count_before),
    }
}

fn version(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
        .parse()
        .unwrap()
}

fn log(connection: &Connection) -> Vec<String> {
    connection
        .prepare("SELECT event FROM log ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn foreign_keys(connection: &Connection) -> bool {
    connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap()
}

fn has_table(connection: &Connection, name: &str) -> bool {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

fn has_column(connection: &Connection, table: &str, column: &str) -> bool {
    connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
            [table, column],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

fn tags(connection: &Connection) -> Vec<(i64, String)> {
    connection
        .prepare("SELECT note, label FROM tags ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn opened_with_foreign_keys(path: &std::path::Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    connection
}

#[test]
fn a_fresh_store_runs_every_step_and_its_backfill_but_not_before() {
    let holder = scratch("fresh");
    let connection =
        storekit::open(&holder.path().join("notes.db"), &migrations(3, THREE), None).unwrap();

    assert_eq!(version(&connection), 3);
    assert_eq!(
        log(&connection),
        vec!["step 1", "step 2", "backfill 2", "step 3"]
    );
    let titles: Vec<String> = connection
        .prepare("SELECT title FROM notes ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(titles, vec!["note 1", "note 2"]);
    assert!(has_table(&connection, "tags"));
}

#[test]
fn reopening_at_the_same_version_runs_no_step_and_calls_before_as_documented() {
    let holder = scratch("same");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(3, THREE), None).unwrap());

    let connection = storekit::open(&path, &migrations(3, THREE), None).unwrap();

    assert_eq!(version(&connection), 3);
    assert_eq!(
        log(&connection),
        vec!["step 1", "step 2", "backfill 2", "step 3", "before"]
    );
}

#[test]
fn reopening_a_current_store_without_before_takes_no_write_lock() {
    let holder = scratch("unlocked");
    let path = holder.path().join("notes.db");
    let plain = Migrations {
        version: 3,
        steps: THREE,
        before: None,
    };
    drop(storekit::open(&path, &plain, None).unwrap());
    let writer = Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let reader = Connection::open(&path).unwrap();
    reader.busy_timeout(std::time::Duration::ZERO).unwrap();

    migrate::run(&reader, &plain).unwrap();

    writer.execute_batch("ROLLBACK").unwrap();
    assert_eq!(version(&reader), 3);
    assert_eq!(
        log(&reader),
        vec!["step 1", "step 2", "backfill 2", "step 3"]
    );
}

#[test]
fn an_older_store_calls_before_once_and_runs_only_the_newer_steps() {
    let holder = scratch("older");
    let path = holder.path().join("notes.db");
    let old = storekit::open(&path, &migrations(1, ONE), None).unwrap();
    assert_eq!(version(&old), 1);
    drop(old);

    let connection = storekit::open(&path, &migrations(3, THREE), None).unwrap();

    assert_eq!(version(&connection), 3);
    assert_eq!(
        log(&connection),
        vec!["step 1", "before", "step 2", "backfill 2", "step 3"]
    );
    let bodies: i64 = connection
        .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(bodies, 2);
}

#[test]
fn a_failing_backfill_returns_its_error_and_the_version_stays_put() {
    let holder = scratch("backfill");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, ONE), None).unwrap());

    let error = storekit::open(&path, &migrations(2, BACKFILL_FAILS), None).unwrap_err();

    assert!(matches!(error, rusqlite::Error::InvalidQuery));
    let connection = Connection::open(&path).unwrap();
    assert_eq!(version(&connection), 1);
    assert_eq!(log(&connection), vec!["step 1", "before"]);
}

#[test]
fn a_fresh_store_whose_first_step_fails_records_no_version() {
    static BROKEN: &[Step] = &[Step::new(1, "CREATE TABLE notes(;", None)];
    let holder = scratch("fresh-broken");
    let path = holder.path().join("notes.db");

    assert!(storekit::open(&path, &migrations(1, BROKEN), None).is_err());

    let connection = Connection::open(&path).unwrap();
    assert!(!has_table(&connection, "schema_meta"));
    assert!(!has_table(&connection, "notes"));
}

#[test]
fn a_rebuild_step_keeps_the_rows_and_their_children_and_turns_foreign_keys_back_on() {
    let holder = scratch("rebuild");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, TAGGED), None).unwrap());

    let connection = storekit::open(&path, &migrations(2, REBUILD), None).unwrap();

    assert_eq!(version(&connection), 2);
    assert!(foreign_keys(&connection));
    let pinned: Vec<(String, i64)> = connection
        .prepare("SELECT body, pinned FROM notes ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(pinned, vec![("one".to_owned(), 0), ("two".to_owned(), 0)]);
    assert_eq!(
        tags(&connection),
        vec![(1, "red".to_owned()), (2, "blue".to_owned())]
    );
    let broken: i64 = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(broken, 0);
}

#[test]
fn foreign_keys_are_back_on_after_a_rebuild_step_fails() {
    let holder = scratch("rebuild-fails");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, ONE), None).unwrap());
    let connection = opened_with_foreign_keys(&path);

    assert!(migrate::run(&connection, &migrations(2, REBUILD_FAILS)).is_err());

    assert!(foreign_keys(&connection));
    assert!(connection.is_autocommit());
    assert_eq!(version(&connection), 1);
    assert!(!has_table(&connection, "half"));
    assert_eq!(log(&connection), vec!["step 1", "before"]);
}

struct Demo;

impl Fixture for Demo {
    fn name(&self) -> &str {
        "demo"
    }

    fn seed(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection.execute("INSERT INTO seeded(name) VALUES ('demo')", [])?;
        Ok(())
    }
}

struct Busy;

impl Fixture for Busy {
    fn name(&self) -> &str {
        "busy"
    }

    fn seed(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection.execute("INSERT INTO seeded(name) VALUES ('busy'), ('busy')", [])?;
        Ok(())
    }
}

struct Half;

impl Fixture for Half {
    fn name(&self) -> &str {
        "half"
    }

    fn seed(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection.execute("INSERT INTO seeded(name) VALUES ('half')", [])?;
        connection.execute("INSERT INTO seeded(name) VALUES (NULL)", [])?;
        Ok(())
    }
}

const FIXTURES: &[&dyn Fixture] = &[&Demo, &Busy, &Half];

fn fixture_store() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(&format!(
            "{SETTINGS_TABLE_SQL} CREATE TABLE seeded(name TEXT NOT NULL);"
        ))
        .unwrap();
    connection
}

fn seeded(connection: &Connection) -> Vec<String> {
    connection
        .prepare("SELECT name FROM seeded ORDER BY rowid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn set_env(key: &str, value: &str) {
    unsafe { std::env::set_var(key, value) };
}

#[test]
fn no_fixture_variable_seeds_nothing() {
    const APP: App = App {
        folder: "storekit-t-fixture-unset",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_UNSET",
    };
    let connection = fixture_store();

    assert_eq!(fixtures::requested(&APP), None);
    assert_eq!(
        fixtures::seed_if_requested(&connection, &APP, FIXTURES).unwrap(),
        None
    );
    assert!(seeded(&connection).is_empty());
    assert_eq!(settings::get(&connection, "fixture_seeded").unwrap(), None);
}

#[test]
fn a_blank_fixture_variable_seeds_nothing() {
    const APP: App = App {
        folder: "storekit-t-fixture-blank",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_BLANK",
    };
    set_env("PITO_STOREKIT_T_FIXTURE_BLANK_FIXTURES", "  \t ");
    let connection = fixture_store();

    assert_eq!(fixtures::requested(&APP), None);
    assert_eq!(
        fixtures::seed_if_requested(&connection, &APP, FIXTURES).unwrap(),
        None
    );
    assert!(seeded(&connection).is_empty());
}

#[test]
fn an_unknown_fixture_is_an_error_and_seeds_nothing() {
    const APP: App = App {
        folder: "storekit-t-fixture-unknown",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_UNKNOWN",
    };
    set_env("PITO_STOREKIT_T_FIXTURE_UNKNOWN_FIXTURES", "nope");
    let connection = fixture_store();

    let error = fixtures::seed_if_requested(&connection, &APP, FIXTURES).unwrap_err();

    assert!(matches!(error, rusqlite::Error::InvalidParameterName(name) if name == "nope"));
    assert!(seeded(&connection).is_empty());
    assert_eq!(settings::get(&connection, "fixture_seeded").unwrap(), None);
}

#[test]
fn a_named_fixture_seeds_once_however_often_the_app_boots() {
    const APP: App = App {
        folder: "storekit-t-fixture-known",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_KNOWN",
    };
    set_env("PITO_STOREKIT_T_FIXTURE_KNOWN_FIXTURES", " busy\n");
    let connection = fixture_store();

    assert_eq!(fixtures::requested(&APP).as_deref(), Some("busy"));
    for _ in 0..3 {
        assert_eq!(
            fixtures::seed_if_requested(&connection, &APP, FIXTURES)
                .unwrap()
                .as_deref(),
            Some("busy")
        );
    }

    assert_eq!(seeded(&connection), vec!["busy", "busy"]);
    assert_eq!(
        settings::get(&connection, "fixture_seeded")
            .unwrap()
            .as_deref(),
        Some("busy")
    );
}

#[test]
fn switching_fixtures_seeds_the_new_one_and_remembers_it() {
    const APP: App = App {
        folder: "storekit-t-fixture-switch",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_SWITCH",
    };
    let connection = fixture_store();
    set_env("PITO_STOREKIT_T_FIXTURE_SWITCH_FIXTURES", "demo");
    fixtures::seed_if_requested(&connection, &APP, FIXTURES).unwrap();
    set_env("PITO_STOREKIT_T_FIXTURE_SWITCH_FIXTURES", "busy");

    fixtures::seed_if_requested(&connection, &APP, FIXTURES).unwrap();

    assert_eq!(seeded(&connection), vec!["demo", "busy", "busy"]);
    assert_eq!(
        settings::get(&connection, "fixture_seeded")
            .unwrap()
            .as_deref(),
        Some("busy")
    );
}

#[test]
fn a_fixture_whose_seed_fails_is_not_remembered() {
    const APP: App = App {
        folder: "storekit-t-fixture-fails",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_FAILS",
    };
    set_env("PITO_STOREKIT_T_FIXTURE_FAILS_FIXTURES", "demo");
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(SETTINGS_TABLE_SQL).unwrap();

    assert!(fixtures::seed_if_requested(&connection, &APP, FIXTURES).is_err());
    assert_eq!(settings::get(&connection, "fixture_seeded").unwrap(), None);
}

#[test]
fn a_fixture_that_fails_halfway_leaves_none_of_its_rows_and_retries_from_scratch() {
    const APP: App = App {
        folder: "storekit-t-fixture-half",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_HALF",
    };
    set_env("PITO_STOREKIT_T_FIXTURE_HALF_FIXTURES", "half");
    let connection = fixture_store();

    for _ in 0..2 {
        assert!(fixtures::seed_if_requested(&connection, &APP, FIXTURES).is_err());
        assert!(seeded(&connection).is_empty());
        assert_eq!(settings::get(&connection, "fixture_seeded").unwrap(), None);
        assert!(connection.is_autocommit());
    }
}

#[test]
fn a_seed_whose_record_cannot_be_written_is_rolled_back() {
    const APP: App = App {
        folder: "storekit-t-fixture-unrecorded",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_UNRECORDED",
    };
    set_env("PITO_STOREKIT_T_FIXTURE_UNRECORDED_FIXTURES", "busy");
    let connection = fixture_store();
    connection
        .execute_batch(
            "CREATE TRIGGER no_record BEFORE INSERT ON settings
             BEGIN SELECT RAISE(ABORT, 'read-only settings'); END;",
        )
        .unwrap();

    assert!(fixtures::seed_if_requested(&connection, &APP, FIXTURES).is_err());

    assert!(seeded(&connection).is_empty());
    assert_eq!(settings::get(&connection, "fixture_seeded").unwrap(), None);
    assert!(connection.is_autocommit());
}

#[test]
fn a_fixture_seeded_inside_the_callers_transaction_rides_it() {
    const APP: App = App {
        folder: "storekit-t-fixture-nested",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_NESTED",
    };
    set_env("PITO_STOREKIT_T_FIXTURE_NESTED_FIXTURES", "demo");
    let connection = fixture_store();
    connection.execute_batch("BEGIN").unwrap();

    fixtures::seed_if_requested(&connection, &APP, FIXTURES).unwrap();

    assert!(!connection.is_autocommit());
    connection.execute_batch("ROLLBACK").unwrap();
    assert!(seeded(&connection).is_empty());
    assert_eq!(settings::get(&connection, "fixture_seeded").unwrap(), None);
}

#[test]
fn an_older_build_leaves_a_newer_stores_version_alone_and_the_newer_build_reopens_it() {
    let holder = scratch("downgrade");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(3, THREE), None).unwrap());

    let older = storekit::open(&path, &migrations(1, ONE), None).unwrap();
    assert_eq!(version(&older), 3);
    drop(older);
    let quiet = Migrations {
        version: 1,
        steps: ONE,
        before: None,
    };
    let older = storekit::open(&path, &quiet, None).unwrap();
    assert_eq!(version(&older), 3);
    drop(older);

    let newer = storekit::open(&path, &migrations(3, THREE), None).unwrap();
    assert_eq!(version(&newer), 3);
    assert!(has_table(&newer, "tags"));
    assert_eq!(
        log(&newer),
        vec![
            "step 1",
            "step 2",
            "backfill 2",
            "step 3",
            "before",
            "before"
        ]
    );
}

#[test]
fn a_file_that_is_not_a_store_is_refused_and_left_byte_for_byte() {
    let holder = scratch("not-a-store");
    let path = holder.path().join("notes.db");
    let bytes = "this is a shopping list, not a store\n".repeat(200);
    std::fs::write(&path, &bytes).unwrap();
    let connection = Connection::open(&path).unwrap();

    let error = migrate::run(&connection, &migrations(3, THREE)).unwrap_err();

    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::NotADatabase)
    );
    drop(connection);
    assert_eq!(std::fs::read(&path).unwrap(), bytes.as_bytes());
    let names: Vec<_> = std::fs::read_dir(holder.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("notes.db")]);
}

#[test]
fn a_step_failing_after_an_earlier_one_keeps_the_earlier_one_and_the_next_open_resumes_there() {
    let holder = scratch("resume");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, ONE), None).unwrap());
    let connection = opened_with_foreign_keys(&path);

    assert!(migrate::run(&connection, &migrations(3, BREAKS_AT_THREE)).is_err());

    assert_eq!(version(&connection), 2);
    assert!(has_column(&connection, "notes", "title"));
    assert_eq!(
        log(&connection),
        vec!["step 1", "before", "step 2", "backfill 2"]
    );
    assert!(foreign_keys(&connection));
    assert!(connection.is_autocommit());

    migrate::run(&connection, &migrations(3, THREE)).unwrap();

    assert_eq!(version(&connection), 3);
    assert_eq!(
        log(&connection),
        vec![
            "step 1",
            "before",
            "step 2",
            "backfill 2",
            "before",
            "step 3"
        ]
    );
    assert!(has_table(&connection, "tags"));
    assert!(foreign_keys(&connection));
}

#[test]
fn a_step_that_fails_on_its_second_statement_leaves_nothing_of_the_first() {
    let holder = scratch("two-statements");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, ONE), None).unwrap());
    let connection = opened_with_foreign_keys(&path);

    assert!(
        migrate::run(
            &connection,
            &migrations(2, TWO_STATEMENTS_FAIL_ON_THE_SECOND)
        )
        .is_err()
    );

    assert_eq!(version(&connection), 1);
    assert!(!has_table(&connection, "first"));
    assert_eq!(log(&connection), vec!["step 1", "before"]);
    assert!(foreign_keys(&connection));
    assert!(connection.is_autocommit());
}

const CRASH_CHILD: &str = "PITO_STOREKIT_T_MIGRATE_CRASH_CHILD";

#[test]
fn migrate_crash_child() {
    let Some(path) = std::env::var_os(CRASH_CHILD) else {
        return;
    };
    let _ = storekit::open(
        std::path::Path::new(&path),
        &migrations(3, CRASHES_IN_THREE),
        None,
    );
    std::process::exit(0);
}

#[test]
fn a_process_that_dies_mid_step_leaves_nothing_of_that_step_and_the_next_open_completes() {
    let holder = scratch("crash");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, ONE), None).unwrap());

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "migrate_crash_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CRASH_CHILD, &path)
        .status()
        .unwrap();

    assert_eq!(status.code(), Some(3));
    let connection = opened_with_foreign_keys(&path);
    assert_eq!(version(&connection), 2);
    assert!(has_column(&connection, "notes", "title"));
    assert!(!has_table(&connection, "tags"));
    assert_eq!(
        log(&connection),
        vec!["step 1", "before", "step 2", "backfill 2"]
    );
    drop(connection);

    let connection = storekit::open(&path, &migrations(3, THREE), None).unwrap();
    assert_eq!(version(&connection), 3);
    assert!(has_table(&connection, "tags"));
    assert_eq!(
        log(&connection),
        vec![
            "step 1",
            "before",
            "step 2",
            "backfill 2",
            "before",
            "step 3"
        ]
    );
    assert!(foreign_keys(&connection));
}

#[test]
fn cascades_fire_in_before_and_plain_steps_beside_a_rebuild_that_keeps_its_children() {
    let holder = scratch("cascades");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, TAGGED), None).unwrap());
    let cascading = Migrations {
        version: 3,
        steps: CASCADES_BESIDE_A_REBUILD,
        before: Some(drop_the_first_note),
    };

    let connection = storekit::open(&path, &cascading, None).unwrap();

    assert_eq!(version(&connection), 3);
    assert!(foreign_keys(&connection));
    assert!(has_column(&connection, "notes", "pinned"));
    let notes: Vec<i64> = connection
        .prepare("SELECT id FROM notes ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(notes, vec![2]);
    assert_eq!(
        tags(&connection),
        vec![(2, "blue".to_owned()), (2, "teal".to_owned())]
    );
    let broken: i64 = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(broken, 0);
}

#[test]
fn migrating_inside_the_callers_transaction_rides_it() {
    let holder = scratch("caller-transaction");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, ONE), None).unwrap());
    let connection = opened_with_foreign_keys(&path);
    connection.execute_batch("BEGIN").unwrap();

    migrate::run(&connection, &migrations(2, REBUILD_FAILS)).unwrap_err();
    assert!(!connection.is_autocommit());
    migrate::run(&connection, &migrations(3, THREE)).unwrap();

    assert!(!connection.is_autocommit());
    assert_eq!(version(&connection), 3);
    assert!(foreign_keys(&connection));
    connection.execute_batch("ROLLBACK").unwrap();
    assert_eq!(version(&connection), 1);
    assert!(!has_column(&connection, "notes", "title"));
    assert!(!has_table(&connection, "half"));
    assert_eq!(log(&connection), vec!["step 1"]);
}

#[test]
fn a_before_that_reads_then_writes_keeps_the_write_lock_and_another_writer_goes_after() {
    let holder = scratch("read-then-write");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, ONE), None).unwrap());
    let contended = Migrations {
        version: 1,
        steps: ONE,
        before: Some(read_then_write_beside_another_writer),
    };

    let connection = storekit::open(&path, &contended, None).unwrap();

    assert_eq!(version(&connection), 1);
    assert_eq!(
        log(&connection),
        vec!["step 1", "before saw 2, other wrote: false"]
    );
    let other = Connection::open(&path).unwrap();
    other.busy_timeout(storekit::DESK_PEN_WAIT).unwrap();
    other
        .execute("INSERT INTO log(event) VALUES ('other after')", [])
        .unwrap();
    assert_eq!(
        log(&connection),
        vec!["step 1", "before saw 2, other wrote: false", "other after"]
    );
}

#[test]
fn a_version_that_cannot_be_recorded_takes_its_step_back_with_it() {
    let holder = scratch("unrecorded-version");
    let path = holder.path().join("notes.db");
    let connection = opened_with_foreign_keys(&path);
    connection
        .execute_batch(
            "CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TRIGGER no_version BEFORE INSERT ON schema_meta
             BEGIN SELECT RAISE(ABORT, 'no version'); END;",
        )
        .unwrap();

    let error = migrate::run(&connection, &migrations(1, ONE)).unwrap_err();

    assert!(error.to_string().contains("no version"), "{error}");
    assert!(!has_table(&connection, "notes"));
    assert!(!has_table(&connection, "log"));
    assert!(connection.is_autocommit());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM schema_meta", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

const ONCE_2: &str = "INSERT INTO log(event) VALUES ('step 2');";
const ONCE_3: &str = "INSERT INTO log(event) VALUES ('step 3');";
static TWIN_STEPS: &[Step] = &[Step::new(1, V1, None), Step::new(2, ONCE_2, None)];
static NEWER_STEPS: &[Step] = &[
    Step::new(1, V1, None),
    Step::new(2, ONCE_2, None),
    Step::new(3, ONCE_3, None),
];
static TWIN_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
static TWIN_OPENED: std::sync::OnceLock<Result<i64, String>> = std::sync::OnceLock::new();
static TWIN_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn open_a_twin_after_the_version_read(sql: &str, _took: std::time::Duration) {
    if !sql.contains("FROM schema_meta WHERE key = 'version'") {
        return;
    }
    let Some(path) = TWIN_PATH.get() else {
        return;
    };
    if TWIN_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let twin = Migrations {
        version: 2,
        steps: TWIN_STEPS,
        before: None,
    };
    let opened = storekit::open(path, &twin, None)
        .map(|connection| version(&connection))
        .map_err(|error| error.to_string());
    let _ = TWIN_OPENED.set(opened);
}

#[test]
fn two_openers_racing_on_an_upgrade_run_each_step_once_and_the_newer_steps_still_run() {
    let holder = scratch("two-openers");
    let path = holder.path().join("notes.db");
    drop(storekit::open(&path, &migrations(1, ONE), None).unwrap());
    TWIN_PATH.set(path.clone()).unwrap();
    let newer = Migrations {
        version: 3,
        steps: NEWER_STEPS,
        before: None,
    };

    let connection =
        storekit::open(&path, &newer, Some(open_a_twin_after_the_version_read)).unwrap();

    assert_eq!(TWIN_OPENED.get(), Some(&Ok(2)));
    assert_eq!(log(&connection), vec!["step 1", "step 2", "step 3"]);
    assert_eq!(version(&connection), 3);
    assert!(foreign_keys(&connection));
}

struct ReadsThenWrites;

impl Fixture for ReadsThenWrites {
    fn name(&self) -> &str {
        "reads-then-writes"
    }

    fn seed(&self, connection: &Connection) -> rusqlite::Result<()> {
        let seen: i64 =
            connection.query_row("SELECT COUNT(*) FROM seeded", [], |row| row.get(0))?;
        let other = Connection::open(connection.path().unwrap_or_default())?;
        other.busy_timeout(std::time::Duration::ZERO)?;
        let other_wrote = other
            .execute("INSERT INTO seeded(name) VALUES ('other inside')", [])
            .is_ok();
        connection.execute(
            "INSERT INTO seeded(name) VALUES (?1)",
            [format!("seed saw {seen}, other wrote: {other_wrote}")],
        )?;
        Ok(())
    }
}

fn make_settings(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(SETTINGS_TABLE_SQL)
}

static SEEDED_STORE: &[Step] = &[Step::new(
    1,
    "CREATE TABLE seeded(name TEXT NOT NULL);",
    Some(make_settings),
)];

#[test]
fn a_seed_that_reads_then_writes_keeps_the_write_lock_and_another_writer_goes_after() {
    const APP: App = App {
        folder: "storekit-t-fixture-lock",
        env_prefix: "PITO_STOREKIT_T_FIXTURE_LOCK",
    };
    set_env("PITO_STOREKIT_T_FIXTURE_LOCK_FIXTURES", "reads-then-writes");
    let holder = scratch("fixture-lock");
    let path = holder.path().join("notes.db");
    let store = Migrations {
        version: 1,
        steps: SEEDED_STORE,
        before: None,
    };
    let connection = storekit::open(&path, &store, None).unwrap();

    let named = fixtures::seed_if_requested(&connection, &APP, &[&ReadsThenWrites]).unwrap();

    assert_eq!(named.as_deref(), Some("reads-then-writes"));
    assert!(connection.is_autocommit());
    assert_eq!(seeded(&connection), vec!["seed saw 0, other wrote: false"]);
    let other = Connection::open(&path).unwrap();
    other.busy_timeout(storekit::DESK_PEN_WAIT).unwrap();
    other
        .execute("INSERT INTO seeded(name) VALUES ('other after')", [])
        .unwrap();
    assert_eq!(
        seeded(&connection),
        vec!["seed saw 0, other wrote: false", "other after"]
    );
}
