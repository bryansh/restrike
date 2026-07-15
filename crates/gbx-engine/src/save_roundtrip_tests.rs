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

/// Builds the full boot-compatible synthetic [`GameData`] (8X8D1.DAX's
/// font/set4/set0, SKY.DAX's 3 blocks, `GEO{GAME_AREA}.DAX` block
/// [`GEO_BLOCK_ID`], `ECL{GAME_AREA}.DAX` block [`ECL_BLOCK_ID`]) — D10:
/// every byte here is self-authored, no extracted game content.
fn synthetic_game_data() -> GameData {
    let set4 = tiny_image_bytes(40); // draw8x8_03 indexes up to set 4's higher items
    let set0 = tiny_image_bytes(40);
    let sky_image = tiny_image_bytes(1);
    let font = tiny_font_bytes();
    let eight_by_eight_d1 = build_dax(&[(201, &font), (0xCA, &set4), (0xCB, &set0)]);
    let sky = build_dax(&[(250, &sky_image), (251, &sky_image), (252, &sky_image)]);
    let geo = build_dax(&[(GEO_BLOCK_ID, &vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE])]);
    let ecl_block = exit_only_ecl_block();
    let ecl = build_dax(&[(ECL_BLOCK_ID, &ecl_block)]);

    GameData::from_files([
        ("8X8D1.DAX".to_string(), eight_by_eight_d1),
        ("SKY.DAX".to_string(), sky),
        (format!("GEO{GAME_AREA}.DAX"), geo),
        (format!("ECL{GAME_AREA}.DAX"), ecl),
    ])
}

/// A hand-authored `savgam?.dat` (1 party member, D-SAVE10 tier 1's spec):
/// known position, a resolvable resident-block/GEO-block pair, zeroed
/// quest flags, one `SetBlock` left empty (no wallset reload needed for
/// the fixture data set).
fn synthetic_master_bytes() -> Vec<u8> {
    let mut buf = vec![0u8; SAVGAM_SIZE];
    let mut off = 0usize;
    buf[off] = GAME_AREA;
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
    let master_bytes = synthetic_master_bytes();
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

fn imported_engine() -> Engine {
    let set = synthetic_save_set();
    import_original(&set, synthetic_game_data(), 1234).expect("synthetic import must succeed")
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

    const GOLDEN: &str = "02bc2a5e6e72a89a5542c7ebff73dfb232f18d1237345bba3e15863d428ce2b9";
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
