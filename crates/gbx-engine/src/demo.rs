//! Local-only demo artifacts (gated on `GBX_DATA_DIR`). Two demos live
//! here: step 2's static-screen compose, and M2 step 4's task deliverable —
//! walking real Tilverton streets (`GEO2.DAX` block 1) headlessly through
//! `Engine::tick`, running the *real* `ECL2.DAX` block 1 scripts (the VM is
//! no longer a stub as of step 4), turning, stepping, and bashing through a
//! real locked door, dumping frames as `.ppm` outside the repo and printing
//! the ScriptMemory unknown-access log + service-call log.

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

/// M2 step 4's local-only exit-gate demo (task deliverable): walks real
/// Tilverton streets headlessly from the original's own boot spawn to a
/// real locked door discovered by BFS over `GEO2.DAX` block 1 via
/// `wall_door_flags` (the same query the engine itself uses): a door on
/// square `(7,12)`'s North edge, reached via West, North, East from spawn.
/// Unlike step 3's demo, the VM here is the real `EclMachine` running the
/// genuine `ECL2.DAX` block 1 scripts — whatever text/menus/effects that
/// content produces are handled by the real widget/text-system wiring, not
/// scripted; `pos`/`facing` are left for the real boot vector to set (no
/// manual override) rather than assumed.
///
/// **Correction (this session, running real content — supersedes step 3's
/// citation):** step 3's research read the spawn as `mapPosX=7, mapPosY=13,
/// mapDirection=0` (North, `seg001.cs:250-252`). Running `ECL2.DAX` block 1
/// vector 4 for real (`run-script --dax ECL2.DAX --block 1 --vector 4`)
/// shows it writes `0xC04B=7, 0xC04C=13, 0xC04D=1` — position matches, but
/// `0xC04D=1` (the halved facing encoding) decodes to raw `2` = **East**,
/// not North. Docketed for a `seg001.cs` re-read; this demo trusts the real
/// engine's own decoded state over the earlier citation.
#[test]
fn walk_tilverton_and_bash_a_real_door() {
    use crate::engine::Engine;
    use crate::input::{ExtKey, InputEvent};
    use crate::movement::Facing;
    use crate::shell::Shell;

    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    let data = load_dir(dir).expect("GBX_DATA_DIR must be readable");
    let mut engine = Engine::new(data, 1).expect("Engine::new must boot against real CotAB data");
    engine.party_predicates_mut().bash_candidates = vec![(25, 0)]; // STR 25: automatic bash success

    let out_dir = std::env::var_os("RESTRIKE_M2_WALK_DEMO_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    fn dump(engine: &mut Engine, path: &std::path::Path) {
        let f = engine.tick(&[]);
        let mut fb = Framebuffer::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
            }
        }
        write_ppm(&fb, path);
    }

    /// Real ECL content may print text (multi-tick pagination), open
    /// engine-owned menus, or hit an unimplemented/unknown opcode (the M2
    /// halt policy just ends the run loudly, never blocking) — this ticks
    /// generously and, if a PressAnyKey/pagination gate opens along the
    /// way and `input` doesn't resolve it, feeds a keypress to clear it, so
    /// the walk isn't derailed by real event text the fixed script traces
    /// never had to handle.
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
            engine.tick(&[InputEvent::Enter]); // clears any pagination/press-any-key gate
        }
        assert!(
            done(engine),
            "trace step did not converge within {max_ticks} ticks"
        );
    }

    // Reach the world menu. The real boot vector sets pos=(7,13),
    // facing=East (see this fn's doc comment).
    tick_until(&mut engine, 200, &[], |e| {
        matches!(e.shell, Shell::WorldMenu { .. })
    });
    assert_eq!(engine.state.pos, (7, 13));
    assert_eq!(engine.state.facing, Facing::East);
    let frame1_path = out_dir.join("restrike-walk-demo-1-spawn.ppm");
    dump(&mut engine, &frame1_path);

    // Turn around (East->West), step, turn right (North), step, turn right
    // (East), step: (7,13) -> (7,12).
    let turns_and_steps: &[InputEvent] = &[
        InputEvent::Ext(ExtKey::Down),  // face West (turn around)
        InputEvent::Ext(ExtKey::Up),    // step to (6,13)
        InputEvent::Ext(ExtKey::Right), // face North
        InputEvent::Ext(ExtKey::Up),    // step to (6,12)
        InputEvent::Ext(ExtKey::Right), // face East
        InputEvent::Ext(ExtKey::Up),    // step to (7,12)
        InputEvent::Ext(ExtKey::Left),  // face North, toward the door
    ];
    for event in turns_and_steps {
        tick_until(&mut engine, 200, &[*event], |e| {
            matches!(e.shell, Shell::WorldMenu { .. })
        });
    }
    assert_eq!(engine.state.pos, (7, 12));
    assert_eq!(engine.state.facing, Facing::North);

    // Step into the locked door: opens the Bash/Exit menu (no move yet).
    tick_until(
        &mut engine,
        200,
        &[InputEvent::Ext(ExtKey::Up)],
        |e| matches!(&e.shell, Shell::Step(flow) if flow.door_widget_is_some()),
    );
    let frame2_path = out_dir.join("restrike-walk-demo-2-door-menu.ppm");
    dump(&mut engine, &frame2_path);

    // Bash it down.
    tick_until(&mut engine, 200, &[InputEvent::Char(b'b')], |e| {
        matches!(e.shell, Shell::WorldMenu { .. })
    });
    assert_eq!(
        engine.state.pos,
        (7, 11),
        "the bash must succeed and step through"
    );
    let frame3_path = out_dir.join("restrike-walk-demo-3-through-door.ppm");
    dump(&mut engine, &frame3_path);

    eprintln!(
        "M2 step 4 walk demo frames written to {}, {}, {}",
        frame1_path.display(),
        frame2_path.display(),
        frame3_path.display()
    );

    let vm = engine.vm_memory();
    eprintln!(
        "unknown-access log: {} distinct (addr, kind) entries",
        vm.unknown_log.entries().len()
    );
    for entry in vm.unknown_log.entries().iter().take(30) {
        eprintln!(
            "  {:#06X} {:?} (pc={:#06X})",
            entry.addr, entry.kind, entry.origin.pc
        );
    }
    eprintln!("service calls: {} total", vm.calls.len());
    for call in vm.calls.iter().take(30) {
        eprintln!("  {call:?}");
    }
    eprintln!("halts: {} total", vm.halts.len());
    for halt in &vm.halts {
        eprintln!(
            "  pc={:#06X} opcode={:#04X}: {}",
            halt.pc, halt.opcode, halt.description
        );
    }
}

/// M2 step 4's boot-scene capture (audit addition, completing that task's
/// demo deliverable): tick the real boot with NO input, dumping a frame each
/// time the screen goes quiet — a gate awaiting a keypress — then feeding
/// Enter, until the world menu arrives. The opening scene, rendered by the
/// real pipeline.
#[test]
fn boot_scene_frames() {
    use crate::engine::Engine;
    use crate::input::InputEvent;
    use crate::shell::Shell;

    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let data = load_dir(std::path::Path::new(&dir)).expect("GBX_DATA_DIR must be readable");
    let mut engine = Engine::new(data, 1).expect("Engine::new must boot against real CotAB data");

    let out_dir = std::env::var_os("RESTRIKE_M2_WALK_DEMO_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    let mut captures = 0u32;
    let mut quiet = 0u32;
    let mut last_serial = u64::MAX;
    for _ in 0..2000 {
        let serial = engine.tick(&[]).serial;
        if matches!(engine.shell, Shell::WorldMenu { .. }) {
            break;
        }
        if serial == last_serial {
            quiet += 1;
        } else {
            quiet = 0;
            last_serial = serial;
        }
        if quiet >= 5 {
            captures += 1;
            let path = out_dir.join(format!("restrike-boot-scene-{captures}.ppm"));
            let f = engine.tick(&[]);
            let mut fb = Framebuffer::new();
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
                }
            }
            write_ppm(&fb, &path);
            eprintln!("boot-scene capture {captures} -> {}", path.display());
            engine.tick(&[InputEvent::Enter]);
            quiet = 0;
            last_serial = u64::MAX;
            if captures >= 6 {
                break;
            }
        }
    }
    assert!(
        captures >= 1,
        "expected at least one boot-scene gate to capture"
    );
    eprintln!(
        "boot unknown-access log ({} entries): {:#?}",
        engine.vm_memory().unknown_log.entries().len(),
        engine.vm_memory().unknown_log.entries()
    );
    eprintln!("boot halts: {:?}", engine.vm_memory().halts);
}

/// M2 step 5's task deliverable 5: dumps the Tilverton spawn square's real
/// 3D corridor viewport at all four facings (turning right after each
/// capture), through the real `EclMachine`, `LoadWalldef`-loaded wallsets,
/// and `crate::corridor` renderer — no scripted geometry, whatever the
/// resident `GEO2.DAX` block 1 and the area's real walldefs actually
/// produce.
#[test]
fn four_facings_at_spawn() {
    use crate::engine::Engine;
    use crate::input::{ExtKey, InputEvent};
    use crate::shell::Shell;

    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let data = load_dir(std::path::Path::new(&dir)).expect("GBX_DATA_DIR must be readable");
    let mut engine = Engine::new(data, 1).expect("Engine::new must boot against real CotAB data");

    let out_dir = std::env::var_os("RESTRIKE_M2_WALK_DEMO_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    fn tick_until_world_menu(engine: &mut Engine, input: &[InputEvent]) {
        let was_area_map = engine.state.area_map_shown;
        engine.tick(input);
        for _ in 0..200 {
            if matches!(engine.shell, Shell::WorldMenu { .. }) {
                break;
            }
            engine.tick(&[InputEvent::Enter]);
        }
        assert!(
            matches!(engine.shell, Shell::WorldMenu { .. }),
            "did not reach the world menu within budget"
        );

        // A discovered engine quirk (step 5, flagged not silently
        // absorbed): the drain-to-last `InputQueue` can leave an
        // unconsumed `Enter` queued by this very loop's own gate-clearing
        // fallback — pushed on the tick that transitions e.g. Boot ->
        // WorldMenu, but that tick's own flow stage never reads it (the
        // newly-created WorldMenu widget doesn't exist until *after* that
        // tick resolves). A later empty-input tick then drains it, and
        // since WorldMenu's hotbar defaults to highlighting its first word
        // ("Area"), a stray `Enter` silently resolves as `'A'`
        // (`ToggleAreaView`) — found via this demo's four-facings capture
        // showing an identical viewport across every facing (the area map
        // doesn't depend on facing beyond the party-arrow glyph). Flush one
        // empty tick here, where the effect is harmless to observe, and if
        // it fired, press `'A'` again to restore the intended view.
        // Docketed alongside §1.11 item 9's existing drain-to-last
        // uncertainty — a DOSBox check settles whether this exact
        // interaction is also present in the original, or is an engine-only
        // seam (a widget created mid-tick never gets a chance to "claim"
        // that tick's own input) worth closing in `shell.rs` directly.
        engine.tick(&[]);
        if engine.state.area_map_shown != was_area_map {
            engine.tick(&[InputEvent::Char(b'a')]);
        }
    }

    fn dump(engine: &mut Engine, path: &std::path::Path) {
        let f = engine.tick(&[]);
        let mut fb = Framebuffer::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
            }
        }
        write_ppm(&fb, path);
    }

    tick_until_world_menu(&mut engine, &[]);
    assert_eq!(
        engine.state.pos,
        (7, 13),
        "spawn position must be unchanged"
    );

    let mut paths = Vec::new();
    for i in 0..4 {
        let facing = engine.state.facing;
        let path = out_dir.join(format!("restrike-four-facings-{i}-{facing:?}.ppm"));
        dump(&mut engine, &path);
        eprintln!(
            "four-facings capture {i}: facing {facing:?} -> {}",
            path.display()
        );
        paths.push(path);
        tick_until_world_menu(&mut engine, &[InputEvent::Ext(ExtKey::Right)]);
    }
    eprintln!(
        "four-facings frames written to: {}",
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let symbols = engine.symbol_sets();
    for slot in 0..3 {
        eprintln!(
            "wallset slot {slot} (LOAD PIECES set {}) loaded: {}",
            slot + 1,
            symbols.wallset(slot).is_some()
        );
    }
}
