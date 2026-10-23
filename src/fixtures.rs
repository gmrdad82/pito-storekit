use crate::paths::App;
use crate::settings;
use rusqlite::Connection;

const SEEDED_KEY: &str = "fixture_seeded";

pub trait Fixture {
    fn name(&self) -> &str;
    fn seed(&self, connection: &Connection) -> rusqlite::Result<()>;
}

pub fn requested(app: &App) -> Option<String> {
    std::env::var(app.env_key("FIXTURES"))
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

pub fn seed_if_requested(
    connection: &Connection,
    app: &App,
    fixtures: &[&dyn Fixture],
) -> rusqlite::Result<Option<String>> {
    seed_named(connection, requested(app), fixtures)
}

fn seed_named(
    connection: &Connection,
    requested: Option<String>,
    fixtures: &[&dyn Fixture],
) -> rusqlite::Result<Option<String>> {
    let Some(name) = requested else {
        return Ok(None);
    };
    if settings::get(connection, SEEDED_KEY)?.as_deref() == Some(&name) {
        return Ok(Some(name));
    }
    let fixture = fixtures
        .iter()
        .find(|fixture| fixture.name() == name)
        .ok_or_else(|| rusqlite::Error::InvalidParameterName(name.clone()))?;
    crate::migrate::atomically(connection, "storekit_fixture", || {
        fixture.seed(connection)?;
        settings::set(connection, SEEDED_KEY, &name)
    })?;
    Ok(Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SETTINGS_TABLE_SQL;

    struct Demo;

    impl Fixture for Demo {
        fn name(&self) -> &str {
            "demo"
        }

        fn seed(&self, connection: &Connection) -> rusqlite::Result<()> {
            connection.execute("INSERT INTO fixture_rows DEFAULT VALUES", [])?;
            Ok(())
        }
    }

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(&format!(
                "{SETTINGS_TABLE_SQL} CREATE TABLE fixture_rows(id INTEGER PRIMARY KEY);"
            ))
            .unwrap();
        connection
    }

    #[test]
    fn the_fixture_door_seeds_once_and_only_when_asked() {
        let connection = connection();
        let fixtures: &[&dyn Fixture] = &[&Demo];
        assert_eq!(seed_named(&connection, None, fixtures).unwrap(), None);
        assert_eq!(count(&connection), 0);
        assert_eq!(
            seed_named(&connection, Some("demo".to_owned()), fixtures).unwrap(),
            Some("demo".to_owned())
        );
        assert_eq!(count(&connection), 1);
        seed_named(&connection, Some("demo".to_owned()), fixtures).unwrap();
        assert_eq!(count(&connection), 1, "the named fixture is seeded once");
    }

    fn count(connection: &Connection) -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM fixture_rows", [], |row| row.get(0))
            .unwrap()
    }
}
