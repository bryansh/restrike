//! Game detection: fingerprint a data directory against known-game signatures.
//!
//! The detection table is empty until real game file hashes are recorded
//! (per D10, no game data or derived signatures ship in this repo yet). An
//! empty table is expected to produce [`Detection::Unknown`] for any input.

use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A single known file's identity within a game's data set.
pub struct GameSignature {
    pub game: &'static str,
    pub file_name: &'static str,
    pub sha256: &'static str,
}

/// Fingerprint table of known Gold Box game files, keyed by name + SHA-256.
///
/// Empty for now: no game data has landed yet (PLAN.md D14). Populated as
/// real fingerprints are recorded from legally-obtained data.
pub const DETECTION_TABLE: &[GameSignature] = &[];

/// SHA-256 digest of a single file, as a lowercase hex string.
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hex_encode(&hasher.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// One scanned file and its digest.
#[derive(Debug, Clone)]
pub struct FileReport {
    pub path: PathBuf,
    pub sha256: String,
}

impl fmt::Display for FileReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}  {}", self.sha256, self.path.display())
    }
}

/// Outcome of scanning a directory against [`DETECTION_TABLE`].
#[derive(Debug)]
pub enum Detection {
    /// No file in the scanned directory matched a known signature.
    Unknown { files: Vec<FileReport> },
    /// At least one file matched a known game's signature.
    Known {
        game: &'static str,
        files: Vec<FileReport>,
    },
}

/// Recursively walk `dir`, hashing every regular file, and match the results
/// against [`DETECTION_TABLE`].
pub fn detect_dir(dir: &Path) -> io::Result<Detection> {
    let mut files = Vec::new();
    walk(dir, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));

    for report in &files {
        if let Some(sig) = DETECTION_TABLE
            .iter()
            .find(|sig| sig.sha256 == report.sha256)
        {
            return Ok(Detection::Known {
                game: sig.game,
                files,
            });
        }
    }

    Ok(Detection::Unknown { files })
}

fn walk(dir: &Path, out: &mut Vec<FileReport>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            walk(&path, out)?;
        } else if file_type.is_file() {
            let sha256 = hash_file(&path)?;
            out.push(FileReport { path, sha256 });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn empty_table_yields_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = File::create(tmp.path().join("SOMEFILE.DAT")).unwrap();
        f.write_all(b"synthetic fixture data, not a real game file")
            .unwrap();

        let result = detect_dir(tmp.path()).unwrap();
        match result {
            Detection::Unknown { files } => assert_eq!(files.len(), 1),
            Detection::Known { .. } => panic!("expected Unknown with an empty detection table"),
        }
    }

    #[test]
    fn walks_nested_directories() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        File::create(tmp.path().join("a.txt"))
            .unwrap()
            .write_all(b"a")
            .unwrap();
        File::create(tmp.path().join("sub/b.txt"))
            .unwrap()
            .write_all(b"b")
            .unwrap();

        let result = detect_dir(tmp.path()).unwrap();
        let files = match result {
            Detection::Unknown { files } => files,
            Detection::Known { files, .. } => files,
        };
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn hash_is_stable_sha256() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("hello.txt");
        File::create(&path).unwrap().write_all(b"hello").unwrap();

        let digest = hash_file(&path).unwrap();
        assert_eq!(
            digest,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
