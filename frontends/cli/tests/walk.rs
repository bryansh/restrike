//! `restrike walk`'s task-brief determinism proof: an in-repo test (no game
//! data) that a fixture trace replays to identical checkpoint hashes twice,
//! and a local-only test (`GBX_DATA_DIR`) that a real-data trace's
//! checkpoint hashes are stable across two runs *in-process* (calling
//! [`restrike_cli::walk::replay`] directly, not spawning the binary twice —
//! a stronger proof that `Engine::tick` itself is deterministic, not just
//! that two separate process invocations happen to agree).

use gbx_engine::engine::Engine;
use gbx_engine::symbols::SymbolSets;
use gbx_formats::font::{self, GLYPH_BYTES, GLYPH_COUNT};
use gbx_formats::game_data::GameData;
use gbx_formats::geo::{GeoBlock, GEO_BLOCK_SIZE};
use gbx_formats::image::{DecodedItem, ImageBlock};
use gbx_vm::test_support::EclBuilder;
use restrike_cli::walk::{replay, Trace};

/// A minimal synthetic set-4 image block, enough for `Engine::build`'s
/// `draw8x8_03` call to succeed (mirrors `gbx-engine/src/engine.rs`'s own
/// test fixture).
fn synthetic_set4() -> ImageBlock {
    ImageBlock {
        height: 8,
        width_cols: 1,
        x_pos: 0,
        y_pos: 0,
        field_9: [0; 8],
        items: (0..40)
            .map(|i| DecodedItem {
                pixels: vec![(i % 16) as u8; 64],
            })
            .collect(),
    }
}

fn synthetic_font() -> font::Font {
    let mut data = Vec::with_capacity(GLYPH_COUNT * GLYPH_BYTES);
    for j in 0..GLYPH_COUNT {
        data.extend_from_slice(&[j as u8; GLYPH_BYTES]);
    }
    font::decode(&data)
}

fn open_geo() -> GeoBlock {
    GeoBlock::parse(&vec![0u8; GEO_BLOCK_SIZE]).unwrap()
}

/// A minimal single-block `ECL{GAME_AREA}.DAX` whose block
/// `INITIAL_ECL_BLOCK` is a real, resolvable-header EXIT-only script
/// (`EclBuilder`-authored, D10) -- the real `EclMachine` always needs real
/// bytecode to load, even for a fixture boot.
fn exit_only_game_data() -> GameData {
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

    let comp: Vec<u8> = raw
        .chunks(128)
        .flat_map(|chunk| {
            let mut v = vec![(chunk.len() - 1) as u8];
            v.extend_from_slice(chunk);
            v
        })
        .collect();
    let mut file = Vec::new();
    file.extend_from_slice(&9u16.to_le_bytes());
    file.push(gbx_engine::engine::INITIAL_ECL_BLOCK);
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&(raw.len() as u16).to_le_bytes());
    file.extend_from_slice(&(comp.len() as u16).to_le_bytes());
    file.extend_from_slice(&comp);

    GameData::from_files([(format!("ECL{}.DAX", gbx_engine::engine::GAME_AREA), file)])
}

fn fixture_engine() -> Engine {
    let mut sets = SymbolSets::new();
    sets.load(4, synthetic_set4());
    Engine::new_fixture(synthetic_font(), sets, open_geo(), exit_only_game_data(), 1)
}

/// The task brief's in-repo determinism proof: replaying the same trace
/// against two freshly-built fixture engines must produce identical
/// checkpoint hashes, in the same order.
#[test]
fn fixture_trace_replays_to_identical_checkpoint_hashes_twice() {
    let trace_text = "\
{\"tick\":1,\"event\":{\"input\":\"Enter\"}}
{\"tick\":3,\"event\":\"checkpoint\"}
{\"tick\":5,\"event\":\"checkpoint\"}
";
    let trace = Trace::parse(trace_text).expect("trace must parse");

    let mut hashes_a = Vec::new();
    let exit_a = replay(
        &mut fixture_engine(),
        &trace,
        &[],
        |tick, hash| hashes_a.push((tick, hash.to_string())),
        |_, _| {},
    );

    let mut hashes_b = Vec::new();
    let exit_b = replay(
        &mut fixture_engine(),
        &trace,
        &[],
        |tick, hash| hashes_b.push((tick, hash.to_string())),
        |_, _| {},
    );

    assert_eq!(hashes_a.len(), 2, "both declared checkpoints must fire");
    assert_eq!(hashes_a, hashes_b);
    assert_eq!(exit_a, exit_b);
    assert_eq!(
        exit_a, hashes_a[1].1,
        "tick 5 is both a checkpoint and exit"
    );
}

/// Local-only tier: a real-data trace's checkpoint hashes are stable across
/// two runs *in-process* -- two independently-booted `Engine`s from two
/// independent `GameData::load_dir` reads, replayed against the same
/// trace, must agree. Silently passes when `GBX_DATA_DIR` is unset.
#[test]
fn real_data_trace_checkpoint_hashes_are_stable_across_two_in_process_runs() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);

    let trace_text = "\
{\"tick\":30,\"event\":\"checkpoint\"}
{\"tick\":90,\"event\":\"checkpoint\"}
";
    let trace = Trace::parse(trace_text).expect("trace must parse");

    let run = || {
        let data = gbx_formats::game_data::load_dir(dir).expect("GBX_DATA_DIR must be readable");
        let mut engine =
            Engine::new(data, 1).expect("Engine::new must boot against real CotAB data");
        let mut hashes = Vec::new();
        let exit = replay(
            &mut engine,
            &trace,
            &[],
            |tick, hash| hashes.push((tick, hash.to_string())),
            |_, _| {},
        );
        (hashes, exit)
    };

    let (hashes_a, exit_a) = run();
    let (hashes_b, exit_b) = run();

    assert_eq!(hashes_a.len(), 2);
    assert_eq!(hashes_a, hashes_b);
    assert_eq!(exit_a, exit_b);
}
