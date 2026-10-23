use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct App {
    pub folder: &'static str,
    pub env_prefix: &'static str,
}

impl App {
    pub fn folder(self) -> String {
        if cfg!(debug_assertions) {
            format!("{}-dev", self.folder)
        } else {
            self.folder.to_owned()
        }
    }

    pub fn config_dir(self) -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| home_dir().join(".config"))
            .join(self.folder())
    }

    pub fn data_dir(self) -> PathBuf {
        self.env_path("DATA").unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| home_dir().join(".local").join("share"))
                .join(self.folder())
        })
    }

    pub fn db_path(self, file: impl AsRef<Path>) -> PathBuf {
        self.env_path("DB")
            .unwrap_or_else(|| self.data_dir().join(file))
    }

    pub fn token_path(self) -> PathBuf {
        self.config_dir().join("token")
    }

    pub fn mcp_token_path(self) -> PathBuf {
        self.data_dir().join("mcp-token")
    }

    pub fn crash_spool_dir(self) -> PathBuf {
        self.data_dir().join("crash")
    }

    pub fn traces_dir(self) -> PathBuf {
        self.data_dir().join("traces")
    }

    fn env_path(self, suffix: &str) -> Option<PathBuf> {
        std::env::var_os(self.env_key(suffix))
            .filter(|value| !value.to_string_lossy().trim().is_empty())
            .map(PathBuf::from)
    }

    pub(crate) fn env_key(self, suffix: &str) -> String {
        format!("{}_{suffix}", self.env_prefix)
    }
}

pub fn tilde(path: &str) -> String {
    tilde_under(path, &home_dir().to_string_lossy())
}

fn tilde_under(path: &str, home: &str) -> String {
    let home = match home.strip_suffix('/') {
        Some(trimmed) if !trimmed.is_empty() => trimmed,
        _ => home,
    };
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => path.to_owned(),
    }
}

#[cfg(unix)]
pub fn set_owner_only_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        let _ = std::fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
pub fn set_owner_only_permissions(_path: &Path) {}

fn home_dir() -> PathBuf {
    dirs::home_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .expect("no home directory on this machine")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_APP: App = App {
        folder: "storekit-path-test",
        env_prefix: "PITO_STOREKIT_PATH_TEST",
    };

    #[test]
    fn the_app_dirs_end_in_the_apps_own_name() {
        assert!(TEST_APP.config_dir().ends_with(TEST_APP.folder()));
        assert!(
            TEST_APP
                .token_path()
                .ends_with(format!("{}/token", TEST_APP.folder()))
        );
    }

    #[test]
    fn a_debug_build_never_names_the_release_folder() {
        assert_eq!(TEST_APP.folder(), "storekit-path-test-dev");
        assert!(!TEST_APP.config_dir().ends_with(TEST_APP.folder));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_the_paths_stay_exactly_where_existing_stores_live() {
        assert_eq!(
            TEST_APP.config_dir(),
            home_dir().join(".config").join(TEST_APP.folder())
        );
        if std::env::var(TEST_APP.env_key("DATA")).is_err() {
            assert_eq!(
                TEST_APP.data_dir(),
                home_dir()
                    .join(".local")
                    .join("share")
                    .join(TEST_APP.folder())
            );
        }
    }

    #[test]
    fn tilde_shortens_the_home_folder_and_what_sits_inside_it() {
        let home = home_dir().to_string_lossy().into_owned();
        assert_eq!(tilde(&home), "~");
        assert_eq!(tilde(&format!("{home}/notes/desk.db")), "~/notes/desk.db");
    }

    #[test]
    fn tilde_leaves_a_path_that_only_shares_the_home_prefix() {
        let home = home_dir().to_string_lossy().into_owned();
        let sibling = format!("{home}y/desk.db");
        assert_eq!(tilde(&sibling), sibling);
        assert_eq!(tilde(&format!("{home}y")), format!("{home}y"));
    }

    #[test]
    fn tilde_reads_a_home_with_a_trailing_slash_as_the_same_folder() {
        assert_eq!(
            tilde_under("/srv/user/notes/desk.db", "/srv/user/"),
            "~/notes/desk.db"
        );
        assert_eq!(tilde_under("/srv/user", "/srv/user/"), "~");
        assert_eq!(tilde_under("/srv/user/", "/srv/user/"), "~/");
        assert_eq!(
            tilde_under("/srv/usery/desk.db", "/srv/user/"),
            "/srv/usery/desk.db"
        );
        assert_eq!(tilde_under("/srv/user/notes", "/srv/user"), "~/notes");
    }

    #[test]
    fn tilde_never_shortens_under_a_root_or_empty_home() {
        assert_eq!(tilde_under("/etc/desk.db", "/"), "/etc/desk.db");
        assert_eq!(tilde_under("/etc/desk.db", ""), "/etc/desk.db");
    }

    #[test]
    fn tilde_leaves_an_unrelated_path_alone() {
        assert_eq!(
            tilde("/storekit-nowhere/desk.db"),
            "/storekit-nowhere/desk.db"
        );
        assert_eq!(tilde("relative/desk.db"), "relative/desk.db");
    }
}
