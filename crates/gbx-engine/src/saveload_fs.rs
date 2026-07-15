//! Host-side filesystem glue for the save/load slots (M3 step 6 deliverable
//! 3) — the impure half kept out of the tick core (D8) and off the wasm
//! target (`#[cfg(not(target_arch = "wasm32"))]` at the module site).
//!
//! A frontend (or the demo/tests) calls these to scan a save directory into a
//! [`SlotDirectory`] the engine can render, and to fulfill the
//! [`SaveLoadRequest`] the save/load screen emits. Slots map to files under a
//! caller-supplied directory (frontends decide where); our snapshots are
//! `.rsav`, originals are read-only `savgam{letter}.dat` sets (D-SAVE12).

use crate::engine::Engine;
use crate::saveload::{
    original_master_filename, rsav_filename, SaveLoadRequest, SlotDirectory, SlotStatus,
    SLOT_LETTERS,
};
use gbx_formats::game_data::GameData;
use std::path::Path;

/// Scans `save_dir` for each slot, preferring our own `.rsav` snapshot over an
/// original `savgam{letter}.dat` when both are present (a slot the player has
/// re-saved in our format supersedes the original they imported from). A
/// missing/unreadable directory yields an all-empty directory, not an error —
/// "no saves yet" is a normal state.
pub fn scan_slot_directory(save_dir: &Path) -> SlotDirectory {
    let mut dir = SlotDirectory::new();
    for &letter in &SLOT_LETTERS {
        let status = if save_dir.join(rsav_filename(letter)).is_file() {
            SlotStatus::RestrikeSave
        } else if save_dir.join(original_master_filename(letter)).is_file() {
            SlotStatus::OriginalSave
        } else {
            SlotStatus::Empty
        };
        dir.set(letter, status);
    }
    dir
}

/// Errors fulfilling a [`SaveLoadRequest`] — a filesystem problem, a rejected
/// `.rsav` (wrong version/flavor/data fingerprint, `save::SaveError`), or an
/// original-import failure (`import::ImportError`). Save-byte *parse* errors
/// for the original format surface earlier, at the `gbx_formats` load step.
#[derive(Debug)]
pub enum SlotIoError {
    Io(std::io::Error),
    Restore(crate::save::SaveError),
    Import(crate::import::ImportError),
    /// The original save set for a slot couldn't be parsed/assembled.
    OriginalParse(String),
}

impl From<std::io::Error> for SlotIoError {
    fn from(e: std::io::Error) -> Self {
        SlotIoError::Io(e)
    }
}

/// Writes the engine's current state to a slot's `.rsav` file
/// ([`Engine::save`] + a plain file write — the "slots map to `.rsav` via
/// `Engine::save`" mapping).
pub fn save_to_slot(engine: &Engine, save_dir: &Path, letter: char) -> Result<(), SlotIoError> {
    std::fs::create_dir_all(save_dir)?;
    let path = save_dir.join(rsav_filename(letter));
    std::fs::write(path, engine.save())?;
    Ok(())
}

/// Restores an engine from a slot's `.rsav` file ([`Engine::restore`]). The
/// caller supplies the matching `GameData` (D-SAVE2 verifies its fingerprint).
pub fn load_from_slot(
    save_dir: &Path,
    letter: char,
    data: GameData,
) -> Result<Engine, SlotIoError> {
    let path = save_dir.join(rsav_filename(letter));
    let bytes = std::fs::read(path)?;
    Engine::restore(&bytes, data).map_err(SlotIoError::Restore)
}

/// Imports a slot's original `savgam{letter}.dat` set into a fresh engine
/// (one-way, D-SAVE12). `seed` seeds the new engine's PRNG (the original
/// format carries none).
pub fn import_original_slot(
    save_dir: &Path,
    letter: char,
    data: GameData,
    seed: u32,
) -> Result<Engine, SlotIoError> {
    // Load the whole save directory once so every section file (master +
    // sibling CHRDAT/.swg/.fx records) is available as a borrowed slice for
    // `load_from_lookup`'s lifetime — the same pattern the import test uses.
    let saves = gbx_formats::game_data::load_dir(save_dir)
        .map_err(|e| SlotIoError::OriginalParse(format!("save dir unreadable: {e:?}")))?;
    let master_name = original_master_filename(letter);
    let master_bytes = saves
        .raw_file(&master_name)
        .ok_or_else(|| SlotIoError::OriginalParse(format!("missing {master_name}")))?;
    let set =
        gbx_formats::save_orig::load_from_lookup(master_bytes, letter, |name| saves.raw_file(name))
            .map_err(|e| SlotIoError::OriginalParse(format!("{e:?}")))?;
    crate::import::import_original(&set, data, seed).map_err(SlotIoError::Import)
}

/// Fulfills a [`SaveLoadRequest`] against `save_dir`, replacing `*engine` on a
/// successful Load/Import. `data`/`seed` are needed only by the load paths
/// (they rebuild an engine); Save ignores them.
pub fn fulfill(
    engine: &mut Engine,
    request: SaveLoadRequest,
    save_dir: &Path,
    data: GameData,
    seed: u32,
) -> Result<(), SlotIoError> {
    match request {
        SaveLoadRequest::Save(letter) => save_to_slot(engine, save_dir, letter),
        SaveLoadRequest::Load(letter) => {
            *engine = load_from_slot(save_dir, letter, data)?;
            Ok(())
        }
        SaveLoadRequest::ImportOriginal(letter) => {
            *engine = import_original_slot(save_dir, letter, data, seed)?;
            Ok(())
        }
    }
}
