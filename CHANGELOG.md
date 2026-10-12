# Changelog

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
