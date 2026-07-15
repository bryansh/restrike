//! `import_original` (`docs/design/save-formats.md` D-SAVE5/D-SAVE7/D-SAVE8,
//! task deliverable 4): the full headless pipeline from a parsed
//! [`gbx_formats::save_orig::OriginalSaveSet`] to a live, tickable
//! [`Engine`] — the M3 exit gate's entry point. Assembly only; parsing lives
//! in `gbx-formats` (PLAN §2: "gbx-formats owns original save files").
//!
//! Reuses exactly the same `rebuild_engine`-style assembly `save.rs` uses
//! for `.rsav` restore (`Engine::assemble`) — both are "given engine state
//! and `GameData`, produce a running `Engine`"; the two entry points differ
//! only in where the state comes from (a decoded `SaveState` vs. a decoded
//! original save).

use crate::engine::{AssembledEngine, Engine, DEFAULT_GEO_BLOCK, GAME_AREA, INITIAL_ECL_BLOCK};
use crate::movement::{Facing, GameClock};
use crate::party::{character_from_record, Party};
use crate::rng::EngineRng;
use crate::shell::{EngineState, GameState, Shell};
use crate::text::{TextCursor, TextPacer};
use crate::vmhost::{
    load_ecl_block, load_geo_block, reload_walldefs, LoadEclError, LoadGeoError, ResidentAssets,
    VmMemoryState, WindowsSnapshot,
};
use gbx_formats::game_data::GameData;
use gbx_formats::save_orig::OriginalSaveSet;
use gbx_vm::{EclMachine, COTAB};
use std::collections::BTreeMap;

/// [`import_original`]'s failure mode — every case is a `GameData`/asset
/// problem (the wrong data set, or a `savgam` naming a block that isn't in
/// it), never malformed *save bytes* (those are [`gbx_formats::save_orig::SaveParseError`]/
/// [`gbx_formats::save_orig::ImportSetError`], surfaced by the parse step
/// this function's caller already ran).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Ecl(LoadEclError),
    Geo(LoadGeoError),
    Boot(String),
}

impl From<LoadEclError> for ImportError {
    fn from(e: LoadEclError) -> Self {
        ImportError::Ecl(e)
    }
}

impl From<LoadGeoError> for ImportError {
    fn from(e: LoadGeoError) -> Self {
        ImportError::Geo(e)
    }
}

/// The §1.6 "unset → area default" sentinel: `LastEclBlockId` /
/// `current_3DMap_block_id` are `0` in a never-chained save (FD-23 — GOG's
/// bundled slot-A save carries `0` for both), which the walk loop reads as
/// "use the area's boot block" (`engine.rs`'s `LastEclBlockId == 0 ->
/// EclBlockId = 1` note). A nonzero id (a chained/visited save) is used
/// verbatim.
fn resolve_block(stored: u8, default: u8) -> u8 {
    if stored == 0 {
        default
    } else {
        stored
    }
}

/// Converts §1.1's raw `area_ptr`/`area2_ptr`/`stru_1B2CA` blobs into a
/// [`WindowsSnapshot`] (D-SAVE7): every word of all three blobs lands in the
/// raw fallback store at the *same* address `VmMemoryState`'s own live
/// dispatch already uses (`AREA_WINDOW`/`TABLE_WINDOW`/`PARTY_WINDOW`,
/// `0x4B00`/`0x7A00`/`0x7C00` word-based) — "raw blob first", so any
/// script-stashed word round-trips even at a cell this codebase hasn't
/// named yet (D-VM5's discovery-backlog guarantee, applied to import). The
/// handful of named cells this codebase's facade *does* understand (the
/// clock, `inDungeon`) are then set from the *same* bytes via
/// [`master_to_engine_state`], not derived from this raw store — matching
/// D-SAVE7's "raw first, named cells read through the facade" without
/// needing a full `EngineVmHost` just to replay word-by-word writes.
fn windows_from_master(master: &gbx_formats::save_orig::MasterSave) -> WindowsSnapshot {
    const AREA_BASE: u16 = 0x4B00;
    const TABLE_BASE: u16 = 0x7A00;
    const PARTY_BASE: u16 = 0x7C00;

    let mut raw_words = BTreeMap::new();
    for (i, chunk) in master.area_ptr.chunks_exact(2).enumerate() {
        raw_words.insert(
            AREA_BASE + i as u16,
            u16::from_le_bytes([chunk[0], chunk[1]]),
        );
    }
    for (i, chunk) in master.stru_1b2ca.chunks_exact(2).enumerate() {
        raw_words.insert(
            TABLE_BASE + i as u16,
            u16::from_le_bytes([chunk[0], chunk[1]]),
        );
    }
    for (i, chunk) in master.area2_ptr.chunks_exact(2).enumerate() {
        raw_words.insert(
            PARTY_BASE + i as u16,
            u16::from_le_bytes([chunk[0], chunk[1]]),
        );
    }

    let mut walldefs: [Option<(u8, u8)>; 3] = [None; 3];
    for sb in &master.set_blocks {
        if sb.block_id == 0 && sb.set_id == 0 {
            continue;
        }
        let set = sb.set_id as u8;
        if let Some(slot) = (set as usize).checked_sub(1).filter(|&s| s < 3) {
            walldefs[slot] = Some((set, sb.block_id as u8));
        }
    }

    WindowsSnapshot {
        raw_words,
        raw_bytes: BTreeMap::new(),
        raw_strings: BTreeMap::new(),
        assets: ResidentAssets {
            map_3d_block: Some(resolve_block(
                master.current_3d_map_block_id(),
                DEFAULT_GEO_BLOCK,
            )),
            walldefs,
            bigpic_block: None,
        },
        ..Default::default()
    }
}

/// Populates [`EngineState`]'s fields the original save carries under its
/// own byte offsets (§1.4) — position/facing from section 6 (authoritative,
/// §1.4: `lastXPos/Y` in `area_ptr` are a shadow copy of these), the game
/// clock and dungeon/wilderness state from `area_ptr`'s named cells, and
/// the handful of `area2_ptr` cells this codebase's `EngineState` already
/// has fields for (`search_flags`, `head_block_id`, `tried_to_exit_map`).
/// Everything else `EngineState` needs stays at [`EngineState::new`]'s
/// fresh defaults (`chained`/`party_killed`/`field_592`/`door_flags`/etc —
/// none of which the original format stores; a fresh exploration state is
/// exactly right here, matching `loadSaveGame`'s own re-entry, D-SAVE8).
fn master_to_engine_state(
    master: &gbx_formats::save_orig::MasterSave,
    ecl_block_id: u8,
) -> EngineState {
    let mut state = EngineState::new();
    state.pos = (master.map_pos_x, master.map_pos_y);
    // §1.4: on-disk `mapDirection` is the *logical* 0..=3 direction (0x033D's
    // "raw (unhalved)" doc comment implies `Facing::raw_code()` is a distinct,
    // doubled representation) — normalize mod 4 before doubling so untrusted
    // save bytes can never hit `Facing::from_raw`'s panic path (D-SAVE10).
    state.facing = Facing::from_raw((master.map_direction % 4) * 2);
    state.search_flags = master.search_flags() as u8;
    state.head_block_id = master.head_block_id() as u8;
    state.tried_to_exit_map = master.tried_to_exit_map();
    state.ecl_block_id = ecl_block_id;
    state.clock = GameClock::from_raw_clock_words(master.clock_words());
    state.game_state = if master.in_dungeon() {
        GameState::DungeonMap
    } else {
        GameState::WildernessMap
    };
    state.last_game_state = state.game_state;
    // loadSaveGame always sets reload_ecl_and_pictures=true (§1.4/§1.5) —
    // the mechanism by which field_200/field_6F2 survive a load without
    // being cleared by vm_init_ecl's fresh-block-entry reset.
    state.reload_ecl_and_pictures = true;
    state
}

/// The full headless import pipeline (D-SAVE5): sections 2-4 into the
/// ScriptMemory windows raw-first then named (D-SAVE7); section 5 bytes
/// discarded, the resident block reloaded fresh by `LastEclBlockId` from
/// `data` (§1.5); the `EclMachine` starts idle, then the walk loop's own
/// entry-vector re-entry (`Shell::boot`) fires exactly as a fresh boot or a
/// native `.rsav` load would; wallsets/3D map reloaded by `setBlocks` ids;
/// the party roster built from the `CHRDAT` records (§1.7 items 1/2/5
/// already resolved by `gbx_formats::save_orig::decode_char_record`).
///
/// `seed` seeds the engine's one PRNG (D9) — the original format carries no
/// PRNG state to restore (it never had a PRNG), so a fresh seed is required
/// input, not a default.
pub fn import_original(
    set: &OriginalSaveSet,
    data: GameData,
    seed: u32,
) -> Result<Engine, ImportError> {
    let master = &set.master;

    let ecl_block_id = resolve_block(master.last_ecl_block_id() as u8, INITIAL_ECL_BLOCK);
    let ecl_bytes = load_ecl_block(&data, GAME_AREA, ecl_block_id)?;
    let mut machine =
        EclMachine::load_block(ecl_bytes, &COTAB).unwrap_or_else(|never| match never {});

    let mut state = master_to_engine_state(master, ecl_block_id);
    let shell = Shell::boot(&mut machine, &mut state);

    let mut vm_memory = VmMemoryState::new();
    vm_memory.restore_windows(windows_from_master(master));

    let geo_block_id = resolve_block(master.current_3d_map_block_id(), DEFAULT_GEO_BLOCK);
    let geo = load_geo_block(&data, GAME_AREA, geo_block_id)?;

    let boot_assets = crate::boot::boot(&data).map_err(|e| ImportError::Boot(format!("{e:?}")))?;
    let mut symbol_sets = boot_assets.symbol_sets;
    reload_walldefs(
        &mut symbol_sets,
        &data,
        GAME_AREA,
        &vm_memory.snapshot().assets.walldefs,
    );

    let party = Party {
        members: set
            .chars
            .iter()
            .map(|oc| character_from_record(&oc.record, oc.items.clone(), oc.affects.clone()))
            .collect(),
    };

    let verify_report = gbx_rules::pack::RuleSet::load().verify(&data);

    Ok(Engine::assemble(AssembledEngine {
        fb: crate::framebuffer::Framebuffer::new(),
        font: boot_assets.font,
        geo,
        data,
        shell,
        state,
        machine,
        vm_memory,
        party,
        rng: EngineRng::new(seed),
        cursor: TextCursor::new(),
        pacer: TextPacer::new(4),
        symbol_sets,
        sky: boot_assets.sky,
        verify_report,
        // u64 provenance, u32 live seed zero-extended (D-OR1) — see engine.rs.
        boot_seed: seed as u64,
        tick_count: 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::save_orig::{decode_char_record, CHAR_RECORD_SIZE};

    fn tiny_char(name: &str) -> gbx_formats::save_orig::CharRecord {
        let mut bytes = vec![0u8; CHAR_RECORD_SIZE];
        bytes[0] = name.len() as u8;
        bytes[1..1 + name.len()].copy_from_slice(name.as_bytes());
        decode_char_record(&bytes).unwrap()
    }

    #[test]
    fn resolve_block_maps_the_zero_sentinel_to_the_area_default() {
        // FD-23: a never-chained save stores 0 ("use the area boot block").
        assert_eq!(resolve_block(0, INITIAL_ECL_BLOCK), INITIAL_ECL_BLOCK);
        assert_eq!(resolve_block(0, DEFAULT_GEO_BLOCK), DEFAULT_GEO_BLOCK);
        // A visited/chained save's nonzero id is used verbatim.
        assert_eq!(resolve_block(7, INITIAL_ECL_BLOCK), 7);
    }

    #[test]
    fn windows_from_master_places_area_words_at_the_live_dispatch_addresses() {
        let mut bytes = vec![0u8; gbx_formats::save_orig::SAVGAM_SIZE];
        // area_ptr starts right after the 1-byte game_area section.
        bytes[1] = 0xAB;
        bytes[2] = 0xCD; // word 0 of area_ptr = 0xCDAB (LE)
        let master = gbx_formats::save_orig::parse_master(&bytes).unwrap();
        let snap = windows_from_master(&master);
        assert_eq!(snap.raw_words.get(&0x4B00), Some(&0xCDAB));
    }

    #[test]
    fn master_to_engine_state_normalizes_out_of_range_direction_without_panicking() {
        let mut bytes = vec![0u8; gbx_formats::save_orig::SAVGAM_SIZE];
        let position_off = 1 + 0x800 + 0x800 + 0x400 + 0x1E00;
        bytes[position_off] = 7; // mapPosX
        bytes[position_off + 1] = 3; // mapPosY
        bytes[position_off + 2] = 255; // mapDirection -- garbage, must not panic
        let master = gbx_formats::save_orig::parse_master(&bytes).unwrap();
        let state = master_to_engine_state(&master, 1);
        assert_eq!(state.pos, (7, 3));
        // 255 % 4 == 3 -> Facing::from_raw(6) == West.
        assert_eq!(state.facing, Facing::West);
    }

    /// Local-tier (D10): the full validated pipeline against GOG's bundled
    /// slot-A save, which ships under `$GBX_DATA_DIR/SAVE/` (the GOG layout —
    /// FD-23's "Also found" note; `save-formats.md` §1.1 assumed the game
    /// root). The save bytes never enter the repo; only the asserted facts
    /// (already committed to the docket under FD-23) appear here. Loud-skips
    /// when `GBX_DATA_DIR` is unset. Run with
    /// `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine`.
    #[test]
    fn imports_gogs_bundled_slot_a_save_from_the_save_subdir() {
        let Some(root) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!(
                "SKIPPED: local tier needs GBX_DATA_DIR \
                 (import::imports_gogs_bundled_slot_a_save_from_the_save_subdir)"
            );
            return;
        };
        let root = std::path::Path::new(&root);
        let data = gbx_formats::game_data::load_dir(root).expect("GBX_DATA_DIR must be readable");

        // GOG reads/writes saves in a `SAVE/` subdirectory of the game dir,
        // not the root (FD-23) — look there, not in `root` itself.
        let save_dir = root.join("SAVE");
        let saves = gbx_formats::game_data::load_dir(&save_dir)
            .expect("GBX_DATA_DIR/SAVE must be readable (GOG's bundled save lives here)");

        let master_bytes = saves
            .raw_file("SAVGAMA.DAT")
            .expect("GBX_DATA_DIR/SAVE/SAVGAMA.DAT (GOG's bundled slot-A save) must exist");
        let set = gbx_formats::save_orig::load_from_lookup(master_bytes, 'A', |name| {
            saves.raw_file(name)
        })
        .expect("the bundled slot-A save set must parse");

        // Facts pinned in docs/fidelity-docket.md FD-23 (D10-clean asserted
        // numbers, not save content): Tilverton (area 2), a party of six.
        assert_eq!(
            set.master.game_area, 2,
            "bundled save is Tilverton (area 2)"
        );
        assert_eq!(set.master.party_count, 6, "MATHEW's party of six");

        let engine =
            import_original(&set, data, 0x5A1E_5A1E).expect("importing the bundled save succeeds");

        assert_eq!(engine.state().pos, (7, 13), "party stands at (7,13)");
        assert_eq!(engine.party().members.len(), 6);

        // MATHEW (slot A1) is an 18/00 paladin: exceptional strength decodes
        // to 100 unclamped (FD-23 item 5 — coab's `Math.Min(_, 25)` would
        // read 25). This save has no stat-drained character, so current ==
        // original throughout.
        let mathew = &engine.party().members[0];
        assert_eq!(mathew.stats.str_exceptional.current, 100);
        assert_eq!(mathew.stats.str_exceptional.original, 100);
    }

    #[test]
    fn party_members_are_built_from_every_char_record() {
        let set = OriginalSaveSet {
            master: {
                let bytes = vec![0u8; gbx_formats::save_orig::SAVGAM_SIZE];
                gbx_formats::save_orig::parse_master(&bytes).unwrap()
            },
            chars: vec![
                gbx_formats::save_orig::OriginalChar {
                    record: tiny_char("Aran"),
                    items: vec![],
                    affects: vec![],
                },
                gbx_formats::save_orig::OriginalChar {
                    record: tiny_char("Bink"),
                    items: vec![],
                    affects: vec![],
                },
            ],
        };
        let party = Party {
            members: set
                .chars
                .iter()
                .map(|oc| character_from_record(&oc.record, oc.items.clone(), oc.affects.clone()))
                .collect(),
        };
        assert_eq!(party.members.len(), 2);
        assert_eq!(party.members[0].name, "Aran");
        assert_eq!(party.members[1].name, "Bink");
    }
}
