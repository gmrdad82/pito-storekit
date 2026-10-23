# Changelog

## v1.0.2 — 2026-10-23

### Changed

- Each migration step now commits on its own with its backfill and its version: a step that fails, or a process killed mid-step, leaves the store at the last good step, and the next open carries on from there.
- Two processes opening the same store at once run each pending migration step once.
- A blank `<PREFIX>_DB` or `<PREFIX>_DATA` counts as unset, and a path that isn't UTF-8 is honoured.
- The README and docs quote a new run of the `measure` example.

### Fixed

- An older build opening a newer store no longer lowers its schema version.
- A backup into a folder whose name isn't UTF-8 returns an error instead of crashing the app.
- A stray file such as `notes-before-upgrade.db`, or a copy stamped in the future, no longer makes a backup run every minute.
- A backup killed halfway no longer blocks the next one, and its leftover files are cleaned up after 14 days.
- A fixture that fails halfway leaves nothing behind.
- `tilde` no longer shortens a path that only starts like the home folder.

## v1.0.1 — 2026-10-14

### Added

- Documentation pages: install and first use, a guide for each task (opening a store, migrations, long jobs, backups, folders, settings and fixtures, measuring), and a reference for the files and names storekit writes.
- Each release carries `docs-vX.Y.Z.tar.gz`: those pages with the API reference, this changelog and their images.

## v1.0.0 — 2026-10-14

### Added

- A `measure` example times opening a store, a long job written in one transaction against `write_sliced` while the app keeps writing, and a daily backup, on your own machine.

### Changed

- pito-storekit is 1.0.0. The API is the same as 0.1.2, so moving to the `v1.0.0` tag needs no code change.
- The README shows each feature with measured numbers and charts, and a first-use block for Linux, WSL and macOS.
- The brand now reads PITO MD in the README, the trademarks note and the package description.

## v0.1.2 — 2026-10-12

### Added

- The repository is MIT licensed, with a README for strangers, a code of conduct, a security policy and CI on Linux and macOS.

### Changed

- The `desk` example prints its paths under `~`.
- Cargo.toml names the licence, the repository and a plain description.

## v0.1.1 — 2026-10-07

### Added

- `storekit::archive::gzip_file` gzips one file and fsyncs it; the backups use it.

### Changed

- The tests keep their scratch files in the tree's `tmp/`.

## v0.1.0 — 2026-09-28

### Added

- The `storekit` crate: `open` and `open_companion`, ordered migrations, sliced background writes, daily gzip backups kept 14 days, app folders, the settings table, fixtures and the fixture-boot guard.
