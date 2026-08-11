//! D-SAVE10 tier-1 synthetic-fixture tests (task deliverable 5): a
//! hand-authored, boot-compatible [`gbx_formats::game_data::GameData`] plus
//! a hand-authored `savgam?.dat`/`CHRDAT` set, tying together every prior
//! deliverable end to end — `import_original` → `Engine::save` →
//! `Engine::restore` → `Engine::save` byte-identity, the committed golden
//! SHA-256, and a version-mismatch rejection. D10-clean: every byte here is
//! self-authored structural data, no extracted game content.
//!
//! Also hosts the D-SAVE10 tier-2 local real-save test (loud-skip when
//! `GBX_DATA_DIR` holds no `savgam?.dat`).

use crate::engine::{Engine, GAME_AREA};
use crate::import::import_original;
use gbx_formats::game_data::GameData;
use gbx_formats::save_orig::{OriginalSaveSet, SAVGAM_SIZE};
use gbx_vm::test_support::EclBuilder;

const GEO_BLOCK_ID: u8 = 1;
const ECL_BLOCK_ID: u8 = 5;
const SAVE_SLOT: char = 'A';

/// A multi-block DAX archive, hand-encoded (mirrors `gbx-formats`'
/// `dax.rs`/`game_data.rs` test helpers' single-block builder, generalized
/// to N blocks so one file can hold everything `boot()`/`ECL`/`GEO` need).
fn build_dax(blocks: &[(u8, &[u8])]) -> Vec<u8> {
    fn rle_compress(raw: &[u8]) -> Vec<u8> {
        raw.chunks(128)
            .flat_map(|chunk| {
                let mut v = vec![(chunk.len() - 1) as u8];
                v.extend_from_slice(chunk);
                v
            })
            .collect()
    }

    let header_bytes = (blocks.len() * 9) as u16;
    let mut data_area = Vec::new();
    let mut entries = Vec::new();
    for &(id, raw) in blocks {
        let comp = rle_compress(raw);
        entries.push((
            id,
            data_area.len() as u32,
            raw.len() as u16,
            comp.len() as u16,
        ));
        data_area.extend_from_slice(&comp);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&header_bytes.to_le_bytes());
    for (id, offset, raw_size, comp_size) in entries {
        out.push(id);
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&raw_size.to_le_bytes());
        out.extend_from_slice(&comp_size.to_le_bytes());
    }
    out.extend_from_slice(&data_area);
    out
}

/// A minimal valid 4bpp image block (`image.rs`'s layout): 8×1 pixels per
/// item, `item_count` items — just enough for `boot()`'s decoder and
/// `crate::frames::draw8x8_03`'s symbol-index lookups (which need set 4
/// populated up to its highest referenced index, matching `engine.rs`'s
/// own `synthetic_set4` test fixture's 40-item shape) to succeed.
fn tiny_image_bytes(item_count: u8) -> Vec<u8> {
    let mut b = vec![0u8; 17];
    b[0..2].copy_from_slice(&1u16.to_le_bytes()); // height = 1
    b[2..4].copy_from_slice(&1u16.to_le_bytes()); // width_cols = 1
    b[8] = item_count;
    for _ in 0..item_count {
        b.extend_from_slice(&[0x12, 0x34, 0x56, 0x78]); // 1 row * 4 bytes packed nibbles
    }
    b
}

/// A minimal valid combat-icon block: one 24×24 item filled with `fill`
/// (`gbx_formats::combat_art::decode_icon`'s shape requirement — exactly one
/// item, non-zero dimensions).
fn tiny_icon_bytes(fill: u8) -> Vec<u8> {
    use gbx_formats::combat_art::CELL_PX;
    let mut b = vec![0u8; 17];
    b[0..2].copy_from_slice(&(CELL_PX as u16).to_le_bytes()); // height = 24
    b[2..4].copy_from_slice(&3u16.to_le_bytes()); // width_cols = 3 -> 24 px
    b[8] = 1; // item_count
    b.extend(std::iter::repeat_n(
        (fill << 4) | fill,
        CELL_PX * CELL_PX / 2,
    ));
    b
}

fn tiny_font_bytes() -> Vec<u8> {
    vec![0u8; gbx_formats::font::GLYPH_COUNT * gbx_formats::font::GLYPH_BYTES]
}

/// A minimal, real, resolvable-header EXIT-only script (mirrors
/// `engine.rs`'s own `exit_only_game_data` test helper).
fn exit_only_ecl_block() -> Vec<u8> {
    let mut b = EclBuilder::new();
    for _ in 0..5 {
        b.raw(&[0]);
        b.imm_word_label("entry");
    }
    b.label("entry");
    b.op(0x00); // EXIT
    let bytecode = b.build_bytes();

    let mut raw = vec![0u8, 0u8]; // load_ecl_dax's 2-byte prefix
    raw.extend_from_slice(&bytecode);
    raw
}

/// The amnesia intro's shape in miniature: an entry vector whose first op is
/// `CALL 0xAE11` (the consolidated redraw gate, `ovr003.cs:1843-1866`), then
/// EXIT. `0x2D`'s operand is the CALL key biased by `0x7FFF`, matching
/// `gbx_vm`'s own conformance fixtures.
fn call_ae11_ecl_block() -> Vec<u8> {
    let mut b = EclBuilder::new();
    for _ in 0..5 {
        b.raw(&[0]);
        b.imm_word_label("entry");
    }
    b.label("entry");
    b.op(0x2D).imm_word(0x7FFFu16.wrapping_add(0xAE11));
    b.op(0x00); // EXIT
    let bytecode = b.build_bytes();

    let mut raw = vec![0u8, 0u8]; // load_ecl_dax's 2-byte prefix
    raw.extend_from_slice(&bytecode);
    raw
}

/// Builds the full boot-compatible synthetic [`GameData`] (8X8D1.DAX's
/// font/set4/set0, COMSPR.DAX's thirteen icon pairs, SKY.DAX's 3 blocks,
/// `GEO{GAME_AREA}.DAX` block [`GEO_BLOCK_ID`], `ECL{GAME_AREA}.DAX` block
/// [`ECL_BLOCK_ID`]) — D10: every byte here is self-authored, no extracted
/// game content.
fn synthetic_game_data() -> GameData {
    synthetic_game_data_with_ecl(exit_only_ecl_block())
}

fn synthetic_game_data_with_ecl(ecl_block: Vec<u8>) -> GameData {
    synthetic_game_data_for_area(GAME_AREA, ecl_block)
}

/// The same fixture set, but with its `ECL`/`GEO` files named for `area` —
/// the seam the area-generalization slice needs, since "which files does this
/// save resolve against" is now a runtime question (D-S1c).
fn synthetic_game_data_for_area(area: u8, ecl_block: Vec<u8>) -> GameData {
    use gbx_formats::combat_art::ATTACK_BLOCK_OFFSET;

    let set4 = tiny_image_bytes(40); // draw8x8_03 indexes up to set 4's higher items
    let set0 = tiny_image_bytes(40);
    let sky_image = tiny_image_bytes(1);
    let font = tiny_font_bytes();
    let eight_by_eight_d1 = build_dax(&[(201, &font), (0xCA, &set4), (0xCB, &set0)]);
    let sky = build_dax(&[(250, &sky_image), (251, &sky_image), (252, &sky_image)]);
    let geo = build_dax(&[(GEO_BLOCK_ID, &vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE])]);
    let ecl = build_dax(&[(ECL_BLOCK_ID, &ecl_block)]);

    // Boot's COMSPR slice: blocks 0..=0x0B and 0x19, each with its +0x80
    // Attack twin (`combat_art::load_comspr_icons`).
    let icon_bytes: Vec<(u8, Vec<u8>)> = (0..=0x0Bu8)
        .chain(std::iter::once(0x19))
        .flat_map(|id| {
            [
                (id, tiny_icon_bytes(1)),
                (id + ATTACK_BLOCK_OFFSET, tiny_icon_bytes(2)),
            ]
        })
        .collect();
    let comspr_refs: Vec<(u8, &[u8])> = icon_bytes.iter().map(|(id, b)| (*id, &b[..])).collect();
    let comspr = build_dax(&comspr_refs);

    GameData::from_files([
        ("8X8D1.DAX".to_string(), eight_by_eight_d1),
        ("COMSPR.DAX".to_string(), comspr),
        ("SKY.DAX".to_string(), sky),
        (format!("GEO{area}.DAX"), geo),
        (format!("ECL{area}.DAX"), ecl),
    ])
}

/// A hand-authored `savgam?.dat` (1 party member, D-SAVE10 tier 1's spec):
/// known position, a resolvable resident-block/GEO-block pair, zeroed
/// quest flags, one `SetBlock` left empty (no wallset reload needed for
/// the fixture data set).
fn synthetic_master_bytes_for_area(game_area: u8) -> Vec<u8> {
    let mut buf = vec![0u8; SAVGAM_SIZE];
    let mut off = 0usize;
    // Section 1: the save's own `gbl.game_area` (`SaveGame`, `ovr017.cs:1150`).
    buf[off] = game_area;
    off += 1;

    let area = &mut buf[off..off + 0x800];
    area[0x18A] = GEO_BLOCK_ID;
    area[0x1E4..0x1E6].copy_from_slice(&(ECL_BLOCK_ID as u16).to_le_bytes());
    area[0x1CC..0x1CE].copy_from_slice(&1u16.to_le_bytes()); // inDungeon
    area[0x18E..0x190].copy_from_slice(&5u16.to_le_bytes()); // minutes ones
    area[0x192..0x194].copy_from_slice(&9u16.to_le_bytes()); // hour
    off += 0x800;

    off += 0x800; // area2_ptr: all-default (no search flags, no head block)
    off += 0x400; // stru_1b2ca
    off += 0x1E00; // ecl_ptr (discarded)

    buf[off] = 7; // mapPosX
    buf[off + 1] = 13; // mapPosY
    buf[off + 2] = 0; // mapDirection (North)
    off += 5;

    off += 1; // last_game_state
    off += 1; // game_state
    off += 12; // set_blocks: all zero (no wallset reload)

    buf[off] = 1; // party_count
    off += 1;

    let names = &mut buf[off..off + 0x148];
    let name = format!("CHRDAT{SAVE_SLOT}1");
    names[0] = 0x29;
    names[1..1 + name.len()].copy_from_slice(name.as_bytes());

    buf
}

fn synthetic_char_bytes(name: &str) -> Vec<u8> {
    let mut buf = vec![0u8; gbx_formats::save_orig::CHAR_RECORD_SIZE];
    buf[0] = name.len() as u8;
    buf[1..1 + name.len()].copy_from_slice(name.as_bytes());
    buf[0x10] = 17; // str current
    buf[0x11] = 18; // str original
    buf[0x74] = 7; // race = human
    buf[0x75] = 2; // class
    buf[0x78] = 30; // hp max
    buf[0x1a4] = 30; // hp current
    buf
}

fn synthetic_save_set() -> OriginalSaveSet {
    synthetic_save_set_for_area(GAME_AREA)
}

fn synthetic_save_set_for_area(game_area: u8) -> OriginalSaveSet {
    let master_bytes = synthetic_master_bytes_for_area(game_area);
    let char_bytes = synthetic_char_bytes("Fenwick");
    let lookup = |file_name: &str| -> Option<&[u8]> {
        if file_name == format!("CHRDAT{SAVE_SLOT}1.SAV") {
            Some(char_bytes.as_slice())
        } else {
            None
        }
    };
    gbx_formats::save_orig::load_from_lookup(&master_bytes, SAVE_SLOT, lookup).unwrap()
}

pub(crate) fn imported_engine() -> Engine {
    let set = synthetic_save_set();
    import_original(&set, synthetic_game_data(), 1234).expect("synthetic import must succeed")
}

/// ★ D-S1c: an original save made in **another area** imports against THAT
/// area's files.
///
/// `MasterSave.game_area` has been parsed since D-SAVE5 and ignored ever
/// since, because `gbl.game_area` was the `engine::GAME_AREA` constant: an
/// area-5 save resolved its `LastEclBlockId`/`current_3DMap_block_id` in
/// `ECL2.DAX`/`GEO2.DAX`, where those ids name different blocks entirely (the
/// id namespace is partitioned 16-per-area). Here the fixture ships ONLY the
/// area-5 files, so an import that still hardcoded 2 cannot even load.
#[test]
fn an_import_resolves_its_blocks_against_the_saves_own_area() {
    const OTHER_AREA: u8 = 5;
    let set = synthetic_save_set_for_area(OTHER_AREA);
    let data = synthetic_game_data_for_area(OTHER_AREA, exit_only_ecl_block());
    let engine = import_original(&set, data, 1234).expect("an area-5 save imports against area 5");
    assert_eq!(engine.state().game_area, OTHER_AREA);
    assert_eq!(
        engine.state().game_area_backup,
        OTHER_AREA,
        "the shadow seeds alongside the live cell, so a stray restore is a no-op"
    );
}

/// ...and the `.rsav` taken there restores back into the same area, resolving
/// its own GEO/wallset ids against it (`save::rebuild_engine`). Before this
/// slice a `.rsav` carried no area at all.
#[test]
fn an_rsav_restore_lands_back_in_the_saved_area() {
    const OTHER_AREA: u8 = 5;
    let set = synthetic_save_set_for_area(OTHER_AREA);
    let data = synthetic_game_data_for_area(OTHER_AREA, exit_only_ecl_block());
    let engine = import_original(&set, data.clone(), 1234).expect("import must succeed");
    let bytes = engine.save();

    let restored = Engine::restore(&bytes, data).expect("restore must succeed");
    assert_eq!(restored.state().game_area, OTHER_AREA);
    assert_eq!(
        restored.save(),
        bytes,
        "restoring and re-saving stays byte-identical"
    );
}

/// ★ The redraw gate survives the imported boot. `import_original` calls
/// `VmMemoryState::new()` (armed) and then `restore_windows`, which writes
/// the snapshot's own flag bytes — and the original save format carries
/// none, so the snapshot's are false. The original's ordering is
/// load-save-THEN-`vm_init_ecl` (`sub_29758`, `ovr003.cs:2262-2278`), whose
/// `byte_1EE91 = true` (`ovr008.cs:94`) is what makes the entry vector's
/// first `CALL 0xAE11` repaint the world — the amnesia intro's page-1 view.
/// Before this, every slot-A boot reached that CALL with a dead gate.
#[test]
fn an_imported_boots_first_call_0xae11_finds_the_redraw_gate_armed() {
    let set = synthetic_save_set();
    let mut engine = import_original(
        &set,
        synthetic_game_data_with_ecl(call_ae11_ecl_block()),
        1234,
    )
    .expect("synthetic import must succeed");

    assert!(
        engine.vm_memory().byte_1ee91,
        "vm_init_ecl's arm must survive restore_windows on the import path"
    );

    for _ in 0..5 {
        engine.tick(&[]);
    }
    let gates: Vec<bool> = engine
        .vm_memory()
        .calls
        .iter()
        .filter_map(|c| match c {
            gbx_vm::RecordedCall::RedrawViewGate { armed } => Some(*armed),
            _ => None,
        })
        .collect();
    assert_eq!(
        gates.first(),
        Some(&true),
        "the imported boot's first CALL 0xAE11 must yield Effect::RedrawView \
         (gate armed); saw {gates:?}"
    );
}

/// A native `.rsav` restore is NOT a `vm_init_ecl` moment — it resumes a
/// machine mid-execution, so the restored save's own flag value stands and
/// nothing re-arms it. (`SaveGame`, `ovr017.cs:1109-1156`, never writes the
/// flag at all; our `.rsav` does, and it must round-trip.)
#[test]
fn an_rsav_restore_keeps_the_saved_redraw_flag_instead_of_re_arming() {
    let mut engine = imported_engine();
    for _ in 0..5 {
        engine.tick(&[]);
    }
    // The exit-only fixture block never calls 0xAE11, so the flag is still
    // armed from import; clear it the way the gate would and round-trip.
    let saved = engine.vm_memory().byte_1ee91;
    let bytes = engine.save();
    let restored = Engine::restore(&bytes, synthetic_game_data()).expect("restore must succeed");
    assert_eq!(
        restored.vm_memory().byte_1ee91,
        saved,
        "restore replays the saved flag, it does not re-run vm_init_ecl"
    );
}

#[test]
fn import_sets_position_and_party_from_the_save() {
    let engine = imported_engine();
    assert_eq!(engine.state().pos, (7, 13));
    assert_eq!(engine.party().members.len(), 1);
    assert_eq!(engine.party().members[0].name, "Fenwick");
    assert_eq!(engine.party().members[0].hit_point_max, 30);
}

#[test]
fn import_reaches_world_menu_headlessly() {
    let mut engine = imported_engine();
    for _ in 0..5 {
        engine.tick(&[]);
    }
    assert!(matches!(
        engine.shell(),
        crate::shell::Shell::WorldMenu { .. }
    ));
}

/// D-SAVE10 tier 1: import → save → load → save byte-identity.
#[test]
fn import_save_load_save_round_trips_byte_identical() {
    let mut engine = imported_engine();
    engine.tick(&[]); // drive it a couple ticks (through boot) before saving
    engine.tick(&[]);

    let bytes1 = engine.save();
    let restored = Engine::restore(&bytes1, synthetic_game_data()).expect("restore must succeed");
    let bytes2 = restored.save();
    assert_eq!(
        bytes1, bytes2,
        "import -> save -> load -> save must be byte-identical"
    );
}

#[test]
fn restore_rejects_a_save_with_the_wrong_data_fingerprint() {
    let engine = imported_engine();
    let bytes = engine.save();
    let other_data = GameData::from_files([("UNRELATED.DAT".to_string(), vec![9, 9, 9])]);
    let err = match Engine::restore(&bytes, other_data) {
        Ok(_) => panic!("restore against mismatched data must be rejected"),
        Err(e) => e,
    };
    assert_eq!(err, crate::save::SaveError::DataFingerprintMismatch);
}

#[test]
fn restore_rejects_an_unknown_save_format_version() {
    let engine = imported_engine();
    let mut bytes = engine.save();
    bytes[6..10].copy_from_slice(&999u32.to_le_bytes());
    let err = match Engine::restore(&bytes, synthetic_game_data()) {
        Ok(_) => panic!("restore of an unknown save-format version must be rejected"),
        Err(e) => e,
    };
    assert_eq!(
        err,
        crate::save::SaveError::UnknownSaveFormatVersion {
            found: 999,
            expected: crate::save::SAVE_FORMAT_VERSION
        }
    );
}

/// The committed cross-platform golden (D-SAVE10 tier 1's "catches
/// `HashMap`-order / header-endianness nondeterminism the in-process
/// round-trip cannot"). If this legitimately needs to change (a
/// deliberate `SaveState`/header format change), bump
/// [`crate::save::SAVE_FORMAT_VERSION`] and recompute this literal.
///
/// wasm32 leg: deferred per the design doc's Fable annotation (CI only
/// `cargo check`s wasm32 today; asserting this golden there needs a wasm
/// test runner, e.g. wasmtime/wasm-pack, not yet added). This test runs
/// on the three native OSes CI already covers.
#[test]
fn golden_hash_of_a_synthetic_rsav_is_stable() {
    let mut engine = imported_engine();
    engine.tick(&[]);
    engine.tick(&[]);
    let bytes = engine.save();

    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(&bytes);
    let hash_hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();

    // Recomputed for SAVE_FORMAT_VERSION 2 (M4 step 1): the `prng` field is now
    // gbx-prng's u32 LCG state, not the old u64 splitmix64 state, so both the
    // version bytes and the trailing state bytes changed. Regenerated by running
    // this test and reading the actual digest — not hand-edited.
    //
    // Recomputed again at M6 slice 1 with the save format UNCHANGED: `boot()`
    // now loads COMSPR, so [`synthetic_game_data`] carries one more file, and
    // the header's `data_fingerprint` (a hash of the exact file set,
    // `save::data_fingerprint`) moved with it. A fixture-set change, not a
    // format change — SAVE_FORMAT_VERSION deliberately stayed at 2.
    //
    // Recomputed again for SAVE_FORMAT_VERSION 3 (the scene-pictures slice):
    // `EngineState` gained `picture` (`crate::picture::PictureLayer`), a real
    // payload-shape change, so both the version bytes and the payload moved.
    // Regenerated by running this test and reading the actual digest — not
    // hand-edited.
    //
    // Recomputed with the format UNCHANGED for the `vm_init_ecl` head reset
    // (Bryan's 2026-08-08 intro-art find): the imported synthetic save's
    // stale `head_block_id` byte is now overwritten to the `0xFF` no-portrait
    // sentinel at the block-entry preamble (`ovr008.cs:109`), so this
    // fixture's saved `EngineState` payload carries `0xFF` where it carried
    // the save's `0x00`. A behavior fix moving one payload byte, not a
    // format change. Regenerated by running this test — not hand-edited.
    // Recomputed for SAVE_FORMAT_VERSION 4 (roll-credits slice 1, area
    // generalization — D-S1c's ONE save break, the version bump the later
    // slices rebase their own serde additions onto). `EngineState` gained four
    // cells: `game_area`/`game_area_backup` (`gbl.game_area` was a compile-
    // time constant until this slice, so a save carried no area at all),
    // `last_pos` (`area_ptr.lastXPos`/`lastYPos`) and `can_cast_spells`. Both
    // the version bytes and the payload moved. Regenerated by running this
    // test and reading the actual digest — not hand-edited.
    //
    // Recomputed for SAVE_FORMAT_VERSION 5 (roll-credits slice 2, the
    // encounter cluster). `EngineState` gained the `Area2` approach cells
    // (`encounter_distance`, `max_encounter_distance`) plus the `gbl` ids and
    // `encounter_flags`; `PictureLayer` gained the `SPRIT` sprite pair; and
    // `VmMemoryState` gained the redraw gate's fifth flag plus `byte_1EE95`/
    // `byte_1EE96`. Both the version bytes and the payload moved. Regenerated
    // by running this test and reading the actual digest — not hand-edited.
    //
    // Recomputed for SAVE_FORMAT_VERSION 6 (roll-credits slice 3, the
    // items/roster/mechanics tail — the slice's ONE break, taken in full up
    // front per §3's churn rule). `EngineState` gained the LOAD CHARACTER
    // family's roster/selection cells (`player_not_found`,
    // `restore_player_ptr`, `redraw_party_summary`, `party_size`), the
    // treasure pool (`pooled_money`, `treasure_items`), `exp_to_add` and
    // `wipe_cause`; `GameOverFlow` gained `cause`/`delay_ticks`. Both the
    // version bytes and the payload moved. Regenerated by running this test
    // and reading the actual digest — not hand-edited.
    const GOLDEN: &str = "6852e8ff6ff359121bb2815f288cad3ebf80f6b29960a380df56651924509c02";
    assert_eq!(hash_hex, GOLDEN, "synthetic .rsav golden hash changed");
}

/// D-SAVE10 tier 2: local real-save import (loud-skip when absent).
#[test]
fn local_tier_imports_a_real_save_if_present() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!(
            "GBX_DATA_DIR not set -- skipping the local real-save tier. To create one: boot \
             CotAB in DOSBox (see docs/dosbox-capture.md for the launch command), play until \
             you reach an exploration/camp state, open the game menu, and Save to slot A. \
             DOSBox's save path must point inside GBX_DATA_DIR (or copy the resulting \
             savgam?.dat/CHRDAT?*.sav files there afterward) for this test to find them."
        );
        return;
    };
    let dir = std::path::Path::new(&dir);
    let data = gbx_formats::game_data::load_dir(dir).expect("GBX_DATA_DIR must be readable");

    let Some(slot) = data
        .file_names()
        .find(|n| n.starts_with("SAVGAM") && n.ends_with(".DAT"))
        .and_then(|n| n.chars().nth(6))
    else {
        eprintln!(
            "GBX_DATA_DIR is set but no savgam?.dat was found under it -- skipping the local \
             real-save tier. See the loud-skip message above for how to create one."
        );
        return;
    };

    let master_bytes = data
        .raw_file(&format!("SAVGAM{slot}.DAT"))
        .expect("just found this file name");
    let lookup = |name: &str| data.raw_file(name);
    let set = gbx_formats::save_orig::load_from_lookup(master_bytes, slot, lookup)
        .expect("a real savgam?.dat must parse cleanly");

    // Structural sanity + field bounds (D-SAVE10 tier 2).
    assert_eq!(set.chars.len(), set.master.party_count as usize);
    for oc in &set.chars {
        assert!(gbx_formats::save_orig::MAIN_STAT_RANGE.contains(&oc.record.stats.int.current));
        assert!(gbx_formats::save_orig::MAIN_STAT_RANGE.contains(&oc.record.stats.wis.current));
        assert!(gbx_formats::save_orig::MAIN_STAT_RANGE.contains(&oc.record.stats.dex.current));
        assert!(gbx_formats::save_orig::MAIN_STAT_RANGE.contains(&oc.record.stats.con.current));
        assert!(gbx_formats::save_orig::MAIN_STAT_RANGE.contains(&oc.record.stats.cha.current));
        assert!(gbx_formats::save_orig::STR_EXCEPTIONAL_RANGE
            .contains(&oc.record.stats.str_exceptional.current));
    }

    let mut engine = import_original(&set, data.clone(), 1)
        .expect("import must succeed against a real save + its matching GameData");
    let pos_before = engine.state().pos;
    eprintln!(
        "local tier: imported {} party member(s), position {:?}",
        engine.party().members.len(),
        pos_before
    );

    // Drive the engine headlessly a few ticks post-import (D-SAVE10 tier 2:
    // "walk one step"), then try a forward step ('H', `world_menu_command`'s
    // Forward key) once the world menu is reached. Not asserted to *change*
    // position (a real wall may legitimately block it) — this exercises the
    // post-import tick/walk path without panicking, which is the point.
    for _ in 0..5 {
        engine.tick(&[]);
    }
    engine.tick(&[crate::input::InputEvent::Char(b'H')]);
    for _ in 0..5 {
        engine.tick(&[]);
    }
    eprintln!(
        "local tier: post-walk position {:?} (was {:?})",
        engine.state().pos,
        pos_before
    );

    let bytes1 = engine.save();
    let restored = Engine::restore(&bytes1, data).expect(".rsav restore must succeed");
    let bytes2 = restored.save();
    assert_eq!(
        bytes1, bytes2,
        "real-save .rsav round-trip must be byte-identical"
    );
}

// --- M3 step 6 deliverable 3: save/load slot filesystem round-trip ---

/// The slot ↔ `.rsav` file mapping (`saveload_fs`): save an engine into a
/// lettered slot under a temp dir, scan it back as `RestrikeSave`, restore
/// from the slot, and assert a byte-identical `.rsav` round-trip. Uses a
/// process-unique temp dir (tests may touch the filesystem; the core tick
/// loop never does — D8).
#[test]
fn saveload_fs_round_trips_a_slot_file() {
    use crate::saveload::SlotStatus;
    use crate::saveload_fs::{load_from_slot, save_to_slot, scan_slot_directory};

    let engine = imported_engine();
    let expected = engine.save();

    let dir = std::env::temp_dir().join(format!("restrike-slot-roundtrip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    save_to_slot(&engine, &dir, 'D').expect("save to slot D");

    // Only slot D is occupied, and it reads back as our own format.
    let scanned = scan_slot_directory(&dir);
    assert_eq!(scanned.status('D'), SlotStatus::RestrikeSave);
    assert_eq!(scanned.occupied_letters(), vec!['D']);

    let restored = load_from_slot(&dir, 'D', synthetic_game_data()).expect("load from slot D");
    assert_eq!(
        restored.save(),
        expected,
        "slot round-trip must reproduce the saved state byte-for-byte"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A fulfilled `Save` request writes the same slot file the direct API does,
/// and a missing-slot `Load` surfaces an error rather than panicking.
#[test]
fn saveload_fs_fulfill_save_then_errors_on_empty_slot() {
    use crate::saveload::SaveLoadRequest;
    use crate::saveload_fs::{fulfill, load_from_slot};

    let mut engine = imported_engine();
    let dir = std::env::temp_dir().join(format!("restrike-slot-fulfill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    fulfill(
        &mut engine,
        SaveLoadRequest::Save('A'),
        &dir,
        synthetic_game_data(),
        7,
    )
    .expect("fulfilling a Save writes the slot");
    assert!(dir.join("SAVGAMA.RSAV").is_file());

    // Loading an unwritten slot is an error, not a panic.
    assert!(load_from_slot(&dir, 'B', synthetic_game_data()).is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

// --- roll-credits slice 0 (G0): the whole in-game save/load loop ---

/// Feeds one key per tick and then idles, so a screen transition that needs a
/// tick to settle gets one.
fn press(engine: &mut Engine, keys: &[u8]) {
    for &k in keys {
        engine.tick(&[crate::input::InputEvent::Char(k)]);
        engine.tick(&[]);
    }
}

/// ★ **The slice-0 acceptance property**: a player saves from inside the game,
/// keeps playing, and later loads — and lands back on the state they saved.
///
/// This drives the real screens with real keys (`Encamp ▸ Save ▸ Save ▸ slot A`,
/// then `Encamp ▸ Save ▸ Load ▸ slot A`) and plays the host's part exactly as
/// `frontends/desktop` does: take the request after the tick,
/// `saveload_fs::fulfill` it, re-scan the directory. Until this slice no
/// frontend called any of that, so the screens emitted requests into the void
/// (D-RC0).
#[test]
fn a_camp_save_then_load_restores_the_saved_state() {
    use crate::saveload::SaveLoadRequest;
    use crate::saveload_fs::{fulfill, scan_slot_directory};

    let dir = std::env::temp_dir().join(format!("restrike-slice0-loop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let mut engine = imported_engine();
    for _ in 0..20 {
        engine.tick(&[]);
    }
    engine.set_slot_directory(scan_slot_directory(&dir));

    // Camp ▸ Save ▸ Save ▸ slot A.
    press(&mut engine, b"essA");
    let request = engine
        .take_io_request()
        .expect("picking a slot in Save mode emits a request");
    assert_eq!(request, SaveLoadRequest::Save('A'));
    fulfill(&mut engine, request, &dir, synthetic_game_data(), 7)
        .expect("the host fulfills the Save");
    engine.set_slot_directory(scan_slot_directory(&dir));
    let saved = engine.save();

    // Keep playing: the search toggle flips real engine state, so a load that
    // silently did nothing could not pass the comparison below.
    press(&mut engine, b"s");
    for _ in 0..20 {
        engine.tick(&[]);
    }
    assert_ne!(engine.save(), saved, "playing on changed the state");

    // Camp ▸ Save ▸ Load ▸ slot A.
    press(&mut engine, b"eslA");
    let request = engine
        .take_io_request()
        .expect("picking a slot in Load mode emits a request");
    assert_eq!(request, SaveLoadRequest::Load('A'));
    fulfill(&mut engine, request, &dir, synthetic_game_data(), 7)
        .expect("the host fulfills the Load");

    assert_eq!(
        engine.save(),
        saved,
        "the loaded engine must be byte-identical to the one that was saved"
    );

    // And the host's post-load repaint is a no-panic no-op-or-redraw on
    // whatever shell the save carried.
    engine.recompose_world_screen();
    engine.tick(&[]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Ticks until `probe()` says `want`, or panics with what it actually said.
fn tick_until_probe(engine: &mut Engine, want: &str, limit: usize) {
    for _ in 0..limit {
        engine.tick(&[]);
        if engine.probe() == want {
            return;
        }
    }
    panic!(
        "never reached {want:?} — stuck at {:?} after {limit} ticks",
        engine.probe()
    );
}

/// ★ **The party-wipe recovery, end to end** (roll-credits G0): a wiped
/// playthrough is resumable from the last save without restarting the process.
///
/// The original's death path unwinds to `startGameMenu` with the party cleared
/// and `Load Saved Game` the only door back in (`ovr006.cs:801-809` →
/// `ovr003.cs:2392-2394` → `seg001.cs:133-153` → `ovr018.cs:103-114`). This
/// walks ours: wipe → the death screen → a key → the load list → a slot → the
/// state that was saved before the party died.
#[test]
fn a_party_wipe_recovers_through_the_load_screen() {
    use crate::saveload::SaveLoadRequest;
    use crate::saveload_fs::{fulfill, save_to_slot, scan_slot_directory};
    use crate::shell::Shell;

    let dir = std::env::temp_dir().join(format!("restrike-slice0-wipe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let mut engine = imported_engine();
    for _ in 0..20 {
        engine.tick(&[]);
    }
    save_to_slot(&engine, &dir, 'A').expect("bank a save before dying");
    engine.set_slot_directory(scan_slot_directory(&dir));
    let alive = engine.save();

    // The wipe. `party_killed` is what a lost fight raises
    // (`tick_combat` → `CombatOutcome::MonstersWin`).
    let before = engine.tick(&[]).hash_hex();
    engine.state.party_killed = true;
    let dying = engine.tick(&[]).hash_hex();
    assert!(matches!(engine.shell(), Shell::GameOver(_)));
    assert!(!engine.state().party_killed, "the flag is consumed at once");
    assert_ne!(before, dying, "the death screen is on the glass");
    assert_eq!(engine.probe(), "game-over/message");

    // The message paces out (`press_any_key`'s own printer), then the
    // `DisplayAndPause` gate arms.
    tick_until_probe(&mut engine, "game-over/press-any-key", 600);

    // Declining the recovery goes back to the death screen — never into the
    // world with a dead party.
    engine.tick(&[crate::input::InputEvent::Char(b' ')]);
    assert_eq!(engine.probe(), "screen", "the load list opened");
    engine.tick(&[crate::input::InputEvent::Escape]);
    engine.tick(&[]);
    assert!(
        matches!(engine.shell(), Shell::GameOver(_)),
        "Exit from the recovery list returns to the death screen"
    );

    // Take the load this time.
    tick_until_probe(&mut engine, "game-over/press-any-key", 600);
    engine.tick(&[crate::input::InputEvent::Char(b' ')]);
    engine.tick(&[crate::input::InputEvent::Char(b'A')]);
    let request = engine
        .take_io_request()
        .expect("picking a slot on the recovery screen emits a Load");
    assert_eq!(request, SaveLoadRequest::Load('A'));
    fulfill(&mut engine, request, &dir, synthetic_game_data(), 7)
        .expect("the host fulfills the recovery Load");
    engine.recompose_world_screen();

    assert_eq!(
        engine.save(),
        alive,
        "the recovered engine is the one that was saved before the wipe"
    );
    assert!(
        !matches!(engine.shell(), Shell::GameOver(_)),
        "and the game is playable again"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A host notice is drawn for the player and then retires itself — the M6
/// forensics rule made mechanical. Pixels only move because a *host* asked
/// them to, which is why no golden is affected by this path existing.
#[test]
fn a_host_notice_shows_then_clears_itself() {
    let mut engine = imported_engine();
    engine.tick(&[]);
    let quiet = engine.tick(&[]).hash_hex();

    engine.report_host_notice("Slot A saved.");
    let shown = engine.tick(&[]).hash_hex();
    assert_ne!(shown, quiet, "the notice put pixels on the screen");
    assert_eq!(engine.host_notice(), Some("Slot A saved."));

    // A keypress dismisses it, and the row goes back to background.
    engine.tick(&[crate::input::InputEvent::Char(b'x')]);
    assert_eq!(engine.host_notice(), None);
}
