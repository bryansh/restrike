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

/// ★ Scans `save_dir` for the `.guy` character files `Add Character to Party`
/// lists (roll-credits slice 9c, `BuildLoadablePlayersLists`'s `"*.guy"` pass,
/// `ovr017.cs:70`).
///
/// A file whose length is not [`gbx_formats::save_orig::CHAR_RECORD_SIZE`] is
/// skipped, which is the original's own filter (`stream.Length ==
/// playerFileSize`, `:22`). The listed name comes from inside the record, not
/// the filename.
pub fn scan_char_files(save_dir: &Path) -> crate::chr_file::CharFileDirectory {
    use crate::chr_file::{CharFileDirectory, CharFileEntry};
    let mut dir = CharFileDirectory::new();
    let Ok(entries) = std::fs::read_dir(save_dir) else {
        return dir;
    };
    let mut found: Vec<CharFileEntry> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_guy = path.extension().is_some_and(|e| {
            e.to_string_lossy()
                .eq_ignore_ascii_case(crate::chr_file::CHAR_EXT)
        });
        if !is_guy {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.len() != gbx_formats::save_orig::CHAR_RECORD_SIZE {
            continue;
        }
        let Ok(record) = gbx_formats::save_orig::decode_char_record(&bytes) else {
            continue;
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        found.push(CharFileEntry {
            stem,
            name: record.name,
            taken: false,
        });
    }
    // Deterministic order: `Directory.GetFiles` is filesystem order in the
    // original, which is not something to reproduce — a stable sort is.
    found.sort_by(|a, b| a.stem.cmp(&b.stem));
    dir.entries = found;
    dir
}

/// ★ Fulfills a [`crate::chr_file::CharFileRequest`] (roll-credits slice 9c).
///
/// `Save` writes the `SavePlayer` trio (`ovr017.cs:174-208`): the 0x1A6-byte
/// record as `<stem>.guy`, the items as `<stem>.swg` and the affects as
/// `<stem>.fx` — and, exactly as the original does, **deletes** the two
/// sidecars first so a character who has lost every item does not keep a stale
/// `.swg` (`:183,195`).
///
/// `Load` reads the trio back (`import_char01`, `:486-566`) and offers the
/// character to the party through [`Engine::add_character`], which applies the
/// join gate. `Ok(None)` means it joined; `Ok(Some(reason))` is the original's
/// own refusal text.
pub fn fulfill_char_file(
    engine: &mut Engine,
    request: crate::chr_file::CharFileRequest,
    save_dir: &Path,
) -> Result<Option<String>, SlotIoError> {
    use crate::chr_file::{CharFileRequest, AFFECTS_EXT, CHAR_EXT, ITEMS_EXT};
    match request {
        CharFileRequest::Save(ch) => {
            std::fs::create_dir_all(save_dir)?;
            let stem = crate::chr_file::clean_stem(&ch.name);
            let record = crate::party::record_from_character(&ch);
            std::fs::write(
                save_dir.join(format!("{stem}.{CHAR_EXT}")),
                gbx_formats::save_orig::encode_char_record(&record),
            )?;
            let items_path = save_dir.join(format!("{stem}.{ITEMS_EXT}"));
            let _ = std::fs::remove_file(&items_path);
            if !ch.items.is_empty() {
                std::fs::write(items_path, ch.items.concat())?;
            }
            let fx_path = save_dir.join(format!("{stem}.{AFFECTS_EXT}"));
            let _ = std::fs::remove_file(&fx_path);
            if !ch.affects.is_empty() {
                std::fs::write(fx_path, ch.affects.concat())?;
            }
            Ok(None)
        }
        CharFileRequest::Load(stem) => {
            let bytes = std::fs::read(save_dir.join(format!("{stem}.{CHAR_EXT}")))?;
            let record = gbx_formats::save_orig::decode_char_record(&bytes)
                .map_err(|e| SlotIoError::OriginalParse(format!("{stem}.{CHAR_EXT}: {e:?}")))?;
            let items = std::fs::read(save_dir.join(format!("{stem}.{ITEMS_EXT}")))
                .ok()
                .and_then(|b| gbx_formats::save_orig::read_items(&b).ok())
                .unwrap_or_default();
            let affects = std::fs::read(save_dir.join(format!("{stem}.{AFFECTS_EXT}")))
                .ok()
                .and_then(|b| gbx_formats::save_orig::read_affects(&b).ok())
                .unwrap_or_default();
            let ch = crate::party::character_from_record(&record, items, affects);
            Ok(engine.add_character(ch).err().map(|e| e.to_string()))
        }
    }
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
///
/// ★ Roll-credits slice 9b: a load the **start menu** asked for returns to the
/// start menu, not into the world — `ovr018.cs:223-228` calls `loadGameMenu()`
/// from inside `startGameMenu`'s own `while (true)`, so the player lands back
/// on the menu with the loaded party in front of them and presses `B` to
/// begin. A replacement engine has no memory of where the request came from,
/// so the bit is carried across here (hosts that do their own I/O can use
/// [`Engine::at_front_door`]/[`Engine::park_at_start_menu`] directly).
pub fn fulfill(
    engine: &mut Engine,
    request: SaveLoadRequest,
    save_dir: &Path,
    data: GameData,
    seed: u32,
) -> Result<(), SlotIoError> {
    let from_front_door = engine.at_front_door();
    match request {
        SaveLoadRequest::Save(letter) => save_to_slot(engine, save_dir, letter),
        SaveLoadRequest::Load(letter) => {
            *engine = load_from_slot(save_dir, letter, data)?;
            if from_front_door {
                engine.park_at_start_menu();
            }
            Ok(())
        }
        SaveLoadRequest::ImportOriginal(letter) => {
            *engine = import_original_slot(save_dir, letter, data, seed)?;
            if from_front_door {
                engine.park_at_start_menu();
            }
            Ok(())
        }
    }
}
