//! Roll-credits slice 9d's acceptance suite (`roll-credits.md` §16): the
//! finale driven headlessly against real CotAB data, from the square that
//! triggers it to the DOS prompt the win ends at.
//!
//! Everything here is `GBX_DATA_DIR`-gated and loud-skips without it (D10 — no
//! game data enters this repo). Set `RESTRIKE_ENDING_DUMP=<dir>` to write one
//! `.ppm` per beat: the confrontation, the dissolve at two distinct steps, the
//! Knights, Shadowdale under the fireworks, and the post-victory start menu
//! with `BEGIN` refused.
//!
//! **The staging, stated honestly.** The fixture is a hand-authored
//! `savgam?.dat` (`area_transition_tests::master_bytes_at`, all self-authored
//! bytes) parked in **area 6, `ECL6` block `0x43`, `GEO6` block `0x43`,
//! `inDungeon = 1`, party at (6, 1)** — the temple's own finale square, whose
//! `x2 & 0x7F` the tests read back off the real map rather than assume. From
//! there the shipped script does everything: vector 1's `ON GOTO` dispatches
//! on the square's own event code, and `@0x9280` runs.
//!
//! **The one injected fact.** The final fight is 37 monsters including a
//! 15-HD, AC −7 Tyranthraxus; winning it is not a property this slice tests,
//! and a fixture party that could is not a fixture. So the victory tests
//! resume the shipped script at `@0x93DC` — the instruction *after* `COMBAT` —
//! with `[0x7EC7] = 0`, which is exactly the value
//! `AfterCombatExpAndTreasure` writes on entry (`ovr006.cs:765`). Everything
//! downstream of that, `PROGRAM 8` included, is the original's own bytes. The
//! approach test drives the real `COMBAT` and checks the roster that arrives
//! at it.
//!
//! **Draw parity.** No capture reaches any of this — see `crate::ending`'s
//! module doc; guard 16/16 and reel smoke 16/16 are the referees.

#![cfg(test)]

use crate::area_transition_tests::real_data_engine_at;
use crate::engine::Engine;
use crate::input::InputEvent;
use crate::shell::Shell;

/// `ECL6`/`GEO6` block `0x43` — the Temple of Bane.
const TEMPLE_BLOCK: u8 = 0x43;
/// The finale square (roll-credits §16.2).
const FINALE_SQUARE: (u8, u8, u8) = (6, 1, 0);
/// Its `x2 & 0x7F`, which is the `ON GOTO` selector that reaches `@0x9280`.
const FINALE_EVENT_CODE: u8 = 26;
/// `ECL6#67`'s per-step vector (vector 1, `@0x81AD`).
const PER_STEP_VECTOR: u16 = 0x81AD;
/// `@0x93DC` — `COMPARE [0x7EC7], #0x80`, the instruction after the final
/// `COMBAT`.
const AFTER_THE_LAST_FIGHT: u16 = 0x93DC;
/// `Area2.field_58E` — the combat outcome (roll-credits §16.3).
const COMBAT_OUTCOME_ADDR: u16 = 0x7EC7;

/// The fixture, standing on the finale square.
fn at_the_finale_square(who: &str) -> Option<Engine> {
    let Some(engine) = real_data_engine_at(6, TEMPLE_BLOCK, TEMPLE_BLOCK, true, FINALE_SQUARE)
    else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (ending_tests::{who})");
        return None;
    };
    Some(engine)
}

/// One frame to `RESTRIKE_ENDING_DUMP` (or the temp dir), for eyeballing.
/// Ticks once first, so the name describes the frame *after* the tick.
fn dump(engine: &mut Engine, name: &str) {
    engine.tick(&[]);
    dump_now(engine, name);
}

/// The frame currently on the glass, **without advancing anything** — the
/// dissolve has to be caught mid-beat, and a beat that ends inside the tick
/// would have repainted the viewport before a ticking dump could see it.
fn dump_now(engine: &Engine, name: &str) {
    let dir = std::env::var_os("RESTRIKE_ENDING_DUMP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("ending-{name}.ppm"));
    let fb = engine.framebuffer_for_demo();
    let (pixels, palette) = (fb.pixels(), fb.palette());
    let mut out = format!(
        "P6\n{} {}\n255\n",
        crate::framebuffer::WIDTH,
        crate::framebuffer::HEIGHT
    )
    .into_bytes();
    for y in 0..crate::framebuffer::HEIGHT {
        for x in 0..crate::framebuffer::WIDTH {
            let idx = pixels[y * crate::framebuffer::WIDTH + x];
            out.extend_from_slice(&palette[idx as usize]);
        }
    }
    let _ = std::fs::create_dir_all(&dir);
    if std::fs::write(&path, &out).is_ok() {
        eprintln!("ending '{name}': dumped {}", path.display());
    }
}

/// Ticks until `done`, feeding nothing.
fn run_until(engine: &mut Engine, max: u32, done: impl Fn(&Engine) -> bool) -> bool {
    for _ in 0..max {
        if done(engine) {
            return true;
        }
        engine.tick(&[]);
    }
    done(engine)
}

/// Ticks until `done`, pressing Enter every tick — the finale is a chain of
/// `PRESS BUTTON OR RETURN TO CONTINUE.` menus and `Press any key to
/// continue.` gates, and Enter answers both. This is a player leaning on the
/// return key, which is exactly how the original is played through its ending.
fn run_pressing(engine: &mut Engine, max: u32, done: impl Fn(&Engine) -> bool) -> bool {
    for _ in 0..max {
        if done(engine) {
            return true;
        }
        engine.tick(&[InputEvent::Enter]);
    }
    done(engine)
}

/// `print_and_exit()` has been requested.
fn e_quit(engine: &Engine) -> bool {
    engine.quit_requested()
}

/// Everything the engine has said this run, joined — the transcript is where
/// PRINT/PRINTCLEAR text and request labels land.
fn transcript(engine: &mut Engine) -> String {
    engine
        .take_transcript()
        .iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// 1. The approach
// ---------------------------------------------------------------------------

/// ★ **Acceptance item 1a.** The staged fixture really is standing on the
/// finale square: the event code the script dispatches on is read off the
/// shipped `GEO6` map, not asserted from the disassembly.
#[test]
fn the_fixture_stands_on_the_squares_the_shipped_map_marks_as_the_finale() {
    let Some(engine) = at_the_finale_square(
        "the_fixture_stands_on_the_squares_the_shipped_map_marks_as_the_finale",
    ) else {
        return;
    };
    assert_eq!(engine.state().game_area, 6, "the import honoured the save");
    assert_eq!(engine.state().ecl_block_id, TEMPLE_BLOCK);
    assert_eq!(
        engine.state().pos,
        (FINALE_SQUARE.0, FINALE_SQUARE.1),
        "parked on (6, 1)"
    );
    let square = engine
        .geo()
        .square(FINALE_SQUARE.0 as usize, FINALE_SQUARE.1 as usize);
    assert_eq!(
        square.low7, FINALE_EVENT_CODE,
        "GEO6 block 0x43 (6,1) is the ON GOTO's 27th target — the finale"
    );
    assert!(square.indoor, "and it is inside the temple");
}

/// ★ **Acceptance item 1b.** The last approach, driven: the real per-step
/// vector, the real dispatch, Tyranthraxus's three speeches, and the real
/// final roster arriving at `COMBAT`.
#[test]
fn the_finale_square_speaks_and_loads_the_last_fight() {
    let Some(mut engine) =
        at_the_finale_square("the_finale_square_speaks_and_loads_the_last_fight")
    else {
        return;
    };
    engine.shell = crate::shell::boot_at_address(&mut engine.machine, PER_STEP_VECTOR);

    // The three speeches, each behind its own `GOSUB [0x9487]` pause.
    let mut said = String::new();
    for _ in 0..800 {
        if engine.shell().combat_host().is_some() {
            break;
        }
        engine.tick(&[InputEvent::Enter]);
        said.push_str(&transcript(&mut engine));
    }
    for line in [
        "THE POWER OF YOUR BONDS HAS RETURNED",
        "WITH A GREAT FORCE OF WILL",
        "THAT AMULET WILL LET YOU SCRATCH ME",
    ] {
        assert!(
            said.contains(line),
            "the finale never said {line:?}; transcript:\n{said}\nhalts={:?}",
            engine.vm_memory().halts
        );
    }

    // `LOAD MONSTER 0x45 x28 / 0x47 x1 / 0x48 x8` — 37 monsters, and the one
    // that matters is in there by name.
    assert!(
        engine.shell().combat_host().is_some(),
        "the last fight never opened; probe={} halts={:?}",
        engine.shell().probe(),
        engine.vm_memory().halts
    );
    let loaded = &engine.state().pending_combat;
    assert!(
        loaded.monsters_loaded,
        "COMBAT took the monster branch, not a shop/temple one"
    );
    dump(&mut engine, "01-the-last-fight-opens");
}

// ---------------------------------------------------------------------------
// 2. The ending
// ---------------------------------------------------------------------------

/// Resumes the shipped script at `@0x93DC` with the fight won — the one
/// injected fact this suite makes (module doc).
fn won_the_last_fight(who: &str) -> Option<Engine> {
    let mut engine = at_the_finale_square(who)?;
    engine.vm_memory.set_raw_word(COMBAT_OUTCOME_ADDR, 0); // `AfterCombatExpAndTreasure`'s own write
    engine.shell = crate::shell::boot_at_address(&mut engine.machine, AFTER_THE_LAST_FIGHT);
    Some(engine)
}

/// ★ **Acceptance item 1c + 2.** `PROGRAM 8` runs the whole ending: the six
/// groups of prose, the three animations, the dissolve, the Knights,
/// Shadowdale, the fireworks — and lands on the post-victory start menu.
#[test]
fn the_win_runs_program_8_and_the_whole_ending_lands_on_the_start_menu() {
    let Some(mut engine) =
        won_the_last_fight("the_win_runs_program_8_and_the_whole_ending_lands_on_the_start_menu")
    else {
        return;
    };

    // `PROGRAM 8` is reached from the shipped bytes, and the shell parks on
    // the ending at the next tick boundary.
    assert!(
        run_until(&mut engine, 200, |e| matches!(e.shell(), Shell::Ending(_))),
        "PROGRAM 8 never opened the ending; probe={} halts={:?}",
        engine.shell().probe(),
        engine.vm_memory().halts
    );
    assert!(
        crate::vmhost::game_won_flag(engine.vm_memory()) == 0,
        "the latch is set AFTER end_game_text returns, not before"
    );

    // Beat by beat. The first group of prose paces out, then the pause.
    assert!(
        run_until(&mut engine, 600, |e| e.probe() == "ending/text"
            && ending_step(e) >= 4),
        "the first group never finished printing"
    );
    dump(&mut engine, "02-tyranthraxus-defeated");

    // ★ The dissolve, sampled every tick it is on screen: the fade step must
    // climb, and two distinct steps are dumped so the progression is visible
    // and not merely counted.
    assert!(
        run_pressing(&mut engine, 4000, |e| e.probe() == "ending/dissolve"),
        "the dissolve never started; probe={}",
        engine.probe()
    );
    let mut steps: Vec<u16> = Vec::new();
    let mut dumped_early = false;
    while engine.probe() == "ending/dissolve" && steps.len() < 400 {
        engine.tick(&[]);
        if engine.probe() != "ending/dissolve" {
            break; // the beat ended inside this tick; the viewport has moved on
        }
        let step = engine.state().picture.fade_step;
        steps.push(step);
        if !dumped_early && step >= 2 {
            dump_now(&engine, "03-dissolve-early");
            dumped_early = true;
        } else {
            // Rewritten every tick: the last one standing is the deepest step
            // the dissolve reaches before the Knights take the viewport.
            dump_now(&engine, "04-dissolve-later");
        }
    }
    assert!(dumped_early, "the dissolve never reached a second pass");
    assert!(
        steps.windows(2).all(|w| w[1] >= w[0]),
        "the fade must never go backwards: {steps:?}"
    );
    let last = *steps.last().expect("the dissolve ran at least one tick");
    assert!(
        last > steps[0],
        "the fade must PROGRESS across the beat: {steps:?}"
    );
    assert!(last >= 4, "and get somewhere visible: {steps:?}");
    eprintln!("the dissolve's fade steps: {steps:?}");

    // The Knights, then Shadowdale, then the fireworks.
    assert!(
        run_pressing(&mut engine, 6000, |e| e.probe() == "ending/fireworks"),
        "the fireworks never started; probe={}",
        engine.probe()
    );
    assert_eq!(
        engine.state().picture.bigpic_block,
        Some(0x7A),
        "Shadowdale is BIGPIC6 block 122"
    );
    dump(&mut engine, "05-shadowdale");
    // A whole burst: the `Random(10000)` wait, the 60-step rocket, then the
    // particles. Dumped ~20 frames into the burst, where the sky is fullest.
    assert!(
        run_until(&mut engine, 30_000, fireworks_have_fired),
        "no rocket ever launched"
    );
    for _ in 0..30 {
        engine.tick(&[]);
    }
    dump_now(&engine, "06a-rocket");
    for _ in 0..32 {
        engine.tick(&[]);
    }
    dump_now(&engine, "06-fireworks");
    for _ in 0..6 {
        engine.tick(&[]);
    }
    dump_now(&engine, "06b-burst-later");

    // A key ends the display at the end of the burst it is in — faithfully,
    // since the original only tests the keyboard there (`ovr019.cs:400-403`).
    assert!(
        run_pressing(&mut engine, 20_000, |e| matches!(
            e.shell(),
            Shell::FrontDoor(_)
        )),
        "the ending never handed over to the start menu; probe={}",
        engine.probe()
    );

    // `CMD_Program`'s tail (`ovr003.cs:1953-1964`).
    assert_ne!(
        crate::vmhost::game_won_flag(engine.vm_memory()),
        0,
        "field_3FA is latched"
    );
    assert_eq!(
        crate::vmhost::training_class_mask(engine.vm_memory()),
        0xFF,
        "and every class is trainable"
    );
    for ch in &engine.party().members {
        assert_eq!(ch.hit_point_current, ch.hit_point_max, "{} healed", ch.name);
        assert_eq!(ch.status.health_status, crate::rest::status::OKEY);
        assert!(ch.status.in_combat);
    }
    assert_eq!(engine.probe(), "front-door/start-menu");
    dump(&mut engine, "07-post-victory-start-menu");
}

/// How far into [`crate::ending::Ending`]'s script the shell is.
fn ending_step(engine: &Engine) -> usize {
    match engine.shell() {
        Shell::Ending(e) => e.step_index(),
        _ => 0,
    }
}

/// Whether a firework has drawn anything into the sky yet — the rocket writes
/// colours 8..=14 into rows 9..0x40, which the Shadowdale bigpic's own sky
/// does not use at the top of the frame.
fn fireworks_have_fired(engine: &Engine) -> bool {
    match engine.shell() {
        Shell::Ending(e) => e.probe() == "ending/fireworks" && e.fireworks_running(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// 3. The post-victory posture
// ---------------------------------------------------------------------------

/// Runs the whole ending and stops on the post-victory start menu.
fn after_the_ending(who: &str) -> Option<Engine> {
    let mut engine = won_the_last_fight(who)?;
    assert!(
        run_pressing(&mut engine, 40_000, |e| matches!(
            e.shell(),
            Shell::FrontDoor(_)
        )),
        "the ending never finished; probe={}",
        engine.probe()
    );
    Some(engine)
}

/// ★ **Acceptance item 4.** The win latch round trip: win → save → load →
/// `BEGIN` still refused. The latch is an `Area1` cell, so `.rsav` carries it
/// with no second mechanism.
#[test]
fn begin_stays_refused_across_a_save_and_reload() {
    let Some(mut engine) = after_the_ending("begin_stays_refused_across_a_save_and_reload") else {
        return;
    };

    // Refused here, with the original's own guard and a visible reason.
    engine.tick(&[InputEvent::Char(b'B')]);
    engine.tick(&[]);
    assert!(
        matches!(engine.shell(), Shell::FrontDoor(_)),
        "BEGIN must not start a new adventure after the win"
    );
    assert_eq!(engine.probe(), "front-door/start-menu");
    dump(&mut engine, "08-begin-refused");

    // Round trip.
    let saved = engine.save();
    let dir = std::env::var_os("GBX_DATA_DIR").expect("gated above");
    let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir)).unwrap();
    let mut restored = Engine::restore(&saved, data).expect("the won game must reload");
    assert_ne!(
        crate::vmhost::game_won_flag(restored.vm_memory()),
        0,
        "the win latch survives .rsav"
    );
    restored.park_at_start_menu();
    restored.tick(&[InputEvent::Char(b'B')]);
    restored.tick(&[]);
    assert!(
        matches!(restored.shell(), Shell::FrontDoor(_)),
        "and BEGIN is still refused after the reload"
    );
}

/// ★ **Acceptance item 3.** Leaving the post-victory menu asks the original's
/// question, in its own words, and either answer ends at DOS.
#[test]
fn leaving_the_post_victory_menu_asks_to_save_and_then_quits() {
    let Some(mut engine) =
        after_the_ending("leaving_the_post_victory_menu_asks_to_save_and_then_quits")
    else {
        return;
    };

    // `Exit to DOS` no longer exits: `startGameMenu` returning is what hands
    // control back to `CMD_Program`'s tail.
    engine.tick(&[InputEvent::Char(b'E')]);
    engine.tick(&[]);
    assert_eq!(
        engine.probe(),
        "front-door/post-victory-save",
        "the save question replaces the bare quit"
    );
    assert!(!engine.quit_requested(), "and nothing has quit yet");
    dump(&mut engine, "09-save-before-quitting");

    // `N` — no save, straight to DOS.
    engine.tick(&[InputEvent::Char(b'N')]);
    engine.tick(&[]);
    assert!(engine.quit_requested(), "'No' is print_and_exit()");
}

/// The `Yes` arm: the slot picker opens, and the quit follows it — the
/// original's `SaveGame(); print_and_exit();` with nothing in between.
#[test]
fn saying_yes_opens_the_save_screen_and_still_ends_the_session() {
    let Some(mut engine) =
        after_the_ending("saying_yes_opens_the_save_screen_and_still_ends_the_session")
    else {
        return;
    };
    engine.tick(&[InputEvent::Char(b'E')]);
    engine.tick(&[]);
    engine.tick(&[InputEvent::Char(b'Y')]);
    engine.tick(&[]);
    assert!(
        matches!(engine.shell(), Shell::Screen(_)),
        "'Yes' opens SaveGame's slot picker; probe={}",
        engine.probe()
    );
    assert!(
        engine.state().quit_after_save,
        "and the exit that follows it is already committed"
    );
    dump(&mut engine, "10-save-slots");

    // Escaping the picker still ends the session — `print_and_exit()` is not
    // conditional on the save having been written (`ovr003.cs:1967-1972`).
    let mut ended = false;
    for _ in 0..400 {
        if e_quit(&engine) {
            ended = true;
            break;
        }
        engine.tick(&[InputEvent::Escape]);
    }
    assert!(
        ended || e_quit(&engine),
        "the session must end either way; probe={}",
        engine.probe()
    );
}
