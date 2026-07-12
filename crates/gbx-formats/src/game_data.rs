//! `GameData`: the in-memory archive set `docs/design/renderer-ui-shell.md`
//! D-UI1 names as the tick core's only data-access surface — "the core does
//! zero I/O ... the frontend reads `GBX_DATA_DIR` ... and hands the bytes
//! over". [`GameData`] itself never touches `std::fs`: it is constructed
//! from `(file name, bytes)` pairs already in memory and exposes keyed
//! `(file, block)` access over the existing [`crate::dax::DaxArchive`], plus
//! the detection fingerprint ([`crate::detect`]). The one filesystem-facing
//! helper, [`load_dir`], is a thin loader frontends/CLI call — it is the
//! only function in this module that touches disk, matching the precedent
//! `crate::detect::detect_dir` already set for this crate's `wasm32` build
//! (fs code compiles cleanly there; it simply has nothing to walk).

use std::collections::BTreeMap;

use crate::dax::{DaxArchive, DaxError};
use crate::detect::{self, Detection};

/// An in-memory set of raw game data files, keyed case-insensitively by
/// file name (matching DOS-era file systems and this codebase's existing
/// `GBX_DATA_DIR` convention). Holds owned bytes — no borrows, so it is
/// trivially constructible from files read off disk, fetched over the
/// network (the web frontend), or hand-built in a test.
#[derive(Debug, Clone, Default)]
pub struct GameData {
    files: BTreeMap<String, Vec<u8>>,
}

/// [`GameData::archive`]/[`GameData::block`]'s failure mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameDataError {
    /// No file with this name was handed to [`GameData::from_files`].
    UnknownFile { file_name: String },
    /// The named file's bytes failed to parse as (or extract from) a DAX
    /// container.
    Dax(DaxError),
}

impl GameData {
    /// Builds a [`GameData`] from `(file name, bytes)` pairs. File names are
    /// stored uppercased; lookups are case-insensitive.
    pub fn from_files(files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Self {
        GameData {
            files: files
                .into_iter()
                .map(|(name, bytes)| (name.to_ascii_uppercase(), bytes))
                .collect(),
        }
    }

    /// The file names this instance holds, uppercased, in sorted order.
    pub fn file_names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// The raw bytes of `file_name`, if present (case-insensitive).
    pub fn raw_file(&self, file_name: &str) -> Option<&[u8]> {
        self.files
            .get(&file_name.to_ascii_uppercase())
            .map(Vec::as_slice)
    }

    /// Parses `file_name`'s bytes as a DAX container's index. Does not
    /// decompress any block payload (`DaxArchive::parse` is index-only) —
    /// see [`GameData::block`] for a specific block's decompressed bytes.
    pub fn archive(&self, file_name: &str) -> Result<DaxArchive<'_>, GameDataError> {
        let bytes = self
            .raw_file(file_name)
            .ok_or_else(|| GameDataError::UnknownFile {
                file_name: file_name.to_string(),
            })?;
        DaxArchive::parse(bytes).map_err(GameDataError::Dax)
    }

    /// Looks up `file_name`, then decompresses and returns block `block_id`
    /// — the (file, block) keyed access D-UI1 asks for, in one call.
    pub fn block(&self, file_name: &str, block_id: u8) -> Result<Vec<u8>, GameDataError> {
        self.archive(file_name)?
            .block_data(block_id)
            .map_err(GameDataError::Dax)
    }

    /// Fingerprints this instance's files against
    /// [`crate::detect::DETECTION_TABLE`], purely in memory (no filesystem
    /// access — see [`crate::detect::detect_bytes`]).
    pub fn detect(&self) -> Detection {
        detect::detect_bytes(
            self.files
                .iter()
                .map(|(name, bytes)| (name.as_str(), bytes.as_slice())),
        )
    }
}

/// Reads every regular file directly inside `dir` (non-recursive — CotAB's
/// data set is a flat directory) into a [`GameData`]. The only
/// filesystem-touching function in this module; `GameData` itself stays
/// zero-I/O per D-UI1. Frontends/the CLI call this; `gbx-engine`'s core
/// never does.
pub fn load_dir(dir: &std::path::Path) -> std::io::Result<GameData> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let bytes = std::fs::read(&path)?;
        files.push((name.to_string(), bytes));
    }
    Ok(GameData::from_files(files))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dax::DaxError;

    /// Hand-authored (D10): a minimal one-block DAX container's bytes
    /// (mirrors `dax.rs`'s own test helper — kept local since this module
    /// tests `GameData`'s wiring, not the DAX format itself).
    fn build_dax(id: u8, raw: &[u8]) -> Vec<u8> {
        let comp: Vec<u8> = {
            let mut out = Vec::new();
            for chunk in raw.chunks(128) {
                out.push((chunk.len() - 1) as u8);
                out.extend_from_slice(chunk);
            }
            out
        };
        let mut out = Vec::new();
        out.extend_from_slice(&9u16.to_le_bytes());
        out.push(id);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        out.extend_from_slice(&(comp.len() as u16).to_le_bytes());
        out.extend_from_slice(&comp);
        out
    }

    #[test]
    fn looks_up_files_case_insensitively() {
        let data = GameData::from_files([("geo2.dax".to_string(), build_dax(1, b"hello"))]);
        assert!(data.raw_file("GEO2.DAX").is_some());
        assert!(data.raw_file("Geo2.Dax").is_some());
        assert!(data.raw_file("geo2.dax").is_some());
    }

    #[test]
    fn unknown_file_errors_cleanly() {
        let data = GameData::from_files([]);
        let err = data.block("NOPE.DAX", 1).unwrap_err();
        assert_eq!(
            err,
            GameDataError::UnknownFile {
                file_name: "NOPE.DAX".to_string()
            }
        );
    }

    #[test]
    fn block_extracts_decompressed_bytes() {
        let data = GameData::from_files([("GEO2.DAX".to_string(), build_dax(3, b"TILVERTON"))]);
        assert_eq!(data.block("GEO2.DAX", 3).unwrap(), b"TILVERTON");
    }

    #[test]
    fn unknown_block_id_surfaces_the_dax_error() {
        let data = GameData::from_files([("GEO2.DAX".to_string(), build_dax(3, b"x"))]);
        let err = data.block("GEO2.DAX", 99).unwrap_err();
        assert_eq!(err, GameDataError::Dax(DaxError::UnknownBlockId { id: 99 }));
    }

    #[test]
    fn file_names_lists_every_loaded_file_uppercased() {
        let data = GameData::from_files([
            ("ecl1.dax".to_string(), vec![0, 0]),
            ("GEO2.DAX".to_string(), vec![0, 0]),
        ]);
        let names: Vec<&str> = data.file_names().collect();
        assert_eq!(names, vec!["ECL1.DAX", "GEO2.DAX"]);
    }

    #[test]
    fn detect_reuses_the_detection_table_purely_in_memory() {
        let data =
            GameData::from_files([("SOMEFILE.DAT".to_string(), b"not a real game file".to_vec())]);
        match data.detect() {
            Detection::Unknown { files } => assert_eq!(files.len(), 1),
            Detection::Known { .. } => panic!("synthetic data must never match a real fingerprint"),
        }
    }

    /// Local-only tier: `load_dir` against the real `GBX_DATA_DIR`, then
    /// `detect()` must report the known game — proving the fs loader and
    /// the in-memory fingerprint compose correctly on real data.
    #[test]
    fn load_dir_and_detect_recognize_real_game_data() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        let data = load_dir(dir).expect("GBX_DATA_DIR must be readable");
        assert!(data.file_names().count() > 0);
        match data.detect() {
            Detection::Known { .. } => {}
            Detection::Unknown { .. } => {
                panic!("GBX_DATA_DIR is set but GameData::detect found no known game")
            }
        }
    }
}
