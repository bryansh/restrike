//! Local-only demo artifacts (gated on `GBX_DATA_DIR`). Two demos live
//! here: step 2's static-screen compose, and step 3's task deliverable —
//! walking real Tilverton streets (`GEO2.DAX` block 1) headlessly through
//! `Engine::tick`, turning, stepping, and bashing through a real locked
//! door, dumping frames as `.ppm` outside the repo.

#![cfg(test)]

use crate::boot::boot;
use crate::framebuffer::{Framebuffer, HEIGHT, WIDTH};
use crate::frames::draw8x8_03;
use crate::text::{draw_string, JobStatus, TextCursor, TextJob, NORMAL_BOTTOM};
use gbx_formats::game_data::load_dir;

fn write_ppm(fb: &Framebuffer, path: &std::path::Path) {
    let mut out = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let idx = fb.get_pixel(x, y);
            out.extend_from_slice(&fb.palette()[idx as usize]);
        }
    }
    std::fs::write(path, &out).expect("failed to write demo .ppm");
}

#[test]
fn compose_empty_exploration_screen() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    let data = load_dir(dir).expect("GBX_DATA_DIR must be readable");
    let assets = boot(&data).expect("boot must succeed against real CotAB data");

    let mut fb = Framebuffer::new();

    draw8x8_03(&mut fb, &assets.symbol_sets).expect("draw8x8_03 must succeed with a booted set 4");

    // Party-panel header glyphs (`PartySummary`, `ovr025.cs:216-261`, §1.9):
    // "Name" at (2,17), "AC  HP" at (2,33).
    draw_string(&mut fb, &assets.font, "Name", 2, 17, 0, 10);
    draw_string(&mut fb, &assets.font, "AC  HP", 2, 33, 0, 10);

    // A sample PRINT into the exploration text window. The string is invented
    // demo text, NOT from game data (D10) — deliberately lore-inaccurate as
    // proof (Tilverton borders Cormyr; it is nowhere near the Moonsea).
    let mut cursor = TextCursor {
        col: NORMAL_BOTTOM.x_start,
        row: NORMAL_BOTTOM.y_start,
    };
    let mut job = TextJob::new(
        "You stand at the gates of Tilverton, the free city of the Moonsea.",
        10,
        NORMAL_BOTTOM,
        true,
        &mut cursor,
        &mut fb,
    );
    loop {
        match job.advance(1_000_000, &mut fb, &assets.font, &mut cursor) {
            JobStatus::Done => break,
            JobStatus::NeedsKey => job.release(&mut fb),
            JobStatus::Continuing => unreachable!("budget was effectively unlimited"),
        }
    }

    let out_path = std::env::var_os("RESTRIKE_M2_DEMO_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("restrike-m2-demo.ppm"));
    write_ppm(&fb, &out_path);
    eprintln!("M2 demo screen written to {}", out_path.display());
}

/// M2 step 3's local-only exit-gate demo (task deliverable): walks real
/// Tilverton streets headlessly from the original's own boot spawn
/// (`mapPosX=7, mapPosY=13, mapDirection=0` — North — `seg001.cs:250-252`,
/// this session's research) to a real locked door discovered this session
/// by BFS over `GEO2.DAX` block 1 via `wall_door_flags` (the same query the
/// engine itself uses): a door on square `(7,12)`'s North edge, reached via
/// West, North, East from spawn. The VM stays a stub (real `EclMachine`
/// binding is step 4); every vector run is scripted as a trivial `Ended`
/// with no effects, so the frames below show pure walk-loop/renderer state,
/// no event text.
#[test]
fn walk_tilverton_and_bash_a_real_door() {
    use crate::engine::Engine;
    use crate::input::{ExtKey, InputEvent};
    use crate::movement::Facing;
    use crate::shell::Shell;
    use gbx_vm::{Exit, VmStep};

    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    let data = load_dir(dir).expect("GBX_DATA_DIR must be readable");
    let mut engine = Engine::new(data, 1).expect("Engine::new must boot against real CotAB data");

    // The original's own Tilverton spawn (`seg001.cs:250-252`).
    engine.state.pos = (7, 13);
    engine.state.facing = Facing::North;
    engine.party_predicates_mut().bash_candidates = vec![(25, 0)]; // STR 25: automatic bash success

    let out_dir = std::env::var_os("RESTRIKE_M2_WALK_DEMO_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    // Every forward step runs a StepFlow (2 scripted vector calls); turns
    // run none. Boot's own entry vector is auto-scripted by `Engine::build`.
    let ended = || vec![VmStep::Done(Exit::Ended)];
    for _ in 0..4 {
        // 3 open-square steps + 1 into the door.
        engine.script_vm_call(ended());
        engine.script_vm_call(ended());
    }

    fn tick_until(
        engine: &mut Engine,
        max_ticks: u32,
        input: &[InputEvent],
        mut done: impl FnMut(&Engine) -> bool,
    ) {
        engine.tick(input);
        for _ in 0..max_ticks {
            if done(engine) {
                return;
            }
            engine.tick(&[]);
        }
        assert!(
            done(engine),
            "trace step did not converge within {max_ticks} ticks"
        );
    }

    // Reach the world menu.
    tick_until(&mut engine, 10, &[], |e| {
        matches!(e.shell, Shell::WorldMenu { .. })
    });
    let frame1_path = out_dir.join("restrike-walk-demo-1-spawn.ppm");
    {
        let f = engine.tick(&[]);
        let mut fb = Framebuffer::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
            }
        }
        write_ppm(&fb, &frame1_path);
    }

    // Turn West, step, turn North, step, turn East, step: (7,13) -> (7,12).
    let turns_and_steps: &[InputEvent] = &[
        InputEvent::Ext(ExtKey::Left),  // face West
        InputEvent::Ext(ExtKey::Up),    // step to (6,13)
        InputEvent::Ext(ExtKey::Right), // face North
        InputEvent::Ext(ExtKey::Up),    // step to (6,12)
        InputEvent::Ext(ExtKey::Right), // face East
        InputEvent::Ext(ExtKey::Up),    // step to (7,12)
        InputEvent::Ext(ExtKey::Left),  // face North, toward the door
    ];
    for event in turns_and_steps {
        tick_until(&mut engine, 10, &[*event], |e| {
            matches!(e.shell, Shell::WorldMenu { .. })
        });
    }
    assert_eq!(engine.state.pos, (7, 12));
    assert_eq!(engine.state.facing, Facing::North);

    // Step into the locked door: opens the Bash/Exit menu (no move yet).
    tick_until(
        &mut engine,
        10,
        &[InputEvent::Ext(ExtKey::Up)],
        |e| matches!(&e.shell, Shell::Step(flow) if flow.door_widget_is_some()),
    );
    let frame2_path = out_dir.join("restrike-walk-demo-2-door-menu.ppm");
    {
        let f = engine.tick(&[]);
        let mut fb = Framebuffer::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
            }
        }
        write_ppm(&fb, &frame2_path);
    }

    // Bash it down.
    tick_until(&mut engine, 10, &[InputEvent::Char(b'b')], |e| {
        matches!(e.shell, Shell::WorldMenu { .. })
    });
    assert_eq!(
        engine.state.pos,
        (7, 11),
        "the bash must succeed and step through"
    );
    let frame3_path = out_dir.join("restrike-walk-demo-3-through-door.ppm");
    {
        let f = engine.tick(&[]);
        let mut fb = Framebuffer::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
            }
        }
        write_ppm(&fb, &frame3_path);
    }

    eprintln!(
        "M2 step 3 walk demo frames written to {}, {}, {}",
        frame1_path.display(),
        frame2_path.display(),
        frame3_path.display()
    );
}
