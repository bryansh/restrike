//! ★ Roll-credits slice 1's acceptance suite (`docs/design/roll-credits.md`
//! §5): the area switch, end to end.
//!
//! The mechanism under test is two instructions long and has no choreography
//! of its own (D-RC1): a script writes the destination area with
//! `SAVE <n>, 0x7F12` (`seg042.set_game_area` via `alter_character`'s `0x312`
//! case, `ovr008.cs:654-657`) and immediately chains with `NEWECL <block>`,
//! whose `load_ecl_dax` interpolates the *new* `gbl.game_area` into the file
//! name at call time (`ovr008.cs:148`). The destination block's own `0x8014`
//! entry vector then opens with `LOAD FILES`/`LOAD PIECES` naming its assets,
//! so every transition carries its own map and wallsets.
//!
//! Three tiers, per the door's acceptance list:
//!
//! 1. **Synthetic** (CI): a two-area fixture proving the whole idiom —
//!    assets swapped, block resident, `vm_init_ecl`'s resets applied.
//! 2. **Loud-halt regression** (CI): an unresolvable cross-file NEWECL still
//!    halts loudly, and a resolvable one never half-transitions.
//! 3. **Real data** (local tier, loud-skip without `GBX_DATA_DIR`): the FD-19
//!    door walked live, and `ECL5#48 @0x8092`'s overland exit driven to
//!    `ECL1#80`'s arrival menu with the right resident block.

#![cfg(test)]

use crate::engine::Engine;
use crate::shell::Shell;
use crate::symbols::SymbolSets;
use crate::test_support::{build_dax_file, ecl_dax_block, labeled_block};
use gbx_formats::game_data::GameData;
use gbx_formats::geo::{GeoBlock, GEO_BLOCK_SIZE};
use gbx_vm::test_support::EclBuilder;

/// The Party-window address `SAVE` writes to switch areas.
const GAME_AREA_ADDR: u16 = 0x7F12;

/// The area the fixture starts in (the engine's boot default) and the one it
/// crosses to.
const FROM_AREA: u8 = 2;
const TO_AREA: u8 = 4;
/// The destination block id — deliberately one that also exists in the source
/// area's file with *different* content, so a transition that forgot to swap
/// files would load the wrong block and be caught rather than silently pass.
const DEST_BLOCK: u8 = 7;

/// A GEO block whose square `(0, 0)` carries a recognisable marker: the
/// `x2` byte's low seven bits. Lets a test say "the resident map is *this*
/// one" without shipping real data (D10).
fn marked_geo(marker: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; GEO_BLOCK_SIZE];
    // Plane 2 (`x2`), index `x + 16 * y`, after the 2-byte header.
    bytes[2 + 2 * 256] = marker & 0x7F;
    bytes
}

fn geo_marker(geo: &GeoBlock) -> u8 {
    geo.square(0, 0).low7
}

/// One walldef block holding a single wallset whose every tile id is
/// `marker` — the same trick as [`marked_geo`], for the `LOAD PIECES` half.
fn marked_walldef(marker: u8) -> Vec<u8> {
    vec![marker; gbx_formats::walldef::WALLSET_SIZE]
}

/// A 1-item 8×8 block whose pixels are all `marker`.
fn marked_8x8(marker: u8) -> Vec<u8> {
    // `gbx_formats::image` header: height, width_cols, x_pos, y_pos,
    // field_9[8], then 4bpp planar data. The decoder is exercised elsewhere;
    // here we only need something that parses.
    let mut bytes = vec![8u8, 1, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.extend(std::iter::repeat_n(marker, 8 * 4));
    bytes
}

/// One area's contribution to [`two_area_game_data`]: its number, the marker
/// byte stamped through its GEO/WALLDEF/8X8D blocks, and its ECL blocks.
type AreaFixture = (u8, u8, Vec<(u8, EclBuilder)>);

/// Builds a `GameData` carrying two whole areas: `ECL{n}.DAX`, `GEO{n}.DAX`,
/// `WALLDEF{n}.DAX` and `8X8D{n}.DAX` for each, plus the boot files a
/// fixture engine needs.
fn two_area_game_data(areas: Vec<AreaFixture>) -> GameData {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (area, marker, blocks) in areas {
        let ecl_blocks: Vec<(u8, Vec<u8>)> = blocks
            .iter()
            .map(|(id, b)| (*id, ecl_dax_block(&b.build_bytes())))
            .collect();
        files.push((format!("ECL{area}.DAX"), build_dax_file(&ecl_blocks)));
        files.push((
            format!("GEO{area}.DAX"),
            build_dax_file(&[(1, marked_geo(marker))]),
        ));
        files.push((
            format!("WALLDEF{area}.DAX"),
            build_dax_file(&[(1, marked_walldef(marker))]),
        ));
        files.push((
            format!("8X8D{area}.DAX"),
            build_dax_file(&[(1, marked_8x8(marker))]),
        ));
    }
    GameData::from_files(files)
}

fn synthetic_set4() -> gbx_formats::image::ImageBlock {
    gbx_formats::image::ImageBlock {
        height: 8,
        width_cols: 1,
        x_pos: 0,
        y_pos: 0,
        field_9: [0; 8],
        items: (0..40)
            .map(|i| gbx_formats::image::DecodedItem {
                pixels: vec![(i % 16) as u8; 64],
            })
            .collect(),
    }
}

fn synthetic_font() -> gbx_formats::font::Font {
    let mut data =
        Vec::with_capacity(gbx_formats::font::GLYPH_COUNT * gbx_formats::font::GLYPH_BYTES);
    for j in 0..gbx_formats::font::GLYPH_COUNT {
        data.extend_from_slice(&[j as u8; gbx_formats::font::GLYPH_BYTES]);
    }
    gbx_formats::font::decode(&data)
}

fn fixture_engine(data: GameData, geo_marker: u8) -> Engine {
    let mut sets = SymbolSets::new();
    sets.load(4, synthetic_set4());
    let geo = GeoBlock::parse(&marked_geo(geo_marker)).unwrap();
    Engine::new_fixture(synthetic_font(), sets, geo, data, 1)
}

fn tick_until_world_menu(engine: &mut Engine, max: u32) {
    for _ in 0..max {
        if matches!(engine.shell(), Shell::WorldMenu { .. }) {
            return;
        }
        engine.tick(&[]);
    }
    panic!("the fixture never reached the world menu");
}

// --- Tier 1: the synthetic two-area fixture ---

/// ★ **The door's acceptance item 1.** `SAVE 4, 0x7F12` + `NEWECL 7`, end to
/// end: the live area moves, the destination block loads out of the *other*
/// file, its own `LOAD FILES`/`LOAD PIECES` swap the resident map and
/// wallset, and `vm_init_ecl`'s resets land.
///
/// The block id (7) exists in both areas with different bodies, so a
/// transition that changed `game_area` but resolved the block against the old
/// file would run the wrong script and fail here rather than pass quietly.
#[test]
fn a_save_to_0x7f12_then_a_newecl_crosses_files_and_swaps_the_assets() {
    let from_block_1 = labeled_block(["entry"; 5], |b| {
        b.label("entry");
        b.op(0x09).imm_byte(TO_AREA).imm_word(GAME_AREA_ADDR); // SAVE 4 -> 0x7F12
        b.op(0x20).imm_byte(DEST_BLOCK); // NEWECL 7
        b.op(0x00);
    });
    // The decoy: same id, in the SOURCE area's file. If the chain resolves
    // against the old area this runs instead, and the assertions below fail.
    let from_block_7 = labeled_block(["entry"; 5], |b| {
        b.label("entry");
        b.op(0x09).imm_byte(0xEE).imm_word(0x7A10); // a marker in the Table window
        b.op(0x00);
    });
    let to_block_7 = labeled_block(["entry"; 5], |b| {
        b.label("entry");
        // `LOAD FILES 1, 1, 0xFF` — 3D map block 1 of the new area.
        b.op(0x21).imm_byte(1).imm_byte(1).imm_byte(0xFF);
        // `LOAD PIECES 1, 0xFF, 0xFF` — wallset 1 from the new area's WALLDEF.
        b.op(0x37).imm_byte(1).imm_byte(0xFF).imm_byte(0xFF);
        b.op(0x00);
    });

    let data = two_area_game_data(vec![
        (FROM_AREA, 0x11, vec![(1, from_block_1), (7, from_block_7)]),
        (TO_AREA, 0x22, vec![(DEST_BLOCK, to_block_7)]),
    ]);
    let mut engine = fixture_engine(data, 0x11);

    // Arm the FD-37 cells so the destination block's `vm_init_ecl` is visibly
    // the thing that clears them.
    engine.state.head_block_id = 0x0A;
    engine.state.can_cast_spells = true;

    tick_until_world_menu(&mut engine, 400);

    assert_eq!(
        engine.state().game_area,
        TO_AREA,
        "SAVE -> 0x7F12 moved the live area"
    );
    assert_eq!(
        engine.state().game_area_backup,
        FROM_AREA,
        "set_game_area pushed the old value to the shadow (seg042.cs:126)"
    );
    assert_eq!(
        engine.state().ecl_block_id,
        DEST_BLOCK,
        "the destination block is resident"
    );
    assert_eq!(
        engine.state().last_ecl_block_id,
        DEST_BLOCK,
        "and committed once its entry vector ended (ovr003.cs:2292-2294)"
    );
    assert_eq!(
        engine.vm_memory().raw_word(0x7A10),
        None,
        "the same-id decoy block in the SOURCE area must never have run"
    );

    // The destination block's own LOAD FILES swapped the resident map...
    assert_eq!(
        geo_marker(engine.geo()),
        0x22,
        "Load3DMap replaced the resident GeoBlock from GEO{TO_AREA}.DAX"
    );
    assert_eq!(engine.vm_memory().assets.map_3d_block, Some(1));
    // ...and LOAD PIECES pulled the new area's wallset.
    assert_eq!(engine.vm_memory().assets.walldefs[0], Some((1, 1)));
    assert!(
        engine.symbol_sets().wallset(0).is_some(),
        "the destination area's wallset is loaded"
    );

    // `vm_init_ecl`'s engine half ran at the chain (`ovr003.cs:491-492`).
    assert_eq!(engine.state().head_block_id, 0xFF, "ovr008.cs:109");
    assert!(!engine.state().can_cast_spells, "ovr008.cs:113");
    assert_eq!(
        engine.vm_memory().raw_word(0x4BE6),
        Some(1),
        "the direct inDungeon poke, ovr008.cs:126"
    );
}

/// The area cell is readable by scripts as well as writable — `SAVE` it, then
/// `COMPARE` it back (`get_player_values`' `0x312` arm, `ovr008.cs:545-548`).
#[test]
fn a_script_reads_back_the_area_it_just_wrote() {
    let block = labeled_block(["entry"; 5], |b| {
        b.label("entry");
        b.op(0x09).imm_byte(5).imm_word(GAME_AREA_ADDR); // SAVE 5 -> area
        b.op(0x09).mem(GAME_AREA_ADDR).imm_word(0x7A20); // SAVE [area] -> table cell
        b.op(0x00);
    });
    let data = two_area_game_data(vec![(FROM_AREA, 0x11, vec![(1, block)])]);
    let mut engine = fixture_engine(data, 0x11);
    tick_until_world_menu(&mut engine, 400);
    assert_eq!(engine.state().game_area, 5);
    assert_eq!(
        engine.vm_memory().raw_word(0x7A20),
        Some(5),
        "the read side answers from the live cell, not the raw store"
    );
}

// --- Tier 2: the loud-halt regression ---

/// ★ **The door's acceptance item 3.** A cross-file NEWECL that cannot
/// resolve must halt LOUDLY — this was the failure mode D-RC1 called "the
/// worst case for the playthrough loop": the `0x7F12` write raw-logged, the
/// chain quietly failed, and the flow carried on as if nothing had happened.
///
/// It must also not half-transition: `game_area` moves (the write really did
/// happen, and the original's does too), but the resident block does not.
#[test]
fn an_unresolvable_cross_file_newecl_halts_loudly_and_leaves_the_block_alone() {
    let block = labeled_block(["entry"; 5], |b| {
        b.label("entry");
        b.op(0x09).imm_byte(6).imm_word(GAME_AREA_ADDR); // SAVE 6 -> an area with no file
        b.op(0x20).imm_byte(DEST_BLOCK);
        b.op(0x00);
    });
    let data = two_area_game_data(vec![(FROM_AREA, 0x11, vec![(1, block)])]);
    let mut engine = fixture_engine(data, 0x11);
    tick_until_world_menu(&mut engine, 400);

    let halts = &engine.vm_memory().halts;
    assert!(
        halts.iter().any(|h| h.description.contains("NEWECL")),
        "an unresolvable chain must be a counted, described halt, not silence: {halts:?}"
    );
    assert_eq!(
        engine.state().game_area,
        6,
        "the write itself still happened"
    );
    assert_eq!(
        engine.state().ecl_block_id,
        DEST_BLOCK,
        "LastEclBlockId/EclBlockId commit before the load, as in CMD_NewECL \
         (ovr003.cs:488-491)"
    );
    assert!(
        !engine.state().chained,
        "a failed chain must clear `chained` rather than stranding the flow"
    );
    assert_eq!(
        geo_marker(engine.geo()),
        0x11,
        "and the resident map is untouched — no half-transition"
    );
}

/// A resolvable chain leaves no halt behind at all — the other half of the
/// same acceptance item.
#[test]
fn a_resolvable_cross_file_newecl_records_no_halt() {
    let from_block_1 = labeled_block(["entry"; 5], |b| {
        b.label("entry");
        b.op(0x09).imm_byte(TO_AREA).imm_word(GAME_AREA_ADDR);
        b.op(0x20).imm_byte(DEST_BLOCK);
        b.op(0x00);
    });
    let to_block = labeled_block(["entry"; 5], |b| {
        b.label("entry");
        b.op(0x00);
    });
    let data = two_area_game_data(vec![
        (FROM_AREA, 0x11, vec![(1, from_block_1)]),
        (TO_AREA, 0x22, vec![(DEST_BLOCK, to_block)]),
    ]);
    let mut engine = fixture_engine(data, 0x11);
    tick_until_world_menu(&mut engine, 400);
    assert!(
        engine.vm_memory().halts.is_empty(),
        "a resolvable transition is silent: {:?}",
        engine.vm_memory().halts
    );
}

// --- Tier 3: real CotAB data (local tier) ---

/// Hand-authors a `savgam?.dat` that parks the engine in `game_area`, resident
/// on ECL block `ecl_block` with GEO block `geo_block`, one party member. The
/// bytes are all self-authored (D10) — only the *game data* it is imported
/// against is real.
/// The party's `(mapPosX, mapPosY, mapDirection)` is a parameter because
/// roll-credits slice 9d needs to stand it on one specific square; `(7, 7, 0)`
/// is what every earlier caller used.
pub(crate) fn master_bytes_at(
    game_area: u8,
    ecl_block: u8,
    geo_block: u8,
    in_dungeon: bool,
    (pos_x, pos_y, dir): (u8, u8, u8),
) -> Vec<u8> {
    use gbx_formats::save_orig::SAVGAM_SIZE;
    let mut buf = vec![0u8; SAVGAM_SIZE];
    let mut off = 0usize;
    buf[off] = game_area; // section 1: gbl.game_area (ovr017.cs:1150)
    off += 1;

    let area = &mut buf[off..off + 0x800];
    area[0x18A] = geo_block; // current_3DMap_block_id
    area[0x1E4..0x1E6].copy_from_slice(&(ecl_block as u16).to_le_bytes()); // LastEclBlockId
    area[0x1CC..0x1CE].copy_from_slice(&u16::from(in_dungeon).to_le_bytes());
    off += 0x800;
    off += 0x800; // area2_ptr
    off += 0x400; // stru_1b2ca
    off += 0x1E00; // ecl_ptr (discarded on import)

    buf[off] = pos_x; // mapPosX
    buf[off + 1] = pos_y; // mapPosY
    buf[off + 2] = dir; // mapDirection
    off += 5;
    off += 1; // last_game_state
    off += 1; // game_state
    off += 12; // set_blocks: none

    buf[off] = 1; // party_count
    off += 1;
    let names = &mut buf[off..off + 0x148];
    names[0] = 0x29;
    names[1..1 + 9].copy_from_slice(b"CHRDATA1\0");
    buf
}

pub(crate) fn char_bytes() -> Vec<u8> {
    let mut buf = vec![0u8; gbx_formats::save_orig::CHAR_RECORD_SIZE];
    let name = b"WALKER";
    buf[0] = name.len() as u8;
    buf[1..1 + name.len()].copy_from_slice(name);
    buf[0x74] = 7; // race
    buf[0x78] = 20; // hp max
    buf[0x1a4] = 20; // hp current
    buf
}

/// Imports the synthetic save against REAL CotAB data — the vehicle for
/// starting a drive anywhere in the shipped content.
pub(crate) fn real_data_engine(
    game_area: u8,
    ecl_block: u8,
    geo_block: u8,
    in_dungeon: bool,
) -> Option<Engine> {
    real_data_engine_at(game_area, ecl_block, geo_block, in_dungeon, (7, 7, 0))
}

/// [`real_data_engine`] with the party standing somewhere specific.
pub(crate) fn real_data_engine_at(
    game_area: u8,
    ecl_block: u8,
    geo_block: u8,
    in_dungeon: bool,
    pos: (u8, u8, u8),
) -> Option<Engine> {
    let dir = std::env::var_os("GBX_DATA_DIR")?;
    let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir))
        .expect("GBX_DATA_DIR must be readable");
    let master = master_bytes_at(game_area, ecl_block, geo_block, in_dungeon, pos);
    let chars = char_bytes();
    let set = gbx_formats::save_orig::load_from_lookup(&master, 'A', |name| {
        if name == "CHRDATA1.SAV" {
            Some(chars.as_slice())
        } else {
            None
        }
    })
    .expect("the synthetic save set must parse");
    Some(crate::import::import_original(&set, data, 0x5A1E_5A1E).expect("import must succeed"))
}

/// ★ **The door's acceptance item 2, second half.** `ECL5#48 @0x8092`'s
/// overland exit driven against real data: `SAVE 1, 0x7F12` then
/// `NEWECL 0x50`, arriving in `ECL1` block 80 — a different FILE, with the
/// destination's own assets on screen.
///
/// The drive starts at `0x8086` (the tail of the exit sequence) rather than
/// the block's entry vector, because everything upstream of it is roll-credits
/// slice 3's opcodes — `FIND ITEM 0x5E` at `@0x8018`, the `LOAD CHARACTER`
/// slot scan at `@0x80A1`, `DESTROY ITEMS` at `@0x8210`. From `0x8086` on, the
/// sequence is exactly what the disassembly shows:
///
/// ```text
/// @0x8086  SAVE #0x00, [0x4BFB]   ; block_area_view = 0 (no area map out here)
/// @0x808C  SAVE #0xFF, [0x7EE1]   ; HeadBlockId = none
/// @0x8092  SAVE #0x01, [0x7F12]   ; >>> game_area = 1 <<<
/// @0x8098  NEWECL #0x50           ; ECL1.DAX block 80
/// ```
///
/// and `ECL1#80`'s own entry vector (`@0x8014`) then declares the overland:
/// `SAVE 0, [0x4BE6]` (inDungeon = 0 → `WildernessMap`),
/// `LOAD FILES 0x7F,0x7F,0x7F` (no 3D map; `load_bigpic(0x79)`),
/// `PICTURE 0x79`, and `COMPARE [0x4BF2], #0x30` — the arrival branch that
/// asks which door the party came through, `0x30` being this very block, 48.
#[test]
fn the_ecl5_overland_exit_crosses_into_ecl1_block_80_on_real_data() {
    // GEO block ids are partitioned per area exactly as ECL's are — `GEO5.DAX`
    // holds {50, 51, 53}, so 50 is this area's own map, not a borrowed id.
    let Some(mut engine) = real_data_engine(5, 48, 50, true) else {
        eprintln!(
            "SKIPPED: local tier needs GBX_DATA_DIR \
             (area_transition_tests::the_ecl5_overland_exit_crosses_into_ecl1_block_80_on_real_data)"
        );
        return;
    };

    assert_eq!(engine.state().game_area, 5, "the import honoured the save");
    assert_eq!(engine.state().ecl_block_id, 48);

    engine.shell = crate::shell::boot_at_address(&mut engine.machine, 0x8086);
    // Stop at ARRIVAL — the door's own bar for this slice. `ECL1#80`'s entry
    // vector reaches its arrival branch within a dozen instructions and then
    // walks on into the overland's own content (menus, `MapCursor`, an
    // unimplemented `CLEAR BOX` at `0x85C4`), all of which is roll-credits
    // slice 7's. `[0x4C9C] == 9` is the branch's own footprint: `@0x8038
    // COMPARE [0x4BF2], #0x30` — "did we come through block 48?" — followed by
    // `@0x803F SAVE #0x09, [0x4C9C]`.
    let arrived = run_until(&mut engine, 2000, |e| {
        e.vm_memory().raw_word(0x4C9C) == Some(9)
    });

    assert!(
        arrived,
        "the arrival branch never ran; halts: {:?}",
        engine.vm_memory().halts
    );
    assert_eq!(
        engine.state().game_area,
        1,
        "SAVE 1 -> 0x7F12 put the party in the overland area"
    );
    assert_eq!(
        engine.state().game_area_backup,
        5,
        "with area 5 pushed to the backup shadow"
    );
    assert_eq!(
        engine.state().ecl_block_id,
        0x50,
        "and ECL1.DAX block 80 is the resident block — a different FILE, \
         reached because load_ecl_dax read the area at call time"
    );
    // The `[0x4C9C] == 9` we waited on IS the LastEclBlockId proof: only
    // `@0x8038 COMPARE [0x4BF2], #0x30` reaching its `@0x803F SAVE #0x09`
    // could have written it, and `0x4BF2` answered 48 because the cell is
    // named now. By the time the assertion runs, the destination's entry
    // vector has finished and re-committed `LastEclBlockId` to its own id —
    // `sub_29677:2196-2199`, faithful.
    assert_eq!(engine.state().last_ecl_block_id, 0x50);
    assert_eq!(
        engine.state().game_state,
        crate::shell::GameState::WildernessMap,
        "ECL1#80's entry vector declares the overland (SAVE 0, [0x4BE6])"
    );
    assert_eq!(
        engine.state().picture.bigpic_block,
        Some(0x79),
        "PICTURE 0x79 put BIGPIC1 block 0x79 on the viewport — area 1's own art"
    );
    assert!(
        engine.vm_memory().halts.is_empty(),
        "the crossing itself must be halt-free: {:?}",
        engine.vm_memory().halts
    );
}

/// Ticks until `done`, feeding Enter so a script menu never stalls the drive.
/// Returns whether `done` was reached.
pub(crate) fn run_until(engine: &mut Engine, max: u32, done: impl Fn(&Engine) -> bool) -> bool {
    for _ in 0..max {
        if done(engine) {
            return true;
        }
        engine.tick(&[crate::input::InputEvent::Enter]);
    }
    done(engine)
}

/// The same crossing's other end: `ECL4#37 @0x8225` is the second (and last)
/// `0x7F12` site in the shipped game, and it chains to the same block.
#[test]
fn the_ecl4_overland_exit_crosses_into_the_same_block() {
    let Some(mut engine) = real_data_engine(4, 37, 37, true) else {
        eprintln!(
            "SKIPPED: local tier needs GBX_DATA_DIR \
             (area_transition_tests::the_ecl4_overland_exit_crosses_into_the_same_block)"
        );
        return;
    };

    engine.shell = crate::shell::boot_at_address(&mut engine.machine, 0x8225);
    // 37 is not one of the four ids `ECL1#80`'s arrival branch tests
    // (`0x51`/`0x30`/`0x40`/`0x04`), so this crossing has no `[0x4C9C]`
    // footprint to wait on — the bar is the resident block plus the overland's
    // own bigpic.
    let arrived = run_until(&mut engine, 2000, |e| {
        e.state().picture.bigpic_block == Some(0x79)
    });

    assert!(arrived, "halts: {:?}", engine.vm_memory().halts);
    assert_eq!(engine.state().game_area, 1);
    assert_eq!(engine.state().game_area_backup, 4);
    assert_eq!(engine.state().ecl_block_id, 0x50);
    assert_eq!(
        engine.state().game_state,
        crate::shell::GameState::WildernessMap
    );
    assert!(
        engine.vm_memory().halts.is_empty(),
        "{:?}",
        engine.vm_memory().halts
    );
}

/// ★ **A real map swap on real data** — the M2 gap FD-19 recorded, closed.
///
/// `Load3DMap` recorded a block id and left the resident `GeoBlock` alone from
/// M2 until this slice, so any `LOAD FILES` naming a different map left the
/// party walking the old one's geometry. Here the import deliberately seats
/// the WRONG map (`GEO3` block 16) under `ECL3` block 21, whose entry vector
/// opens `LOAD FILES #0x15, #0x02, #0xFF` — 0x15 is 21, this block's own map.
/// The resident geometry must actually become that other block's.
///
/// (`GEO3.DAX` holds exactly {16, 17, 21}: the id namespace is partitioned
/// per area across every asset family, which is the whole reason `game_area`
/// had to become state.)
#[test]
fn a_real_load_files_swaps_the_resident_geo_block() {
    let Some(engine) = real_data_engine(3, 21, 16, true) else {
        eprintln!(
            "SKIPPED: local tier needs GBX_DATA_DIR \
             (area_transition_tests::a_real_load_files_swaps_the_resident_geo_block)"
        );
        return;
    };
    let wrong_map = geo_squares(&engine);

    // The same area/block, but seated on its OWN map from the start.
    let Some(reference) = real_data_engine(3, 21, 21, true) else {
        return;
    };
    let right_map = geo_squares(&reference);
    assert_ne!(
        wrong_map, right_map,
        "the fixture only means anything if blocks 16 and 21 differ"
    );

    // Now let block 21's own entry vector run.
    let mut engine = engine;
    for _ in 0..2000 {
        if matches!(engine.shell(), Shell::WorldMenu { .. }) {
            break;
        }
        engine.tick(&[crate::input::InputEvent::Enter]);
    }

    assert_eq!(
        engine.vm_memory().assets.map_3d_block,
        Some(21),
        "LOAD FILES #0x15 named block 21"
    );
    assert_eq!(
        geo_squares(&engine),
        right_map,
        "and the resident GeoBlock IS that map now — not merely recorded"
    );
    assert!(
        engine.vm_memory().halts.is_empty(),
        "{:?}",
        engine.vm_memory().halts
    );
}

/// Every square's `x2` low bits — a cheap whole-map fingerprint.
fn geo_squares(engine: &Engine) -> Vec<u8> {
    (0..16)
        .flat_map(|y| (0..16).map(move |x| (x, y)))
        .map(|(x, y)| engine.geo().square(x, y).low7)
        .collect()
}

/// `Load3DMap`'s own failure is loud too (`ovr031.cs:697-700` is a hard
/// `LogAndExit`) — and leaves the resident map alone rather than blanking it.
#[test]
fn a_load_files_naming_a_missing_geo_block_halts_and_keeps_the_old_map() {
    let block = labeled_block(["entry"; 5], |b| {
        b.label("entry");
        b.op(0x21).imm_byte(9).imm_byte(1).imm_byte(0xFF); // GEO block 9: absent
        b.op(0x00);
    });
    let data = two_area_game_data(vec![(FROM_AREA, 0x11, vec![(1, block)])]);
    let mut engine = fixture_engine(data, 0x11);
    tick_until_world_menu(&mut engine, 400);

    assert!(
        engine
            .vm_memory()
            .halts
            .iter()
            .any(|h| h.description.contains("Load3DMap")),
        "a missing GEO block must be reported: {:?}",
        engine.vm_memory().halts
    );
    assert_eq!(geo_marker(engine.geo()), 0x11);
    assert_eq!(
        engine.vm_memory().assets.map_3d_block,
        None,
        "a failed load records no resident id"
    );
}
