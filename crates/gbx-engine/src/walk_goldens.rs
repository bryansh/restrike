//! M2 step 3 deliverable: framebuffer-hash goldens (D-UI7) driven through
//! `Engine::tick` end-to-end — a synthetic fixture GEO block (this module's
//! own [`fixture_geo`]) + step-2 fixture boot assets + a real
//! `EclBuilder`-authored ECL block (M2 step 4: the VM is real, not a stub),
//! walked by a pinned input trace. Checkpoints are explicit
//! `(trace, tick_index)` pairs — [`walk_trace`]'s input schedule plus the
//! pinned tick indices in [`golden_walk_trace`] — never named moments.
//!
//! Regenerate pinned hashes with `RESTRIKE_REGEN_GOLDENS=1`; a `.ppm` is
//! dumped to the system temp dir on mismatch or during regen, mirroring
//! `hash_goldens.rs`'s existing pattern (step 2).

#![cfg(test)]

use crate::engine::{Engine, GAME_AREA, INITIAL_ECL_BLOCK};
use crate::input::{ExtKey, InputEvent};
use crate::test_support::exit_only_block;
use gbx_formats::font::{self, Font};
use gbx_formats::geo::{GeoBlock, GEO_BLOCK_SIZE};
use gbx_formats::image::{DecodedItem, ImageBlock};

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

fn synthetic_font() -> Font {
    let mut data = Vec::with_capacity(font::GLYPH_COUNT * font::GLYPH_BYTES);
    for j in 0..font::GLYPH_COUNT {
        data.extend_from_slice(&[j as u8; font::GLYPH_BYTES]);
    }
    font::decode(&data)
}

/// A small hand-authored corridor (D10): the party starts at `(5,5)` facing
/// East. Stepping East crosses one open square, then a locked door; beyond
/// it is open. Built directly via [`GeoBlock::parse`] over a synthetic
/// `0x402`-byte payload (`gbx-formats::geo` ships no fixture-map helper of
/// its own — each consumer authors its own bytes, matching `movement.rs`'s
/// tests).
fn fixture_geo() -> GeoBlock {
    let mut data = vec![0u8; GEO_BLOCK_SIZE];
    let idx = |x: usize, y: usize| x + 16 * y;

    // Square (5,5): East wall type 3, door state 1 (open) — the first step.
    data[2 + idx(5, 5)] |= 3;
    data[2 + 3 * 256 + idx(5, 5)] = 0b01 << 2; // door_east = 1

    // Square (6,5): East wall type 4, door state 2 (locked) — the door.
    data[2 + idx(6, 5)] |= 4;
    data[2 + 3 * 256 + idx(6, 5)] = 0b10 << 2; // door_east = 2

    GeoBlock::parse(&data).unwrap()
}

fn fixture_engine(seed: u64) -> Engine {
    let mut sets = crate::symbols::SymbolSets::new();
    sets.load(4, synthetic_set4());
    // The resident block's every vector resolves to a trivial EXIT — this
    // trace exercises walk-loop/renderer state, not real script content
    // (real-content H2 conformance lives in `shell.rs`'s test module).
    let data =
        crate::test_support::ecl_game_data(GAME_AREA, vec![(INITIAL_ECL_BLOCK, exit_only_block())]);
    let mut engine = Engine::new_fixture(synthetic_font(), sets, fixture_geo(), data, seed);
    engine.state.pos = (5, 5);
    engine.state.facing = crate::movement::Facing::East;
    engine.party_predicates_mut().bash_candidates = vec![(25, 0)]; // automatic bash success
    engine
}

fn write_ppm(name: &str, frame: &crate::engine::Frame) {
    let path = std::env::temp_dir().join(format!("restrike-walk-golden-{name}.ppm"));
    let mut out = format!(
        "P6\n{} {}\n255\n",
        crate::framebuffer::WIDTH,
        crate::framebuffer::HEIGHT
    )
    .into_bytes();
    for y in 0..crate::framebuffer::HEIGHT {
        for x in 0..crate::framebuffer::WIDTH {
            let idx = frame.pixels[y * crate::framebuffer::WIDTH + x];
            out.extend_from_slice(&frame.palette[idx as usize]);
        }
    }
    if std::fs::write(&path, &out).is_ok() {
        eprintln!("walk golden '{name}': dumped {}", path.display());
    }
}

fn check_golden(name: &str, frame: &crate::engine::Frame, expected_hex: &str) {
    let actual = frame.hash_hex();
    let regen = std::env::var_os("RESTRIKE_REGEN_GOLDENS").is_some();
    if regen {
        eprintln!("walk golden '{name}': {actual}");
        write_ppm(name, frame);
        return;
    }
    if actual != expected_hex {
        write_ppm(name, frame);
    }
    assert_eq!(
        actual, expected_hex,
        "walk golden '{name}' mismatched — see dumped .ppm (or rerun with RESTRIKE_REGEN_GOLDENS=1)"
    );
}

/// The pinned input trace: `(tick_index, events)`, sparse (every
/// unlisted tick gets no input). Walks: reach the world menu, turn to face
/// the corridor (already facing East, so a no-op turn exercises the sound
/// path harmlessly is skipped — this trace turns 180 then back to prove
/// turning is exercised), step into the open square, bash through the
/// locked door, and land on the far side.
fn walk_trace() -> Vec<(u32, Vec<InputEvent>)> {
    vec![
        (3, vec![InputEvent::Ext(ExtKey::Down)]), // turn around (P, no sound)
        (5, vec![InputEvent::Ext(ExtKey::Down)]), // turn around again -> facing East once more
        (8, vec![InputEvent::Ext(ExtKey::Up)]),   // step forward into the open square
        (14, vec![InputEvent::Ext(ExtKey::Up)]),  // step forward into the locked door
        (20, vec![InputEvent::Char(b'b')]),       // Bash
    ]
}

/// Ticks `engine` through `trace` for `total_ticks`, recording the frame
/// hash and `(pos, facing)` at every tick — a discovery/diagnostic aid kept
/// as a `#[test]` so `cargo test -- --nocapture` can re-derive the pinned
/// indices/hashes below if the flow-control shape ever changes.
#[test]
fn walk_trace_diagnostic() {
    let mut e = fixture_engine(1);
    let trace = walk_trace();
    for tick_index in 0..40u32 {
        let events: &[InputEvent] = trace
            .iter()
            .find(|(i, _)| *i == tick_index)
            .map(|(_, e)| e.as_slice())
            .unwrap_or(&[]);
        let hash = e.tick(events).hash_hex();
        eprintln!(
            "tick {tick_index}: pos={:?} facing={:?} hash={hash}",
            e.state.pos, e.state.facing
        );
    }
    assert_eq!(
        e.state.pos,
        (7, 5),
        "the walk must end past the bashed door"
    );
}

/// The pinned golden checkpoints (D-UI7): explicit `(trace, tick_index)`
/// pairs — [`walk_trace`]'s input schedule plus the pinned tick indices in
/// [`golden_walk_trace`] — never named moments.
#[test]
fn golden_walk_trace() {
    let mut e = fixture_engine(1);
    let trace = walk_trace();
    let checkpoints: &[(u32, &str)] = &[
        // Idle in WorldMenu, facing East (pos (5,5)) — the stable boot frame.
        (
            2,
            "be057d0aaba383704b96422b6dd010e185f0ee32b266d462f2fb4d29fe509ed0",
        ),
        // Mid-turn-around: facing West after the first 180° turn.
        (
            4,
            "631e87fc85a5acf05f3a2936f61a6714b9f471dcc655abf414d3a405ecc6a220",
        ),
        // Stepped into the open square (pos (6,5)), facing East again.
        (
            12,
            "93fd35bd984572558636bac1f54ab84d862f1a852cda503f02bea1fcc08c887c",
        ),
        // Bashed through the locked door (pos (7,5)).
        (
            22,
            "e8e6507f170102c259d58e015f686c7dc3b4cab3794cef28a18f3d13aeb74fe2",
        ),
    ];
    let mut next_checkpoint = 0usize;
    for tick_index in 0..30u32 {
        let events: &[InputEvent] = trace
            .iter()
            .find(|(i, _)| *i == tick_index)
            .map(|(_, e)| e.as_slice())
            .unwrap_or(&[]);
        let f = e.tick(events);
        if next_checkpoint < checkpoints.len() && checkpoints[next_checkpoint].0 == tick_index {
            let (_, expected) = checkpoints[next_checkpoint];
            check_golden(&format!("tick-{tick_index}"), &f, expected);
            next_checkpoint += 1;
        }
    }
    assert_eq!(
        e.state.pos,
        (7, 5),
        "the walk must end past the bashed door"
    );
}
