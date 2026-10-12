use crate::migrate::{self, Migrations};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Db = Arc<Mutex<Connection>>;

pub const DESK_PEN_WAIT: Duration = Duration::from_secs(1);

pub const JOB_PEN_WAIT: Duration = Duration::from_secs(30);

pub const SLOW_SQL_ALARM: Duration = Duration::from_millis(250);

pub fn open(
    path: &Path,
    migrations: &Migrations,
    mark: Option<fn(&str, Duration)>,
) -> rusqlite::Result<Connection> {
    let mut connection = Connection::open(path)?;
    connection.profile(mark);
    connection.busy_timeout(DESK_PEN_WAIT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "cache_size", "-32768")?;
    connection.pragma_update(None, "mmap_size", "268435456")?;
    migrate::run(&connection, migrations)?;
    Ok(connection)
}

pub fn open_companion(
    path: &Path,
    mark: Option<fn(&str, Duration)>,
) -> rusqlite::Result<Connection> {
    let mut connection = Connection::open(path)?;
    connection.profile(mark);
    connection.busy_timeout(JOB_PEN_WAIT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::{Migrations, Step};
    use std::path::PathBuf;

    fn scratch(tag: &str) -> tempfile::TempDir {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix(tag)
            .tempdir_in(root)
            .unwrap()
    }

    #[test]
    fn the_pen_waits_and_slow_alarm_keep_the_estate_ruling() {
        assert_eq!(DESK_PEN_WAIT, Duration::from_secs(1));
        assert_eq!(JOB_PEN_WAIT, Duration::from_secs(30));
        assert_eq!(SLOW_SQL_ALARM, Duration::from_millis(250));
    }

    #[test]
    fn open_applies_the_desk_pragmas_and_migrates() {
        static STEPS: &[Step] = &[Step::new(1, "CREATE TABLE desk(id INTEGER);", None)];
        let migrations = Migrations {
            version: 1,
            steps: STEPS,
            before: None,
        };
        let holder = scratch("open-pragmas-");
        let connection = open(&holder.path().join("desk.db"), &migrations, None).unwrap();
        assert_eq!(pragma::<String>(&connection, "journal_mode"), "wal");
        assert_eq!(pragma::<i64>(&connection, "foreign_keys"), 1);
        assert_eq!(pragma::<i64>(&connection, "synchronous"), 1);
        assert_eq!(pragma::<i64>(&connection, "cache_size"), -32_768);
        assert_eq!(pragma::<i64>(&connection, "mmap_size"), 268_435_456);
    }

    #[test]
    fn open_companion_never_migrates() {
        let holder = scratch("companion-no-migrate-");
        let connection = open_companion(&holder.path().join("desk.db"), None).unwrap();
        assert!(
            connection
                .query_row("SELECT value FROM schema_meta", [], |row| row
                    .get::<_, String>(0))
                .is_err()
        );
    }

    fn pragma<T: rusqlite::types::FromSql>(connection: &Connection, name: &str) -> T {
        connection
            .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .unwrap()
    }
}
