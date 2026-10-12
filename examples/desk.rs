use rusqlite::Connection;
use std::path::Path;
use storekit::fixtures::Fixture;
use storekit::migrate::{Migrations, Step};
use storekit::paths::App;

const APP: App = App {
    folder: "storekit-example",
    env_prefix: "PITO_EXAMPLE",
};

const STEPS: &[Step] = &[
    Step::new(
        1,
        "CREATE TABLE IF NOT EXISTS settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
        None,
    ),
    Step::new(
        2,
        "ALTER TABLE notes ADD COLUMN summary TEXT NOT NULL DEFAULT '';",
        None,
    ),
];

const MIGRATIONS: Migrations = Migrations {
    version: 2,
    steps: STEPS,
    before: None,
};

struct Demo;

impl Fixture for Demo {
    fn name(&self) -> &str {
        "demo"
    }

    fn seed(&self, connection: &Connection) -> rusqlite::Result<()> {
        for body in ["first", "second", "third"] {
            connection.execute("INSERT INTO notes(body) VALUES (?1)", [body])?;
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = APP.data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let db_path = APP.db_path("desk.db");
    let connection = storekit::open(&db_path, &MIGRATIONS, None)?;
    storekit::settings::set(&connection, "example_setting", "written")?;
    let fixture = storekit::fixtures::seed_if_requested(&connection, &APP, &[&Demo])?;
    let backup_dir = data_dir.join("backups");
    let kept = if storekit::backup::due(&backup_dir, APP.folder, now_unix()) {
        Some(storekit::backup::keep(
            &connection,
            &backup_dir,
            APP.folder,
        )?)
    } else {
        None
    };

    let version: i64 = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |row| row.get::<_, String>(0),
        )?
        .parse()?;
    let rows: i64 = connection.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
    let backup_count = std::fs::read_dir(&backup_dir)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "gz")
        })
        .count();

    println!("folder={}", APP.folder());
    println!("config={}", shown(&APP.config_dir()));
    println!("data={}", shown(&data_dir));
    println!("db={}", shown(&db_path));
    println!("wal={}", sidecar_exists(&db_path, "-wal"));
    println!("shm={}", sidecar_exists(&db_path, "-shm"));
    println!("schema_version={version}");
    println!(
        "setting={}",
        storekit::settings::get(&connection, "example_setting")?.unwrap()
    );
    println!("fixture={}", fixture.as_deref().unwrap_or("none"));
    println!("rows={rows}");
    println!(
        "backup={}",
        kept.as_ref()
            .map(|backup| shown(&backup.path))
            .unwrap_or_else(|| "already-kept-today".to_owned())
    );
    println!("backup_count={backup_count}");
    println!(
        "latest={}",
        storekit::backup::latest(&backup_dir, APP.folder)
            .map(|unix| unix.to_string())
            .unwrap_or_else(|| "none".to_owned())
    );
    Ok(())
}

fn shown(path: &Path) -> String {
    storekit::paths::tilde(&path.display().to_string())
}

fn sidecar_exists(database: &Path, suffix: &str) -> bool {
    Path::new(&format!("{}{suffix}", database.display())).exists()
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
