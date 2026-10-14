# Changelog

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
