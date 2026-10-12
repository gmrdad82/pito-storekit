use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

pub fn gzip_file(from: &Path, into: &Path) -> std::io::Result<()> {
    let mut source = std::fs::File::open(from)?;
    let target = std::fs::File::create(into)?;
    let mut encoder = flate2::write::GzEncoder::new(target, flate2::Compression::default());
    std::io::copy(&mut source, &mut encoder)?;
    encoder.finish()?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(prefix: &str) -> tempfile::TempDir {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/testscratch");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(&root)
            .unwrap()
    }

    #[test]
    fn gzip_file_round_trips_the_exact_bytes() {
        let dir = scratch("archive-gzip-");
        let from = dir.path().join("plain");
        let into = dir.path().join("compressed.gz");
        std::fs::write(&from, b"the quick brown fox jumps over the lazy dog").unwrap();

        gzip_file(&from, &into).unwrap();

        let compressed = std::fs::read(&into).unwrap();
        assert!(!compressed.is_empty());
        let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
        let mut restored = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut restored).unwrap();
        assert_eq!(restored, b"the quick brown fox jumps over the lazy dog");
    }
}
