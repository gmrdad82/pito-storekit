use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA_META_SQL: &str =
    "CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);";

#[derive(Debug, Clone, Copy)]
pub struct Step {
    pub to: i64,
    pub sql: &'static str,
    pub backfill: Option<fn(&Connection) -> rusqlite::Result<()>>,
    foreign_keys_off: bool,
}

impl Step {
    pub const fn new(
        to: i64,
        sql: &'static str,
        backfill: Option<fn(&Connection) -> rusqlite::Result<()>>,
    ) -> Self {
        Self {
            to,
            sql,
            backfill,
            foreign_keys_off: false,
        }
    }

    pub const fn with_foreign_keys_off(mut self) -> Self {
        self.foreign_keys_off = true;
        self
    }
}

#[derive(Debug)]
pub struct Migrations {
    pub version: i64,
    pub steps: &'static [Step],
    pub before: Option<fn(&Connection) -> rusqlite::Result<()>>,
}

pub fn run(connection: &Connection, migrations: &Migrations) -> rusqlite::Result<()> {
    let current = stored_version(connection)?;
    if current > 0 && current >= migrations.version && migrations.before.is_none() {
        return Ok(());
    }

    if current > 0
        && let Some(before) = migrations.before
    {
        atomically(connection, MIGRATE_SAVEPOINT, || before(connection))?;
    }

    let mut seen = current;
    for step in migrations.steps.iter().filter(|step| step.to > current) {
        seen = run_step(connection, step, seen)?;
    }

    atomically(connection, MIGRATE_SAVEPOINT, || {
        raise_version(connection, migrations.version)
    })
}

fn run_step(connection: &Connection, step: &Step, seen: i64) -> rusqlite::Result<i64> {
    let switch = step.foreign_keys_off && connection.is_autocommit();
    if switch {
        connection.pragma_update(None, "foreign_keys", "OFF")?;
    }
    let ran = atomically(connection, MIGRATE_SAVEPOINT, || {
        let stored = stored_version(connection)?;
        if step.to <= stored && stored > seen {
            return Ok(seen);
        }
        connection.execute_batch(step.sql)?;
        if let Some(backfill) = step.backfill {
            backfill(connection)?;
        }
        raise_version(connection, step.to)?;
        Ok(stored.max(step.to))
    });
    let restored = if switch {
        connection.pragma_update(None, "foreign_keys", "ON")
    } else {
        Ok(())
    };
    let seen = ran?;
    restored?;
    Ok(seen)
}

const MIGRATE_SAVEPOINT: &str = "storekit_migrate";

pub(crate) fn atomically<T>(
    connection: &Connection,
    savepoint: &str,
    work: impl FnOnce() -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let (start, finish, undo) = if connection.is_autocommit() {
        (
            "BEGIN IMMEDIATE".to_owned(),
            "COMMIT".to_owned(),
            "ROLLBACK".to_owned(),
        )
    } else {
        (
            format!("SAVEPOINT {savepoint}"),
            format!("RELEASE {savepoint}"),
            format!("ROLLBACK TO {savepoint}; RELEASE {savepoint}"),
        )
    };
    connection.execute_batch(&start)?;
    let done = work().and_then(|value| connection.execute_batch(&finish).map(|()| value));
    if done.is_err() {
        let _ = connection.execute_batch(&undo);
    }
    done
}

fn raise_version(connection: &Connection, to: i64) -> rusqlite::Result<()> {
    connection.execute_batch(SCHEMA_META_SQL)?;
    if to > stored_version(connection)? {
        connection.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![to.to_string()],
        )?;
    }
    Ok(())
}

fn stored_version(connection: &Connection) -> rusqlite::Result<i64> {
    let has_meta: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_meta')",
        [],
        |row| row.get(0),
    )?;
    if !has_meta {
        return Ok(0);
    }
    Ok(connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .and_then(|value| value.parse().ok())
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    static OUT_OF_NUMBER_ORDER: &[Step] = &[
        Step::new(
            10,
            "CREATE TABLE order_log(value INTEGER); INSERT INTO order_log VALUES (10);",
            None,
        ),
        Step::new(8, "INSERT INTO order_log VALUES (8);", None),
        Step::new(2, "INSERT INTO order_log VALUES (2);", None),
    ];
    static DONE_REBUILD: &[Step] =
        &[Step::new(1, "CREATE TABLE rebuilt(id INTEGER);", None).with_foreign_keys_off()];

    #[test]
    fn the_runner_honours_listed_order_and_records_the_version_once() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO schema_meta VALUES ('version', '0');
                 CREATE TABLE version_writes(count INTEGER NOT NULL);
                 INSERT INTO version_writes VALUES (0);
                 CREATE TRIGGER count_version_writes AFTER UPDATE ON schema_meta
                 WHEN new.key = 'version'
                 BEGIN UPDATE version_writes SET count = count + 1; END;",
            )
            .unwrap();
        let migrations = Migrations {
            version: 10,
            steps: OUT_OF_NUMBER_ORDER,
            before: None,
        };

        run(&connection, &migrations).unwrap();

        let listed = connection
            .prepare("SELECT value FROM order_log ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(listed, vec![10, 8, 2]);
        assert_eq!(stored_version(&connection).unwrap(), 10);
        assert_eq!(
            connection
                .query_row("SELECT count FROM version_writes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1,
            "a step below the stored version never rewrites it"
        );
    }

    fn parent_and_child() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE parent(id INTEGER PRIMARY KEY);
                 CREATE TABLE child(id INTEGER PRIMARY KEY, parent INTEGER NOT NULL REFERENCES parent(id));
                 INSERT INTO parent VALUES (1), (2);
                 INSERT INTO child VALUES (10, 1), (20, 2);
                 CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO schema_meta VALUES ('version', '1');",
            )
            .unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        connection
    }

    fn foreign_keys(connection: &Connection) -> bool {
        connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_rebuild_already_applied_leaves_foreign_keys_as_the_caller_set_them() {
        let connection = parent_and_child();
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        let migrations = Migrations {
            version: 1,
            steps: DONE_REBUILD,
            before: None,
        };

        run(&connection, &migrations).unwrap();

        assert!(!foreign_keys(&connection));
        assert_eq!(stored_version(&connection).unwrap(), 1);
    }
}
