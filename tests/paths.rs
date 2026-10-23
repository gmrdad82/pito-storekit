use std::path::{Path, PathBuf};
use storekit::paths::{self, App};

fn scratch(tag: &str) -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("paths-it-{tag}-"))
        .tempdir_in(root)
        .unwrap()
}

fn set_env(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    unsafe { std::env::set_var(key, value) };
}

#[test]
fn the_data_override_moves_the_store_and_every_file_beside_it() {
    const APP: App = App {
        folder: "storekit-t-data",
        env_prefix: "PITO_STOREKIT_T_DATA",
    };
    let holder = scratch("data");
    set_env("PITO_STOREKIT_T_DATA_DATA", holder.path());

    assert_eq!(APP.data_dir(), holder.path());
    assert_eq!(APP.db_path("notes.db"), holder.path().join("notes.db"));
    assert_eq!(APP.mcp_token_path(), holder.path().join("mcp-token"));
    assert_eq!(APP.crash_spool_dir(), holder.path().join("crash"));
    assert_eq!(APP.traces_dir(), holder.path().join("traces"));
    assert_eq!(APP.token_path(), APP.config_dir().join("token"));
    assert!(!APP.token_path().starts_with(holder.path()));
}

#[test]
fn the_db_override_names_the_store_alone() {
    const APP: App = App {
        folder: "storekit-t-db",
        env_prefix: "PITO_STOREKIT_T_DB",
    };
    let holder = scratch("db");
    let store = holder.path().join("elsewhere").join("custom.db");
    set_env("PITO_STOREKIT_T_DB_DB", &store);

    assert_eq!(APP.db_path("notes.db"), store);
    assert_eq!(APP.db_path("other.db"), store);
    assert!(APP.data_dir().ends_with(APP.folder()));
}

#[test]
fn a_blank_db_override_counts_as_unset() {
    const APP: App = App {
        folder: "storekit-t-blank-db",
        env_prefix: "PITO_STOREKIT_T_BLANK_DB",
    };
    for blank in ["", "  "] {
        set_env("PITO_STOREKIT_T_BLANK_DB_DB", blank);

        let store = APP.db_path("notes.db");

        assert!(store.is_absolute(), "{blank:?} gave {store:?}");
        assert_eq!(store, APP.data_dir().join("notes.db"));
        assert!(store.ends_with(Path::new(&APP.folder()).join("notes.db")));
    }
}

#[test]
fn a_blank_data_override_counts_as_unset() {
    const APP: App = App {
        folder: "storekit-t-blank-data",
        env_prefix: "PITO_STOREKIT_T_BLANK_DATA",
    };
    for blank in ["", " \t "] {
        set_env("PITO_STOREKIT_T_BLANK_DATA_DATA", blank);

        let data = APP.data_dir();

        assert!(data.is_absolute(), "{blank:?} gave {data:?}");
        assert!(data.ends_with(APP.folder()));
        assert_eq!(APP.db_path("notes.db"), data.join("notes.db"));
        assert_eq!(APP.traces_dir(), data.join("traces"));
    }
}

#[cfg(unix)]
#[test]
fn a_db_override_that_is_not_utf8_is_honoured() {
    use std::os::unix::ffi::OsStrExt;

    const APP: App = App {
        folder: "storekit-t-raw-db",
        env_prefix: "PITO_STOREKIT_T_RAW_DB",
    };
    let holder = scratch("raw");
    let mut raw = holder.path().as_os_str().as_bytes().to_vec();
    raw.extend_from_slice(b"/notes-\xff.db");
    let store = PathBuf::from(std::ffi::OsStr::from_bytes(&raw));
    set_env("PITO_STOREKIT_T_RAW_DB_DB", &store);

    assert_eq!(APP.db_path("notes.db"), store);
}

#[cfg(target_os = "linux")]
#[test]
fn on_linux_the_folders_sit_where_the_readme_says() {
    const APP: App = App {
        folder: "storekit-t-xdg",
        env_prefix: "PITO_STOREKIT_T_XDG",
    };
    let home = dirs::home_dir().unwrap();
    let base = |variable: &str, fallback: &str| {
        std::env::var_os(variable)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(fallback))
    };
    let data = base("XDG_DATA_HOME", ".local/share");
    let config = base("XDG_CONFIG_HOME", ".config");
    let folder = if cfg!(debug_assertions) {
        "storekit-t-xdg-dev"
    } else {
        "storekit-t-xdg"
    };

    assert_eq!(APP.db_path("notes.db"), data.join(folder).join("notes.db"));
    assert_eq!(APP.token_path(), config.join(folder).join("token"));
    assert_eq!(APP.mcp_token_path(), data.join(folder).join("mcp-token"));
    assert_eq!(APP.crash_spool_dir(), data.join(folder).join("crash"));
    assert_eq!(APP.traces_dir(), data.join(folder).join("traces"));
}

#[cfg(unix)]
#[test]
fn owner_only_permissions_leave_0600_and_keep_the_bytes() {
    use std::os::unix::fs::PermissionsExt;

    let holder = scratch("perm");
    let token = holder.path().join("token");
    std::fs::write(&token, b"secret").unwrap();
    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();

    paths::set_owner_only_permissions(&token);

    let mode = std::fs::metadata(&token).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(std::fs::read(&token).unwrap(), b"secret");
}

#[test]
fn owner_only_permissions_on_a_missing_file_create_nothing() {
    let holder = scratch("perm-missing");
    let missing = holder.path().join("not-there");

    paths::set_owner_only_permissions(&missing);

    assert!(!missing.exists());
}

const RIG_CHILD: &str = "PITO_STOREKIT_T_RIG_CHILD";

#[test]
fn rig_guard_child() {
    let Some(data_dir) = std::env::var_os(RIG_CHILD) else {
        return;
    };
    const APP: App = App {
        folder: "storekit-t-rig",
        env_prefix: "PITO_STOREKIT_T_RIG",
    };
    storekit::rig::guard(&APP, Path::new(&data_dir));
    println!("rig: booted");
}

fn run_rig_child(fixture: Option<&str>) -> std::process::Output {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "rig_guard_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(RIG_CHILD, "")
        .env_remove("PITO_STOREKIT_T_RIG_FIXTURES");
    if let Some(fixture) = fixture {
        command.env("PITO_STOREKIT_T_RIG_FIXTURES", fixture);
    }
    command.output().unwrap()
}

#[test]
fn the_rig_guard_exits_2_when_a_fixture_is_named_outside_any_rig() {
    let output = run_rig_child(Some("demo"));

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("rig: refusing to boot"), "{stderr}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("rig: booted"));
}

#[test]
fn the_rig_guard_lets_a_boot_with_no_fixture_through() {
    let output = run_rig_child(None);

    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("rig: booted"));
}
