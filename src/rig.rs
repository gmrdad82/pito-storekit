use crate::fixtures;
use crate::paths::App;
use std::path::Path;

const REFUSAL: &str = "rig: refusing to boot: a fixture is named but /tmp is not the tree's tmp/";

pub fn guard(app: &App, data_dir: &Path) {
    if let Err(error) = guard_at(
        Path::new("/tmp/.rig"),
        fixtures::requested(app).is_some(),
        data_dir,
    ) {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

pub fn guard_at(stamp: &Path, requested: bool, data_dir: &Path) -> Result<(), String> {
    if !requested {
        return Ok(());
    }
    let rig = std::fs::read_to_string(stamp).map_err(|_| REFUSAL.to_owned())?;
    let rig = rig.trim();
    if rig.is_empty() || !data_dir.starts_with(rig) {
        return Err(REFUSAL.to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> tempfile::TempDir {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix(&format!("rig-{tag}-"))
            .tempdir_in(root)
            .unwrap()
    }

    #[test]
    fn a_missing_stamp_refuses_a_fixture_boot() {
        let holder = scratch("missing");
        assert_eq!(
            guard_at(&holder.path().join("missing"), true, holder.path()),
            Err(REFUSAL.to_owned())
        );
    }

    #[test]
    fn a_data_dir_outside_the_stamped_rig_is_refused() {
        let holder = scratch("outside");
        let rig = holder.path().join("rig");
        std::fs::create_dir_all(&rig).unwrap();
        let stamp = holder.path().join("stamp");
        std::fs::write(&stamp, rig.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(
            guard_at(&stamp, true, &holder.path().join("elsewhere")),
            Err(REFUSAL.to_owned())
        );
    }

    #[test]
    fn a_data_dir_under_the_stamped_rig_passes() {
        let holder = scratch("inside");
        let rig = holder.path().join("rig");
        let data = rig.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let stamp = holder.path().join("stamp");
        std::fs::write(&stamp, format!("{}\n", rig.display())).unwrap();
        assert_eq!(guard_at(&stamp, true, &data), Ok(()));
    }

    #[test]
    fn no_fixture_named_needs_no_stamp() {
        let holder = scratch("no-fixture");
        assert_eq!(
            guard_at(&holder.path().join("missing"), false, holder.path()),
            Ok(())
        );
    }
}
