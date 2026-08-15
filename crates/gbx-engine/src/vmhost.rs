//! The real `VmHost` implementation (D-VM4/D-VM5, task deliverables 1-2):
//! `ScriptMemory`'s window dispatch (named cells + raw fallback + the
//! unknown-access log) and `EngineServices` (the M2 subset gets real
//! implementations; everything else is a logged M3/M4 stub).
//!
//! Derived by reading coab for behavior (D11, never copied) — a dedicated
//! research pass this session read `engine/ovr008.cs`'s
//! `vm_GetMemoryValueType`/`vm_GetMemoryValue`/`vm_SetMemoryValue` in full,
//! `load_ecl_dax` (`:136-154`), `seg001.cs`'s `game_area` boot init, and
//! `sub_30580` (`:220-276`); every address/behavior below cites that pass.
//! Per the M2 step 4 task brief's scope note, `load_3d_map`/`load_bigpic`
//! **record** resident-block state; they do not draw (3D/area-map rendering
//! is step 5). `load_bigpic` keeps that posture for a different reason since
//! the scene-pictures slice: it is only ever called for the *wilderness*
//! overland map (`ovr003.cs:532,1010`, `in_dungeon == 0`), whose display goes
//! through `RedrawView`'s non-dungeon branch (`ovr029.cs:41-44`) and so waits
//! on a wilderness `game_state` this engine does not reach yet.
//! `CMD_Picture`'s own `blockId >= 0x78` arm — the BIGPIC path that *does*
//! draw — lives in `crate::picture`. `load_walldef` graduated to a real implementation in step 5
//! (task deliverable 1): it now actually loads the walldef's tile-id table
//! and its paired 8×8 pixel data into [`crate::symbols::SymbolSets`], which
//! `crate::corridor`'s renderer reads from.

use crate::movement::Facing;
use crate::shell::{EngineState, GameState};
use crate::symbols::SymbolSets;
use gbx_formats::game_data::{GameData, GameDataError};
use gbx_formats::geo::GeoBlock;
use gbx_formats::monster::MonsterFile;
use gbx_formats::walldef::WalldefBlock;
use gbx_vm::{
    BlockBytes, ItemHandle, MissingData, MonsterHandle, Origin, PlayerId, ProgramOutcome,
    RecordedCall, ScriptMemory, VmRng, VmString, ECL_BLOCK_SIZE,
};
use std::collections::{HashMap, HashSet};

/// `Status` (`Classes/Enums.cs`; `charsheet.rs`'s `STATUS` table is the same
/// indexing): the health-status codes the out-of-combat damage path assigns.
const HEALTH_OKAY: u8 = 0;
const HEALTH_ANIMATED: u8 = 1;
const HEALTH_UNCONSCIOUS: u8 = 4;
const HEALTH_DYING: u8 = 5;
const HEALTH_DEAD: u8 = 6;

/// `Control.NPC_Base` (`Classes/Player.cs:318-325`) — the morale-byte floor
/// that marks a roster member as an NPC.
const NPC_BASE: u8 = 0x80;

/// The color code every wallset's paired 8×8 symbol data is loaded masked
/// against — the same convention as boot's `Load8x8D` (`boot.rs`'s
/// `BOOT_MASK`, design doc §1.3).
const WALLSET_MASK: u8 = 13;

/// `load_ecl_dax` (`ovr008.cs:136-154`, this session's research §2/§3):
/// `block_id` within `"ECL{game_area}.DAX"` — the file name embeds
/// `game_area`, so the same numeric `block_id` in a different area is a
/// different block entirely. This session's `Engine` fixes `game_area = 2`
/// (matching the already-validated M1/step-3 precedent — real Tilverton
/// data lives in `GEO2.DAX`/`ECL2.DAX`); the research pass also found the
/// literal boot-time default is `1` (`InitFirst`/`InitAgain`,
/// `seg001.cs:276-277,369-370`, with a same-file `game_area = 2` branch for
/// non-demo play seemingly clobbered right after — flagged as UNSURE/a
/// possible transliteration quirk, docketed rather than resolved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadEclError {
    GameData(GameDataError),
    /// The decoded payload doesn't leave enough bytes after the 2-byte
    /// prefix `load_ecl_dax` skips (`ovr008.cs:151`) to be a real block.
    TooShort,
}

impl From<GameDataError> for LoadEclError {
    fn from(e: GameDataError) -> Self {
        LoadEclError::GameData(e)
    }
}

/// Loads block `block_id` from `"ECL{game_area}.DAX"` and prepares it as a
/// resident [`BlockBytes`] (the `ecl_block_payload` 2-byte-prefix skip,
/// `dax.rs`'s own citation to `ovr008.cs:151`), oversize-truncated to
/// [`ECL_BLOCK_SIZE`] exactly like `frontends/cli/run_script.rs`'s loader.
pub fn load_ecl_block(
    data: &GameData,
    game_area: u8,
    block_id: u8,
) -> Result<BlockBytes, LoadEclError> {
    let raw = data.block(&format!("ECL{game_area}.DAX"), block_id)?;
    let payload = gbx_formats::dax::ecl_block_payload(&raw);
    if payload.is_empty() {
        return Err(LoadEclError::TooShort);
    }
    let payload = &payload[..payload.len().min(ECL_BLOCK_SIZE)];
    Ok(BlockBytes::from_bytes(payload))
}

/// Loads GEO block `block_id` from `"GEO{game_area}.DAX"` — the same
/// `game_area`-embeds-the-filename convention `load_ecl_block`/`LoadWalldef`
/// use. Added for original-save import (`docs/design/save-formats.md`
/// D-SAVE5, task deliverable 4): unlike ECL/walldef reload, the live VM's
/// `EngineServices::load_3d_map` only *records* the resident block id
/// (`ovr031.Load3DMap`'s asset-swap isn't wired into a running script path
/// this milestone, `renderer-ui-shell.md` FD-19) — import has no running
/// script to drive that call anyway (the `EclMachine` starts idle, D-SAVE8),
/// so it resolves the GEO block directly.
pub fn load_geo_block(
    data: &GameData,
    game_area: u8,
    block_id: u8,
) -> Result<gbx_formats::geo::GeoBlock, LoadGeoError> {
    let raw = data.block(&format!("GEO{game_area}.DAX"), block_id)?;
    gbx_formats::geo::GeoBlock::parse(&raw).map_err(LoadGeoError::Geo)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadGeoError {
    GameData(GameDataError),
    Geo(gbx_formats::geo::GeoError),
}

impl From<GameDataError> for LoadGeoError {
    fn from(e: GameDataError) -> Self {
        LoadGeoError::GameData(e)
    }
}

/// `LoadWalldef`'s pixel/tile-id loading core (`ovr031.cs:642-687`), factored
/// out of the `EngineServices` trait method so original-save import
/// (task deliverable 4) can reload wallsets by `setBlocks` id without
/// constructing a full `EngineVmHost` — see `EngineVmHost::load_walldef`'s
/// doc comment for the full citation and the multi-sub-block/rebase
/// behavior this replicates exactly. Silent no-op on any load failure
/// (missing block, malformed data), matching the trait method's own
/// fault-tolerance.
fn load_walldef_pixels(symbols: &mut SymbolSets, data: &GameData, game_area: u8, set: u8, id: u8) {
    if !(1..=3).contains(&set) {
        return;
    }

    let Ok(raw) = data.block(&format!("WALLDEF{game_area}.DAX"), id) else {
        return;
    };
    let Ok(walldef) = WalldefBlock::parse(&raw) else {
        return;
    };
    let block_count = walldef.wallset_count();
    if block_count == 0 {
        return;
    }

    let rebase = (crate::symbols::SYMBOL_SET_FIX[set as usize] as i32
        - crate::symbols::SYMBOL_SET_FIX[1] as i32) as u8;
    let sym_file = format!("8X8D{game_area}.DAX");

    for block in 0..block_count {
        let idx = set as usize + block;
        if !(1..=3).contains(&idx) {
            continue;
        }

        let mut tiles = [0u8; gbx_formats::walldef::WALLSET_SIZE];
        for style in 0..gbx_formats::walldef::STYLES_PER_WALLSET {
            for i in 0..gbx_formats::walldef::TILE_IDS_PER_STYLE {
                let raw_id = walldef.tile_id(block, style, i).unwrap_or(0);
                tiles[style * gbx_formats::walldef::TILE_IDS_PER_STYLE + i] = if raw_id >= 0x2D {
                    raw_id.wrapping_add(rebase)
                } else {
                    raw_id
                };
            }
        }

        let sym_block_id = if block_count > 1 {
            id.wrapping_mul(10).wrapping_add(block as u8 + 1)
        } else {
            id
        };
        let Ok(bytes) = data.block(&sym_file, sym_block_id) else {
            continue;
        };
        let Ok(decoded) = gbx_formats::image::decode(&bytes, Some(WALLSET_MASK)) else {
            continue;
        };

        symbols.load(idx, decoded);
        symbols.load_wallset(idx - 1, crate::symbols::WallsetSlot::from_tiles(tiles));
    }
}

/// Reloads every non-empty `setBlocks[0..2]` slot's wallset (§1.5, task
/// deliverable 4) — the import/restore counterpart of
/// `EngineVmHost::load_walldef`'s live-VM path, driven directly since
/// there's no running script during import (`EclMachine` starts idle).
pub fn reload_walldefs(
    symbols: &mut SymbolSets,
    data: &GameData,
    game_area: u8,
    set_blocks: &[Option<(u8, u8)>; 3],
) {
    for entry in set_blocks.iter().flatten() {
        let (set, id) = *entry;
        load_walldef_pixels(symbols, data, game_area, set, id);
    }
}

// --- Window ranges (D-VM5 / this session's research §1.0) ---

const AREA_WINDOW: std::ops::RangeInclusive<u16> = 0x4B00..=0x4EFF;
const TABLE_WINDOW: std::ops::RangeInclusive<u16> = 0x7A00..=0x7BFF;
const PARTY_WINDOW: std::ops::RangeInclusive<u16> = 0x7C00..=0x7FFF;

/// The Area window's ECL-clock word cluster (FD-31 resolved): 7 consecutive
/// addresses `0x4BC6..=0x4BCC` under the halved mapping — `field_18C`,
/// minutes-ones, minutes-tens, hour, day, year, `field_198` (`Area1.cs`
/// DataOffsets `0x18C..=0x198`, the `field_6A00_Get`/`Set` switches being
/// the authority over the file's own duplicate-attribute typo on
/// `field_18C`).
const CLOCK_BASE: u16 = 0x4BC6;
/// `area_ptr.inDungeon` (`Classes/Area1.cs:65-66`, DataOffset `0x1CC`).
const IN_DUNGEON_ADDR: u16 = 0x4BE6;
/// ★ `area_ptr.lastXPos`/`lastYPos` (`Classes/Area1.cs:72-75`, DataOffsets
/// `0x1E0`/`0x1E2`; plain get/set cases at `:245-250`/`:489-493`) — the walk
/// loop's record of where the party stood when this step's per-step script
/// finished, written immediately before `locked_door()` and compared against
/// the live position immediately after (`ovr003.cs:2371-2381`).
///
/// **This is FD-19.** Tilverton's (7,12)-North event copies both cells
/// straight back into `mapPosX`/`mapPosY` (`ECL2#1 @0x9444`/`@0x944B`) to
/// bounce the party off the wrong entrance. Unnamed, they answered 0 from the
/// raw store — which is where the docketed "party lands at (0,0)" came from.
const LAST_XPOS_ADDR: u16 = 0x4BF0;
const LAST_YPOS_ADDR: u16 = 0x4BF1;
/// ★ `area_ptr.LastEclBlockId` (`Classes/Area1.cs:76-77`, DataOffset `0x1E4`;
/// `:252-254`/`:498-499`) — "which block did we arrive from", the id
/// `CMD_NewECL` commits before chaining (`ovr003.cs:488`).
///
/// Every multi-entrance block branches on it in its own entry vector:
/// `ECL1#80 @0x8024` tests it against `0x51`, `0x30` (= `ECL5#48`, the
/// overland exit this slice drives), `0x40` and `0x04` to decide which edge of
/// the wilderness the party is standing on, and `ECL2#1 @0x8032`/`ECL2#2
/// @0x8022` do the same against their own ids. `EngineState` has carried the
/// value since M2; it was simply never wired to the address scripts read it
/// at, so after the first chain every arrival branch took its default arm.
const LAST_ECL_BLOCK_ID_ADDR: u16 = 0x4BF2;
/// ★ `area_ptr.block_area_view` (`Classes/Area1.cs:79`, DataOffset `0x1F6`;
/// setter case `:256-258`) — **FD-41's unread cell**. Nonzero forbids the
/// area map: `main_3d_world_menu`'s `'A'` shows the timed `"Not Here"`
/// instead of toggling (`ovr015.cs:368-379`), and `RedrawView` force-clears
/// `mapAreaDisplay` on every redraw while it is set (`ovr029.cs:34-38`).
///
/// A live cell, not a boot-time config: `ECL5#48 @0x8086` writes it `0`
/// immediately before crossing to the overland, so both consumers read it at
/// the moment they need it ([`VmMemoryState::block_area_view`]).
const BLOCK_AREA_VIEW_ADDR: u16 = 0x4BFB;
/// ★ `area_ptr.current_city` (`Classes/Area1.cs:154`, DataOffset `0x342`;
/// setter case `:390-392`) — the index into `MapCursor`'s 33-city coordinate
/// tables (`crate::mapcursor`), i.e. where the blinking cursor sits on the
/// Dalelands map.
///
/// `ECL1#80` owns it: `@0x809F SAVE [0x4C9B],[0x4CA1]` sets it to the city
/// the party is standing at, and `@0x910C GETTABLE 0x9D13,[0x4C9D],[0x4CA1]`
/// re-points it mid-journey at the route's own map cell, so a travel
/// encounter's "PRESS BUTTON" prompt blinks where the encounter happened.
/// `ovr011.cs:750` copies it into `gbl.current_city` at wilderness-floor
/// setup, which is where the terrain generator reads it from.
const CURRENT_CITY_ADDR: u16 = 0x4CA1;
/// `area2_ptr.rest_incounter_period`/`rest_incounter_percentage`
/// (`Classes/Area2.cs:56-59`, DataOffsets `0x5A4`/`0x5A6`) — Rest's random-
/// encounter schedule, read by the camp loop (`ovr021.cs:586-594`) and zeroed
/// at every block entry (`ovr008.cs:111-112`). Script-writable through the
/// Party window (`field_800_Set`'s own `0x5a4`/`0x5a6` cases), and real
/// content arms them per block: `ECL4#37 @0x822E`/`@0x8234` writes percentage
/// `0x0A`, period `0x1E` immediately after its overland `NEWECL`.
const REST_INCOUNTER_PERIOD_ADDR: u16 = 0x7ED2;
const REST_INCOUNTER_PERCENTAGE_ADDR: u16 = 0x7ED3;
/// `area_ptr.field_200` (`Classes/Area1.cs:92-93`) — a 33-word script scratch
/// table at DataOffset `0x200`, i.e. 33 CONSECUTIVE window addresses under the
/// halved Area mapping. `RestField200Values` zeroes all 33
/// (`Classes/Area1.cs:658-663`: `loop_var <= 32` — the field's own "1-32"
/// comment undercounts its own loop). Real content lives here: `ECL2#1`
/// writes `@0x987B` and reads `@0x813B`/`@0x9747`.
const FIELD_200_BASE: u16 = 0x4C00;
const FIELD_200_LEN: u16 = 33;
/// `area2_ptr.field_6F2 .. field_704` (`Classes/Area2.cs:89-108`) — ten words
/// at DataOffsets `0x6F2..=0x704`, ten consecutive Party-window addresses.
/// `RestField6F2Values` zeroes them one by one (`Classes/Area2.cs:320-332`).
/// `ECL5#48` uses the first as its `LOAD CHARACTER` slot-scan counter
/// (`@0x809B`) and the last as a `HORIZONTAL MENU` destination (`@0x821A`).
const FIELD_6F2_BASE: u16 = 0x7F79;
const FIELD_6F2_LEN: u16 = 10;
/// Both addresses set the same `byte_1EE94` redraw-dirty flag on write
/// (research §1.5) — and under the confirmed halved mapping (see
/// `crate::picture::PICTURE_FADE_ADDR`'s derivation) they ARE
/// `Area1.outdoor_sky_colour` / `indoor_sky_colour` (`DataOffset`
/// `0x1FA`/`0x1FC`), which is exactly why writing a sky colour dirties the
/// view. `crate::corridor` reads them at these same addresses (FD-31).
const FORCE_REDRAW_ADDRS: [u16; 2] = [0x4BFD, 0x4BFE];

/// `area2_ptr.HeadBlockId` (`Classes/Area2.cs:62`, `DataOffset 0x5C2`) —
/// `CMD_Picture`'s arm selector (`ovr003.cs:322`), written directly by real
/// ECL content: every `PICTURE` in Tilverton's `ECL2` block 1 is preceded by
/// `SAVE <head-id>, 0x7EE1` and followed by `SAVE 0xFF, 0x7EE1`.
///
/// The Area2 window's mapping is `DataOffset = (addr - 0x7C00) * 2`
/// (`ovr008.cs:721` passes `(location * 2) + 0x800` to `field_800_Set`, and
/// `0x7C00 * 2 + 0x800` is `0x10000` — zero in the `ushort` index it masks
/// to), so `0x5C2 / 2 + 0x7C00 == 0x7EE1`. The rest of this window is still
/// raw+logged (`alter_character` is unmodelled); this one cell is named
/// because the picture layer reads it every `PICTURE`.
const HEAD_BLOCK_ID_ADDR: u16 = 0x7EE1;

/// ★ `area2_ptr.max_encounter_distance` / `area2_ptr.encounter_distance`
/// (`Classes/Area2.cs:40-43`, DataOffsets `0x580`/`0x582`). Under the Area2
/// window's `DataOffset = (addr - 0x7C00) * 2` mapping these are `0x7EC0` and
/// `0x7EC1`.
///
/// Both are genuinely script-addressable: `0x580` has an explicit
/// `field_800_Set` case (`Area2.cs:207-209`) and `0x582` reaches the same
/// field through the reflection default (`Area2.cs:314-316`) — the asymmetry
/// is a C#-typing artifact of the port, not a behavior difference. Naming
/// them here keeps a script write and the encounter cluster's own service
/// accessors ([`EngineServices::encounter_distance`]) looking at one cell.
const MAX_ENCOUNTER_DISTANCE_ADDR: u16 = 0x7EC0;
const ENCOUNTER_DISTANCE_ADDR: u16 = 0x7EC1;

/// ★ `area2_ptr.game_area` (`Classes/Area2.cs:70-71`, `DataOffset 0x624`) —
/// **the area switch** (roll-credits D-RC1/D-S1a). Under the Area2 window's
/// `DataOffset = (addr - 0x7C00) * 2` mapping, `0x624 / 2 + 0x7C00 == 0x7F12`.
///
/// Write side: `vm_SetMemoryValue`'s Party arm calls `alter_character`, whose
/// `switch_var == 0x312` case is `seg042.set_game_area(value)`
/// (`ovr008.cs:654-657` → `seg042.cs:124-128`) — backup ← live, live ← value.
/// Read side: `get_player_values`' own `arg_4 == 0x312` case returns
/// `gbl.game_area`, the **live** global, and sets its found-flag, so
/// `field_800_Get` (which would have answered from the Area2 struct shadow)
/// is never consulted (`ovr008.cs:545-548`, `:833-838`).
///
/// The original's write *also* lands in the `area2_ptr.game_area` struct byte
/// on its way through `field_800_Set` (`ovr008.cs:721`). That shadow is never
/// independently observable: `SaveGame` re-syncs it *from* `gbl.game_area`
/// before serializing (`ovr017.cs:1146`) and `loadSaveGame` reads it straight
/// back *into* `gbl.game_area` (`:1072`). Our `.rsav` carries
/// [`EngineState::game_area`](crate::shell::EngineState::game_area) directly,
/// so this dispatch models the live pair only — deliberately, not by omission.
const GAME_AREA_ADDR: u16 = 0x7F12;

/// ★ `area2_ptr.EnterTemple` (`Classes/Area2.cs:65`, `DataOffset 0x5C4`) and
/// `.EnterShop` (`:79`, `0x6D8`) — **the two flags that turn a `COMBAT` opcode
/// into a shop** (roll-credits slice 6 / G8).
///
/// `CMD_Combat`'s non-monster branch (`ovr003.cs:974-992`) reads them, clears
/// the one it found, and dispatches; nothing else in the game does. Under the
/// Area2 window's `DataOffset = (addr - 0x7C00) * 2` mapping they are
/// `0x7C00 + 0x2E2` and `0x7C00 + 0x36C`.
///
/// Both write sites are plain `SAVE <1> → <addr>` instructions, which is why
/// the opcode census could never show a temple: there is no temple opcode. The
/// four shipped temple sites and the shop sites are enumerated in
/// [`crate::shell::describe_combat_branch`].
const ENTER_TEMPLE_ADDR: u16 = 0x7EE2;
const ENTER_SHOP_ADDR: u16 = 0x7F6C;

/// ★ `area2_ptr.training_class_mask` (`Classes/Area2.cs`, `DataOffset 0x550`;
/// under the Area2 window's `DataOffset = (addr - 0x7C00) * 2` mapping,
/// `0x7C00 + 0x2A8`) — the town script's training permission, and
/// `startGameMenu`'s gate on `Train Character`/`Human Change Classes`
/// (`ovr018.cs:86-95`). `BEGIN Adventuring` spends it (`:269`), so walking out
/// of the menu into the world revokes it until a hall grants it again.
///
/// No cell of ours is written by anything yet: the shipped grant is a plain
/// `SAVE <mask> → 0x7EA8` in a town block, so it round-trips through the raw
/// window store exactly like every other unmodelled Area2 cell. Named here
/// rather than open-coded so the two readers agree.
const TRAINING_CLASS_MASK_ADDR: u16 = 0x7EA8;

/// ★ `area_ptr.field_3FA` (`Classes/Area1.cs:194`, `DataOffset 0x3FA` →
/// `0x4B00 + 0x1FD`) — the **game-won latch**. Exactly one writer exists in
/// the whole program, `CMD_Program`'s `var_1 == 8` arm (`ovr003.cs:1953`,
/// `field_3FA = 0xFF` right after `end_game_text()`), and exactly one reader:
/// `startGameMenu`'s BEGIN guard (`ovr018.cs:243`). After the ending, BEGIN
/// stops working and the menu's remaining job is the "You've won. Save before
/// quitting?" prompt (`:1965-1971`).
const GAME_WON_ADDR: u16 = 0x4CFD;

/// [`TRAINING_CLASS_MASK_ADDR`]'s current value.
pub fn training_class_mask(vm: &VmMemoryState) -> u16 {
    vm.raw_word(TRAINING_CLASS_MASK_ADDR).unwrap_or(0)
}

/// `gbl.area2_ptr.training_class_mask = 0` (`ovr018.cs:269`).
pub fn clear_training_class_mask(vm: &mut VmMemoryState) {
    vm.set_raw_word(TRAINING_CLASS_MASK_ADDR, 0);
}

/// [`GAME_WON_ADDR`]'s current value — non-zero once the game has been won.
pub fn game_won_flag(vm: &VmMemoryState) -> u16 {
    vm.raw_word(GAME_WON_ADDR).unwrap_or(0)
}

/// ★ `get_player_values`' `arg_4 == 0x100` case (`ovr008.cs:424-441`) — the
/// **only** cell a script uses to ask "is the selected character actually
/// there?", and the one every WHO (0x39) site tests on the very next
/// instruction (`COMPARE [0x7D00], 1`).
///
/// Under the Party window's `switch_var = addr - 0x7C00` mapping it is
/// `0x7C00 + 0x100`. Three answers, in the original's own order:
///
/// - `1` — `SelectedPlayer.in_combat` (alive, conscious, on the board);
/// - `0x80` — a member who is not;
/// - `0` — `gbl.player_not_found`, which **overrides both** and is
///   **cleared by the read** (`:436-440`). A one-shot: LOAD CHARACTER's
///   out-of-range arm arms it, the next read of this cell consumes it.
///
/// `alter_character`'s write side (`:618-632`) is the mirror: `>= 0x80`
/// clears `in_combat` (and `0x87` specifically means petrified), `0` arms
/// `redrawPartySummary1`.
const SELECTED_PLAYER_STATE_ADDR: u16 = 0x7D00;

/// ★ `CMD_Spell`'s per-slot read, `es:[player_ptr + index + 0x1E]`
/// (`ovr003:2EA0-2EA5`) — the byte at `charStruct` offset `0x1E + index`.
///
/// `spell_list` is 84 bytes at `0x1E`, so `index` in `1..=83` reads it (slot
/// **0 is never examined**, because `CMD_Spell` seeds its cursor at 1) and
/// `84..=101` walks off the end into the fields that follow. Those are named
/// here rather than clamped, because every not-found scan reaches all
/// eighteen of them and the comparison against them is real: a character
/// whose `hit_point_max` equals the queried spell id answers "found, slot
/// 90".
///
/// | index | `charStruct` | field |
/// |---|---|---|
/// | 84 | `0x72` | `spell_to_learn_count` |
/// | 85 | `0x73` | `thac0_base` |
/// | 86 | `0x74` | `race` |
/// | 87 | `0x75` | `class` |
/// | 88, 89 | `0x76`, `0x77` | `age`, low then high byte |
/// | 90 | `0x78` | `hit_point_max` |
/// | 91..=101 | `0x79`..=`0x83` | `spellBook[0..=10]` |
fn char_scan_byte(member: &crate::party::Character, index: u8) -> u8 {
    let i = index as usize;
    if i < 84 {
        return member.magic.spell_list.get(i).copied().unwrap_or(0);
    }
    match i {
        84 => member.magic.spell_to_learn_count,
        85 => member.combat.thac0_base as u8,
        86 => member.race,
        87 => member.class_id,
        88 => member.age.to_le_bytes()[0],
        89 => member.age.to_le_bytes()[1],
        90 => member.hit_point_max,
        91..=101 => member
            .magic
            .spell_book
            .get(i - 91)
            .copied()
            .unwrap_or_default(),
        _ => 0,
    }
}

/// One access kind, for the unknown-access log's `(addr, kind)` dedup key
/// (D-VM5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AccessKind {
    Read,
    Write,
    ReadByte,
    WriteByte,
    ReadString,
    WriteString,
}

/// One first-seen unknown access — the discovery backlog (D-VM5, PLAN §2.2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnknownAccess {
    pub addr: u16,
    pub kind: AccessKind,
    pub origin: Origin,
}

/// Dedups per `(addr, kind)`, keeping only the first-seen `Origin` — the
/// unknown-access log is a discovery backlog, not a full trace.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct UnknownAccessLog {
    seen: HashSet<(u16, AccessKind)>,
    entries: Vec<UnknownAccess>,
}

impl UnknownAccessLog {
    fn record(&mut self, addr: u16, kind: AccessKind, origin: Origin) {
        if self.seen.insert((addr, kind)) {
            self.entries.push(UnknownAccess { addr, kind, origin });
        }
    }

    pub fn entries(&self) -> &[UnknownAccess] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A halted vector run's diagnostic (the M2 halt policy, task deliverable
/// 4): every `VmError` is downgraded to a loud, counted status-line-visible
/// event rather than propagating a hard failure — the flow treats the run
/// as ended. Decoupled from `gbx_vm::VmError`'s own shape so this struct can
/// stay serde-derivable without changing that crate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HaltRecord {
    pub pc: u16,
    pub opcode: u8,
    pub description: String,
}

pub fn describe_halt(err: &gbx_vm::VmError) -> HaltRecord {
    use gbx_vm::VmError;
    match *err {
        VmError::UnknownOpcode { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: format!(
                "opcode {opcode:#04X} has no dialect entry (the original engine would wedge here too)"
            ),
        },
        VmError::Unimplemented { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: format!(
                "opcode {opcode:#04X} is known to the dialect but not yet implemented by this interpreter"
            ),
        },
        VmError::StringOperandTypeMismatch { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: "a string-mode operand was fed to a numeric-only opcode".to_string(),
        },
        VmError::UnresolvedOperand { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: "a destination/target operand had no resolvable raw word".to_string(),
        },
        VmError::MissingAsset { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: "a required .dax asset was missing".to_string(),
        },
        VmError::DivisionByZero { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description:
                "DIVIDE by zero (the original engine crashes uncaught here too)".to_string(),
        },
        VmError::StepWhilePending
        | VmError::ResumeWithoutPending
        | VmError::ReplyMismatch
        | VmError::Idle => HaltRecord {
            pc: 0,
            opcode: 0,
            description: format!("{err:?} (engine-orchestration bug, not a content issue)"),
        },
    }
}

/// Resident-asset bookkeeping (§1.3's `setBlocks[0..2]`): recorded so the
/// state is observable/serializable, never drawn (step 5's job).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResidentAssets {
    pub map_3d_block: Option<u8>,
    /// `(set, id)` per `LOAD FILES`/`LOAD PIECES`' up-to-3 walldef slots.
    pub walldefs: [Option<(u8, u8)>; 3],
    pub bigpic_block: Option<u8>,
}

/// One transcript-worthy content event (M2 step 8's transcript-mode task
/// deliverable): every PRINT/PRINTCLEAR text job the text system starts, and
/// every VM `Request`'s widget-opening label — the player-visible content a
/// DOSBox side-by-side transcript needs to be diffed against. Deliberately
/// coarser than the full `Effect`/`Widget` machinery (no pacing/pagination
/// detail) — this is a content log, not a replay format.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TranscriptEntry {
    /// PRINT (0x11) / PRINTCLEAR (0x12) — `clear_first` distinguishes them.
    Print { text: String, clear_first: bool },
    /// A VM `Request`'s widget-opening label (e.g. a HORIZONTAL MENU's joined
    /// option text, or `"delay"`/`"combat (stub)"` for the non-textual
    /// requests) — logged at the same point `widget_for_request` builds the
    /// `Widget`, so a transcript shows exactly what interaction the player
    /// was presented, not just prose text.
    Request(String),
}

/// Everything `ScriptMemory`/`EngineServices` needs beyond `EngineState`
/// (D-VM5's raw fallback store + log, the M2-slice named-but-inert Global
/// cells, resident-asset bookkeeping, the service-call log, and halt
/// diagnostics). Persists across ticks in [`crate::engine::Engine`].
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VmMemoryState {
    raw_words: HashMap<u16, u16>,
    raw_bytes: HashMap<u16, u8>,
    raw_strings: HashMap<u16, VmString>,
    pub unknown_log: UnknownAccessLog,
    /// A diagnostic trace, not save-relevant state — `RecordedCall` doesn't
    /// derive serde (it's `gbx-vm`'s own H4-oracle-trace type), and this log
    /// is meant for the demo's/inspector's read, not round-tripping.
    #[serde(skip)]
    pub calls: Vec<RecordedCall>,
    pub halts: Vec<HaltRecord>,
    /// The content log for `restrike walk --transcript` (M2 step 8) — not
    /// save-relevant state, like `calls`; a frontend drains it per tick via
    /// [`crate::engine::Engine::take_transcript`].
    #[serde(skip)]
    pub transcript: Vec<TranscriptEntry>,
    pub assets: ResidentAssets,
    /// `0x3DE` (`word_1EE76`): the CALL `0x3201` sound-variant selector —
    /// the one "dead-ish" Global cell with a real consumer
    /// ([`call_sound_variant`]).
    word_1ee76: u16,
    /// `0xB8`/`0xB9` (`word_1EE78`/`word_1EE7A`): write-only, no consumer
    /// found anywhere in the reference source (research §1.3) — stored
    /// verbatim anyway (cheap, and preserves round-trip if a consumer
    /// surfaces later), never read back through this facade.
    word_1ee78: u16,
    word_1ee7a: u16,
    /// `byte_1EE91`/`byte_1EE94` (redraw-dirty flags, `vm_SetMemoryValue`
    /// locations `0xBF68+0xF1`/`0xF7` and `0x4B00`-relative `0xFD`/`0xFE`)
    /// and `gbl.positionChanged` (`mapPosX`/`mapPosY`/`mapDirection`
    /// writes, `MovePositionForward`): the three flags a dedicated step 5
    /// research pass found consolidated at a single real gate, `CMD_Call`
    /// case `0xAE11` (`ovr003.cs:1844-1860`) — `if (spriteChanged ||
    /// displayPlayerSprite || byte_1EE91 || positionChanged || byte_1EE94)
    /// { RedrawView(); display_map_position_time(); <clear all five> }`.
    pub byte_1ee91: bool,
    pub byte_1ee94: bool,
    pub position_changed: bool,
    /// `gbl.spriteChanged`: set by `sub_30580` (encounter-visual dispatch)
    /// and `CMD_Picture` — same consolidated gate above.
    pub sprite_changed: bool,
    /// ★ `gbl.displayPlayerSprite` (`byte_1EE8F`, `Classes/Gbl.cs:409`) — the
    /// gate's **fifth** flag, and FD-34's: "a 3D approach sprite is overlaid
    /// on the dungeon view, so the view must be repainted to erase it". Set
    /// only by `sub_30580`'s `SPRIT` load (`ovr008.cs:237`); cleared by
    /// `CMD_SpriteOff` (`ovr003.cs:1714`), by `CMD_Picture`'s `0xFF` arm
    /// (`:351`) and by the gate itself (`:1861`).
    pub display_player_sprite: bool,
    /// `gbl.byte_1EE95` — "we are inside the ENCOUNTER MENU". Set at the head
    /// of `CMD_EncounterMenu` (`ovr003.cs:1245`), cleared at its tail
    /// (`:1536`), and read at exactly one place: `sub_30580`'s close-up gate
    /// (`ovr008.cs:257`). It is what keeps the 3D approach sprite on screen
    /// while the player picks COMBAT/WAIT/FLEE/ADVANCE, instead of swapping to
    /// the face the moment the distance reaches 0.
    pub byte_1ee95: bool,
    /// `gbl.byte_1EE96` — which `HeadBlockId` the close-up currently on screen
    /// was built from (`ovr008.cs:253,259`). Its only two sites are that
    /// change-detector; the original never initializes it, so it starts at 0,
    /// which is a *valid* head id — an encounter whose `HeadBlockId` is 0 and
    /// whose `encounter_flags[1]` is already set therefore skips its own
    /// close-up refresh. Replicated, not "fixed".
    pub byte_1ee96: u8,
    /// `gbl.can_draw_bigpic`: set at many command/init sites and
    /// unconditionally by `LoadPic`; read only by `RedrawView`'s own
    /// non-dungeon (wilderness/bigpic) branch, and cleared by every
    /// `RedrawView` (`ovr029.cs:46`, `crate::corridor::redraw_view`'s tail).
    /// The non-dungeon branch itself needs a wilderness `game_state` this
    /// engine does not reach yet — `CMD_Picture`'s own `blockId >= 0x78` arm
    /// is the BIGPIC path the scene-pictures slice draws.
    pub can_draw_bigpic: bool,
    /// `gbl.byte_1EE8D`: set by `CMD_Picture`'s plain arms and its `0xFF`
    /// clear, cleared by `set_and_draw_head_body` (`ovr008.cs:210`). Its one
    /// consumer is `CMD_HorizontalMenu`'s overlay decision,
    /// `useOverlay = spriteChanged && byte_1EE8D` (`ovr003.cs:730-738`) —
    /// which is what makes an encounter menu re-blit (and so *animate*) the
    /// picture behind it. This engine tracks the flag; driving the menu-loop
    /// animation from it is docketed, not done here.
    pub byte_1ee8d: bool,
    /// ★ `sub_30580`'s pending draw plans, one per dispatch, drained in order
    /// by [`gbx_vm::Effect::EncounterVisual`].
    ///
    /// **Not serialized**, on exactly [`crate::monster::PendingCombat`]'s
    /// rationale: a plan exists only between an instruction's execution and
    /// the same tick's presentation drain, and no save is ever taken there. A
    /// restored save deserializes it empty, which is the truthful state.
    #[serde(skip)]
    encounter_visual_plans: std::collections::VecDeque<crate::picture::EncounterVisualPlan>,
}

impl VmMemoryState {
    /// `CMD_Call` case `0xAE11`'s consolidated redraw gate
    /// (`ovr003.cs:1848-1860`): this session's `crate::corridor::redraw_view`
    /// runs unconditionally at every world-menu-visible point instead of
    /// gating on these flags (`shell.rs`'s `enter_world_menu` doc comment
    /// explains why that's a safe, documented simplification for a
    /// deterministic immediate-mode renderer) — but the flags themselves
    /// are still cleared here, at the same logical point the original
    /// clears them, so their *state* stays faithful even though nothing
    /// currently reads them to decide *whether* to redraw.
    pub(crate) fn clear_redraw_flags(&mut self) {
        self.byte_1ee91 = false;
        self.byte_1ee94 = false;
        self.position_changed = false;
        self.sprite_changed = false;
        // ★ `displayPlayerSprite` is deliberately NOT cleared here. The
        // original's gate clears all five (`ovr003.cs:1861`), but this method
        // is also `crate::corridor::redraw_view`'s tail — a documented
        // over-clear (see this method's doc comment) that `RedrawView` itself
        // does not perform (`ovr029.cs:10-49` touches only `can_draw_bigpic`).
        // For the other four the over-clear is harmless; for this one it is
        // not: `sub_30580`'s already-loaded arm calls `RedrawView()` and then
        // re-blits the sprite one band closer (`ovr008.cs:241-249`) with the
        // flag still standing, and clearing it there would leave the next
        // `CALL 0xAE11` unable to erase the sprite it just drew. The flag is
        // cleared at its own three sites instead: the gate
        // ([`EngineVmHost::redraw_view_gate`]), SPRITE OFF
        // ([`EngineVmHost::sprite_off`]) and `CMD_Picture`'s `0xFF` arm
        // ([`crate::picture::cmd_clear_picture`]).
    }
}

impl VmMemoryState {
    pub fn new() -> Self {
        // `vm_init_ecl` arms `byte_1EE91` at every ECL initialization
        // (`ovr008.cs:94`), and it has always run before the first script op
        // of a session — so a fresh engine starts with the redraw gate armed,
        // which is what makes a fresh block's first `CALL 0xAE11` repaint the
        // world (the amnesia intro's page-1 view, Bryan's 2026-08-08 DOSBox
        // side-by-side). `begin_chain` re-arms it per NEWECL, and
        // `import_original` re-arms it AFTER its `restore_windows` — the two
        // other `vm_init_ecl` sites.
        let mut vm = Self::default();
        vm.vm_init_ecl_redraw_flags();
        vm
    }

    /// `vm_init_ecl`'s redraw-flag half (`ovr008.cs:91,94`): `spriteChanged
    /// = false` then `byte_1EE91 = true`. Every engine-side `vm_init_ecl`
    /// analogue runs this — fresh boot ([`VmMemoryState::new`]), `CMD_NewECL`
    /// (`shell.rs`'s `begin_chain`, `ovr003.cs:491-492`), and the load path
    /// (`import.rs`, `sub_29758`'s `ovr003.cs:2278`).
    ///
    /// **Ordering matters and is the whole point of this being a named
    /// method:** the original loads the save into its windows and only THEN
    /// calls `vm_init_ecl` (`ovr003.cs:2262-2278` — `load_ecl_dax`, then
    /// `vm_init_ecl`, then `RunEclVm(ecl_initial_entryPoint)`), so on the
    /// import path the arm must follow [`VmMemoryState::restore_windows`],
    /// which otherwise writes the snapshot's own (false) flag bytes straight
    /// over it. A native `.rsav` restore is NOT a `vm_init_ecl` moment — it
    /// resumes a machine mid-execution — so it deliberately does not call
    /// this and keeps the flag value the save carried.
    pub(crate) fn vm_init_ecl_redraw_flags(&mut self) {
        self.sprite_changed = false;
        self.byte_1ee91 = true;
    }

    /// Writes one Area/Party window word straight into the raw backing store,
    /// **without** the `ScriptMemory` write hooks — the engine-side equivalent
    /// of assigning an `area_ptr`/`area2_ptr` struct field directly, which is
    /// what `vm_init_ecl` does throughout (and what makes its `inDungeon = 1`
    /// the asymmetry FD-37 flagged).
    pub(crate) fn poke_raw(&mut self, addr: u16, value: u16) {
        self.raw_words.insert(addr, value);
    }
}

/// ★ **`vm_init_ecl`** (`sub_301E8`, `ovr008.cs:89-132`) — the engine half,
/// complete, in one place (roll-credits D-S1d; **closes FD-37**).
///
/// Every ECL initialization runs this: fresh boot and original-save import
/// via `sub_29758`'s preamble (`ovr003.cs:2278`), and `CMD_NewECL`
/// (`ovr003.cs:491-492`). A native `.rsav` restore is *not* one of them — it
/// resumes a machine mid-execution, so it deliberately does not call this.
///
/// Cell by cell against the original's body, including what is deliberately
/// absent and why:
///
/// | line | cell | here |
/// |---|---|---|
/// | `:91,94` | `spriteChanged=false`, `byte_1EE91=true` | [`VmMemoryState::vm_init_ecl_redraw_flags`] |
/// | `:92-93` | `redrawPartySummary1/2=false` | no engine cell — our roster panel repaints every walk-loop tick unconditionally (`engine.rs`'s `tick`), so there is no dirty flag to clear |
/// | `:96-97` | `encounter_flags[0..1]=false` | ✔ on [`EngineState::encounter_flags`] (landed with the encounter cluster) |
/// | `:98` | `monster_icon_id=8` | ✔ on `PendingCombat` |
/// | `:99` | `ecl_offset=0x8000` | structural: `EclMachine` addresses its block from `ECL_BLOCK_BASE` |
/// | `:100` | `byte_1DA70=false` | no engine cell, no reader found |
/// | `:102` | `vmCallStack.Clear()` | structural: `EclMachine::load_block` starts with an empty call stack |
/// | `:104-107` | `compare_flags[0..5]=false` | structural: `load_block` starts `flags: [false; 6]` — the `gbx-vm` seam FD-37 asked for turns out to be unnecessary, because every site that runs `vm_init_ecl` also rebuilds the machine |
/// | `:109` | `HeadBlockId=0xFF` | ✔ |
/// | `:111-112` | `rest_incounter_period/percentage=0` | ✔ raw Party cells |
/// | `:113` | `can_cast_spells=false` | ✔ (a cell no script can address — see [`EngineState::can_cast_spells`]) |
/// | `:115-124` | five `vm_LoadCmdSets` into the vector table | structural: `load_block`/`reinit` re-read the block header |
/// | `:126` | `inDungeon=1`, **direct** | ✔ raw cell only, `game_state` untouched |
/// | `:128-131` | the two table restores, `reload_ecl_and_pictures`-gated | ✔ |
///
/// **`:126` is the asymmetry, and it is load-bearing.** The original assigns
/// `gbl.area_ptr.inDungeon = 1` on the struct, bypassing `vm_SetMemoryValue`'s
/// Area hook (`ovr008.cs:700-708`) — the hook is the only thing that turns an
/// `inDungeon` write into a `game_state`/`last_game_state` update. So after
/// every block entry the *cell* says "dungeon" while `game_state` keeps
/// whatever the load computed. That is exactly what lets a party walk out of
/// the overland (`ECL1#80 @0x8014` sets the cell to 0, `game_state` becomes
/// `WildernessMap`) and back into a dungeon block whose `LOAD FILES` gate
/// (`ovr003.cs:519-521`, `area_ptr.inDungeon != 0`) then still loads the 3D
/// map. Reading the cell from `game_state`, as this engine used to, would have
/// refused that load.
///
/// **`:128-131`** restores two script scratch tables — `field_200` (33 words)
/// and `field_6F2..704` (10 words) — but only when `reload_ecl_and_pictures`
/// is false. `loadSaveGame` sets that flag (`ovr017.cs:983`) and the walk loop
/// clears it after the first world-menu entry (`ovr003.cs:2313`), so the one
/// initialization that *skips* the wipe is the one right after a save load:
/// the mechanism by which a loaded game's per-block scratch survives, while
/// every NEWECL in normal play starts its block with both tables zeroed.
pub(crate) fn vm_init_ecl(state: &mut EngineState, vm: &mut VmMemoryState) {
    vm.vm_init_ecl_redraw_flags(); // :91, :94
    state.encounter_flags = [false; 2]; // :96-97
    state.pending_combat.monster_icon_id = 8; // :98
    state.head_block_id = 0xFF; // :109
    vm.poke_raw(REST_INCOUNTER_PERIOD_ADDR, 0); // :111
    vm.poke_raw(REST_INCOUNTER_PERCENTAGE_ADDR, 0); // :112
    state.can_cast_spells = false; // :113
    vm.poke_raw(IN_DUNGEON_ADDR, 1); // :126 — DIRECT, no `game_state` hook

    // :128-131
    if !state.reload_ecl_and_pictures {
        for i in 0..FIELD_200_LEN {
            vm.poke_raw(FIELD_200_BASE + i, 0);
        }
        for i in 0..FIELD_6F2_LEN {
            vm.poke_raw(FIELD_6F2_BASE + i, 0);
        }
    }
}

impl VmMemoryState {
    /// The raw fallback word store's current value at `addr` (D-VM5's
    /// round-trip guarantee for any cell without a named cell above) — a
    /// read-only seam for tests and the eventual inspector (D-UI8), not
    /// used by `ScriptMemory` dispatch itself (which owns its own
    /// insert/lookup calls directly).
    pub fn raw_word(&self, addr: u16) -> Option<u16> {
        self.raw_words.get(&addr).copied()
    }

    /// Seeds a raw window cell — the seam a drive that wants to start
    /// mid-script uses to put the script's own bookkeeping where the
    /// instructions upstream of it would have left it.
    pub fn set_raw_word(&mut self, addr: u16, value: u16) {
        self.raw_words.insert(addr, value);
    }

    /// ★ `gbl.area_ptr.inDungeon`, read from the RAW cell exactly as
    /// `field_6A00_Get`'s `case 0x1CC` does (`Classes/Area1.cs:495-496`) —
    /// **not** derived from `game_state`. FD-37 established why: `vm_init_ecl`
    /// writes the struct field directly (`ovr008.cs:126`), bypassing the hook
    /// that maintains `game_state`, so the two legitimately disagree and every
    /// consumer must read the one the original reads. `sub_30580`'s
    /// sprite-load gate (`ovr008.cs:233`) is such a consumer.
    pub fn in_dungeon(&self) -> bool {
        self.raw_word(IN_DUNGEON_ADDR).unwrap_or(0) != 0
    }

    /// ★ `gbl.area_ptr.block_area_view` ([`BLOCK_AREA_VIEW_ADDR`]) — FD-41.
    /// Read live at both consumers (`RedrawView`'s force-clear and the world
    /// menu's `'A'`), because a block can `SAVE` into it at any time.
    pub fn block_area_view(&self) -> u16 {
        self.raw_word(BLOCK_AREA_VIEW_ADDR).unwrap_or(0)
    }

    /// ★ `gbl.area_ptr.current_city` ([`CURRENT_CITY_ADDR`]) — the
    /// [`crate::mapcursor`] table index, and `SetupWildernessFloor`'s own
    /// terrain seed (`ovr011.cs:750`).
    pub fn current_city(&self) -> u8 {
        self.raw_word(CURRENT_CITY_ADDR).unwrap_or(0) as u8
    }

    /// Queues one [`crate::picture::EncounterVisualPlan`] (execution time).
    pub(crate) fn push_encounter_visual_plan(&mut self, plan: crate::picture::EncounterVisualPlan) {
        self.encounter_visual_plans.push_back(plan);
    }

    /// Takes the oldest queued plan (presentation time). `None` means the
    /// effect arrived without a matching dispatch — impossible by
    /// construction, and a no-op rather than a panic if it ever happens.
    pub(crate) fn pop_encounter_visual_plan(
        &mut self,
    ) -> Option<crate::picture::EncounterVisualPlan> {
        self.encounter_visual_plans.pop_front()
    }

    /// The raw fallback byte store's current value at `addr` — [`raw_word`](Self::raw_word)'s
    /// counterpart for `ScriptMemory::read_byte`/`write_byte` traffic, added
    /// for `tools/inspect`'s ScriptMemory watch pane (D-UI8) so it can show
    /// the raw-store contents across all three access widths, not just
    /// words.
    pub fn raw_byte(&self, addr: u16) -> Option<u8> {
        self.raw_bytes.get(&addr).copied()
    }

    /// The raw fallback string store's current value at `addr` —
    /// [`raw_word`](Self::raw_word)'s counterpart for
    /// `ScriptMemory::read_string`/`write_string` traffic.
    pub fn raw_string(&self, addr: u16) -> Option<&VmString> {
        self.raw_strings.get(&addr)
    }
}

/// A deterministic, `.rsav`-storable projection of [`VmMemoryState`]
/// (`docs/design/save-formats.md` D-SAVE3, task deliverable 3): the Area/
/// Party/Table/Global window backings (named cells + the raw fallback
/// store) plus resident-asset ids. `BTreeMap`s, never `HashMap`s (D-SAVE1's
/// CI-enforced determinism invariant) — `VmMemoryState`'s own live
/// `HashMap`s stay as they are (no risk to the already-tested M2 dispatch
/// code); this type only exists at the save/restore boundary.
///
/// Deliberately excludes what D-SAVE3 calls diagnostic, not state: the
/// unknown-access log, the service-call log, and past halt records. A
/// restored `VmMemoryState` starts those three empty, exactly like a fresh
/// [`VmMemoryState::new`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowsSnapshot {
    pub raw_words: std::collections::BTreeMap<u16, u16>,
    pub raw_bytes: std::collections::BTreeMap<u16, u8>,
    pub raw_strings: std::collections::BTreeMap<u16, VmString>,
    pub assets: ResidentAssets,
    pub word_1ee76: u16,
    pub word_1ee78: u16,
    pub word_1ee7a: u16,
    pub byte_1ee91: bool,
    pub byte_1ee94: bool,
    pub position_changed: bool,
    pub sprite_changed: bool,
    pub can_draw_bigpic: bool,
}

impl VmMemoryState {
    /// Extracts a deterministic [`WindowsSnapshot`] — the `.rsav` payload's
    /// `windows` field (D-SAVE3).
    pub fn snapshot(&self) -> WindowsSnapshot {
        WindowsSnapshot {
            raw_words: self.raw_words.iter().map(|(&k, &v)| (k, v)).collect(),
            raw_bytes: self.raw_bytes.iter().map(|(&k, &v)| (k, v)).collect(),
            raw_strings: self
                .raw_strings
                .iter()
                .map(|(&k, v)| (k, v.clone()))
                .collect(),
            assets: self.assets.clone(),
            word_1ee76: self.word_1ee76,
            word_1ee78: self.word_1ee78,
            word_1ee7a: self.word_1ee7a,
            byte_1ee91: self.byte_1ee91,
            byte_1ee94: self.byte_1ee94,
            position_changed: self.position_changed,
            sprite_changed: self.sprite_changed,
            can_draw_bigpic: self.can_draw_bigpic,
        }
    }

    /// Populates raw window words/bytes/strings **and** the named-but-inert
    /// Global cells directly from a snapshot (used by both `.rsav` restore,
    /// D-SAVE3, and original-save import, D-SAVE7 — import writes the raw
    /// blob first via this same path, satisfying "named cells are then read
    /// through the same facade" for every window that already has one).
    /// `unknown_log`/`calls`/`halts`/`transcript` are left at their current
    /// (fresh) values — this never clears diagnostics that don't yet exist
    /// on a freshly-built `VmMemoryState`.
    pub fn restore_windows(&mut self, snapshot: WindowsSnapshot) {
        self.raw_words = snapshot.raw_words.into_iter().collect();
        self.raw_bytes = snapshot.raw_bytes.into_iter().collect();
        self.raw_strings = snapshot.raw_strings.into_iter().collect();
        self.assets = snapshot.assets;
        self.word_1ee76 = snapshot.word_1ee76;
        self.word_1ee78 = snapshot.word_1ee78;
        self.word_1ee7a = snapshot.word_1ee7a;
        self.byte_1ee91 = snapshot.byte_1ee91;
        self.byte_1ee94 = snapshot.byte_1ee94;
        self.position_changed = snapshot.position_changed;
        self.sprite_changed = snapshot.sprite_changed;
        self.can_draw_bigpic = snapshot.can_draw_bigpic;
    }
}

/// The real `VmHost`: borrows engine state fresh each pump (`shell.rs`
/// constructs one per `EclMachine::step`/`resume` call).
pub struct EngineVmHost<'a> {
    pub state: &'a mut EngineState,
    pub vm: &'a mut VmMemoryState,
    pub geo: &'a mut GeoBlock,
    pub party: &'a mut dyn crate::movement::PartyPredicates,
    /// ★ The real roster (`gbl.TeamList`), needed by the services that walk
    /// the party member-by-member: `CMD_PartyStrength`'s power sum
    /// (`ovr003.cs:776-805`) and ENCOUNTER MENU's `calc_group_movement`
    /// (`ovr008.cs:1370-1398`). Distinct from [`Self::party`], the M2
    /// door-predicate abstraction.
    pub roster: &'a mut crate::party::Party,
    pub rng: &'a mut crate::rng::EngineRng,
    pub sounds: &'a mut Vec<crate::shell::SoundEvent>,
    /// `load_walldef`'s real data source (step 5, task deliverable 1):
    /// `"WALLDEF{game_area}.DAX"`/`"8X8D{game_area}.DAX"`, the same
    /// `game_area`-embeds-the-filename convention `load_ecl_dax`/boot's
    /// `Load8x8D` already use.
    pub data: &'a GameData,
    pub symbols: &'a mut SymbolSets,
}

impl EngineVmHost<'_> {
    /// The live `gbl.game_area` (roll-credits D-S1b) — read off the state at
    /// each service call, never snapshotted at host construction: within a
    /// single VM step a `SAVE <n>, 0x7F12` can move it, and every subsequent
    /// `{area}`-keyed load in the original reads the new value.
    fn game_area(&self) -> u8 {
        self.state.game_area
    }
}

impl EngineVmHost<'_> {
    fn wall_square(&self) -> &gbx_formats::geo::Square {
        self.geo
            .square(self.state.pos.0 as usize, self.state.pos.1 as usize)
    }

    /// `mapWallType` (`getMap_wall_type`): the facing edge's raw wall-type
    /// nibble (`0` = no wall).
    fn wall_type_value(&self) -> u16 {
        let sq = self.wall_square();
        let v = match self.state.facing {
            Facing::North => sq.wall_north,
            Facing::East => sq.wall_east,
            Facing::South => sq.wall_south,
            Facing::West => sq.wall_west,
        };
        v as u16
    }

    /// `mapWallRoof` (`get_wall_x2`): the current square's reconstructed
    /// `x2` byte (`indoor<<7 | floor_flag<<6 | low7`, `gbx_formats::geo`'s
    /// own decomposition, undone here).
    fn wall_roof_value(&self) -> u16 {
        let sq = self.wall_square();
        let mut x2 = sq.low7 & 0x7F;
        if sq.indoor {
            x2 |= 0x80;
        }
        if sq.floor_flag {
            x2 |= 0x40;
        }
        x2 as u16
    }
}

impl ScriptMemory for EngineVmHost<'_> {
    fn read(&mut self, addr: u16, origin: Origin) -> u16 {
        if AREA_WINDOW.contains(&addr) {
            return self.read_area(addr, origin);
        }
        if addr == HEAD_BLOCK_ID_ADDR {
            return self.state.head_block_id as u16;
        }
        if addr == MAX_ENCOUNTER_DISTANCE_ADDR {
            return self.state.max_encounter_distance;
        }
        if addr == ENCOUNTER_DISTANCE_ADDR {
            return self.state.encounter_distance as u16;
        }
        if addr == GAME_AREA_ADDR {
            return self.state.game_area as u16;
        }
        if addr == ENTER_TEMPLE_ADDR {
            return self.state.enter_temple;
        }
        if addr == ENTER_SHOP_ADDR {
            return self.state.enter_shop;
        }
        if addr == SELECTED_PLAYER_STATE_ADDR {
            return self.read_selected_player_state();
        }
        if TABLE_WINDOW.contains(&addr) || PARTY_WINDOW.contains(&addr) {
            self.vm.unknown_log.record(addr, AccessKind::Read, origin);
            return self.vm.raw_words.get(&addr).copied().unwrap_or(0);
        }
        self.read_global(addr, origin)
    }

    fn write(&mut self, addr: u16, value: u16, origin: Origin) {
        if AREA_WINDOW.contains(&addr) {
            return self.write_area(addr, value, origin);
        }
        if addr == HEAD_BLOCK_ID_ADDR {
            self.state.head_block_id = value as u8;
            return;
        }
        if addr == MAX_ENCOUNTER_DISTANCE_ADDR {
            self.state.max_encounter_distance = value;
            return;
        }
        if addr == ENCOUNTER_DISTANCE_ADDR {
            // `ushort` in the original; ours is the `u8` the ray and every
            // consumer actually use (0..2), so a wild script write truncates
            // — noted rather than widened, since no shipped script writes it.
            self.state.encounter_distance = value as u8;
            return;
        }
        if addr == GAME_AREA_ADDR {
            self.state.set_game_area(value as u8);
            return;
        }
        if addr == ENTER_TEMPLE_ADDR {
            self.state.enter_temple = value;
            return;
        }
        if addr == ENTER_SHOP_ADDR {
            self.state.enter_shop = value;
            return;
        }
        if TABLE_WINDOW.contains(&addr) || PARTY_WINDOW.contains(&addr) {
            self.vm.unknown_log.record(addr, AccessKind::Write, origin);
            self.vm.raw_words.insert(addr, value);
            return;
        }
        self.write_global(addr, value, origin);
    }

    fn read_byte(&mut self, addr: u16, origin: Origin) -> u8 {
        self.vm
            .unknown_log
            .record(addr, AccessKind::ReadByte, origin);
        self.vm.raw_bytes.get(&addr).copied().unwrap_or(0)
    }

    fn write_byte(&mut self, addr: u16, value: u8, origin: Origin) {
        self.vm
            .unknown_log
            .record(addr, AccessKind::WriteByte, origin);
        self.vm.raw_bytes.insert(addr, value);
    }

    fn read_string(&mut self, addr: u16, origin: Origin) -> VmString {
        self.vm
            .unknown_log
            .record(addr, AccessKind::ReadString, origin);
        self.vm.raw_strings.get(&addr).cloned().unwrap_or_default()
    }

    fn write_string(&mut self, addr: u16, s: &VmString, origin: Origin) {
        self.vm
            .unknown_log
            .record(addr, AccessKind::WriteString, origin);
        self.vm.raw_strings.insert(addr, s.clone());
    }
}

impl EngineVmHost<'_> {
    /// [`SELECTED_PLAYER_STATE_ADDR`]'s read, `get_player_values`'
    /// `arg_4 == 0x100` case verbatim (`ovr008.cs:424-441`) — including the
    /// **read-clears** of `player_not_found`.
    fn read_selected_player_state(&mut self) -> u16 {
        let mut value = match self.roster.members.get(self.state.selected_player as usize) {
            Some(member) if member.status.in_combat => 1,
            _ => 0x80,
        };
        if self.state.player_not_found {
            value = 0;
        }
        self.state.player_not_found = false;
        value
    }

    /// The Area window (`0x4B00..=0x4EFF`): the ECL clock cluster + the two
    /// named flags, everything else raw+logged (research §1.5).
    fn read_area(&mut self, addr: u16, origin: Origin) -> u16 {
        // FD-31 RESOLVED: under the Area window's confirmed
        // `addr = 0x4B00 + DataOffset/2` mapping, the seven clock words
        // (`Area1.cs` DataOffsets `0x18C..=0x198`, stride 2) live at seven
        // CONSECUTIVE addresses `0x4BC6..=0x4BCC`. The stride-2 addressing
        // this replaced answered only even addresses — and the real
        // Tilverton entry script reads `0x4BC9`, the HOUR, which fell to the
        // raw store (0 on a fresh boot, the save's stale byte after an
        // import): time-of-day gates never saw the live clock.
        if (CLOCK_BASE..=CLOCK_BASE + 6).contains(&addr) {
            let idx = (addr - CLOCK_BASE) as usize;
            return self.state.clock.raw_clock_words()[idx];
        }
        if addr == IN_DUNGEON_ADDR {
            // ★ The CELL, not `game_state` (FD-37): the original's read side is
            // `field_6A00_Get`'s `case 0x1CC: return inDungeon`
            // (`Classes/Area1.cs:495-496`), the raw struct field. `vm_init_ecl`
            // writes that field directly at every block entry
            // (`ovr008.cs:126`) without going through the hook that maintains
            // `game_state`, so the two legitimately disagree — and `LOAD
            // FILES`' 3D-map gate reads the cell, which is what lets a block
            // entered from the wilderness still load its dungeon map.
            return self.vm.raw_words.get(&addr).copied().unwrap_or(0);
        }
        if addr == LAST_XPOS_ADDR {
            return self.state.last_pos.0 as u16;
        }
        if addr == LAST_YPOS_ADDR {
            return self.state.last_pos.1 as u16;
        }
        if addr == LAST_ECL_BLOCK_ID_ADDR {
            return self.state.last_ecl_block_id as u16;
        }
        self.vm.unknown_log.record(addr, AccessKind::Read, origin);
        self.vm.raw_words.get(&addr).copied().unwrap_or(0)
    }

    fn write_area(&mut self, addr: u16, value: u16, origin: Origin) {
        if addr == IN_DUNGEON_ADDR {
            // The hook's own guard is `gbl.area_ptr.inDungeon != value`
            // (`ovr008.cs:702`) — the CELL's previous value, not `game_state`.
            // The two can differ (see the read arm above), and it is the cell
            // the original compares.
            let cur = self.vm.raw_words.get(&addr).copied().unwrap_or(0);
            if cur != value {
                self.state.last_game_state = self.state.game_state;
                self.state.game_state = if value == 0 {
                    GameState::WildernessMap
                } else {
                    GameState::DungeonMap
                };
            }
            self.vm.raw_words.insert(addr, value);
            return;
        }
        if addr == LAST_XPOS_ADDR {
            self.state.last_pos.0 = value as u8;
            return;
        }
        if addr == LAST_YPOS_ADDR {
            self.state.last_pos.1 = value as u8;
            return;
        }
        if addr == LAST_ECL_BLOCK_ID_ADDR {
            self.state.last_ecl_block_id = value as u8;
            return;
        }
        if FORCE_REDRAW_ADDRS.contains(&addr) {
            self.vm.byte_1ee94 = true;
            self.vm.raw_words.insert(addr, value);
            return;
        }
        self.vm.unknown_log.record(addr, AccessKind::Write, origin);
        self.vm.raw_words.insert(addr, value);
    }

    /// The Global window's named cells (research §1.1/§1.2) — everything
    /// unmatched round-trips through the raw store + unknown-access log
    /// (D-VM5's deliberate design choice: scripts still get back what they
    /// stash, even at an address the original's own switch silently drops —
    /// see `0x2CB`'s docket note below).
    fn read_global(&mut self, addr: u16, origin: Origin) -> u16 {
        match addr {
            // Confirmed dead cells: write no-op, field never assigned
            // elsewhere in the reference source — always reads 0.
            0x00B1 | 0x00FB | 0x00FC => 0,
            // The raw (unhalved) facing read, distinct from 0xC04D below.
            0x033D => self.state.facing.raw_code() as u16,
            0x035F => 0, // stub case in the original, no assignment
            0xC04B => self.state.pos.0 as u16,
            0xC04C => self.state.pos.1 as u16,
            0xC04D => (self.state.facing.raw_code() / 2) as u16,
            0xC04E => self.wall_type_value(),
            0xC04F => self.wall_roof_value(),
            0xC059 => 0, // stub read case (write sets byte_1EE91; read here never reflects it)
            _ => {
                self.vm.unknown_log.record(addr, AccessKind::Read, origin);
                self.vm.raw_words.get(&addr).copied().unwrap_or(0)
            }
        }
    }

    fn write_global(&mut self, addr: u16, value: u16, origin: Origin) {
        match addr {
            // Confirmed no-op writes (research §1.1) — dropped, not stored.
            0x00B1 | 0x00FB | 0x00FC => {}
            0x03DE => self.vm.word_1ee76 = value,
            0x00B8 => self.vm.word_1ee78 = value,
            0x00B9 => self.vm.word_1ee7a = value,
            0xC04B => {
                self.state.pos.0 = value as u8;
                self.vm.position_changed = true;
            }
            0xC04C => {
                self.state.pos.1 = value as u8;
                self.vm.position_changed = true;
            }
            0xC04D => {
                // The original's do-while normalizes any input to `%4`
                // before expanding to the raw facing code (research §1.1).
                let normalized = (value % 4) as u8;
                self.state.facing = Facing::from_raw(normalized * 2);
                self.vm.position_changed = true;
            }
            0xC059 | 0xC05F => self.vm.byte_1ee91 = true,
            // 0xC04E/0xC04F (wall type/roof) are read-only through this
            // dispatch in the original (no write case) — silently dropped,
            // matching that exactly (not even round-tripped via raw store).
            0xC04E | 0xC04F => {}
            // `0x2CB` (SURPRISE's write target, `CMD_Surprise`,
            // `ovr003.cs:967`): this session's research found no matching
            // case in either the original's read or write switch — the
            // write appears to be a genuine no-op in the reference source
            // (flagged, not resolved; see design doc docket). Falling
            // through to the raw store here is *more* functional than the
            // original (round-trips the value instead of dropping it),
            // which is D-VM5's own explicit "unknown cells still
            // round-trip" design choice — a deliberate, documented
            // divergence in the safe direction, not a fidelity miss.
            _ => {
                self.vm.unknown_log.record(addr, AccessKind::Write, origin);
                self.vm.raw_words.insert(addr, value);
            }
        }
    }
}

// --- EngineServices (D-VM4's placement rule; M2 subset real, rest logged M3/M4 stubs) ---

impl gbx_vm::EngineServices for EngineVmHost<'_> {
    /// ★ `CMD_LoadCharacter` in full (`sub_262E9`, `ovr003:02E9-03C8`) —
    /// see the trait's own doc comment for the binary transcription and for
    /// coab's slot-0 mis-transcription.
    fn retarget_selected_player(&mut self, index: u8) {
        self.vm
            .calls
            .push(RecordedCall::RetargetSelectedPlayer { index });
        self.state.restore_player_ptr = true; // `:02EF`
        let high_bit = index & 0x80 != 0; // `:0318-031D`
        let slot = (index & 0x7F) as usize; // `:0320-0325`

        // `:0328-0354` — the cursor starts at the list HEAD and walks `slot`
        // links, stopping early at a null. Index 0 therefore selects member 0.
        if slot < self.roster.members.len() {
            self.state.selected_player = slot as u8; // `:035E-0367`
            self.state.player_not_found = false; // `:036B`
        } else {
            self.state.player_not_found = true; // `:0372`
        }

        // `:0377-03C0` — the dump arm. Both summary flags must be armed;
        // `LastSelectedPlayer == SelectedPlayer` disarms the EXIT-time restore
        // (there would be nothing left to restore to).
        if high_bit && self.state.redraw_party_summary == [true, true] {
            if self.state.last_selected_player == self.state.selected_player {
                self.state.restore_player_ptr = false; // `:039E`
            }
            <Self as gbx_vm::EngineServices>::free_current_player(self, true, false);
            self.state.redraw_party_summary = [false, false]; // `:03BB-03C0`
        }
    }

    /// `CMD_Dump` (`ovr003.cs:2007-2018`): free the selected member, then
    /// mirror the new selection into `LastSelectedPlayer` — the mirror is what
    /// stops a later EXIT restoring a member that no longer exists.
    fn dump_selected_player(&mut self) {
        self.vm.calls.push(RecordedCall::DumpSelectedPlayer);
        <Self as gbx_vm::EngineServices>::free_current_player(self, true, false);
        self.state.last_selected_player = self.state.selected_player;
    }

    /// `area2_ptr.party_size`. Seeded at import/boot from the roster and
    /// maintained by `FreeCurrentPlayer`; a zero here means the state predates
    /// the field, so the roster length stands in.
    fn party_size(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::PartySize);
        if self.state.party_size == 0 {
            self.roster.members.len() as u8
        } else {
            self.state.party_size
        }
    }

    fn team_size(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::TeamSize);
        self.roster.members.len() as u8
    }

    fn selected_player(&mut self) -> PlayerId {
        self.vm.calls.push(RecordedCall::SelectedPlayer);
        PlayerId(self.state.selected_player)
    }

    fn set_selected_player(&mut self, player: PlayerId) {
        self.vm
            .calls
            .push(RecordedCall::SetSelectedPlayer { player });
        self.state.selected_player = player.0;
    }

    /// DAMAGE's closing scan (`ovr003:2C04-2C43`) — and the *only* condition
    /// is `in_combat`, not health status: a member knocked unconscious is
    /// already `in_combat == false` by the time this runs, which is what makes
    /// the test work at all.
    fn party_wipe_check(&mut self) -> bool {
        self.vm.calls.push(RecordedCall::PartyWipeCheck);
        let killed = !self.roster.members.iter().any(|m| m.status.in_combat);
        self.state.party_killed = killed;
        if killed {
            self.state.wipe_cause = crate::shell::WipeCause::EclDamage;
        }
        killed
    }

    /// ★ `FreeCurrentPlayer` (`ovr018.cs:1580-1607`): remove `SelectedPlayer`
    /// from the roster, drop `party_size` unless the caller asked to leave it,
    /// and re-seat the selection **one slot back** (`index > 0 ? index - 1 :
    /// 0`, `:1600`) — not on the member that slid into the vacated slot. An
    /// emptied roster leaves the selection at 0, matching the original's
    /// `null` as closely as an index can.
    ///
    /// `ReleaseCombatIcon` is combat-art bookkeeping our own icon loader does
    /// not need (it takes no slot table out of combat).
    fn free_current_player(&mut self, free_icon: bool, leave_party_size: bool) -> PlayerId {
        self.vm.calls.push(RecordedCall::FreeCurrentPlayer {
            free_icon,
            leave_party_size,
        });
        let index = self.state.selected_player as usize;
        if index >= self.roster.members.len() {
            return PlayerId(self.state.selected_player);
        }
        self.roster.members.remove(index);
        if !leave_party_size {
            self.state.party_size = self.state.party_size.saturating_sub(1);
        }
        let next = index.saturating_sub(1);
        self.state.selected_player = if self.roster.members.is_empty() {
            0
        } else {
            next as u8
        };
        PlayerId(self.state.selected_player)
    }

    /// ★ `CMD_PartyStrength`'s power sum (`ovr003.cs:776-805`), transcribed
    /// term for term over `gbl.TeamList`:
    ///
    /// ```text
    /// armor_class = ac      > 60 ? ac      - 60 : 0
    /// hit_bonus   = hitBonus > 39 ? hitBonus - 39 : 0
    /// power += (byte)((cleric*4 + hp + armor_class*5 + hit_bonus*5 + magic*8) / 10)
    /// ```
    ///
    /// Three things the C# says out loud and this must not smooth over:
    /// `player.ac` is the RAW stored byte (`Player.cs:597`; display AC is
    /// `0x3C - ac`), so `> 60` means "better than AC -1" rather than
    /// "worse than AC 60"; the per-member term is cast to `byte` *before*
    /// being added, so a single monstrous member truncates rather than
    /// saturating; and `power_value` is itself a `byte`, so the running total
    /// wraps. Draw-free.
    fn party_strength(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::PartyStrength);
        let mut power: u8 = 0;
        for member in self.roster.members.iter() {
            let hit_points = member.hit_point_current as i32;
            // `player.ac` is a `byte` in the original; ours is stored `i8`.
            let ac = member.combat.ac as u8 as i32;
            let hit_bonus = member.combat.thac0_current as i32;
            let magic_power = member.skill_level(crate::party::SKILL_MAGIC_USER);
            let cleric_power = member.skill_level(crate::party::SKILL_CLERIC);

            let armor_class = if ac > 60 { ac - 60 } else { 0 };
            let hit_bonus = if hit_bonus > 39 { hit_bonus - 39 } else { 0 };

            let term =
                (cleric_power * 4 + hit_points + armor_class * 5 + hit_bonus * 5 + magic_power * 8)
                    / 10;
            power = power.wrapping_add(term as u8);
        }
        power
    }

    fn check_party(&mut self, query: u16) -> u16 {
        self.vm.calls.push(RecordedCall::CheckParty { query });
        0
    }

    /// `CMD_FindItem`'s scan (`ovr003.cs:1573-1584`): every roster member,
    /// every item, first match wins. The item's `type` is byte `0x2E` of its
    /// `.swg` record (`Classes/Item.cs:118`).
    fn party_has_item(&mut self, item_type: u8) -> bool {
        self.vm.calls.push(RecordedCall::PartyHasItem { item_type });
        self.roster.members.iter().any(|m| {
            m.items
                .iter()
                .any(|item| gbx_formats::save_orig::item_type(item) == item_type)
        })
    }

    /// `CMD_FindSpecial` (`ovr003.cs:2021-2039`) — `SelectedPlayer` only, not
    /// the whole party (unlike FIND ITEM, which is the pair's asymmetry).
    fn find_special(&mut self, affect_type: u8) -> bool {
        self.vm
            .calls
            .push(RecordedCall::FindSpecial { affect_type });
        let selected = self.state.selected_player as usize;
        self.roster
            .members
            .get(selected)
            .is_some_and(|m| m.has_affect(affect_type))
    }

    /// `CMD_DestroyItems` (`ovr003.cs:2042-2055`): `RemoveAll` by type across
    /// the whole roster, then `reclac_player_values` per member — which here
    /// means rebuilding [`crate::party::Character::readied_items`], since the
    /// removal renumbers every index after it and a stale set would ready the
    /// wrong item.
    fn destroy_items(&mut self, item_type: u8) {
        self.vm.calls.push(RecordedCall::DestroyItems { item_type });
        for member in self.roster.members.iter_mut() {
            member
                .items
                .retain(|item| gbx_formats::save_orig::item_type(item) != item_type);
            member.readied_items = member
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| gbx_formats::save_orig::item_readied(item))
                .map(|(i, _)| i)
                .collect();
        }
    }

    /// ★ ROB's money half (`sub_31DEF`, `ovr008:1DEF-1F19`) — all seven
    /// denominations scaled and truncated; see [`crate::party::Money::scale_all`]
    /// for the coab correction. Draw-free.
    fn rob_money(&mut self, player: PlayerId, scale: f64) {
        self.vm.calls.push(RecordedCall::RobMoney { player, scale });
        if let Some(member) = self.roster.members.get_mut(player.0 as usize) {
            member.money.scale_all(scale);
        }
    }

    /// ★ ROB's item half (`sub_31F1C`, `ovr008:1F1C-1FCF`).
    ///
    /// Walks the item list front to back. Per item: the weight ladder
    /// reduces `chance` **in place** (`> 255` → `-90`, else `> 24` → `-50`,
    /// clamped at 0 — and the write-back at `ovr008:1F59`/`:1F7C` is what
    /// makes the reduction cumulative down the list), then one
    /// `roll_dice(100, 1)` is drawn **whatever the chance is** and the item
    /// is lost on `roll <= chance`. The weight test is the unsigned word
    /// compare the binary uses (`jbe`).
    fn rob_items(&mut self, player: PlayerId, chance: u8) {
        self.vm
            .calls
            .push(RecordedCall::RobItems { player, chance });
        let Some(member) = self.roster.members.get(player.0 as usize) else {
            return;
        };
        let weights: Vec<u16> = member
            .items
            .iter()
            .map(|it| gbx_formats::save_orig::item_weight(it) as u16)
            .collect();
        let mut chance = chance;
        let mut lost = Vec::new();
        for (index, weight) in weights.iter().enumerate() {
            // `cmp [bp+arg_4], 90 / jbe` — at exactly 90 the original stores
            // 0, which is what `saturating_sub` gives too.
            if *weight > 255 {
                chance = chance.saturating_sub(90);
            } else if *weight > 24 {
                chance = chance.saturating_sub(50);
            }
            // `roll_dice(100, 1)` — drawn for every item, before the compare.
            let roll = 1 + self.rng.random(100) as u8;
            if roll <= chance {
                lost.push(index);
            }
        }
        let Some(member) = self.roster.members.get_mut(player.0 as usize) else {
            return;
        };
        // `lose_item` per hit, back to front so the surviving indices stay
        // valid (the original walks a linked list and cannot be confused).
        for index in lost.into_iter().rev() {
            crate::items::lose_item(member, index);
        }
    }

    fn party_surprise_check(&mut self) -> (u8, u8) {
        self.vm.calls.push(RecordedCall::PartySurpriseCheck);
        (0, 0)
    }

    /// `CMD_LoadMonster` (`ovr003.cs:238`) → `load_mob` (`ovr017.cs:824`):
    /// decode block `monster_id` of `MON{game_area}CHA.DAX` as a full 0x1A6
    /// `Player` record (`new Player(data, 0)`) and accumulate it into the
    /// pending-combat roster `num_copies` times (M4 combat #6). The `CPIC`
    /// `icon_block_id` rides along onto every copy (M6 slice 6 — the shell's
    /// combat host loads the monster's combat icon from it); the SPC/ITM
    /// companion files (`load_mob`'s innate affects + carried items) are
    /// effect state deferred past this slice. A missing/undecodable `.dax` is coab's hard
    /// `print_and_exit` (`ovr017.cs:836`); we surface it as `MissingData` →
    /// the interpreter's `VmError::MissingAsset` non-aborting analogue
    /// (`machine.rs:92`).
    fn load_monster(
        &mut self,
        monster_id: u8,
        num_copies: u8,
        icon_block_id: u8,
    ) -> Result<MonsterHandle, MissingData> {
        self.vm.calls.push(RecordedCall::LoadMonster {
            monster_id,
            num_copies,
            icon_block_id,
        });
        let file = gbx_formats::monster::monster_filename(
            self.game_area(),
            gbx_formats::monster::MonsterFile::Cha,
        );
        let block = self
            .data
            .block(&file, monster_id)
            .map_err(|_| MissingData)?;
        let record = gbx_formats::monster::MonsterRecord::from_cha_block(monster_id, &block)
            .map_err(|_| MissingData)?;
        let mut monster = crate::monster::LoadedMonster::from_record(&record);
        // ★ `load_mob`'s optional `MON{area}ITM.DAX` companion
        // (`ovr017.cs:861-869`) — a monster's carried items. Combat never
        // reads them (its attacks come from the record's own profiles); they
        // exist so the corpse has something to drop into the treasure pool
        // (`calc_battle_exp`, `ovr006.cs:36-43`). A file or block that is
        // absent is not an error in the original either (`decode_size != 0`).
        let itm = gbx_formats::monster::monster_filename(self.game_area(), MonsterFile::Itm);
        monster.award.items = self
            .data
            .block(&itm, monster_id)
            .ok()
            .and_then(|b| gbx_formats::save_orig::read_items(&b).ok())
            .unwrap_or_default();
        self.state
            .pending_combat
            .load(monster, num_copies, icon_block_id);
        Ok(MonsterHandle(monster_id as u16))
    }

    /// `CMD_SetupMonster`'s three stores (`ovr003.cs:225-227`). The
    /// ray/clamp/dispatch that follow them are the VM's own sequence
    /// (`machine.rs`'s `op_setup_monster`), because the clamp is visible
    /// arithmetic over an operand.
    fn setup_monster(&mut self, sprite_id: u8, max_distance: u16, pic_id: u8) {
        self.vm.calls.push(RecordedCall::SetupMonster {
            sprite_id,
            max_distance,
            pic_id,
        });
        self.state.sprite_block_id = sprite_id; // `:225`
        self.state.max_encounter_distance = max_distance; // `:226`
        self.state.pic_block_id = pic_id; // `:227`
    }

    /// `CMD_ClearMonsters` (`ovr003.cs:758`): drop the pending-combat roster
    /// and reset its flags (M4 combat #6).
    fn clear_monsters(&mut self) {
        self.vm.calls.push(RecordedCall::ClearMonsters);
        self.state.pending_combat.clear();
    }

    /// ★ `CMD_AddNPC` (`ovr003.cs:1769-1782`) → `load_npc` (`ovr017.cs:878`)
    /// → `load_mob` (`:824`) → `AssignPlayerIconId` (`:892`). The game's only
    /// join mechanism; roll-credits §7.1 has the six shipped sites.
    ///
    /// The `party_size <= 7` gate is `load_npc`'s own (`:880`) and is a **silent
    /// no-op** when it fails — a full party simply does not gain the NPC, with
    /// no message. `party_size` is deliberately NOT incremented: the joined
    /// NPC sits past it in the roster, which is exactly what combat's
    /// `nonTeamMember` split reads.
    ///
    /// `MON{area}SPC` (innate affects) and `MON{area}ITM` (carried items) ride
    /// along when present, both optional in the original (`decode_size != 0`).
    fn add_npc(&mut self, monster_id: u8, morale: u8) -> Result<(), MissingData> {
        self.vm
            .calls
            .push(RecordedCall::AddNpc { monster_id, morale });
        if <Self as gbx_vm::EngineServices>::party_size(self) > 7 {
            return Ok(()); // `ovr017.cs:880`
        }
        let area = self.game_area();
        let cha =
            gbx_formats::monster::monster_filename(area, gbx_formats::monster::MonsterFile::Cha);
        let block = self.data.block(&cha, monster_id).map_err(|_| MissingData)?;
        let record = gbx_formats::save_orig::decode_char_record(&block).map_err(|_| MissingData)?;

        // `load_mob`'s two optional companions (`ovr017.cs:846-869`).
        let spc =
            gbx_formats::monster::monster_filename(area, gbx_formats::monster::MonsterFile::Spc);
        let affects = self
            .data
            .block(&spc, monster_id)
            .ok()
            .and_then(|b| gbx_formats::save_orig::read_affects(&b).ok())
            .unwrap_or_default();
        let itm =
            gbx_formats::monster::monster_filename(area, gbx_formats::monster::MonsterFile::Itm);
        let items = self
            .data
            .block(&itm, monster_id)
            .ok()
            .and_then(|b| gbx_formats::save_orig::read_items(&b).ok())
            .unwrap_or_default();

        let mut member = crate::party::character_from_record(&record, items, affects);
        member.monster_index = monster_id; // `ovr017.cs:884` — `player.mod_id`
                                           // `:1778` — the operand's morale REPLACES the record's own byte.
        member.control_morale = (morale >> 1).wrapping_add(NPC_BASE);
        self.roster.members.push(member); // `AssignPlayerIconId`, `:896`
        self.state.selected_player = (self.roster.members.len() - 1) as u8; // `:897`
        Ok(())
    }

    fn setup_duel(&mut self, is_duel: bool) {
        self.vm.calls.push(RecordedCall::SetupDuel { is_duel });
    }

    /// ★ `calc_group_movement` (`ovr008.cs:1370-1398`): `(slowest, fastest)`
    /// effective movement across the party, `haste` doubling and `slow`
    /// halving each member's own rate. Draw-free.
    ///
    /// The original's degenerate empty-`TeamList` case is preserved verbatim:
    /// the out-params are pre-seeded `(u8::MAX, u8::MIN)` and an empty loop
    /// leaves them there, so "no party" reports a slowest of 255 — which,
    /// against ENCOUNTER MENU's `init_min >= var_407` flee test, would *pass*.
    /// Replicated, not corrected; an empty party never reaches the opcode.
    fn calc_group_movement(&mut self) -> (u8, u8) {
        self.vm.calls.push(RecordedCall::CalcGroupMovement);
        let mut min = u8::MAX;
        let mut max = u8::MIN;
        for member in self.roster.members.iter() {
            let mut movement = member.combat.movement;
            if member.has_affect(crate::party::AFFECT_HASTE) {
                movement = movement.wrapping_mul(2); // `:1380-1383`, a byte
            } else if member.has_affect(crate::party::AFFECT_SLOW) {
                movement /= 2; // `:1384-1387`
            }
            min = min.min(movement);
            max = max.max(movement);
        }
        (min, max)
    }

    /// `sub_304B4` (`ovr008.cs:156-203`) — the forward line-of-sight ray,
    /// already transcribed for combat placement as
    /// [`crate::combat::encounter_distance`]. Its wilderness arm's direct
    /// `area2_ptr.encounter_distance = 2` write (`:164`) is replicated here:
    /// the original really does store through the cell before returning, so a
    /// caller that ignores the return value still sees 2.
    fn approach_distance(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::ApproachDistance);
        let in_dungeon = matches!(self.state.game_state, crate::shell::GameState::DungeonMap);
        let map_dir = crate::shell::facing_to_map_dir(self.state.facing);
        let distance = crate::combat::encounter_distance(
            self.geo,
            map_dir,
            self.state.pos.0 as i32,
            self.state.pos.1 as i32,
            in_dungeon,
        );
        if !in_dungeon {
            self.state.encounter_distance = 2; // `:164`
        }
        distance
    }

    fn encounter_distance(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::EncounterDistance);
        self.state.encounter_distance
    }

    fn set_encounter_distance(&mut self, value: u8) {
        self.vm
            .calls
            .push(RecordedCall::SetEncounterDistance { value });
        self.state.encounter_distance = value;
    }

    fn load_encounter_visual(&mut self) {
        self.vm.calls.push(RecordedCall::LoadEncounterVisual);
        crate::picture::encounter_visual_state(self.state, self.vm);
    }

    fn sprite_off(&mut self) -> bool {
        self.vm.calls.push(RecordedCall::SpriteOff);
        if !self.vm.display_player_sprite {
            return false;
        }
        self.vm.can_draw_bigpic = true; // `ovr003.cs:1712`
        self.vm.display_player_sprite = false; // `:1714`
        self.vm.sprite_changed = false; // `:1715`
        true
    }

    fn set_encounter_menu_active(&mut self, active: bool) {
        self.vm
            .calls
            .push(RecordedCall::SetEncounterMenuActive { active });
        self.vm.byte_1ee95 = active;
    }

    fn create_item(&mut self, item_type: u8) -> ItemHandle {
        self.vm.calls.push(RecordedCall::CreateItem { item_type });
        ItemHandle(0)
    }

    /// TREASURE's table arm (`ovr003.cs:1083-1099`): every `Item.StructSize`
    /// record in block `block_id` of `ITEM{game_area}.DAX`, appended to the
    /// pool. The block is read whole and split on the fixed record size — a
    /// trailing partial record is a malformed file, which the original would
    /// have walked off the end of; we drop it.
    fn load_treasure_items(&mut self, block_id: u8) -> Result<(), MissingData> {
        self.vm
            .calls
            .push(RecordedCall::LoadTreasureItems { block_id });
        let file = format!("ITEM{}.DAX", self.game_area()); // `:1085`
        let data = self.data.block(&file, block_id).map_err(|_| MissingData)?;
        for record in data.chunks_exact(gbx_formats::save_orig::ITEM_RECORD_SIZE) {
            self.state.treasure_items.push(record.to_vec());
        }
        Ok(())
    }

    fn set_pooled_coin(&mut self, coin: u8, value: u16) {
        self.vm
            .calls
            .push(RecordedCall::SetPooledCoin { coin, value });
        self.state.pooled_money.set(coin as usize, value as i32);
    }

    /// ★ SPELL (0x3B)'s scan, `sub_28E33` (`ovr003:2E33-2F22`) verbatim —
    /// including the 18-byte overrun past `spell_list`. See
    /// [`EngineServices::find_spell_in_party`] for the three corrections to
    /// coab; [`char_scan_byte`] is the read.
    ///
    /// Draw-free.
    fn find_spell_in_party(&mut self, spell_id: u8) -> (u8, u8) {
        self.vm
            .calls
            .push(RecordedCall::FindSpellInParty { spell_id });

        let count = self.roster.members.len();
        let mut spell_index: u8 = 1;
        let mut player_index: u8 = 0;
        let mut found = false;
        for (i, member) in self.roster.members.iter().enumerate() {
            // `mov [bp+var_2], 1` at the head of every outer iteration
            // (`ovr003:2E97`).
            spell_index = 1;
            loop {
                if char_scan_byte(member, spell_index) == spell_id {
                    found = true;
                    break;
                }
                // `cmp var_2, 100 / ja` — the increment stops at 101, and the
                // byte at 101 has already been compared by then.
                if spell_index > 100 {
                    break;
                }
                spell_index += 1;
            }
            if found {
                break;
            }
            // `ovr003:2EE6-2EF4` — the increment is at the BOTTOM and gated on
            // the NEXT cursor being non-null, so an exhausted scan leaves
            // `player_index` on the LAST member, never one past it.
            if i + 1 < count {
                player_index += 1;
            }
        }
        // `cmp var_2, 100 / jbe / mov var_2, 0xFF` (`ovr003:2EF9-2EFF`): a
        // value test, not a `found` test.
        if spell_index > 100 {
            spell_index = 0xFF;
        }
        (spell_index, player_index)
    }

    fn roll(&mut self, max: u8) -> u8 {
        self.vm.calls.push(RecordedCall::Roll { max });
        // CORRECTED off-by-one (oracle-rig §6 ledger): the contract is
        // `seg051.Random(max)` = `Next() % max`, EXCLUSIVE 0..max
        // (seg051.cs:33-40; CMD_Random pre-increments at machine.rs op_random).
        // The old `roll_uniform(max)` was inclusive 0..=max and could return
        // `operand+1`. `random(max)` restores the exclusive bound. The
        // mechanical rename `roll_uniform(max) -> random(max+1)` would have
        // frozen the bug — do not.
        self.rng.random(max as u16) as u8
    }

    fn roll_dice(&mut self, size: u8, count: u8) -> u16 {
        self.vm.calls.push(RecordedCall::RollDice { size, count });
        // `roll_total += Random(dice_size) + 1` per die (ovr024.cs:586-598).
        // `1 + random(size)` is the faithful translation. `size == 0` now
        // *draws* (random(0) advances then returns 0 -> +1) — the binary draws;
        // the old `roll_uniform(size-1)` short-circuited without drawing.
        // (Byte-truncation of the total, ovr024.cs:595's `(byte)roll_total`,
        // is docketed as FD-29 — unreachable at current call sites.)
        let mut total = 0u16;
        for _ in 0..count {
            total += 1 + self.rng.random(size as u16);
        }
        total
    }

    /// `do_saving_throw(bonus, type, player)` (`ovr024:12F1`, doc §48) — the
    /// same transcription combat already carries
    /// ([`crate::combat::saving_throw`]), reached here with a roster member
    /// rather than a combatant. **Draws one d20.**
    fn roll_saving_throw(&mut self, player: PlayerId, bonus: u8, save_type: u8) -> bool {
        self.vm.calls.push(RecordedCall::RollSavingThrow {
            player,
            bonus,
            save_type,
        });
        let Some(member) = self.roster.members.get(player.0 as usize) else {
            // The original would have dereferenced a null here; there is no
            // reachable operand that gets it there (see `op_damage`).
            return false;
        };
        let target = member
            .skills
            .save_verse
            .get(save_type as usize)
            .copied()
            .unwrap_or(0) as i32;
        let save_bonus = member.status.save_bonus as i32 + bonus as i32;
        let roll = 1 + self.rng.random(20) as i32;
        roll + save_bonus >= target
    }

    /// `sub_641DD(bonus, player)` — DAMAGE's per-victim to-hit check. The
    /// operand is a to-hit number in the raw-AC compare space, exactly like a
    /// combatant's `hitBonus`, tested against the victim's stored AC.
    /// **Draws one d20.**
    fn can_hit_target(&mut self, player: PlayerId, bonus: u8) -> bool {
        self.vm
            .calls
            .push(RecordedCall::CanHitTarget { player, bonus });
        let Some(member) = self.roster.members.get(player.0 as usize) else {
            return false;
        };
        let ac = member.combat.ac as i32;
        let roll = 1 + self.rng.random(20) as i32;
        roll + bonus as i32 >= ac
    }

    /// ★ `sub_32200` (`ovr008.cs:1401-1437`) → `damage_player`
    /// (`ovr025.cs:1183-1243`), transcribed whole because its thresholds are
    /// the difference between an unconscious party member and a dead one:
    ///
    /// - Already `dead`? `sub_32200` returns immediately (`:1403`) — a corpse
    ///   takes no further damage and prints nothing.
    /// - `neg_hp` is the overkill past 0. **`neg_hp > 9` is death**
    ///   (`:1197`), and so is being reduced to exactly 0 while `animated`.
    /// - Otherwise any overkill at all is `dying` (`:1206`), and landing
    ///   exactly on 0 is `unconscious` (`:1215`).
    /// - Only `okey`/`animated` keep their HP; every other outcome zeroes it
    ///   and clears `in_combat` (`:1226-1227`) — which is precisely what
    ///   DAMAGE's own wipe scan then counts.
    ///
    /// The combat-only arms (`actions.bleeding`, the friend/foe counters, the
    /// delay reset) belong to a `CombatState`, not to the roster this service
    /// walks; a script DAMAGE never runs inside one.
    fn apply_damage(&mut self, player: PlayerId, damage: u16) {
        self.vm
            .calls
            .push(RecordedCall::ApplyDamage { player, damage });
        let Some(member) = self.roster.members.get_mut(player.0 as usize) else {
            return;
        };
        if member.status.health_status == HEALTH_DEAD {
            return; // `ovr008.cs:1403`
        }
        let hp = member.hit_point_current as i32;
        let damage = damage as i32;
        let (new_hp, neg_hp) = if hp >= damage {
            (hp - damage, 0)
        } else {
            (0, damage - hp)
        };
        if neg_hp > 9 || (new_hp == 0 && member.status.health_status == HEALTH_ANIMATED) {
            member.status.health_status = HEALTH_DEAD;
        } else if neg_hp > 0 {
            member.status.health_status = HEALTH_DYING;
        } else if new_hp == 0 {
            member.status.health_status = HEALTH_UNCONSCIOUS;
        }
        if member.status.health_status == HEALTH_OKAY
            || member.status.health_status == HEALTH_ANIMATED
        {
            member.hit_point_current = new_hp as u8;
        } else {
            member.status.in_combat = false;
            member.hit_point_current = 0;
        }
    }

    /// ★ `Load3DMap` (`ovr031.cs:690-705`) — **the resident map really
    /// changes now** (roll-credits slice 1; this is FD-19's "settled by").
    ///
    /// The original decodes `GEO{game_area}.DAX` block `blockId` straight into
    /// `gbl.geo_ptr` and records the id in `area_ptr.current_3DMap_block_id`.
    /// Since M2 this host recorded the id and left the resident `GeoBlock`
    /// alone (a documented step-5+ deferral), so every `LOAD FILES` naming a
    /// different map left the party walking the old one's geometry — which is
    /// what made a cross-block transition look like it half-worked.
    ///
    /// A failed load is a **hard stop** in the original
    /// (`Logger.LogAndExit("Unable to load geo in Load3DMap.")`, `:699`,
    /// including its `bytesRead != 0x402` size check). `EngineServices` has no
    /// error channel here, so the failure lands in `vm_memory.halts` — loud,
    /// counted, and visible to the same diagnostics as a VM halt — and the
    /// resident block is left untouched rather than half-swapped.
    fn load_3d_map(&mut self, block_id: u8) {
        self.vm.calls.push(RecordedCall::Load3dMap { block_id });
        let area = self.game_area();
        match load_geo_block(self.data, area, block_id) {
            Ok(block) => {
                *self.geo = block;
                self.vm.assets.map_3d_block = Some(block_id);
            }
            Err(err) => {
                self.vm.halts.push(HaltRecord {
                    pc: 0,
                    opcode: 0,
                    description: format!(
                        "Load3DMap: GEO{area}.DAX block {block_id} did not load ({err:?}) — \
                         the resident map is unchanged"
                    ),
                });
            }
        }
    }

    /// `LoadWalldef` (`ovr031.cs:642-687`, step 5 task deliverable 1) — a
    /// dedicated research pass this session read the function (plus
    /// `Classes/GeoBlock.cs`'s `WallDefs`/`WallDefBlock.Offset`) in full and
    /// found a load call can populate *multiple consecutive* wallset slots,
    /// not just `set` itself: it loads the walldef block's raw tile-id data
    /// from `"WALLDEF{game_area}.DAX"` block `id`, which may hold several
    /// internal 780-byte sub-blocks (`WalldefBlock::wallset_count`); for
    /// each sub-block `n` (`0`-indexed), the *target* slot is `set + n`, and
    /// only sub-blocks landing in `1..=3` are kept (`idx = symbolSet + block`,
    /// `:664-682`) — so `LoadWalldef(1, id)` with a 3-sub-block walldef
    /// populates sets 1, 2, *and* 3 in one call. Each kept sub-block's
    /// paired 8×8 pixel data loads from `"8X8D{game_area}.DAX"` at `id`
    /// (single sub-block) or `id*10 + n + 1` (multiple, 1-based, `:673-679`)
    /// into `SymbolSets`' matching pixel slot. The `>=0x2D` rebase (`var_A =
    /// symbol_set_fix[set] - symbol_set_fix[1]`, computed once from the
    /// call's *original* `set` parameter, `:658`) is applied to every
    /// touched sub-block's tile ids (wrapping byte add, `GeoBlock.cs:84`)
    /// before storing — baked in, not reapplied at lookup time. Bookkeeping
    /// (`vm.assets.walldefs`) records only the original `set` slot's
    /// `(set, id)` pair, matching a real asymmetry this research pass found
    /// in the original itself (`:669` vs `:684-685` — every touched slot
    /// gets real texture data, only the one matching the call's own `set`
    /// gets its `setBlocks` entry written). Any load failure (missing
    /// block, malformed data) is a silent no-op for that sub-block beyond
    /// the call log — real CotAB data never hits this path (this session's
    /// demo/tests load every wallset the walk exercises without error).
    fn load_walldef(&mut self, set: u8, id: u8) {
        self.vm.calls.push(RecordedCall::LoadWalldef { set, id });
        let slot = (set.saturating_sub(1)) as usize;
        if let Some(entry) = self.vm.assets.walldefs.get_mut(slot) {
            *entry = Some((set, id));
        }
        let area = self.game_area();
        load_walldef_pixels(self.symbols, self.data, area, set, id);
    }

    fn load_bigpic(&mut self, id: u8) {
        self.vm.calls.push(RecordedCall::LoadBigpic { id });
        self.vm.assets.bigpic_block = Some(id);
        // `gbl.bigpic_block_id` and `DaxArrayFreeDaxBlocks(byte_1D556)`
        // (`ovr030.cs:228-239`): the picture layer is the one place that
        // bookkeeping lives, so it is written here too. The decoded-asset
        // cache is keyed by block id and needs no explicit free — a stale
        // entry is never *selected*, only re-keyed.
        self.state.picture.bigpic_block = Some(id);
        self.state.picture.anim_block = None;
        self.state.picture.anim_frame = 0;
        // The same free writes `lastDaxBlockId = 0xFF` (`ovr030.cs:164`) —
        // see `crate::picture::CITY_SCENE_PIC_BLOCK`.
        self.state.picture.last_dax_block = crate::picture::NO_DAX_BLOCK;
    }

    fn last_dax_block(&mut self) -> u8 {
        self.state.picture.last_dax_block
    }

    fn reset_wall_set(&mut self, index: u8) {
        self.vm.calls.push(RecordedCall::ResetWallSet { index });
        if let Some(entry) = self.vm.assets.walldefs.get_mut(index as usize) {
            *entry = None;
        }
        if (index as usize) < crate::symbols::WALLSET_SLOT_COUNT {
            self.symbols.reset_wallset(index as usize);
        }
    }

    fn step_game_time(&mut self, time_slot: u8, amount: u8) {
        self.vm
            .calls
            .push(RecordedCall::StepGameTime { time_slot, amount });
        self.state.clock.step(time_slot, amount);
    }

    /// `CALL 0x401F` (`MovePositionForward`, research §4's summary table):
    /// advances one cell in the facing direction, wrapping map coords —
    /// the raw advance, with none of the walk loop's door gating
    /// (`movement::move_party_forward` is the higher-level function for
    /// that; this mirrors the original's own lower-level primitive).
    fn move_position_forward(&mut self) {
        self.vm.calls.push(RecordedCall::MovePositionForward);
        let (dx, dy) = self.state.facing.delta();
        self.state.pos.0 = (self.state.pos.0 as i32 + dx).rem_euclid(16) as u8;
        self.state.pos.1 = (self.state.pos.1 as i32 + dy).rem_euclid(16) as u8;
        self.vm.position_changed = true;
    }

    fn wall_roof(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::WallRoof);
        self.wall_roof_value() as u8
    }

    fn wall_type(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::WallType);
        self.wall_type_value() as u8
    }

    /// The `0xAE11` consolidated redraw gate (`ovr003.cs:1848-1860`):
    /// check-and-clear at execution time; the guarded draw is
    /// `Effect::RedrawView`'s job at present time. ★ All five inner flags are
    /// modeled as of the encounter slice — `displayPlayerSprite`, FD-34's,
    /// joined the check when `sub_30580` grew a real state pass. (The outer
    /// `byte_1AB0B` conjunct remains FD-35.)
    fn redraw_view_gate(&mut self) -> bool {
        let armed = self.vm.sprite_changed
            || self.vm.display_player_sprite
            || self.vm.byte_1ee91
            || self.vm.position_changed
            || self.vm.byte_1ee94;
        self.vm.clear_redraw_flags();
        self.vm.display_player_sprite = false; // `:1861`, the fifth clear
        self.vm.calls.push(RecordedCall::RedrawViewGate { armed });
        armed
    }

    /// CALL `0x3201`'s variant selector (research §1.3/§1.5): `word_1EE76
    /// == 8` -> `sound_a`-class, `== 10` -> `sound_b`-class, else
    /// `sound_a`-class. Real sound-catalog ids are a documented placeholder
    /// (`movement::SOUND_A`'s doc comment) pending a `seg044.cs` read.
    /// `CMD_Program` (`ovr003.cs:1929-1987`). Cases 0 and 8 are start-menu /
    /// end-game flows this engine does not have yet (FD-45, roll-credits G9
    /// owns the ending); case 9 is `TryEncamp`, which belongs to G3's camp
    /// work. Case 3 — the party-killed flag plus an EXIT — is real here.
    fn program(&mut self, code: u8) -> ProgramOutcome {
        self.vm.calls.push(RecordedCall::Program { code });
        // `:1934-1938` — every case restores the selection first.
        if self.state.restore_player_ptr {
            self.state.selected_player = self.state.last_selected_player;
            self.state.restore_player_ptr = false;
        }
        match code {
            3 => {
                self.state.party_killed = true; // `:1984`
                ProgramOutcome::Exit
            }
            // ★ Roll-credits slice 9c, closing §14.8's residual: `var_1 == 0`
            // is `startGameMenu()` and then a conditional `LoadPic()`
            // (`ovr003.cs:1941-1948`). The menu is a whole screen, so the
            // handler raises the request and the shell parks on it; the
            // script resumes into the walk loop when `BEGIN` is pressed.
            //
            // No shipped script reaches this — the one `PROGRAM` use is
            // `ECL1#82`'s, inside the deferred attract mode — so the arm is
            // implemented and observable rather than exercised in play.
            //
            // **The one deviation, named:** the original's menu is modal
            // *inside* the instruction, so the rest of the block runs after
            // it. Ours runs the rest of the block first and opens the menu
            // where the block hands the party back to the walk loop — which
            // is the same place `startGameMenu` returns to in `sub_29758`.
            // Nothing in the block is skipped; only the modal's position
            // relative to it moves.
            0 => {
                self.state.pending_start_menu = true;
                self.vm
                    .transcript
                    .push(crate::vmhost::TranscriptEntry::Request(
                        "PROGRAM 0: startGameMenu".to_string(),
                    ));
                ProgramOutcome::Continue
            }
            _ => {
                self.vm
                    .transcript
                    .push(crate::vmhost::TranscriptEntry::Request(format!(
                        "PROGRAM {code}: not implemented (docket FD-45)"
                    )));
                if code == 9 {
                    ProgramOutcome::Exit // `:1980` — TryEncamp then CMD_Exit
                } else {
                    ProgramOutcome::Continue
                }
            }
        }
    }

    fn call_sound_variant(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::CallSoundVariant);
        if self.vm.word_1ee76 == 10 {
            1
        } else {
            crate::movement::SOUND_A
        }
    }
}

impl VmRng for EngineVmHost<'_> {
    fn random(&mut self, n: u16) -> u16 {
        self.rng.random(n)
    }
}

impl gbx_vm::VmHost for EngineVmHost<'_> {
    fn rng(&mut self) -> &mut dyn VmRng {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::DefaultPartyPredicates;
    use crate::rng::EngineRng;
    use crate::shell::SoundEvent;
    use gbx_formats::geo::GEO_BLOCK_SIZE;
    use gbx_vm::EngineServices;

    const GAME_AREA: u8 = 2;

    struct Fixture {
        state: EngineState,
        vm: VmMemoryState,
        geo: GeoBlock,
        party: DefaultPartyPredicates,
        roster: crate::party::Party,
        rng: EngineRng,
        sounds: Vec<SoundEvent>,
        data: GameData,
        symbols: SymbolSets,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_data(GameData::from_files(Vec::<(String, Vec<u8>)>::new()))
        }

        fn with_data(data: GameData) -> Self {
            let mut state = EngineState::new();
            state.game_area = GAME_AREA;
            state.game_area_backup = GAME_AREA;
            Fixture {
                state,
                vm: VmMemoryState::new(),
                geo: GeoBlock::parse(&vec![0u8; GEO_BLOCK_SIZE]).unwrap(),
                party: DefaultPartyPredicates::default(),
                roster: crate::party::Party::default(),
                rng: EngineRng::new(1),
                sounds: Vec::new(),
                data,
                symbols: SymbolSets::new(),
            }
        }

        fn host(&mut self) -> EngineVmHost<'_> {
            EngineVmHost {
                state: &mut self.state,
                vm: &mut self.vm,
                geo: &mut self.geo,
                party: &mut self.party,
                roster: &mut self.roster,
                rng: &mut self.rng,
                sounds: &mut self.sounds,
                data: &self.data,
                symbols: &mut self.symbols,
            }
        }
    }

    // --- roll-credits slice 3: the roster/item services -------------------

    /// A `.swg`-shaped item record with the two fields the tail's opcodes
    /// read: `type` @0x2E and `readied` @0x34 (`Classes/Item.cs:118`, `:124`).
    fn item(item_type: u8, readied: bool) -> Vec<u8> {
        let mut rec = vec![0u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
        rec[0x2E] = item_type;
        rec[0x34] = u8::from(readied);
        rec
    }

    fn member(name: &str, hp: u8) -> crate::party::Character {
        let mut rec = vec![0u8; gbx_formats::save_orig::CHAR_RECORD_SIZE];
        rec[0] = name.len() as u8;
        rec[1..1 + name.len()].copy_from_slice(name.as_bytes());
        rec[0x78] = hp;
        rec[0x1a4] = hp;
        let record = gbx_formats::save_orig::decode_char_record(&rec).unwrap();
        let mut ch = crate::party::character_from_record(&record, Vec::new(), Vec::new());
        ch.status.in_combat = true;
        ch
    }

    #[test]
    fn find_item_scans_every_member_not_just_the_selected_one() {
        let mut f = Fixture::new();
        f.roster.members.push(member("A", 10));
        let mut third = member("C", 10);
        third.items.push(item(0x5E, false));
        f.roster.members.push(member("B", 10));
        f.roster.members.push(third);

        let mut h = f.host();
        assert!(h.party_has_item(0x5E));
        assert!(!h.party_has_item(0x5F));
    }

    // --- roll-credits slice 9a: SPELL (0x3B)'s scan ----------------------

    fn caster(name: &str, slots: &[(usize, u8)]) -> crate::party::Character {
        let mut ch = member(name, 10);
        ch.magic.spell_list = vec![0u8; 84];
        for (slot, id) in slots {
            ch.magic.spell_list[*slot] = *id;
        }
        ch.magic.spell_book = vec![0u8; 100];
        ch
    }

    /// A hit reports the slot index and the member who had it.
    #[test]
    fn find_spell_in_party_reports_the_slot_and_the_holder() {
        let mut f = Fixture::new();
        f.roster.members.push(caster("A", &[(1, 0x03)]));
        f.roster.members.push(caster("B", &[(4, 0x16)]));
        f.roster.members.push(caster("C", &[]));

        let mut h = f.host();
        assert_eq!(h.find_spell_in_party(0x16), (4, 1));
    }

    /// ★ **The scan starts at slot 1**: `var_2` is seeded to 1 and the read is
    /// `[player + var_2 + 0x1E]` (`ovr003:2E74`, `:2EA0-2EA5`), so
    /// `spell_list[0]` is never examined.
    #[test]
    fn find_spell_in_party_never_examines_slot_zero() {
        let mut f = Fixture::new();
        f.roster.members.push(caster("A", &[(0, 0x16)]));

        let mut h = f.host();
        assert_eq!(
            h.find_spell_in_party(0x16),
            (0xFF, 0),
            "the only copy sits in the one slot the original skips"
        );
    }

    /// ★ **The not-found pair is `(0xFF, Count - 1)`**, not `(0xFF, 0xFF)`:
    /// `player_index` is incremented at the bottom of the outer body and only
    /// when the next cursor is non-null (`ovr003:2EE6-2EF4`).
    #[test]
    fn find_spell_in_party_not_found_leaves_the_index_on_the_last_member() {
        let mut f = Fixture::new();
        for name in ["A", "B", "C", "D"] {
            f.roster.members.push(caster(name, &[]));
        }

        let mut h = f.host();
        assert_eq!(h.find_spell_in_party(0x16), (0xFF, 3));
    }

    /// ★ **The scan overruns `spell_list` by eighteen bytes.** Slot 90 is
    /// `charStruct 0x78`, `hit_point_max` — so a character with 22 maximum hit
    /// points "has" spell `0x16`, which is the id both shipped sites ask for.
    #[test]
    fn find_spell_in_party_walks_past_the_list_into_hit_point_max() {
        let mut f = Fixture::new();
        let mut ch = caster("A", &[]);
        ch.hit_point_max = 0x16;
        f.roster.members.push(ch);

        let mut h = f.host();
        assert_eq!(
            h.find_spell_in_party(0x16),
            (90, 0),
            "★ the original's own overrun, reproduced"
        );
    }

    /// The same overrun reaches `race` (slot 86) and `spellBook[0]` (slot 91).
    #[test]
    fn find_spell_in_party_overrun_covers_the_whole_eighteen_byte_tail() {
        type SetField = fn(&mut crate::party::Character, u8);
        let cases: [(u8, SetField); 4] = [
            (84, |c, v| c.magic.spell_to_learn_count = v),
            (86, |c, v| c.race = v),
            (90, |c, v| c.hit_point_max = v),
            (91, |c, v| c.magic.spell_book[0] = v),
        ];
        for (slot, set) in cases {
            let mut f = Fixture::new();
            let mut ch = caster("A", &[]);
            set(&mut ch, 0x33);
            f.roster.members.push(ch);
            let mut h = f.host();
            assert_eq!(h.find_spell_in_party(0x33), (slot, 0), "slot {slot}");
        }
    }

    /// ★ `[0x7D00]` — `get_player_values`' `arg_4 == 0x100` case
    /// (`ovr008.cs:424-441`): `1` when the selected member is `in_combat`,
    /// `0x80` when not, `0` when `player_not_found` — and the read **clears**
    /// the flag, so the next one answers normally.
    #[test]
    fn the_selected_player_state_cell_answers_three_ways_and_clears_the_flag() {
        let mut f = Fixture::new();
        let mut down = member("B", 10);
        down.status.in_combat = false;
        f.roster.members.push(member("A", 10));
        f.roster.members.push(down);
        let origin = Origin { pc: 0x8000 };

        let mut h = f.host();
        assert_eq!(h.read(0x7D00, origin), 1, "member 0 is in_combat");

        f.state.selected_player = 1;
        let mut h = f.host();
        assert_eq!(h.read(0x7D00, origin), 0x80, "member 1 is not");

        // LOAD CHARACTER's out-of-range arm arms the flag; the first read of
        // this cell consumes it, the second reads through.
        f.state.selected_player = 0;
        let mut h = f.host();
        h.retarget_selected_player(9);
        assert!(f.state.player_not_found);
        let mut h = f.host();
        assert_eq!(h.read(0x7D00, origin), 0, "not-found overrides in_combat");
        assert!(!f.state.player_not_found, "★ the read cleared it");
        let mut h = f.host();
        assert_eq!(h.read(0x7D00, origin), 1, "and the next read is normal");
    }

    /// An item record carrying a weight, for ROB's weight ladder
    /// (`Classes/Item.cs` `weight` @0x37).
    fn item_weighing(weight: i16) -> Vec<u8> {
        let mut rec = vec![0u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
        rec[0x37..0x39].copy_from_slice(&weight.to_le_bytes());
        rec
    }

    /// ★ ROB's money half scales **all seven** denominations (`sub_31DEF`,
    /// `ovr008:1DEF-1F19`) — coab's `MoneySet.ScaleAll` stops at platinum.
    #[test]
    fn rob_money_scales_gems_and_jewelry_too() {
        let mut f = Fixture::new();
        let mut m = member("A", 10);
        m.money = crate::party::Money {
            copper: 100,
            silver: 50,
            electrum: 33,
            gold: 20,
            platinum: 7,
            gems: 9,
            jewelry: 3,
        };
        f.roster.members.push(m);

        let mut h = f.host();
        h.rob_money(PlayerId(0), 0.25); // ROB ..., 0x4B — 75% taken

        let money = f.roster.members[0].money;
        assert_eq!(money.copper, 25);
        assert_eq!(money.silver, 12, "50 * 0.25 = 12.5, truncated");
        assert_eq!(money.electrum, 8, "33 * 0.25 = 8.25");
        assert_eq!(money.gold, 5);
        assert_eq!(money.platinum, 1, "7 * 0.25 = 1.75");
        assert_eq!(money.gems, 2, "★ coab's ScaleAll would have left this 9");
        assert_eq!(money.jewelry, 0, "★ and this 3");
    }

    /// The six scales the shipped ROB operands produce, each against a value
    /// whose exact product is a whole number — the `f64`-for-`Real`
    /// substitution has to land on the same integer.
    #[test]
    fn the_shipped_rob_scales_truncate_exactly() {
        // (percent taken, coins in, coins out) — the exact rational answer.
        for (taken, coins, expect) in [
            (0x4Bu8, 40i16, 10i16), // 0.25
            (0x32, 41, 20),         // 0.5
            (0x14, 5, 4),           // 0.8  — exact product 4
            (0x5F, 20, 1),          // 0.05 — exact product 1
            (0x28, 5, 3),           // 0.6  — exact product 3
            (0x0A, 10, 9),          // 0.9  — exact product 9
        ] {
            let mut money = crate::party::Money {
                gold: coins,
                ..Default::default()
            };
            money.scale_all(f64::from(100 - taken) / 100.0);
            assert_eq!(money.gold, expect, "{taken:#04X} on {coins} coins");
        }
    }

    /// ★ ROB's item half: one d100 per item **whatever the chance**, the
    /// weight ladder reducing the chance **cumulatively** down the list
    /// (`ovr008:1F59`/`:1F7C` write the reduced value back to the parameter).
    #[test]
    fn rob_items_burns_one_roll_per_item_and_the_weight_ladder_is_cumulative() {
        let mut f = Fixture::new();
        let mut m = member("A", 10);
        // A heavy item (-90) followed by two featherweights: after the first,
        // the chance is 125-90 = 35 for the REST of the list too.
        m.items = vec![
            item_weighing(300),
            item_weighing(1),
            item_weighing(1),
            item_weighing(1),
        ];
        f.roster.members.push(m);

        // Predict the four `roll_dice(100, 1)` draws off a reference PRNG on
        // the same seed, and the chance ladder by hand.
        let mut reference = EngineRng::new(1);
        let rolls: Vec<u8> = (0..4).map(|_| 1 + reference.random(100) as u8).collect();
        let weights = [300i16, 1, 1, 1];
        // 125 - 90 at item 0, and the reduction STAYS for the rest of the list.
        let chances = [35u8, 35, 35, 35];
        let survivors: Vec<i16> = (0..4)
            .filter(|&i| rolls[i] > chances[i])
            .map(|i| weights[i])
            .collect();

        let mut h = f.host();
        h.rob_items(PlayerId(0), 0x7D);

        assert_eq!(
            f.rng.state(),
            reference.state(),
            "exactly one roll per item, chance never short-circuits"
        );
        assert_eq!(
            f.roster.members[0]
                .items
                .iter()
                .map(|it| gbx_formats::save_orig::item_weight(it))
                .collect::<Vec<_>>(),
            survivors,
            "the ladder demoted a certainty to 35% for the whole rest of the list"
        );
        assert!(
            !survivors.is_empty(),
            "chance 125 unreduced would have taken every item"
        );
    }

    /// Chance 0 still draws once per item and takes nothing (`roll >= 1`).
    #[test]
    fn rob_items_with_chance_zero_draws_but_never_takes() {
        let mut f = Fixture::new();
        let mut m = member("A", 10);
        m.items = vec![item_weighing(1), item_weighing(1)];
        f.roster.members.push(m);

        let mut reference = EngineRng::new(1);
        reference.random(100);
        reference.random(100);

        let mut h = f.host();
        h.rob_items(PlayerId(0), 0);
        assert_eq!(f.rng.state(), reference.state(), "two draws, both wasted");
        assert_eq!(f.roster.members[0].items.len(), 2);
    }

    /// DESTROY ITEMS removes by type across the whole roster **and** rebuilds
    /// each member's readied set — the removal renumbers every index after it,
    /// so a stale set would ready the wrong item.
    #[test]
    fn destroy_items_removes_by_type_and_renumbers_the_readied_set() {
        let mut f = Fixture::new();
        let mut m = member("A", 10);
        m.items = vec![item(0x5E, false), item(0x60, true), item(0x5E, false)];
        m.readied_items = [1].into_iter().collect();
        f.roster.members.push(m);

        let mut h = f.host();
        h.destroy_items(0x5E);

        let m = &f.roster.members[0];
        assert_eq!(m.items.len(), 1);
        assert_eq!(gbx_formats::save_orig::item_type(&m.items[0]), 0x60);
        assert_eq!(
            m.readied_items.iter().copied().collect::<Vec<_>>(),
            vec![0],
            "the readied item moved from index 1 to index 0"
        );
    }

    /// ★ The binary's slot semantics, against coab's: `LOAD CHARACTER 0`
    /// selects member 0 and is FOUND (`ovr003:0328` `jbe` skips the walk with
    /// the cursor already on the list head). coab's transliteration returns
    /// null for index 0, which would make the shipped `ECL5#48` slot scan miss
    /// the first party member entirely.
    #[test]
    fn load_character_slot_zero_selects_the_first_member() {
        let mut f = Fixture::new();
        f.roster.members.push(member("A", 10));
        f.roster.members.push(member("B", 10));
        f.state.selected_player = 1;

        let mut h = f.host();
        h.retarget_selected_player(0);
        assert_eq!(f.state.selected_player, 0);
        assert!(!f.state.player_not_found);
        assert!(f.state.restore_player_ptr, "`ovr003:02EF`, unconditional");
    }

    #[test]
    fn load_character_out_of_range_sets_player_not_found_and_keeps_the_selection() {
        let mut f = Fixture::new();
        f.roster.members.push(member("A", 10));
        f.state.selected_player = 0;

        let mut h = f.host();
        h.retarget_selected_player(5);
        assert!(f.state.player_not_found);
        assert_eq!(f.state.selected_player, 0, "the miss leaves it alone");
    }

    /// The high bit alone does nothing: both summary flags must be armed too
    /// (`ovr003:0377-0389`).
    #[test]
    fn load_character_high_bit_only_removes_when_both_summary_flags_are_armed() {
        let mut f = Fixture::new();
        f.roster.members.push(member("A", 10));
        f.roster.members.push(member("B", 10));

        let mut h = f.host();
        h.retarget_selected_player(0x81);
        assert_eq!(f.roster.members.len(), 2, "flags unarmed: nothing removed");

        f.state.redraw_party_summary = [true, true];
        let mut h = f.host();
        h.retarget_selected_player(0x81);
        assert_eq!(f.roster.members.len(), 1);
        assert_eq!(f.roster.members[0].name, "A");
        assert_eq!(
            f.state.redraw_party_summary,
            [false, false],
            "`:03BB-03C0` clears them"
        );
    }

    /// `FreeCurrentPlayer` re-seats one slot BACK (`ovr018.cs:1600`), not on
    /// the member that slid into the vacated slot.
    #[test]
    fn free_current_player_reseats_one_slot_back_and_drops_party_size() {
        let mut f = Fixture::new();
        for name in ["A", "B", "C"] {
            f.roster.members.push(member(name, 10));
        }
        f.state.party_size = 3;
        f.state.selected_player = 2;

        let mut h = f.host();
        h.free_current_player(true, false);
        assert_eq!(f.state.selected_player, 1);
        assert_eq!(f.state.party_size, 2);

        f.state.selected_player = 0;
        let mut h = f.host();
        h.free_current_player(true, true);
        assert_eq!(f.state.selected_player, 0);
        assert_eq!(f.state.party_size, 2, "leave_party_size held it");
    }

    /// ★ `damage_player`'s thresholds (`ovr025.cs:1183-1243`) — the ones that
    /// decide unconscious vs dying vs dead.
    #[test]
    fn apply_damage_walks_the_full_status_ladder() {
        let cases = [
            // (hp, damage, expected status, expected hp, still in combat?)
            (10u8, 3u16, HEALTH_OKAY, 7u8, true),
            (10, 10, HEALTH_UNCONSCIOUS, 0, false),
            (10, 15, HEALTH_DYING, 0, false),
            (10, 20, HEALTH_DEAD, 0, false),
        ];
        for (hp, damage, status, left, in_combat) in cases {
            let mut f = Fixture::new();
            f.roster.members.push(member("A", hp));
            let mut h = f.host();
            h.apply_damage(PlayerId(0), damage);
            let m = &f.roster.members[0];
            assert_eq!(m.status.health_status, status, "hp {hp} - {damage}");
            assert_eq!(m.hit_point_current, left);
            assert_eq!(m.status.in_combat, in_combat);
        }
    }

    #[test]
    fn apply_damage_never_touches_a_corpse() {
        let mut f = Fixture::new();
        let mut m = member("A", 10);
        m.status.health_status = HEALTH_DEAD;
        m.hit_point_current = 0;
        f.roster.members.push(m);

        let mut h = f.host();
        h.apply_damage(PlayerId(0), 40);
        assert_eq!(f.roster.members[0].status.health_status, HEALTH_DEAD);
    }

    /// DAMAGE's wipe scan reads `in_combat`, nothing else — and it is the flag
    /// `apply_damage` clears on the way down.
    #[test]
    fn party_wipe_check_fires_only_when_nobody_is_left_in_combat() {
        let mut f = Fixture::new();
        f.roster.members.push(member("A", 10));
        f.roster.members.push(member("B", 10));

        let mut h = f.host();
        assert!(!h.party_wipe_check());

        f.roster.members[0].status.in_combat = false;
        let mut h = f.host();
        assert!(!h.party_wipe_check());

        f.roster.members[1].status.in_combat = false;
        let mut h = f.host();
        assert!(h.party_wipe_check());
        assert!(f.state.party_killed);
        assert_eq!(f.state.wipe_cause, crate::shell::WipeCause::EclDamage);
    }

    /// `load_npc`'s silent `party_size <= 7` gate (`ovr017.cs:880`).
    #[test]
    fn add_npc_refuses_silently_once_the_party_is_full() {
        let mut f = Fixture::new();
        f.state.party_size = 8;
        let mut h = f.host();
        assert_eq!(h.add_npc(0x16, 0x64), Ok(()));
        assert!(f.roster.members.is_empty(), "no join, and no error either");
    }

    fn origin() -> Origin {
        Origin { pc: 0x8100 }
    }

    #[test]
    fn map_pos_round_trips_and_sets_position_changed() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0xC04B, 7, origin());
        host.write(0xC04C, 13, origin());
        assert_eq!(host.read(0xC04B, origin()), 7);
        assert_eq!(host.read(0xC04C, origin()), 13);
        assert!(host.vm.position_changed);
        assert_eq!(host.state.pos, (7, 13));
    }

    #[test]
    fn facing_write_at_0xc04d_uses_the_halved_encoding_and_normalizes_mod_4() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0xC04D, 2, origin()); // halved 2 -> raw South (4)
        assert_eq!(host.state.facing, Facing::South);
        host.write(0xC04D, 5, origin()); // 5 % 4 = 1 -> raw East (2)
        assert_eq!(host.state.facing, Facing::East);
    }

    #[test]
    fn facing_reads_differ_between_0xc04d_halved_and_0x033d_raw() {
        let mut f = Fixture::new();
        f.state.facing = Facing::South; // raw 4, halved 2
        let mut host = f.host();
        assert_eq!(host.read(0xC04D, origin()), 2);
        assert_eq!(host.read(0x033D, origin()), 4);
    }

    #[test]
    fn dead_cells_b1_fb_fc_are_write_no_ops_and_always_read_zero() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0x00B1, 0xFFFF, origin());
        host.write(0x00FB, 0xFFFF, origin());
        host.write(0x00FC, 0xFFFF, origin());
        assert_eq!(host.read(0x00B1, origin()), 0);
        assert_eq!(host.read(0x00FB, origin()), 0);
        assert_eq!(host.read(0x00FC, origin()), 0);
    }

    #[test]
    fn wall_type_and_roof_are_read_only_through_scriptmemory() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0xC04E, 5, origin());
        host.write(0xC04F, 5, origin());
        // Both must stay whatever the GEO block says (0 on an all-open
        // fixture), never the attempted write.
        assert_eq!(host.read(0xC04E, origin()), 0);
        assert_eq!(host.read(0xC04F, origin()), 0);
    }

    #[test]
    fn in_dungeon_write_flips_game_state_only_on_actual_change() {
        let mut f = Fixture::new();
        f.state.game_state = GameState::DungeonMap;
        vm_init_ecl(&mut f.state, &mut f.vm); // the cell starts at 1, as it does in play
        let mut host = f.host();
        host.write(IN_DUNGEON_ADDR, 0, origin()); // -> WildernessMap
        assert_eq!(host.state.game_state, GameState::WildernessMap);
        assert_eq!(host.state.last_game_state, GameState::DungeonMap);
        // Writing the same value again must not touch last_game_state.
        host.state.last_game_state = GameState::DungeonMap; // poke to detect a spurious re-save
        host.write(IN_DUNGEON_ADDR, 0, origin());
        assert_eq!(host.state.last_game_state, GameState::DungeonMap);
    }

    /// ★ FD-37's asymmetry, made observable: `vm_init_ecl` pokes
    /// `area_ptr.inDungeon` to 1 **directly** (`ovr008.cs:126`), bypassing the
    /// hook that maintains `game_state` — so a block entered from the overland
    /// reads the cell as "dungeon" while `game_state` still says wilderness.
    /// That is what lets the new block's `LOAD FILES` 3D-map gate
    /// (`ovr003.cs:519-521`) fire on the way back indoors.
    #[test]
    fn vm_init_ecl_pokes_in_dungeon_without_touching_game_state() {
        let mut f = Fixture::new();
        // Block entry first: that is what makes the cell say 1 (`:126`).
        vm_init_ecl(&mut f.state, &mut f.vm);
        // Then walk out to the overland the way `ECL1#80 @0x8014` does.
        {
            let mut host = f.host();
            host.write(IN_DUNGEON_ADDR, 0, origin());
        }
        assert_eq!(f.state.game_state, GameState::WildernessMap);

        vm_init_ecl(&mut f.state, &mut f.vm);
        assert_eq!(
            f.state.game_state,
            GameState::WildernessMap,
            "the direct write must NOT run the game_state hook"
        );
        let mut host = f.host();
        assert_eq!(
            host.read(IN_DUNGEON_ADDR, origin()),
            1,
            "...but the cell a script (and LOAD FILES) reads says dungeon again"
        );
    }

    /// The rest of `vm_init_ecl`'s engine half (`ovr008.cs:98,109,111-113`).
    #[test]
    fn vm_init_ecl_resets_the_head_rest_schedule_icon_id_and_spell_gate() {
        let mut f = Fixture::new();
        f.state.head_block_id = 0x0A;
        f.state.can_cast_spells = true;
        f.state.pending_combat.monster_icon_id = 12;
        {
            let mut host = f.host();
            host.write(REST_INCOUNTER_PERIOD_ADDR, 0x1E, origin());
            host.write(REST_INCOUNTER_PERCENTAGE_ADDR, 0x0A, origin());
        }

        vm_init_ecl(&mut f.state, &mut f.vm);

        assert_eq!(f.state.head_block_id, 0xFF, ":109");
        assert!(!f.state.can_cast_spells, ":113");
        assert_eq!(f.state.pending_combat.monster_icon_id, 8, ":98");
        let mut host = f.host();
        assert_eq!(host.read(REST_INCOUNTER_PERIOD_ADDR, origin()), 0, ":111");
        assert_eq!(
            host.read(REST_INCOUNTER_PERCENTAGE_ADDR, origin()),
            0,
            ":112"
        );
    }

    /// ★ The `reload_ecl_and_pictures` arm (`ovr008.cs:128-131`): normal block
    /// entry wipes both script scratch tables; the one initialization that
    /// follows a save load does not — which is exactly how a loaded game keeps
    /// the per-block flags it was saved with.
    #[test]
    fn the_table_restores_are_skipped_only_when_reloading_from_a_save() {
        let seed = |f: &mut Fixture| {
            let mut host = f.host();
            host.write(FIELD_200_BASE, 7, origin());
            host.write(FIELD_200_BASE + FIELD_200_LEN - 1, 7, origin());
            host.write(FIELD_6F2_BASE, 7, origin());
            host.write(FIELD_6F2_BASE + FIELD_6F2_LEN - 1, 7, origin());
        };
        let read_all = |f: &mut Fixture| {
            let mut host = f.host();
            [
                host.read(FIELD_200_BASE, origin()),
                host.read(FIELD_200_BASE + FIELD_200_LEN - 1, origin()),
                host.read(FIELD_6F2_BASE, origin()),
                host.read(FIELD_6F2_BASE + FIELD_6F2_LEN - 1, origin()),
            ]
        };

        let mut f = Fixture::new();
        f.state.reload_ecl_and_pictures = false;
        seed(&mut f);
        vm_init_ecl(&mut f.state, &mut f.vm);
        assert_eq!(read_all(&mut f), [0, 0, 0, 0], "a NEWECL wipes both tables");

        let mut f = Fixture::new();
        f.state.reload_ecl_and_pictures = true;
        seed(&mut f);
        vm_init_ecl(&mut f.state, &mut f.vm);
        assert_eq!(
            read_all(&mut f),
            [7, 7, 7, 7],
            "the initialization right after a save load leaves them alone"
        );
    }

    /// The wipe covers the WHOLE of both tables — 33 words and 10, per
    /// `RestField200Values`' own `loop_var <= 32` and `RestField6F2Values`'
    /// ten assignments — and stops at their edges.
    #[test]
    fn the_table_restores_cover_exactly_their_documented_extents() {
        let mut f = Fixture::new();
        {
            let mut host = f.host();
            for i in 0..FIELD_200_LEN + 2 {
                host.write(FIELD_200_BASE + i, 9, origin());
            }
            for i in 0..FIELD_6F2_LEN + 2 {
                host.write(FIELD_6F2_BASE + i, 9, origin());
            }
        }
        vm_init_ecl(&mut f.state, &mut f.vm);
        let mut host = f.host();
        for i in 0..FIELD_200_LEN {
            assert_eq!(host.read(FIELD_200_BASE + i, origin()), 0, "field_200[{i}]");
        }
        assert_eq!(host.read(FIELD_200_BASE + FIELD_200_LEN, origin()), 9);
        for i in 0..FIELD_6F2_LEN {
            assert_eq!(host.read(FIELD_6F2_BASE + i, origin()), 0, "field_6F2[{i}]");
        }
        assert_eq!(host.read(FIELD_6F2_BASE + FIELD_6F2_LEN, origin()), 9);
    }

    #[test]
    fn clock_cells_reflect_game_clock_after_step_game_time() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.step_game_time(1, 100); // 100 units, normal rate
                                     // FD-31: the seven clock words are CONSECUTIVE (`0x4BC6..=0x4BCC`
                                     // under the halved Area mapping) — hour is `CLOCK_BASE + 3`.
        let hour_addr = CLOCK_BASE + 3;
        assert!(host.read(hour_addr, origin()) > 0 || host.read(CLOCK_BASE + 1, origin()) > 0);
    }

    /// ★ FD-31's live consequence: the real Tilverton entry script reads
    /// `0x4BC9` — the HOUR under the halved mapping. The stride-2 dispatch
    /// this replaced only answered even addresses, so this exact read fell
    /// to the raw store and time-of-day gates never saw the live clock.
    #[test]
    fn the_hour_cell_the_tilverton_script_reads_is_the_live_clock() {
        let mut f = Fixture::new();
        let mut host = f.host();
        // `step_game_time(1, N)` is N MINUTES (roll-credits slice 7's
        // correction): 100 of them is 1:40.
        host.step_game_time(1, 100);
        assert_eq!(host.read(0x4BC9, origin()), 1, "hour");
        assert_eq!(host.read(0x4BC7, origin()), 0, "minutes ones (40 % 10)");
        assert_eq!(host.read(0x4BC8, origin()), 4, "minutes tens");
    }

    /// ★ D-S1a: `SAVE <n>, 0x7F12` is `seg042.set_game_area`
    /// (`ovr008.cs:654-657` → `seg042.cs:124-128`) — backup ← live, live ←
    /// value — and the read side answers from the LIVE global
    /// (`get_player_values`' `arg_4 == 0x312` arm, `ovr008.cs:545-548`), never
    /// the raw store the rest of the Party window falls back to.
    #[test]
    fn writing_0x7f12_is_set_game_area_and_reading_it_returns_the_live_cell() {
        let mut f = Fixture::new();
        let mut host = f.host();
        assert_eq!(host.read(GAME_AREA_ADDR, origin()), GAME_AREA as u16);

        host.write(GAME_AREA_ADDR, 1, origin());
        assert_eq!(host.state.game_area, 1, "live cell takes the written value");
        assert_eq!(
            host.state.game_area_backup, GAME_AREA,
            "the previous live value is pushed to the backup shadow"
        );
        assert_eq!(host.read(GAME_AREA_ADDR, origin()), 1);

        // A second switch shifts the shadow again — it is a one-deep push,
        // not a stack.
        host.write(GAME_AREA_ADDR, 5, origin());
        assert_eq!((host.state.game_area, host.state.game_area_backup), (5, 1));

        // ...and `restore_game_area` pops it (`seg042.cs:131-134`).
        host.state.restore_game_area();
        assert_eq!(host.state.game_area, 1);
    }

    /// The area cell is NOT the raw-store round-trip every other Party-window
    /// address gets: a raw write at `0x7F12` would leave `game_area` alone and
    /// break every `{area}`-keyed load downstream, so the case must be reached
    /// before the window's fallback arm.
    #[test]
    fn the_game_area_cell_never_falls_through_to_the_raw_store() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(GAME_AREA_ADDR, 4, origin());
        assert_eq!(host.vm.raw_word(GAME_AREA_ADDR), None);
        assert!(
            host.vm.unknown_log.is_empty(),
            "a named cell is not an unknown access"
        );
    }

    /// D-S1b: the service surface reads `game_area` off the state at CALL
    /// TIME. `load_monster` names `MON{area}CHA.DAX` (`load_mob`,
    /// `ovr017.cs:826,830`), so a mid-run area switch must redirect it — the
    /// host holds no snapshot to go stale.
    #[test]
    fn load_monster_follows_a_mid_run_area_switch() {
        let data = GameData::from_files([(
            "MON5CHA.DAX".to_string(),
            crate::test_support::build_dax_file(&[(3, vec![0u8; 0x1A6])]),
        )]);
        let mut f = Fixture::with_data(data);
        let mut host = f.host();
        // Area 2 has no MON2CHA.DAX in this fixture set.
        assert!(host.load_monster(3, 1, 0).is_err());
        host.write(GAME_AREA_ADDR, 5, origin());
        assert!(
            host.load_monster(3, 1, 0).is_ok(),
            "after SAVE 5 -> 0x7F12 the same monster id resolves in MON5CHA.DAX"
        );
    }

    /// ★ FD-19's three cells, named at last. `lastXPos`/`lastYPos` are what
    /// `ECL2#1 @0x9444`/`@0x944B` copy back into the position to refuse the
    /// (7,12)-North entrance; `LastEclBlockId` is what every multi-entrance
    /// block's own entry vector branches on (`ECL1#80 @0x8024`).
    #[test]
    fn the_arrival_cells_read_engine_state_rather_than_the_raw_store() {
        let mut f = Fixture::new();
        f.state.last_pos = (7, 12);
        f.state.last_ecl_block_id = 0x30; // ECL5#48, the overland exit
        let mut host = f.host();
        assert_eq!(host.read(LAST_XPOS_ADDR, origin()), 7);
        assert_eq!(host.read(LAST_YPOS_ADDR, origin()), 12);
        assert_eq!(host.read(LAST_ECL_BLOCK_ID_ADDR, origin()), 0x30);
        assert!(
            host.vm.unknown_log.is_empty(),
            "named cells are not unknown accesses"
        );

        // All three are plain read/write in the original's own switch
        // (`Classes/Area1.cs:245-254`), so a script can move them.
        host.write(LAST_XPOS_ADDR, 3, origin());
        host.write(LAST_ECL_BLOCK_ID_ADDR, 2, origin());
        assert_eq!(host.state.last_pos.0, 3);
        assert_eq!(host.state.last_ecl_block_id, 2);
    }

    #[test]
    fn party_and_table_windows_go_to_the_raw_store_and_log_unknown_access() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0x7C10, 42, origin());
        assert_eq!(host.read(0x7C10, origin()), 42);
        host.write(0x7A10, 7, origin());
        assert_eq!(host.read(0x7A10, origin()), 7);
        assert_eq!(host.vm.unknown_log.entries().len(), 4); // write+read for each
    }

    #[test]
    fn unmatched_global_address_round_trips_via_the_raw_store() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0x2CB, 99, origin()); // SURPRISE's target — no case in the original
        assert_eq!(host.read(0x2CB, origin()), 99);
        assert_eq!(host.vm.unknown_log.entries().len(), 2);
    }

    #[test]
    fn move_position_forward_wraps_and_sets_position_changed() {
        let mut f = Fixture::new();
        f.state.pos = (15, 0);
        f.state.facing = Facing::East;
        let mut host = f.host();
        host.move_position_forward();
        assert_eq!(host.state.pos, (0, 0));
        assert!(host.vm.position_changed);
    }

    #[test]
    fn call_sound_variant_selects_by_word_1ee76() {
        let mut f = Fixture::new();
        {
            let mut host = f.host();
            host.write(0x03DE, 10, origin());
        }
        let mut host = f.host();
        assert_eq!(host.call_sound_variant(), 1);
    }

    #[test]
    fn service_calls_are_logged() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.clear_monsters();
        assert_eq!(host.vm.calls, vec![RecordedCall::ClearMonsters]);
    }
}
