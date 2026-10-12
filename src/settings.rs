use rusqlite::{Connection, OptionalExtension, params};

pub const SETTINGS_TABLE_SQL: &str =
    "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);";

pub fn get(connection: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
}

pub fn set(connection: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn clear(connection: &Connection, key: &str) -> rusqlite::Result<()> {
    connection.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_set_and_clear_share_one_table_contract() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SETTINGS_TABLE_SQL).unwrap();
        assert_eq!(get(&connection, "desk").unwrap(), None);
        set(&connection, "desk", "paper").unwrap();
        assert_eq!(get(&connection, "desk").unwrap().as_deref(), Some("paper"));
        clear(&connection, "desk").unwrap();
        assert_eq!(get(&connection, "desk").unwrap(), None);
    }
}
