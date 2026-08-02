//! ★ **The M6a reel smoke** (`docs/design/combat-visualizer.md` D-CV8, §4 M6a's
//! done-condition): every closed capture plays end to end **through the reel,
//! with pixels**, and with draw equality asserted live.
//!
//! This is the frontier guard's twin. The guard proves the fifteen captures
//! replay operand-exact *headlessly*; this proves the same fifteen replay
//! operand-exact while a `CombatScene` composes every beat, advances a presented
//! board by events alone, reconciles it against the real roster at every step
//! boundary, and paints a full 320×200 frame for each tick along the way. If any
//! of that presentation ever perturbed the fight, the reel's own live assert
//! would fire mid-capture — and the two suites would disagree.
//!
//! It found real bugs on its first run: two hp-changing sites (spell damage and
//! the round-end regeneration tick) whose mutations no `ActionEvent` carried, so
//! the presented board drifted. Nothing headless could have noticed.
//!
//! **Local tier (D10):** captures and game art are local-only, never in the
//! repo/CI. Gated on `GBX_TRACES_DIR`/`GBX_DATA_DIR` and loud-skipping per
//! capture when a file is absent — the guard's own posture, so a plain
//! `cargo test` stays green with nothing installed.
//!
//! Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-oracle \
//!   --test reel_smoke -- --nocapture`

use gbx_engine::engine::Engine;
use gbx_formats::game_data::{load_dir, GameData};
use gbx_oracle::replay;
use std::path::PathBuf;

/// D-CV3's host tick multiplier. The reel is a per-tick state machine, so this
/// changes wall time only — the composed schedule, the presented board and the
/// draw stream are bit-identical at any multiplier (the engine's own
/// `the_tick_multiplier_changes_wall_time_and_nothing_else` pins that). Without
/// it, sewer-fight-2's 49 rounds would smoke-test at the original's pace.
const TURBO: u32 = 512;

/// A runaway guard: no closed capture is anywhere near this many host ticks even
/// at 1×, so tripping it means the reel stopped making progress.
const MAX_FRAMES: u64 = 200_000;

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("GBX_DATA_DIR").map(PathBuf::from)
}

/// Plays one capture through the reel and returns its final progress.
///
/// Every failure mode inside is a panic by design: a capture divergence carries
/// `h4_replay`'s diagnostic out of the engine, and a presented-board drift
/// carries the drifted field. Neither should be swallowed into a boolean.
fn play(data: GameData, path: &PathBuf, capture: &str) -> gbx_engine::combat::reel::ReelProgress {
    let text = std::fs::read_to_string(path).expect("capture readable");
    let mut input = replay::reel_input_from_capture(capture, &text, replay::load_item_data())
        .unwrap_or_else(|e| panic!("{capture}: {e}"));
    input.tick_multiplier = TURBO;
    let mut engine =
        Engine::new_reel(data, input).unwrap_or_else(|e| panic!("{capture}: the reel: {e}"));

    let mut frames = 0u64;
    let mut sounds = 0usize;
    while !engine.reel_progress().expect("watch mode").finished {
        // The frame is genuinely drawn every tick — this is the whole point of
        // the smoke over a headless replay. Touching a pixel keeps the paint
        // from being optimized into nothing and asserts the framebuffer is the
        // shape D-UI1 promises.
        let frame = engine.tick(&[]);
        assert_eq!(frame.pixels.len(), 320 * 200);
        sounds += frame.sounds.len();
        frames += 1;
        assert!(frames < MAX_FRAMES, "{capture}: the reel stopped advancing");
    }
    let p = engine.reel_progress().expect("watch mode");
    eprintln!(
        "OK  {capture} — {} draws checked over {} steps / {frames} frames, {sounds} sound cues",
        p.draws_checked, p.steps
    );
    p
}

#[test]
fn every_closed_capture_plays_through_the_reel_with_live_draw_equality() {
    let Some(traces) = replay::traces_dir() else {
        eprintln!(
            "SKIPPED: the reel smoke needs the local traces dir \
             (set GBX_DATA_DIR or GBX_TRACES_DIR; captures are local-only, D10)"
        );
        return;
    };
    let Some(dir) = data_dir() else {
        eprintln!(
            "SKIPPED: the reel draws REAL art (CHEAD/CBODY/CPIC/DUNGCOM/RANDCOM) \
             — set GBX_DATA_DIR to the game data"
        );
        return;
    };
    let data = load_dir(&dir).expect("GBX_DATA_DIR must be readable");

    let mut played = 0usize;
    let mut total_draws = 0usize;
    for capture in replay::sidecar::pinned_captures() {
        let path = traces.join(capture);
        if !path.exists() {
            eprintln!("SKIPPED (absent, D10): {capture}");
            continue;
        }
        // A ranged capture needs the `ITEMS` table; without it the assembly
        // refuses (rather than replaying a different, melee-only fight).
        if replay::capture_has_loadout(capture) && replay::load_item_data().is_none() {
            eprintln!("SKIPPED (ITEMS absent, D10): {capture}");
            continue;
        }
        let p = play(data.clone(), &path, capture);
        assert!(p.finished, "{capture}: the reel did not reach the end");
        assert!(
            p.draws_expected > 0,
            "{capture}: no captured draws to check against — the equality assert \
             would have been silently off"
        );
        assert_eq!(
            p.draws_checked, p.draws_expected,
            "{capture}: our fight and the capture ended on different draw counts"
        );
        played += 1;
        total_draws += p.draws_checked;
    }

    if played == 0 {
        eprintln!(
            "SKIPPED: no pinned captures present under {}",
            traces.display()
        );
    } else {
        eprintln!(
            "reel smoke: {played}/{} captures played with pixels, {total_draws} draws \
             checked live",
            replay::sidecar::pinned_captures().count()
        );
    }
}

/// The other half of the M6a claim: the reel refuses an unpinned monster rather
/// than drawing it as an empty slot.
///
/// A viewer whose enemies are invisible is worse than one that will not start,
/// and this is the only failure the sidecar's icon half exists to prevent — so
/// it is worth a test that does not depend on which capture is installed.
#[test]
fn a_capture_with_no_icon_pins_is_refused_rather_than_drawn_blank() {
    let Some(traces) = replay::traces_dir() else {
        eprintln!("SKIPPED: needs the local traces dir (D10)");
        return;
    };
    let Some(dir) = data_dir() else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (D10)");
        return;
    };
    // Any pinned capture will do; combat4 is the canonical one.
    let capture = "combat4.gbxtrace";
    let path = traces.join(capture);
    if !path.exists() {
        eprintln!("SKIPPED (absent, D10): {capture}");
        return;
    }
    let data = load_dir(&dir).expect("GBX_DATA_DIR must be readable");
    let text = std::fs::read_to_string(&path).expect("capture readable");
    let mut input = replay::reel_input_from_capture(capture, &text, replay::load_item_data())
        .expect("combat4 assembles");
    input.art.monster_blocks.clear();

    let err = Engine::new_reel(data, input).err().expect("must refuse");
    let message = err.to_string();
    assert!(
        message.contains("BAR PATRON") && message.contains("monster_icons"),
        "the refusal must name the monster and the fix: {message}"
    );
}
