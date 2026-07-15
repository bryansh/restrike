//! Save/load slot model (`ovr017.cs`, M3 step 6 deliverable 3) — the pure,
//! platform-independent half: slot identity, per-slot status, the filename
//! convention, and the [`SaveLoadRequest`] the save/load *screen* emits.
//!
//! **D8 (no I/O in the tick core).** The save/load screen never touches the
//! filesystem: it renders from a host-injected [`SlotDirectory`] and, on a
//! selection, sets a [`SaveLoadRequest`] the host takes after the tick
//! ([`Engine::take_io_request`](crate::engine::Engine::take_io_request)). The
//! host (a frontend, or the demo/tests) maps slots to files, calls
//! [`Engine::save`](crate::engine::Engine::save)/`restore`/`import_original`,
//! and re-scans. The filesystem glue lives in
//! [`crate::saveload_fs`](crate::saveload_fs) (off the wasm target).
//!
//! Slot semantics read from coab (D11): `SaveGame` (`ovr017.cs:1109`) prompts
//! `"A B C D E F G H I J"` — ten lettered slots; `loadGameMenu`
//! (`ovr017.cs:929-975`) lists only the occupied letters. Original saves live
//! at `savgam{letter}.dat` (`ovr017.cs:937/1129`).

/// The ten lettered save slots (`ovr017.cs:935` `'A'..='J'`).
pub const SLOT_LETTERS: [char; 10] = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J'];

/// What a save slot currently holds (host-determined by scanning the save dir).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SlotStatus {
    #[default]
    Empty,
    /// A restrike `.rsav` snapshot (our own format).
    RestrikeSave,
    /// An original CotAB `savgam{letter}.dat` set — loadable one-way via
    /// import (D-SAVE12: never written back).
    OriginalSave,
}

impl SlotStatus {
    /// A short status label for the slot list (functional UI vocabulary).
    pub fn label(self) -> &'static str {
        match self {
            SlotStatus::Empty => "(empty)",
            SlotStatus::RestrikeSave => "(saved game)",
            SlotStatus::OriginalSave => "(original save)",
        }
    }

    /// Whether a slot can be loaded/imported (non-empty).
    pub fn is_occupied(self) -> bool {
        self != SlotStatus::Empty
    }
}

/// Host-injected view of what each slot holds — the save/load screen renders
/// from this (never from the filesystem). Indexed by [`SLOT_LETTERS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SlotDirectory {
    statuses: [SlotStatus; 10],
}

impl Default for SlotDirectory {
    fn default() -> Self {
        SlotDirectory {
            statuses: [SlotStatus::Empty; 10],
        }
    }
}

impl SlotDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    fn index_of(letter: char) -> Option<usize> {
        SLOT_LETTERS
            .iter()
            .position(|&c| c == letter.to_ascii_uppercase())
    }

    pub fn status(&self, letter: char) -> SlotStatus {
        Self::index_of(letter)
            .map(|i| self.statuses[i])
            .unwrap_or(SlotStatus::Empty)
    }

    pub fn set(&mut self, letter: char, status: SlotStatus) {
        if let Some(i) = Self::index_of(letter) {
            self.statuses[i] = status;
        }
    }

    /// The (letter, status) pairs for all ten slots, in order.
    pub fn entries(&self) -> impl Iterator<Item = (char, SlotStatus)> + '_ {
        SLOT_LETTERS.iter().zip(self.statuses).map(|(&c, s)| (c, s))
    }

    /// The occupied slot letters only (`loadGameMenu`'s available list).
    pub fn occupied_letters(&self) -> Vec<char> {
        self.entries()
            .filter(|(_, s)| s.is_occupied())
            .map(|(c, _)| c)
            .collect()
    }
}

/// What the save/load screen asks the host to do once a slot is chosen. The
/// host performs the actual file I/O + engine save/restore/import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SaveLoadRequest {
    /// Write the current engine state to this slot's `.rsav` file.
    Save(char),
    /// Restore the engine from this slot's `.rsav` file.
    Load(char),
    /// One-way import of this slot's original `savgam{letter}.dat` set
    /// (D-SAVE12 — the original format is never written back).
    ImportOriginal(char),
}

/// Our snapshot filename for a slot (`.rsav`, our format) — e.g.
/// `SAVGAMA.RSAV`. Parallels coab's `savgam{letter}.dat` naming so a save
/// directory holds our slots alongside (never overwriting) the originals.
pub fn rsav_filename(letter: char) -> String {
    format!("SAVGAM{}.RSAV", letter.to_ascii_uppercase())
}

/// The original-format master filename for a slot (`ovr017.cs:937/1129`) —
/// what a host scans for to detect an importable slot. Read-only for us
/// (D-SAVE12).
pub fn original_master_filename(letter: char) -> String {
    format!("SAVGAM{}.DAT", letter.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_match_the_slot_conventions() {
        assert_eq!(rsav_filename('A'), "SAVGAMA.RSAV");
        assert_eq!(rsav_filename('j'), "SAVGAMJ.RSAV");
        assert_eq!(original_master_filename('B'), "SAVGAMB.DAT");
    }

    #[test]
    fn slot_directory_tracks_status_and_occupancy() {
        let mut dir = SlotDirectory::new();
        assert_eq!(dir.status('A'), SlotStatus::Empty);
        assert!(dir.occupied_letters().is_empty());
        dir.set('A', SlotStatus::RestrikeSave);
        dir.set('C', SlotStatus::OriginalSave);
        assert_eq!(dir.status('a'), SlotStatus::RestrikeSave);
        assert_eq!(dir.occupied_letters(), vec!['A', 'C']);
        assert_eq!(dir.entries().count(), 10);
    }

    #[test]
    fn out_of_range_letters_are_ignored_not_panics() {
        let mut dir = SlotDirectory::new();
        dir.set('Z', SlotStatus::RestrikeSave); // no such slot
        assert_eq!(dir.status('Z'), SlotStatus::Empty);
        assert!(dir.occupied_letters().is_empty());
    }
}
