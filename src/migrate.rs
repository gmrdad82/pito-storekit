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
    connection.execute_batch(SCHEMA_META_SQL)?;
    let current = stored_version(connection)?;

    if current > 0
        && let Some(before) = migrations.before
    {
        before(connection)?;
    }

    for step in migrations.steps.iter().filter(|step| step.to > current) {
        run_step(connection, step)?;
    }

    if current != migrations.version {
        connection.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![migrations.version.to_string()],
        )?;
    }
    Ok(())
}

fn run_step(connection: &Connection, step: &Step) -> rusqlite::Result<()> {
    if !step.foreign_keys_off {
        connection.execute_batch(step.sql)?;
    } else {
        connection.pragma_update(None, "foreign_keys", "OFF")?;
        let rebuilt = connection.execute_batch(step.sql);
        let restored = connection.pragma_update(None, "foreign_keys", "ON");
        rebuilt?;
        restored?;
    }
    if let Some(backfill) = step.backfill {
        backfill(connection)?;
    }
    Ok(())
}

fn stored_version(connection: &Connection) -> rusqlite::Result<i64> {
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
    static REBUILD_STEPS: &[Step] =
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
            "the target version is recorded only once after every listed step"
        );
    }

    #[test]
    fn a_rebuild_restores_foreign_keys() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        let migrations = Migrations {
            version: 1,
            steps: REBUILD_STEPS,
            before: None,
        };

        run(&connection, &migrations).unwrap();

        assert!(
            connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, bool>(0))
                .unwrap()
        );
    }
}
