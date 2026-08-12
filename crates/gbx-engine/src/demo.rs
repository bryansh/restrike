//! Local-only demo artifacts (gated on `GBX_DATA_DIR`). The originals are
//! step 2's static-screen compose and M2 step 4's task deliverable — walking
//! real Tilverton streets (`GEO2.DAX` block 1) headlessly through
//! `Engine::tick`, running the *real* `ECL2.DAX` block 1 scripts (the VM is
//! no longer a stub as of step 4), turning, stepping, and bashing through a
//! real locked door, dumping frames as `.ppm` outside the repo and printing
//! the ScriptMemory unknown-access log + service-call log.
//!
//! M6 slice 6 adds the two that close M6b: [`m6b_boot_to_the_bar_brawl`] and
//! [`m6b_a_party_wipe_shows_its_ending_before_game_over`] — boot the bundled
//! save, walk to the Tilverton bar, and watch the fight happen on screen.

#![cfg(test)]

use crate::boot::boot;
use crate::framebuffer::{Framebuffer, HEIGHT, WIDTH};
use crate::frames::draw8x8_03;
use crate::text::{draw_string, JobStatus, TextCursor, TextJob, NORMAL_BOTTOM};
use gbx_formats::game_data::load_dir;

/// A `Frame`'s pixels, straight to `.ppm` — for drives that must dump the
/// frame they just ticked rather than spend an extra tick to get one.
fn write_ppm_pixels(pixels: &[u8], path: &std::path::Path) {
    let fb = Framebuffer::new();
    let mut out = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for &idx in pixels.iter() {
        out.extend_from_slice(&fb.palette()[idx as usize]);
    }
    std::fs::write(path, &out).expect("failed to write demo .ppm");
}

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
///
/// **Second correction (M2 step 8, post-DIVIDE):** see the inline comment at
/// this fn's final assertion — the door no longer "bashes through" now that
/// the per-step script runs to completion; it turns out to gate a real area
/// transition M2 doesn't implement (FD-19).
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
        let mut last_serial = u64::MAX;
        let mut quiet = 0u32;
        for _ in 0..max_ticks {
            if done(engine) {
                return;
            }
            // Feed a key only when the screen has gone quiet — a gate
            // actually waiting on input. Blind Enter-spam piles keys into
            // the queue, and drain-to-last hands the newest one to whatever
            // widget opens next, where Enter selects the highlighted first
            // word ("Area" in the world menu, "Bash" in the door menu).
            let feed: &[InputEvent] = if quiet >= 2 {
                quiet = 0;
                &[InputEvent::Enter]
            } else {
                &[]
            };
            let serial = engine.tick(feed).serial;
            if serial == last_serial {
                quiet += 1;
            } else {
                quiet = 0;
                last_serial = serial;
            }
        }
        assert!(
            done(engine),
            "trace step did not converge within {max_ticks} ticks"
        );
    }

    // Reach the world menu. The real boot vector sets pos=(7,13),
    // facing=East (see this fn's doc comment).
    tick_until(&mut engine, 600, &[], |e| {
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
        tick_until(&mut engine, 600, &[*event], |e| {
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
    tick_until(&mut engine, 600, &[InputEvent::Char(b'b')], |e| {
        matches!(e.shell, Shell::WorldMenu { .. })
    });
    // ★ FD-19 RESOLVED (roll-credits slice 1). Two earlier readings of this
    // door were both wrong, and the disassembly settles it.
    //
    // M2 step 4 expected (7,11) — a plain bash-through. That was only ever
    // true because vector 1 halted on DIVIDE before the door's real logic ran
    // (FD-9). M2 step 8 then read the (0,0) landing as a half-implemented
    // *area transition*, citing a `Load3dMap { block_id: 1 }` in the service
    // log — but that call is the boot `LOAD FILES`, the only one in the whole
    // run, and the door writes no `0x7F12` at all (the disassembly has exactly
    // two `SAVE _, 0x7F12` sites in the shipped game, `ECL4#37 @0x8225` and
    // `ECL5#48 @0x8092`, neither of them here).
    //
    // What the script actually does (`ECL2.DAX` block 1): the (7,12)-North
    // event forks. The refusal arm — the one a party without the story flag
    // takes — prints its in-fiction "wrong entrance" line and then copies
    // `area_ptr.lastXPos`/`lastYPos` straight back into `mapPosX`/`mapPosY`
    // (`@0x9444`/`@0x944B`) to put the party where it stood. Nothing
    // maintained those two cells (`ovr003.cs:2371-2372`), so they read 0 and
    // the party was teleported to the origin. THAT was the (0,0).
    //
    // The other arm is a guarded fight (`@0x945D`: CLEARMONSTERS, LOAD
    // MONSTER 0×5, COMBAT) and, on a win, a real teleport to (9,3) facing
    // south followed by `NEWECL 2` (`@0x987B`→`@0x9911`→`@0x9830`) — a
    // same-area block change, not a cross-area one.
    //
    // So the correct outcome for this walk is: the party is turned away and
    // put back on the square it stepped from.
    assert_eq!(
        engine.state.pos,
        (7, 12),
        "the refusal arm restores lastXPos/lastYPos — the party is turned away, not teleported"
    );
    assert_eq!(
        engine.state.last_pos,
        (7, 12),
        "and those cells hold the position the walk loop recorded before the door"
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

/// M3 step 6 deliverable 1's acceptance check (local-only, `GBX_DATA_DIR`):
/// import GOG's bundled slot-A save and render MATHEW's real character sheet,
/// asserting every value on `charsheet-mathew-slotA.png` is reproduced *from
/// the real save bytes* (the synthetic-fixture test in `charsheet.rs` proves
/// the transforms; this proves they land on the genuine record). Dumps the
/// rendered sheet as a `.ppm` outside the repo (D10). Loud-skips without data.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture render_mathews_real_character_sheet`
#[test]
fn render_mathews_real_character_sheet() {
    use crate::charsheet::{render_sheet, sheet_view};

    let Some(root) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (render_mathews_real_character_sheet)");
        return;
    };
    let root = std::path::Path::new(&root);
    let data = load_dir(root).expect("GBX_DATA_DIR must be readable");

    // GOG's bundled save lives under SAVE/ (FD-23).
    let save_dir = root.join("SAVE");
    let saves = load_dir(&save_dir).expect("GBX_DATA_DIR/SAVE must be readable");
    let master_bytes = saves
        .raw_file("SAVGAMA.DAT")
        .expect("GBX_DATA_DIR/SAVE/SAVGAMA.DAT must exist");
    let set =
        gbx_formats::save_orig::load_from_lookup(master_bytes, 'A', |name| saves.raw_file(name))
            .expect("the bundled slot-A save set must parse");

    let engine = crate::import::import_original(&set, data, 0x5A1E_5A1E)
        .expect("importing the bundled save succeeds");

    let mathew = &engine.party().members[0];
    let view = sheet_view(mathew);
    eprintln!("MATHEW sheet: {view:#?}");

    // The reference-capture acceptance values (charsheet-mathew-slotA.png).
    assert_eq!(view.name, "MATHEW");
    assert_eq!(view.identity, "Male Human Age 20");
    assert_eq!(view.alignment, "Lawful Good");
    assert_eq!(view.class, "Paladin");
    assert_eq!(view.stats[0].value, "18");
    assert_eq!(view.stats[0].exceptional.as_deref(), Some("(00)"));
    assert_eq!(view.level, "5");
    assert_eq!(view.exp, 25000);
    assert_eq!(view.hp_current, 49);
    assert_eq!(view.ac, 7);
    assert_eq!(view.thac0, 13);
    assert_eq!(view.encumbrance, 300);
    assert_eq!(view.movement, 12);
    assert_eq!(view.status, "Okay");
    assert_eq!(view.damage, "1d2+6");
    assert_eq!(
        view.money,
        vec![crate::charsheet::CoinRow {
            name: "Platinum".into(),
            amount: 300
        }]
    );

    // Render it through the real font/symbol pipeline and dump the frame.
    let mut fb = Framebuffer::new();
    render_sheet(&mut fb, engine.font(), engine.symbol_sets(), &view);
    let out = std::env::var_os("RESTRIKE_M2_WALK_DEMO_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("restrike-charsheet-mathew.ppm");
    write_ppm(&fb, &out);
    eprintln!("MATHEW character sheet rendered to {}", out.display());
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

// --- M3 step 6 deliverable 6: the exit-gate demo ---

/// The M3 exit gate (local-only, `GBX_DATA_DIR`): import GOG's bundled slot-A
/// save → walk a few squares in Tilverton → enter a shop and buy an item →
/// train an XP-eligible character with pack-correct numbers → `Engine::save`
/// → `Engine::restore` → assert the save round-trips byte-identically (the
/// state-hash equality). Prints a step-by-step transcript.
///
/// One reproducible command:
/// `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture m3_exit_gate`
#[test]
fn m3_exit_gate() {
    use crate::engine::Engine;
    use crate::input::{ExtKey, InputEvent};
    use crate::shell::Shell;
    use crate::shop::{Shop, ShopItem};
    use sha2::{Digest, Sha256};

    let Some(root) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: M3 exit gate needs GBX_DATA_DIR (m3_exit_gate)");
        return;
    };
    let root = std::path::Path::new(&root);
    let data = load_dir(root).expect("GBX_DATA_DIR must be readable");

    // --- Step 1: import the bundled slot-A save ---
    let save_dir = root.join("SAVE");
    let saves = load_dir(&save_dir).expect("GBX_DATA_DIR/SAVE must be readable");
    let master = saves
        .raw_file("SAVGAMA.DAT")
        .expect("bundled SAVGAMA.DAT must exist");
    let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n))
        .expect("bundled slot-A save must parse");
    let mut engine =
        crate::import::import_original(&set, data.clone(), 0x5A1E_5A1E).expect("import succeeds");
    eprintln!("== M3 EXIT GATE ==");
    eprintln!(
        "[1] imported slot-A: Tilverton at {:?}, {} members",
        engine.state().pos,
        engine.party().members.len()
    );

    /// Ticks to the world menu, feeding Enter on any gate that goes quiet so
    /// real event text doesn't stall the walk (the M2 demo's pattern).
    fn to_world_menu(engine: &mut Engine, input: &[InputEvent]) {
        engine.tick(input);
        let mut last = u64::MAX;
        let mut quiet = 0u32;
        for _ in 0..800 {
            if matches!(engine.shell(), Shell::WorldMenu { .. }) {
                return;
            }
            let feed: &[InputEvent] = if quiet >= 2 {
                quiet = 0;
                &[InputEvent::Enter]
            } else {
                &[]
            };
            let serial = engine.tick(feed).serial;
            if serial == last {
                quiet += 1;
            } else {
                quiet = 0;
                last = serial;
            }
        }
        assert!(
            matches!(engine.shell(), Shell::WorldMenu { .. }),
            "did not reach the world menu"
        );
    }

    // --- Step 2: walk a few squares ---
    to_world_menu(&mut engine, &[]);
    let spawn = engine.state().pos;
    // Turn around (face West) then step a couple of squares.
    to_world_menu(&mut engine, &[InputEvent::Ext(ExtKey::Down)]);
    to_world_menu(&mut engine, &[InputEvent::Ext(ExtKey::Up)]);
    to_world_menu(&mut engine, &[InputEvent::Ext(ExtKey::Up)]);
    let walked = engine.state().pos;
    eprintln!("[2] walked {spawn:?} -> {walked:?}");
    assert_ne!(walked, spawn, "the party moved");

    // --- Step 3: enter a shop and buy an item ---
    // Tilverton's arms shop stock (the real inventory comes from the ECL
    // TREASURE opcode, M6 — see shop.rs; here it is host-supplied, D10-clean).
    engine.state.selected_player = 0;
    let buyer_name = engine.party().members[0].name.clone();
    let items_before = engine.party().members[0].items.len();
    let shop = Shop::new(
        vec![
            ShopItem::synthetic("Dagger", 2, 10),
            ShopItem::synthetic("Long Sword", 10, 60),
        ],
        0x00,
    );
    engine.enter_shop(shop);
    // Buy → pick the first item.
    engine.tick(&[InputEvent::Char(b'b')]);
    engine.tick(&[InputEvent::Enter]);
    engine.tick(&[]);
    let items_after = engine.party().members[0].items.len();
    eprintln!(
        "[3] {buyer_name} bought an item: inventory {items_before} -> {items_after}, weight {}",
        engine.party().members[0].combat.weight
    );
    assert_eq!(items_after, items_before + 1, "an item was purchased");
    // Leave the shop back to the walk loop: Esc closes the Buy list, then
    // Exit from the shop menu returns to the world menu.
    engine.tick(&[InputEvent::Escape]);
    engine.tick(&[InputEvent::Char(b'e')]);
    to_world_menu(&mut engine, &[]);

    // --- Step 4: train an eligible character ---
    // Probe the bundled six for a naturally XP-eligible member.
    let natural = engine.party().members.iter().position(|m| {
        !crate::training::trainable_classes(m, engine.rules(), crate::training::TRAINS_ALL_CLASSES)
            .is_empty()
    });
    let trainee = match natural {
        Some(i) => {
            eprintln!("[4] member {i} is XP-eligible naturally");
            i
        }
        None => {
            // DEV-ONLY HOOK (clearly marked): no bundled member has enough XP
            // *yet*. Since roll-credits slice 3 the party really does earn
            // experience from fights (`crate::award`), so this is a shortcut
            // past the grinding a demo has no business doing — not a stand-in
            // for a missing mechanism, which is what it was before.
            // (MATHEW the paladin is L5 with 25000 XP; L5->L6 needs 45001).
            // Grant member 0 exactly the threshold so training proceeds — the
            // *training numbers* below are still fully pack-correct.
            let m = &mut engine.party.members[0];
            let (class, level) = (
                m.class_levels()[0].class,
                m.class_levels()[0].level as usize,
            );
            let threshold =
                gbx_rules::adnd1::progression::exp_threshold(engine.rules(), class, level)
                    .expect("a trainable class has a threshold");
            engine.party.members[0].exp = threshold;
            eprintln!(
                "[4] DEV-HOOK: granted member 0 the L{level}->L{} XP threshold ({threshold})",
                level + 1
            );
            0
        }
    };
    engine.state.selected_player = trainee as u8;
    let level_before = engine.party().members[trainee].class_level;
    let hp_before = engine.party().members[trainee].hit_point_max;
    engine.open_training();
    engine.tick(&[InputEvent::Char(b't')]); // Train
    engine.tick(&[]);
    let level_after = engine.party().members[trainee].class_level;
    let hp_after = engine.party().members[trainee].hit_point_max;
    eprintln!(
        "[4] trained member {trainee}: levels {:?} -> {:?}, HP {hp_before} -> {hp_after}",
        level_before, level_after
    );
    assert_ne!(level_before, level_after, "the trainee leveled up");
    assert!(hp_after >= hp_before, "HP did not decrease");
    engine.tick(&[InputEvent::Char(b'e')]); // leave training
    to_world_menu(&mut engine, &[]);

    // --- Step 5: save → restore → assert state-hash equality ---
    let bytes1 = engine.save();
    let hash1 = Sha256::digest(&bytes1);
    let restored = Engine::restore(&bytes1, data).expect("restore succeeds");
    let bytes2 = restored.save();
    let hash2 = Sha256::digest(&bytes2);
    eprintln!("[5] save {} bytes, hash {:x}", bytes1.len(), hash1);
    assert_eq!(
        hash1, hash2,
        "save->restore->save state hash must be identical"
    );
    // The trained level survived the round trip.
    assert_eq!(
        restored.party().members[trainee].class_level,
        level_after,
        "the trained level survives save/restore"
    );
    eprintln!("[5] state-hash equality holds; trained level survives restore");
    eprintln!("== EXIT GATE PASSED ==");
}

// --- M4 combat #4: the watchable fight demo, now driven by the REAL melee AI ---

/// The watchable fight demo (local-tier, `GBX_DATA_DIR`-gated like the other
/// demos so it stays out of CI — it uses **no game data**, only synthetic
/// D10-clean stats; a "goblin" here is invented, not a decoded `MON2CHA` record).
///
/// A synthetic party-vs-goblins encounter placed on an open floor and run to
/// completion through the **real melee AI** ([`CombatWorld::run_combat`]):
/// `CalculateInitiative`, `FindNextCombatant` selection, and each combatant's
/// full `PlayerQuickFight` turn — the `field_15` mode-gate, the two behavior-guard
/// d7s, `find_target`'s random pick, and the `sub_35DB1` approach-and-attack — all
/// drawing from the one `EngineRng` stream (D9). Unlike the M4-combat-#2 version
/// (a placeholder "first living enemy" picker), this **is** a faithful run: the AI
/// picks and closes on its own targets, so the draw stream is the real one.
///
/// The transcript prints the placed battlefield, the survivors' HP after each
/// round, and the outcome. `ac`/`hit_bonus` use the original's raw encoding
/// (display AC = `0x3C - ac`; a hit needs `d20 + hit_bonus >= raw_ac`).
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture watch_a_fight`
#[test]
fn watch_a_fight() {
    use crate::combat::{
        place_combatants, CombatMap, CombatOutcome, CombatWorld, Fighter, GridPos, PlacementInput,
        Team, DEFAULT_NO_ACTION_LIMIT,
    };
    use crate::rng::EngineRng;

    if std::env::var_os("GBX_DATA_DIR").is_none() {
        eprintln!("SKIPPED: fight demo runs in the local tier (GBX_DATA_DIR) — watch_a_fight");
        return;
    }

    struct Stat {
        name: &'static str,
        team: Team,
        hp: i32,
        raw_ac: u8,
        hit_bonus: i32,
        movement: i32,
        dice: (u8, u8, u8), // (count, size, bonus)
    }
    let stat = |name, team, hp, raw_ac, hit_bonus, movement, dice| Stat {
        name,
        team,
        hp,
        raw_ac,
        hit_bonus,
        movement,
        dice,
    };
    let stats = [
        stat("Kethra", Team::Party, 26, 54, 45, 12, (1, 8, 2)), // longsword 1d8+2, disp AC 6
        stat("Dolan", Team::Party, 22, 52, 44, 12, (1, 10, 1)), // bastard sword 1d10+1
        stat("Sable", Team::Party, 18, 50, 43, 12, (1, 6, 3)),  // short sword 1d6+3
        stat("Snik", Team::Monster, 7, 48, 41, 9, (1, 6, 0)),   // spear 1d6, disp AC 12
        stat("Grub", Team::Monster, 7, 48, 41, 9, (1, 6, 0)),
        stat("Yark", Team::Monster, 7, 48, 41, 9, (1, 6, 0)),
        stat("Mool", Team::Monster, 7, 48, 41, 9, (1, 6, 0)),
        stat("Zeth", Team::Monster, 7, 48, 41, 9, (1, 6, 0)),
    ];
    let names: Vec<&str> = stats.iter().map(|s| s.name).collect();

    // Place both teams on open floor (party facing north, goblins one tile ahead —
    // `encounter_distance` must stay small enough that the iso diamond fits the
    // 50×25 field, §11; a larger value pushes a team off-map).
    let mut map = CombatMap::uniform(0x17);
    let placement_inputs: Vec<PlacementInput> = stats
        .iter()
        .map(|s| PlacementInput {
            team: s.team,
            size: 1,
            in_combat: true,
        })
        .collect();
    let placements = place_combatants(&mut map, &placement_inputs, 0, 1, GridPos::new(0, 0), None);

    let fighters: Vec<Fighter> = stats
        .iter()
        .enumerate()
        .map(|(i, s)| {
            Fighter::new_melee(
                i,
                s.team,
                s.team == Team::Monster, // NPC = monster (draws the morale d100)
                placements[i].pos,
                s.hp,
                s.raw_ac,
                s.hit_bonus,
                s.movement,
                s.dice,
                0, // delay — CalculateInitiative sets it each round
                1, // attack1_left — one swing/round
            )
        })
        .collect();
    let mut world = CombatWorld::new(map, fighters);

    let seed = 0x0C0F_FEE0u32;
    let mut rng = EngineRng::new(seed);

    eprintln!("== A FIGHT ==  (seed {seed:#010x}; synthetic, D10-clean; the REAL melee AI)");
    for (i, s) in stats.iter().enumerate() {
        let p = world.fighters[i].pos;
        eprintln!(
            "  {:<6} {:<7} AC {:>2}  HP {:>2}  @({:>2},{:>2})",
            if s.team == Team::Party {
                "party"
            } else {
                "goblin"
            },
            s.name,
            0x3C - s.raw_ac as i32,
            s.hp,
            p.x,
            p.y,
        );
    }

    let outcome = world.run_combat_observed(&mut rng, DEFAULT_NO_ACTION_LIMIT, |w, round| {
        let living: Vec<String> = w
            .fighters
            .iter()
            .filter(|f| f.in_combat)
            .map(|f| format!("{} {}hp", names[f.id], f.hp_current))
            .collect();
        eprintln!("── after round {} ──  {}", round + 1, living.join("  "));
    });

    eprintln!(
        "\n== OUTCOME: {} ==",
        match outcome {
            CombatOutcome::PartyWins => "the party stands",
            CombatOutcome::MonstersWin => "the goblins win",
            CombatOutcome::Stalemate => "stalemate (round cap reached)",
        }
    );
    for (i, s) in stats.iter().enumerate() {
        let f = &world.fighters[i];
        eprintln!(
            "  {:<7} {}",
            s.name,
            if f.in_combat {
                format!("HP {}/{}", f.hp_current, f.hp_max)
            } else {
                "DOWN".to_string()
            }
        );
    }

    // The only invariant: the real AI actually fought (someone took damage) — the
    // outcome itself is the seed's to determine, not the demo's to assert.
    assert!(
        world.fighters.iter().any(|f| f.hp_current < f.hp_max),
        "the melee AI closed and traded blows"
    );
}

// --- M4 combat #4: the watchable BATTLEFIELD demo, driven by the REAL AI ---

/// The watchable **battlefield** demo (local-tier, `GBX_DATA_DIR`-gated; uses no
/// game data — only synthetic D10-clean stats and a synthetic terrain grid).
///
/// This exercises the whole M4 combat stack end to end: the **combat map** with
/// per-tile passability, deterministic **placement** (`place_combatants`), the
/// **wall-respecting range** (`build_near_targets`/`get_target_range`), and the
/// full **melee AI** ([`CombatWorld::run_combat`]) — initiative, `FindNextCombatant`
/// selection, and each combatant's `PlayerQuickFight` turn (the `field_15`
/// mode-gate, the behavior-guard d7s, `find_target`, and the `sub_35DB1`
/// approach-and-attack). Unlike the M4-combat-#3 version (a placeholder picker +
/// draw-free greedy mover), this **is** faithful: the AI selects targets, closes
/// (its `CanMove`/`sub_3E748` steps respect the rock), and swings — all drawing
/// from the one `EngineRng` (D9). A text battlefield renders after each round so
/// the fight is *visible*.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture watch_a_battlefield`
#[test]
fn watch_a_battlefield() {
    use crate::combat::{
        place_combatants, CombatMap, CombatOutcome, CombatWorld, Fighter, GridPos, PlacementInput,
        Team, TilePassability, DEFAULT_NO_ACTION_LIMIT,
    };
    use crate::rng::EngineRng;

    if std::env::var_os("GBX_DATA_DIR").is_none() {
        eprintln!(
            "SKIPPED: battlefield demo runs in the local tier (GBX_DATA_DIR) — watch_a_battlefield"
        );
        return;
    }

    const FLOOR: u8 = 0x17; // passable floor (move_cost 1)
    const ROCK: u8 = 1; // move_cost 0xFF → wall

    // (glyph, team, hp, raw_ac, hit_bonus, movement, (count,size,bonus)); names are
    // in the legend below.
    struct Stat {
        glyph: char,
        team: Team,
        hp: i32,
        raw_ac: u8,
        hit_bonus: i32,
        movement: i32,
        dice: (u8, u8, u8),
    }
    let s = |glyph, team, hp, raw_ac, hit_bonus, movement, dice| Stat {
        glyph,
        team,
        hp,
        raw_ac,
        hit_bonus,
        movement,
        dice,
    };
    let stats = [
        s('K', Team::Party, 26, 54, 45, 12, (1, 8, 2)),
        s('D', Team::Party, 22, 52, 44, 9, (1, 10, 1)),
        s('S', Team::Party, 18, 50, 43, 12, (1, 6, 3)),
        s('a', Team::Monster, 7, 48, 41, 9, (1, 6, 0)),
        s('b', Team::Monster, 7, 48, 41, 9, (1, 6, 0)),
        s('c', Team::Monster, 7, 48, 41, 9, (1, 6, 0)),
        s('d', Team::Monster, 7, 48, 41, 9, (1, 6, 0)),
    ];
    let glyphs: Vec<char> = stats.iter().map(|s| s.glyph).collect();

    // A synthetic field with a rock outcrop behind the party (south) as scenery —
    // deliberately not between the lines (the AI's move is the DATA_2B8 offset
    // approach, faithful but not a full wall-router, so a barrier can stall it,
    // exactly as coab's can).
    let mut map = CombatMap::uniform(FLOOR);
    for x in 25..=27 {
        map.set_tile(GridPos::new(x, 16), ROCK);
    }
    let roster_inputs: Vec<PlacementInput> = stats
        .iter()
        .map(|s| PlacementInput {
            team: s.team,
            size: 1,
            in_combat: true,
        })
        .collect();
    let placements = place_combatants(&mut map, &roster_inputs, 0, 1, GridPos::new(0, 0), None);
    for p in &placements {
        assert!(p.placed, "everyone finds a cell on the open field");
    }

    let fighters: Vec<Fighter> = stats
        .iter()
        .enumerate()
        .map(|(i, st)| {
            Fighter::new_melee(
                i,
                st.team,
                st.team == Team::Monster,
                placements[i].pos,
                st.hp,
                st.raw_ac,
                st.hit_bonus,
                st.movement,
                st.dice,
                0,
                1,
            )
        })
        .collect();
    let mut world = CombatWorld::new(map, fighters);

    // The ASCII renderer: crop to the live combatants' bounding box (+margin) and
    // draw terrain + glyphs from the live world.
    fn render(world: &CombatWorld, glyphs: &[char]) {
        let live: Vec<(usize, GridPos)> = world
            .fighters
            .iter()
            .enumerate()
            .filter(|(_, f)| f.in_combat)
            .map(|(i, f)| (i, f.pos))
            .collect();
        if live.is_empty() {
            return;
        }
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (_, p) in &live {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        let (min_x, min_y) = ((min_x - 2).max(0), (min_y - 2).max(0));
        let (max_x, max_y) = ((max_x + 2).min(49), (max_y + 2).min(24));
        for y in min_y..=max_y {
            let mut row = String::from("      ");
            for x in min_x..=max_x {
                let here = GridPos::new(x, y);
                if let Some((i, _)) = live.iter().find(|(_, p)| *p == here) {
                    row.push(glyphs[*i]);
                } else {
                    row.push(match world.map.passability(here) {
                        TilePassability::Passable { .. } => '.',
                        TilePassability::Wall => '#',
                        TilePassability::Void => ' ',
                    });
                }
                row.push(' ');
            }
            eprintln!("{row}");
        }
    }

    let seed = 0x0C0F_FEE0u32;
    let mut rng = EngineRng::new(seed);

    eprintln!(
        "== A BATTLEFIELD ==  (seed {seed:#010x}; synthetic map + roster, D10-clean; REAL AI)"
    );
    eprintln!("Party: K Kethra  D Dolan  S Sable   Goblins: a Snik  b Grub  c Yark  d Mool");
    eprintln!("Legend: '.' floor  '#' rock (impassable)  ' ' off-field\n");
    eprintln!("Initial deployment (place_combatants, party facing north):");
    render(&world, &glyphs);

    let outcome = world.run_combat_observed(&mut rng, DEFAULT_NO_ACTION_LIMIT, |w, round| {
        eprintln!("\n── after round {} ──", round + 1);
        render(w, &glyphs);
    });

    eprintln!(
        "\n== OUTCOME: {} ==",
        match outcome {
            CombatOutcome::PartyWins => "the party stands",
            CombatOutcome::MonstersWin => "the goblins win",
            CombatOutcome::Stalemate => "stalemate (round cap reached)",
        }
    );
    assert!(
        world.fighters.iter().any(|f| f.hp_current < f.hp_max),
        "the melee AI closed and traded blows on the battlefield"
    );
}

// --- M4 combat #5: the PAYOFF — a fight assembled from REAL game data ---

/// The real-data fight (local-tier, `GBX_DATA_DIR`-gated): the model-unification
/// payoff. Unlike the two synthetic demos above, the monster team here is decoded
/// from **real game data** — the bundled `MON2CHA.DAX` Tilverton records, each a
/// full 0x1A6 `Player` record (`gbx_formats::monster` → [`LoadedMonster`]) — and the
/// battlefield terrain is derived from a **real area map**, `GEO2.DAX` block 1
/// (Tilverton City), whose wall topology stamps the combat map's obstacles. A small
/// synthetic D10-clean party is the other side. The whole fight then runs through
/// the **one unified tick engine** ([`crate::combat::CombatState`] via
/// [`CombatState::run_combat_observed`](crate::combat::CombatState::run_combat_observed),
/// a thin driver over `step()`) to a victor — proving the single model works on real
/// game data. This is the last piece before the ECL `COMBAT`-opcode encounter
/// trigger wires a *running script* into this same engine.
///
/// **Provisional terrain derivation (documented, not the faithful path):** the real
/// `SetupGroundTiles` paints a rotated iso combat *diamond* from the source area's
/// walls (`build_background_tiles_*`, `ovr011.cs:149`) — that derivation is deferred
/// with the encounter-trigger slice (see the §11 note in `combat.rs`). Here the GEO
/// block's fully-enclosed (4-walled) squares are stamped as rock obstacles onto an
/// otherwise-open field via [`CombatMap::from_ground`] (the derivation slice 3
/// built), and the deployment core is kept clear so the roster places. It is real
/// GEO data shaping a real fight, not the faithful diamond — flagged as such.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture watch_a_real_data_fight`
#[test]
fn watch_a_real_data_fight() {
    use crate::combat::{
        place_combatants, CombatMap, CombatOutcome, CombatState, Combatant, GridPos,
        PlacementInput, Team, TilePassability, DEFAULT_NO_ACTION_LIMIT, MAP_H, MAP_W,
    };
    use crate::monster::LoadedMonster;
    use crate::rng::EngineRng;
    use gbx_formats::geo::GeoBlock;
    use gbx_formats::monster::parse_cha_archive;

    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: real-data fight needs GBX_DATA_DIR — watch_a_real_data_fight");
        return;
    };
    let dir = std::path::Path::new(&dir);
    let data = load_dir(dir).expect("GBX_DATA_DIR must be readable");

    // --- REAL monster roster: decode MON2CHA.DAX (Tilverton, area 2) ---
    // A monster IS a 0x1A6 Player record (coab load_mob → new Player(data, 0)).
    let cha = data
        .raw_file("MON2CHA.DAX")
        .expect("GBX_DATA_DIR/MON2CHA.DAX must exist");
    let entries = parse_cha_archive(cha).expect("MON2CHA.DAX parses");
    let decoded: Vec<LoadedMonster> = entries
        .iter()
        .map(|e| LoadedMonster::from_record(&e.monster))
        .collect();
    // A themed Tilverton street ambush, by record index (0 ROYAL GUARD, 1 FIRE
    // KNIFE, 2 THIEF). Whatever the shipped records hold drives the fight; nothing
    // here is invented.
    let monster_picks: &[usize] = &[0, 1, 2];

    // --- REAL area terrain: GEO2.DAX block 1 (Tilverton City) ---
    let geo = GeoBlock::parse(&data.block("GEO2.DAX", 1).expect("GEO2.DAX block 1 loads"))
        .expect("GEO2 block 1 parses");
    const FLOOR: u8 = 0x17; // passable floor (move_cost 1)
    const ROCK: u8 = 1; // move_cost 0xFF → wall
    let mut ground = vec![FLOOR; (MAP_W * MAP_H) as usize];
    let mut rock_cells = 0usize;
    // Provisional overlay: a fully-enclosed (4-walled) GEO square → a rock obstacle
    // at combat cell (gx+17, gy+3), placing the 16×16 patch over the visible field.
    for gy in 0..16usize {
        for gx in 0..16usize {
            let s = geo.square(gx, gy);
            let walls = [s.wall_north, s.wall_east, s.wall_south, s.wall_west]
                .iter()
                .filter(|&&w| w != 0)
                .count();
            if walls == 4 {
                let (cx, cy) = (gx as i32 + 17, gy as i32 + 3);
                if (0..MAP_W).contains(&cx) && (0..MAP_H).contains(&cy) {
                    ground[(cy * MAP_W + cx) as usize] = ROCK;
                    rock_cells += 1;
                }
            }
        }
    }
    let mut map = CombatMap::from_ground(ground);
    // Keep the deployment diamond clear so the roster places (the faithful diamond
    // derivation is deferred — see this fn's doc comment).
    for y in 6..=16 {
        for x in 20..=30 {
            map.set_tile(GridPos::new(x, y), FLOOR);
        }
    }

    // --- The synthetic D10-clean party (the "small synthetic party" side) ---
    struct P {
        name: &'static str,
        hp: i32,
        raw_ac: u8,
        hit_bonus: i32,
        movement: i32,
        dice: (u8, u8, u8),
    }
    let party = [
        P {
            name: "Ravd",
            hp: 30,
            raw_ac: 54,
            hit_bonus: 45,
            movement: 12,
            dice: (1, 8, 3),
        },
        P {
            name: "Ilma",
            hp: 26,
            raw_ac: 52,
            hit_bonus: 44,
            movement: 12,
            dice: (1, 10, 2),
        },
        P {
            name: "Bex",
            hp: 22,
            raw_ac: 50,
            hit_bonus: 43,
            movement: 12,
            dice: (1, 6, 3),
        },
    ];

    // Build the roster in TeamList order: party (Team::Party) then monsters.
    let placement_inputs: Vec<PlacementInput> = party
        .iter()
        .map(|_| PlacementInput {
            team: Team::Party,
            size: 1,
            in_combat: true,
        })
        .chain(monster_picks.iter().map(|_| PlacementInput {
            team: Team::Monster,
            size: 1,
            in_combat: true,
        }))
        .collect();
    let placements = place_combatants(&mut map, &placement_inputs, 0, 1, GridPos::new(0, 0), None);
    assert!(
        placements.iter().all(|p| p.placed),
        "everyone finds a cell on the GEO-derived field"
    );

    // Party Combatants (synthetic stats).
    let mut fighters: Vec<Combatant> = party
        .iter()
        .enumerate()
        .map(|(i, p)| {
            Combatant::new_melee(
                i,
                Team::Party,
                false,
                placements[i].pos,
                p.hp,
                p.raw_ac,
                p.hit_bonus,
                p.movement,
                p.dice,
                0, // delay — CalculateInitiative sets it each round
                1, // one swing/round
            )
        })
        .collect();
    // Monster Combatants — every stat from the decoded record. `hit_bonus` is the
    // record's stored THAC0 (`@0x73`, already in the raw-AC offset space, so the
    // `d20 + hit_bonus >= raw_ac` compare is faithful; the DexReaction/strength
    // folding into hitBonus@0x199 is a BattleSetup concern, deferred). raw AC is
    // the on-disk `@0x19a` byte; damage is attack profile 1.
    for (k, &ri) in monster_picks.iter().enumerate() {
        let m = &decoded[ri];
        let a1 = m.attacks[0];
        let id = party.len() + k;
        fighters.push(Combatant::new_melee(
            id,
            Team::Monster,
            m.is_npc(),
            placements[id].pos,
            m.hit_point_max as i32,
            m.ac as u8,
            m.thac0 as i32,
            m.movement as i32,
            (a1.dice_count, a1.dice_size, a1.damage_bonus as u8),
            0,
            1,
        ));
    }
    let names: Vec<String> = party
        .iter()
        .map(|p| p.name.to_string())
        .chain(monster_picks.iter().map(|&ri| decoded[ri].name.clone()))
        .collect();

    let mut state = CombatState::new(map, fighters);

    let seed = 0x0C0F_FEE0u32;
    let mut rng = EngineRng::new(seed);

    eprintln!("== A REAL-DATA FIGHT ==  (seed {seed:#010x}; the ONE unified tick engine)");
    eprintln!(
        "Monsters: decoded from MON2CHA.DAX (real 0x1A6 records)   Terrain: GEO2.DAX block 1 \
         ({rock_cells} rock cells derived)"
    );
    eprintln!("Party is synthetic, D10-clean.\n");
    for (i, f) in state.fighters.iter().enumerate() {
        eprintln!(
            "  {:<6} {:<12} AC {:>2}  HP {:>3}  hit+{:<2} {}d{}+{}  @({:>2},{:>2})",
            if f.team == Team::Party {
                "party"
            } else {
                "mob"
            },
            names[i],
            0x3C - f.ac as i32,
            f.hp_max,
            f.hit_bonus,
            f.dice_count,
            f.dice_size,
            f.damage_bonus,
            f.pos.x,
            f.pos.y,
        );
    }

    // Render the live battlefield (crop to the combatants' bounding box + margin).
    fn render(state: &CombatState) {
        let glyph = |i: usize, team: Team| -> char {
            let base = if team == Team::Party { b'A' } else { b'a' };
            (base + (i as u8 % 26)) as char
        };
        let live: Vec<(usize, GridPos, Team)> = state
            .fighters
            .iter()
            .enumerate()
            .filter(|(_, f)| f.in_combat)
            .map(|(i, f)| (i, f.pos, f.team))
            .collect();
        if live.is_empty() {
            return;
        }
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (_, p, _) in &live {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        let (min_x, min_y) = ((min_x - 2).max(0), (min_y - 2).max(0));
        let (max_x, max_y) = ((max_x + 2).min(MAP_W - 1), (max_y + 2).min(MAP_H - 1));
        for y in min_y..=max_y {
            let mut row = String::from("      ");
            for x in min_x..=max_x {
                let here = GridPos::new(x, y);
                if let Some((i, _, team)) = live.iter().find(|(_, p, _)| *p == here) {
                    row.push(glyph(*i, *team));
                } else {
                    row.push(match state.map.passability(here) {
                        TilePassability::Passable { .. } => '.',
                        TilePassability::Wall => '#',
                        TilePassability::Void => ' ',
                    });
                }
                row.push(' ');
            }
            eprintln!("{row}");
        }
    }

    eprintln!("\nInitial deployment (A.. party, a.. monsters; '#' GEO-derived rock):");
    render(&state);

    let outcome = state.run_combat_observed(&mut rng, DEFAULT_NO_ACTION_LIMIT, |s, round| {
        let living: Vec<String> = s
            .fighters
            .iter()
            .filter(|f| f.in_combat)
            .map(|f| format!("{} {}hp", names[f.id], f.hp_current))
            .collect();
        eprintln!("── after round {} ──  {}", round + 1, living.join("  "));
    });

    eprintln!(
        "\n== OUTCOME: {} ==",
        match outcome {
            CombatOutcome::PartyWins => "the party stands",
            CombatOutcome::MonstersWin => "the Tilverton mob wins",
            CombatOutcome::Stalemate => "stalemate (round cap reached)",
        }
    );
    for (i, f) in state.fighters.iter().enumerate() {
        eprintln!(
            "  {:<12} {}",
            names[i],
            if f.in_combat {
                format!("HP {}/{}", f.hp_current, f.hp_max)
            } else {
                "DOWN".to_string()
            }
        );
    }

    // The real monster records genuinely drove a fight through the one engine — the
    // outcome is the seed's to determine (like the synthetic demos), the assertion
    // is only that the unified engine closed and traded blows on real data.
    assert!(
        state.fighters.iter().any(|f| f.hp_current < f.hp_max),
        "the unified engine fought on real MON2CHA data"
    );
}

/// M6 slice 3's eyeball pass (local-only, `GBX_DATA_DIR`): build a
/// [`CombatScene`](crate::combat::scene::CombatScene) over the **real** art —
/// boot's COMSPR icons, `DUNGCOM`+`RANDCOM` ground tiles, `CHEAD`/`CBODY`
/// party icons and a `CPIC` monster — and dump the rendered screens as `.ppm`
/// **outside the repo** (D10: no real art ever lands in-tree, so this is a
/// demo, not a golden — the goldens run on synthetic fixtures).
///
/// Also dumps the six `RANDCOM` tiles side by side, which is how doc §6 item
/// 3 ("atlas slot 0x25 is unnamed in coab — identify from pixels") gets
/// answered.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture watch_a_real_art_combat_scene`
#[test]
fn watch_a_real_art_combat_scene() {
    use crate::combat::scene::{
        layout, render, CombatScene, EntrySnapshot, FocusCursor, PanelSummary, PresentedCombatant,
    };
    use crate::combat::{CombatMap, GridPos, HealthStatus, Team};
    use crate::combat_art::{self, IconPose};
    use crate::party::IconInfo;
    use gbx_formats::combat_art::CELL_PX;

    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!(
            "SKIPPED: the scene demo runs in the local tier (GBX_DATA_DIR) — \
             watch_a_real_art_combat_scene"
        );
        return;
    };
    let data = load_dir(std::path::Path::new(&dir)).expect("GBX_DATA_DIR must be readable");
    let assets = boot(&data).expect("boot must succeed against real CotAB data");

    let out_dir = std::env::var_os("RESTRIKE_SCENE_DEMO_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    // --- the art: boot's COMSPR slots, then the combat-entry loads ---------
    let mut icons = assets.combat_icons.clone();
    let colours: [u8; 6] = [0x91, 0xA2, 0xB3, 0xC4, 0xE6, 0xF7];
    for (slot, (head, weapon)) in [(0u8, 0u8), (3, 5), (7, 12)].into_iter().enumerate() {
        let info = IconInfo {
            head_icon: head,
            weapon_icon: weapon,
            icon_id: slot as u8,
            icon_size: 1,
            colours,
        };
        icons.set(
            slot,
            combat_art::load_party_icon(&data, &info, true).expect("party icon"),
        );
    }
    // A monster type in slot 8 — CPIC2 block 0 is Tilverton's first picture.
    icons.set(
        8,
        combat_art::load_monster_icon(&data, 2, 0).expect("monster icon"),
    );
    let tiles = combat_art::load_ground_tiles(&data, true).expect("dungeon ground tiles");

    // --- the RANDCOM strip (doc §6 item 3) --------------------------------
    let mut strip = Framebuffer::new();
    for (i, slot) in (0x22..0x28usize).enumerate() {
        let tile = tiles.tile(slot).expect("RANDCOM slot");
        for y in 0..CELL_PX {
            for x in 0..CELL_PX {
                let v = tile[y * CELL_PX + x];
                // 2× nearest-neighbour, laid out left to right with a gap.
                for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    strip.set_pixel(8 + i * 52 + x * 2 + dx, 40 + y * 2 + dy, v);
                }
            }
        }
    }
    draw_string(
        &mut strip,
        &assets.font,
        "RANDCOM 22 23 24 25 26 27",
        2,
        1,
        0,
        10,
    );
    let path = out_dir.join("restrike-scene-randcom-strip.ppm");
    write_ppm(&strip, &path);
    eprintln!("RANDCOM tiles 0x22..0x27 -> {}", path.display());
    // Doc §6 item 3: slot 0x25 (background tile 0x1D) is unnamed in coab.
    // These six lines are the identification — slot 0x16 (the DUNGCOM floor
    // ground tile 0x17 maps to) is printed alongside as the comparison.
    for slot in [0x16usize, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27] {
        let tile = tiles.tile(slot).expect("atlas slot");
        let seen: std::collections::BTreeSet<u8> = tile.iter().copied().collect();
        eprintln!(
            "  atlas slot 0x{slot:02X}: palette codes {seen:?}, {} non-zero pixels",
            tile.iter().filter(|&&p| p != 0).count()
        );
    }

    // --- a small room of real dungeon tiles --------------------------------
    // Ground value 0x17 is a cost-1 floor; 1 is a wall. Both come off the
    // real `BackGroundTiles` table, so the picture is real dungeon art laid
    // out by hand (faithful floor *generation* is slice 6).
    let mut map = CombatMap::uniform(0x17);
    for x in 18..30 {
        map.set_tile(GridPos::new(x, 9), 1);
        map.set_tile(GridPos::new(x, 17), 1);
    }
    for y in 9..18 {
        map.set_tile(GridPos::new(18, y), 1);
        map.set_tile(GridPos::new(29, y), 1);
    }
    map.set_tile(GridPos::new(22, 12), 0x1A); // a table (RANDCOM 0x22)
    map.set_tile(GridPos::new(23, 12), 0x1B); // a chair (RANDCOM 0x23)

    let combatant =
        |id: usize, team: Team, x: i32, y: i32, slot: usize, dir: u8| PresentedCombatant {
            id,
            name: ["KETHRA", "DOLAN", "SABLE", "BRIGAND"][id].to_string(),
            team,
            non_team_member: false,
            icon_slot: slot,
            size: 1,
            pos: GridPos::new(x, y),
            direction: dir,
            pose: IconPose::Normal,
            hp_current: if id == 0 { 17 } else { 22 },
            hp_max: 22,
            ac: 0x36,
            health_status: HealthStatus::Okey,
            in_combat: true,
        };
    let snapshot = EntrySnapshot {
        roster: vec![
            combatant(0, Team::Party, 21, 13, 0, 2),
            combatant(1, Team::Party, 21, 14, 1, 2),
            combatant(2, Team::Party, 20, 12, 2, 6),
            combatant(3, Team::Monster, 25, 13, 8, 6),
        ],
        map,
        camera_top_left: GridPos::new(19, 10),
    };
    let mut scene = CombatScene::new(snapshot, crate::combat::scene::SceneArt::new(tiles, icons));

    let mut fb = Framebuffer::new();
    scene
        .render(&mut fb, &assets.symbol_sets)
        .expect("the real-art scene must render");
    scene.clear_text_surfaces(&mut fb);
    scene.draw_panel(
        &mut fb,
        &assets.font,
        &PanelSummary {
            name: "KETHRA".to_string(),
            team: Team::Party,
            in_combat: true,
            hp_current: 17,
            hp_max: 22,
            ac: 0x36,
            health_status: HealthStatus::Okey,
            readied_weapon: Some("Long Sword".to_string()),
            held: false,
        },
    );
    let path = out_dir.join("restrike-scene-real-art.ppm");
    write_ppm(&fb, &path);
    eprintln!("combat scene (real art) -> {}", path.display());

    // The same screen with the focus box on the brigand — the Aim view.
    scene.set_focus(Some(FocusCursor {
        pos: GridPos::new(25, 13),
        size: 1,
    }));
    let mut aim = Framebuffer::new();
    scene
        .render(&mut aim, &assets.symbol_sets)
        .expect("aim view");
    render::clear_status_line(&mut aim);
    draw_string(
        &mut aim,
        &assets.font,
        "Range = 4",
        layout::STATUS_ROW,
        0,
        0,
        10,
    );
    let path = out_dir.join("restrike-scene-real-art-aim.ppm");
    write_ppm(&aim, &path);
    eprintln!("combat scene, aim cursor (real art) -> {}", path.display());
}

/// M6 slice 4's eyeball pass (local-only, `GBX_DATA_DIR`): play a real fight
/// through the [`CombatScene`](crate::combat::scene::CombatScene) timeline and
/// dump the frames as a numbered `.ppm` sequence **outside the repo** (D10) —
/// the slice-3 still picture, now moving.
///
/// Every frame is one tick of `scene.tick(1)`, so the sequence IS the schedule:
/// six frames of an attack pose, twenty-four of a message, nine one-frame
/// death-flash alternations. Play it back at 60 fps and it is the original's
/// pacing.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture watch_a_real_art_combat_reel`
/// then e.g. `ffmpeg -framerate 60 -i /tmp/restrike-reel-%04d.ppm reel.mp4`.
#[test]
fn watch_a_real_art_combat_reel() {
    use crate::combat::scene::{CombatScene, CombatantIdentity, EntrySnapshot, SceneArt};
    use crate::combat::{
        ActionEvent, ActionSink, CombatMap, CombatState, CombatStep, Combatant, GridPos, Team,
    };
    use crate::combat_art;
    use crate::party::IconInfo;
    use crate::rng::EngineRng;
    use std::cell::RefCell;
    use std::rc::Rc;

    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!(
            "SKIPPED: the reel demo runs in the local tier (GBX_DATA_DIR) — \
             watch_a_real_art_combat_reel"
        );
        return;
    };
    let data = load_dir(std::path::Path::new(&dir)).expect("GBX_DATA_DIR must be readable");
    let assets = boot(&data).expect("boot must succeed against real CotAB data");
    let out_dir = std::env::var_os("RESTRIKE_SCENE_DEMO_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    // --- the art (as slice 3's still does) --------------------------------
    let mut icons = assets.combat_icons.clone();
    let colours: [u8; 6] = [0x91, 0xA2, 0xB3, 0xC4, 0xE6, 0xF7];
    for (slot, (head, weapon)) in [(0u8, 0u8), (3, 5), (7, 12)].into_iter().enumerate() {
        let info = IconInfo {
            head_icon: head,
            weapon_icon: weapon,
            icon_id: slot as u8,
            icon_size: 1,
            colours,
        };
        icons.set(
            slot,
            combat_art::load_party_icon(&data, &info, true).expect("party icon"),
        );
    }
    icons.set(
        8,
        combat_art::load_monster_icon(&data, 2, 0).expect("monster icon"),
    );
    let tiles = combat_art::load_ground_tiles(&data, true).expect("dungeon ground tiles");

    // --- a small walled room, and a fight inside it ------------------------
    let mut map = CombatMap::uniform(0x17);
    for x in 17..31 {
        map.set_tile(GridPos::new(x, 8), 1);
        map.set_tile(GridPos::new(x, 17), 1);
    }
    for y in 8..18 {
        map.set_tile(GridPos::new(17, y), 1);
        map.set_tile(GridPos::new(30, y), 1);
    }
    let names = ["KETHRA", "DOLAN", "SABLE", "BRIGAND", "CUTPURSE"];
    let mut fighters: Vec<Combatant> = Vec::new();
    for (i, (team, x, y, hp, ac, hit, dice)) in [
        (Team::Party, 19, 11, 18, 5, 16, (1, 8, 1)),
        (Team::Party, 19, 12, 22, 4, 15, (2, 4, 0)),
        (Team::Party, 19, 13, 14, 6, 17, (1, 6, 0)),
        (Team::Monster, 27, 12, 11, 7, 18, (1, 6, 0)),
        (Team::Monster, 27, 13, 9, 7, 18, (1, 4, 1)),
    ]
    .into_iter()
    .enumerate()
    {
        fighters.push(Combatant::new_melee(
            i,
            team,
            team == Team::Monster,
            GridPos::new(x, y),
            hp,
            ac,
            hit,
            10,
            dice,
            3,
            1,
        ));
    }
    // `CombatState::new` already selects the real melee AI driver.
    let mut state = CombatState::new(map, fighters);

    #[derive(Clone, Default)]
    struct Batch(Rc<RefCell<Vec<ActionEvent>>>);
    struct BatchSink(Rc<RefCell<Vec<ActionEvent>>>);
    impl ActionSink for BatchSink {
        fn on_action(&mut self, event: ActionEvent) {
            self.0.borrow_mut().push(event);
        }
    }
    let batch = Batch::default();
    state.attach_action_sink(Box::new(BatchSink(Rc::clone(&batch.0))));

    let mut rng = EngineRng::new(0x0C0F_FEE0);
    assert_ne!(state.step(&mut rng), CombatStep::Ended);
    let identities: Vec<CombatantIdentity> = (0..state.roster().len())
        .map(|i| CombatantIdentity::new(names[i], if i < 3 { i } else { 8 }))
        .collect();
    let mut scene = CombatScene::new(
        EntrySnapshot::from_state(&state, &identities),
        SceneArt::new(tiles, icons),
    );
    scene.set_weapon_names(
        [(1u8, "Long Sword".to_string()), (7, "Club".to_string())]
            .into_iter()
            .collect(),
    );
    scene.refresh_panels(&state);
    scene.reconcile(&state).expect("the entry snapshot matches");

    // --- play it, one .ppm per tick ---------------------------------------
    const MAX_FRAMES: usize = 900; // 15 seconds at 60 Hz — a few turns
    let mut frames = 0usize;
    let mut sounds: Vec<u8> = Vec::new();
    'reel: loop {
        let events = std::mem::take(&mut *batch.0.borrow_mut());
        scene.begin_step(&events);
        while scene.is_playing() {
            for cue in scene.tick(1) {
                sounds.push(cue.0);
            }
            let mut fb = Framebuffer::new();
            scene
                .render_frame(&mut fb, &assets.symbol_sets, &assets.font)
                .expect("the reel must render");
            write_ppm(&fb, &out_dir.join(format!("restrike-reel-{frames:04}.ppm")));
            frames += 1;
            if frames >= MAX_FRAMES {
                break 'reel;
            }
        }
        scene.reconcile(&state).expect("the board reconciles");
        scene.refresh_panels(&state);
        if state.step(&mut rng) == CombatStep::Ended {
            break;
        }
    }

    eprintln!(
        "combat reel: {frames} frames -> {}/restrike-reel-*.ppm ({} sound cues)",
        out_dir.display(),
        sounds.len()
    );
    assert!(frames > 60, "the reel played at least a second of beats");
}

use crate::input::{ExtKey, InputEvent};

/// ★ **A scripted player** (doc §9.6's manual-fight demo).
///
/// It reads only what a player sees — whose turn it is, which menu is open,
/// where the pieces are — and presses one key per tick: walk at the nearest
/// foe, and walk *into* it to swing (`sub_33F03`).
pub(crate) fn scripted_player_key(e: &crate::engine::Engine) -> Option<InputEvent> {
    use crate::combat::scene::MenuStage;
    let host = e.shell().combat_host()?;
    // A player presses keys at the menu, not through an animation — and the
    // menus deliberately read nothing while one is playing, so an early press
    // would just queue up and arrive two-at-a-time.
    if host.scene().is_none_or(|s| s.is_playing()) {
        return None;
    }
    let ui = host.manual()?;
    let state = host.state();
    let actor = ui.actor();
    let me = &state.roster()[actor];
    let Some(foe) = state
        .roster()
        .iter()
        .filter(|c| c.in_combat && c.team != me.team)
        .min_by_key(|c| (c.pos.x - me.pos.x).abs().max((c.pos.y - me.pos.y).abs()))
    else {
        // The last foe fell on this very turn: spend what is left of it and
        // let `BattleRoundChecks` ask its question.
        return Some(match ui.stage() {
            MenuStage::Moving => InputEvent::Enter,
            MenuStage::Done => InputEvent::Char(b'Q'),
            _ => InputEvent::Char(b'D'),
        });
    };
    let bearing = crate::combat::target_direction(me.pos, foe.pos);

    // The first direction that gets somewhere: into the foe if it is next
    // door (that is the swing), else onto passable, unoccupied ground, trying
    // the bearing first and fanning out — the same idea as the AI's own
    // `CanMove` retry, done by hand.
    // The first direction that gets somewhere, asked of the core itself
    // (§9.3's own forks): into the foe if it is next door — that is the swing —
    // else a step the moves left can pay for. The bearing first, then fanning
    // out, which is the same idea as the AI's `CanMove` retry, done by hand.
    let mut step = None;
    for offset in [0i32, 1, -1, 2, -2, 3, -3, 4] {
        let dir = ((bearing as i32 + offset).rem_euclid(8)) as u8;
        match state.move_step_preview(actor, dir) {
            crate::combat::StepPreview::Attack { target } => {
                // Only a weapon that can swing in melee may walk into someone;
                // an archer aims instead (`sub_33F03`'s own gate).
                if target == foe.id && state.weapon_can_swing_in_melee(actor) {
                    step = Some(dir);
                    break;
                }
            }
            crate::combat::StepPreview::Step => {
                step = Some(dir);
                break;
            }
            crate::combat::StepPreview::Blocked | crate::combat::StepPreview::OffMap => {}
        }
    }

    // An archer (or anyone whose foe is out of walking reach this turn) shoots
    // from the Aim menu instead.
    //
    // The candidate comes out of the **aim list** rather than the roster: the
    // list is what `Next` walks (`copy_sorted_players`, wall-respecting), so a
    // foe that is commit-legal but not in it could never be cycled onto.
    let shootable = (!state.weapon_can_swing_in_melee(actor) || step.is_none())
        .then(|| {
            state
                .aim_list(actor)
                .into_iter()
                .find(|&id| state.roster()[id].team != me.team && state.can_commit_aim(actor, id))
        })
        .flatten();
    if let Some(target) = shootable {
        return Some(InputEvent::Char(match ui.stage() {
            MenuStage::Main => b'A',
            MenuStage::Aim if ui.aim_target() == Some(target) => b'T',
            MenuStage::Aim => b'N',
            MenuStage::Moving => return Some(InputEvent::Enter),
            MenuStage::Done => b'Q',
            _ => b'E',
        }));
    }

    let can_step = me.move_left > 1 && step.is_some();
    Some(match (ui.stage(), can_step) {
        // A keypad key opens the movement loop *with* its step
        // (`ovr009.cs:239-252`); a plain letter there is a menu word.
        (MenuStage::Main, true) | (MenuStage::Moving, true) => {
            InputEvent::Ext(keypad_for(step.expect("checked"))?)
        }
        // Nowhere to go: leave the loop, then spend the turn.
        (MenuStage::Moving, false) => InputEvent::Enter,
        (MenuStage::Main, false) => InputEvent::Char(b'D'),
        (MenuStage::Done, _) => InputEvent::Char(b'Q'),
        _ => InputEvent::Char(b'E'),
    })
}

/// The keypad key that walks in `dir` — the inverse of `keypad_ctrl_codes`
/// composed with the movement table (§1.7).
pub(crate) fn keypad_for(dir: u8) -> Option<crate::input::ExtKey> {
    use ExtKey::*;
    Some(match dir {
        0 => Kp8,
        1 => Kp9,
        2 => Kp6,
        3 => Kp3,
        4 => Kp2,
        5 => Kp1,
        6 => Kp4,
        7 => Kp7,
        _ => return None,
    })
}

/// ★ **M6b's done-condition, headlessly ticked** (`docs/design/combat-visualizer.md`
/// §4 M6b / §8): boot the real bundled save, **walk to the Tilverton bar**, and
/// watch the brawl happen on screen — entry beats, QuickFight, the VM resuming
/// afterwards — dumping `.ppm` frames outside the repo (D10) for the eyeball
/// pass.
///
/// **The route is the game's own.** GOG's bundled slot-A save spawns the real
/// party (MATHEW/MARK/TRAVIS/LEDERA/SHARA/PHILIPPE) at Tilverton City `(7,13)`
/// facing north. Three steps north and one west lands on `(6,10)` — the square
/// whose step script runs `LOAD MONSTER`×10 then `COMBAT`, i.e. the bar. Nothing
/// is staged: the walk loop's own `StepFlow` fires `ECL2.DAX` block 1's vector
/// 0, and the script does the rest.
///
/// What the frames show, in order: the exploration view, "A battle begins..."
/// on the prompt line, the combat screen with the real party icons and the ten
/// patrons on the projected bar floor, several mid-fight moments, and the
/// exploration view restored.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine --release \
///   -- --nocapture --ignored m6b_boot_to_the_bar_brawl`
///
/// (`--ignored` because it plays a whole fight at the faithful tick rate; it is
/// a demo for a human, not a CI gate — the CI half is
/// `shell_combat_tests`.)
#[test]
#[ignore]
fn m6b_boot_to_the_bar_brawl() {
    let Some(run) = BarBrawl::open("m6b-bar-brawl", false) else {
        return;
    };
    let outcome = run.play();
    assert!(
        outcome.saw_announce,
        "BattleSetup's \"A battle begins...\" beat played before the fight"
    );
    assert!(outcome.rounds > 0, "the brawl ran rounds");
    assert!(
        outcome.combat_line.contains("party wins"),
        "the real party beat the patrons: {}",
        outcome.combat_line
    );
    assert!(
        outcome.resumed_to_the_walk_loop,
        "the VM resumed and the shell went back to the walk loop"
    );
    assert!(!outcome.party_killed, "nobody died, so no game-over signal");
    // ★ roll-credits slice 3: the fight PAID. Naturally-earned experience,
    // from real monster records, through the real award path.
    assert!(
        outcome.exp_each > 0,
        "the won fight awarded experience: {}",
        outcome.exp_each
    );
    assert!(
        outcome.party_exp_gained > 0,
        "and it reached the party's records"
    );
}

/// ★ **The ECL DAMAGE opcode's own death screen** (roll-credits slice 3's
/// acceptance): the wipe flow's *other* variant, side by side with combat's.
///
/// The two are genuinely different presentations — different words, a wider
/// box starting one row higher, a hard three-second hold the player cannot
/// skip, and a colour-15 prompt instead of the combat screen's colour-13 one
/// — so this dumps both and asserts the pacing difference the ticks make
/// visible.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   --release the_two_death_screens -- --ignored --nocapture`
#[test]
#[ignore = "local-only demo (writes frames); run explicitly"]
fn the_two_death_screens() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (the_two_death_screens)");
        return;
    };
    let data = load_dir(std::path::Path::new(&dir)).expect("GBX_DATA_DIR must be readable");
    let out_dir = std::env::temp_dir();

    let mut prompt_ticks = Vec::new();
    for (label, cause) in [
        ("combat", crate::shell::WipeCause::Combat),
        ("ecl-damage", crate::shell::WipeCause::EclDamage),
    ] {
        let mut engine = crate::engine::Engine::new(data.clone(), 1)
            .expect("Engine::new must boot against real CotAB data");
        engine.state.wipe_cause = cause;
        engine.state.party_killed = true;
        // Long enough for the message to finish printing and, for the ECL
        // variant, for its `SysDelay(3000)` to run out.
        let mut prompt_tick = None;
        for tick in 0..600usize {
            engine.tick(&[]);
            if prompt_tick.is_none() && frame_has_prompt(&engine) {
                prompt_tick = Some(tick);
            }
        }
        let path = out_dir.join(format!("restrike-death-screen-{label}.ppm"));
        write_ppm(engine.framebuffer_for_demo(), &path);
        eprintln!(
            "      {label}: prompt at tick {prompt_tick:?} -> {}",
            path.display()
        );
        assert!(
            matches!(engine.shell(), crate::shell::Shell::GameOver(_)),
            "{label}: the wipe flow opened"
        );
        prompt_ticks.push(prompt_tick.expect("the prompt came up"));
    }
    // ★ `SysDelay(3000)` (`ovr003:2C82`): the ECL variant holds its screen for
    // three seconds — 180 ticks at 60 Hz — before it will even draw the
    // prompt, let alone take a key. The combat screen has no such beat.
    assert!(
        prompt_ticks[1] >= prompt_ticks[0] + 120,
        "the ECL variant must hold its screen far longer: {prompt_ticks:?}"
    );
}

/// Whether the prompt row carries any lit pixel — the cheapest "the prompt is
/// up" probe that does not re-implement the font.
fn frame_has_prompt(engine: &crate::engine::Engine) -> bool {
    let fb = engine.framebuffer_for_demo();
    (0..8).any(|dy| (0..320).any(|x| fb.get_pixel(x, 0x18 * 8 + dy) != 0))
}

/// ★ The other half of M6b's done-condition: **a party wipe shows its full
/// ending before GameOver** (§8.2's MUST).
///
/// The same walk to the same bar, with the party's hit points poked to 1 first
/// (a demo-only poke — the records are otherwise untouched) so the ten patrons
/// win. The frames show the fight ending, the final beats, and the restored
/// exploration screen — and only then does the shell unwind to `GameOver`.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine --release \
///   -- --nocapture --ignored m6b_a_party_wipe_shows_its_ending_before_game_over`
#[test]
#[ignore]
fn m6b_a_party_wipe_shows_its_ending_before_game_over() {
    let Some(run) = BarBrawl::open("m6b-bar-wipe", true) else {
        return;
    };
    let outcome = run.play();
    assert!(
        outcome.combat_line.contains("party wiped"),
        "the doomed party lost: {}",
        outcome.combat_line
    );
    assert!(outcome.party_killed, "a wipe raises the game-over signal");
    let known = outcome
        .outcome_known_tick
        .expect("the outcome became known during the fight");
    let flagged = outcome.flag_tick.expect("party_killed went up");
    let over = outcome
        .game_over_tick
        .expect("the shell unwound to GameOver");
    eprintln!(
        "  ordering: outcome known @{known}, party_killed @{flagged}, GameOver @{over} \
         ({} ticks of ending on screen)",
        flagged - known
    );
    assert!(
        flagged > known,
        "party_killed must wait for the ending to finish playing"
    );
    assert!(over > flagged, "and GameOver arrives a tick later still");
    assert!(
        flagged - known > 10,
        "the ending was on screen for real time, not one tick"
    );
}

/// ★ **M6c's done-condition** (`docs/design/combat-visualizer.md` §4 M6c): the
/// same walk to the same Tilverton bar, and then the brawl **played by hand** —
/// every party turn opens the real combat menu, a scripted player walks its
/// combatant at the nearest patron and swings by walking into it, and the fight
/// is won from the menus.
///
/// What the frames show: the combat screen with `Move View Aim Use Cast Turn
/// Quick Done` on the prompt row, the movement loop's "Move/Attack, Move Left =
/// N", mid-fight moments, and the exploration view restored afterwards.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine --release \
///   -- --nocapture --ignored m6c_a_hand_played_bar_brawl`
#[test]
#[ignore]
fn m6c_a_hand_played_bar_brawl() {
    let Some(mut run) = BarBrawl::open("m6c-manual-brawl", false) else {
        return;
    };
    run.manual = true;
    // Every party member fights by hand, whatever the save's own auto-fight
    // byte says — that is the fight this demo exists to play.
    for m in &mut run.engine.party.members {
        m.status.quick_fight = 0;
    }
    let outcome = run.play();
    assert!(outcome.rounds > 0, "the brawl ran rounds");
    assert!(
        outcome.combat_line.contains("party wins"),
        "the hand-played party beat the patrons: {}",
        outcome.combat_line
    );
    assert!(
        outcome.resumed_to_the_walk_loop,
        "the VM resumed and the shell went back to the walk loop"
    );
}

/// The shared boot-and-walk rig for the M6b/M6c bar demos.
struct BarBrawl {
    engine: crate::engine::Engine,
    out_dir: std::path::PathBuf,
    label: &'static str,
    /// ★ M6c: who fights. `false` presses `Quick` at the first menu (the
    /// QuickFight brawl M6b watches); `true` plays every party turn from the
    /// menus with [`scripted_player_key`].
    manual: bool,
}

#[derive(Default)]
struct BrawlOutcome {
    saw_announce: bool,
    combat_line: String,
    rounds: usize,
    resumed_to_the_walk_loop: bool,
    party_killed: bool,
    outcome_known_tick: Option<usize>,
    flag_tick: Option<usize>,
    game_over_tick: Option<usize>,
    /// ★ roll-credits slice 3: `gbl.exp_to_add` and the total the roster
    /// actually gained — the award, observed rather than asserted from the
    /// formula.
    exp_each: i32,
    party_exp_gained: i32,
}

/// Which way `from` must face to step onto the adjacent cell `to`.
fn step_facing(from: (u8, u8), to: (u8, u8)) -> crate::movement::Facing {
    use crate::movement::Facing;
    match (to.0 as i32 - from.0 as i32, to.1 as i32 - from.1 as i32) {
        (0, -1) => Facing::North,
        (1, 0) => Facing::East,
        (0, 1) => Facing::South,
        (-1, 0) => Facing::West,
        d => panic!("{from:?} -> {to:?} is not one step ({d:?})"),
    }
}

/// A shortest route over the area's **open** edges (`wall_door_flags` +
/// `DoorState::Open`) — the same passability the walk loop applies, minus the
/// locked-door menu, which a demo has no business opening.
fn plan_route(
    geo: &gbx_formats::geo::GeoBlock,
    from: (u8, u8),
    to: (u8, u8),
) -> Option<Vec<(u8, u8)>> {
    use crate::movement::{wall_door_flags, DoorState, Facing};
    use std::collections::{HashMap, VecDeque};
    let mut came: HashMap<(u8, u8), (u8, u8)> = HashMap::new();
    let mut queue = VecDeque::from([from]);
    came.insert(from, from);
    while let Some(cell) = queue.pop_front() {
        if cell == to {
            let mut path = vec![to];
            let mut cur = to;
            while cur != from {
                cur = came[&cur];
                path.push(cur);
            }
            path.reverse();
            path.remove(0); // the cell we are standing on
            return Some(path);
        }
        for facing in [Facing::North, Facing::East, Facing::South, Facing::West] {
            let square = geo.square(cell.0 as usize, cell.1 as usize);
            if DoorState::from_flag(wall_door_flags(square, facing)) != DoorState::Open {
                continue;
            }
            let (dx, dy) = facing.delta();
            let (nx, ny) = (cell.0 as i32 + dx, cell.1 as i32 + dy);
            if !(0..16).contains(&nx) || !(0..16).contains(&ny) {
                continue;
            }
            let next = (nx as u8, ny as u8);
            if came.contains_key(&next) {
                continue;
            }
            came.insert(next, cell);
            queue.push_back(next);
        }
    }
    None
}

impl BarBrawl {
    /// The bar: the Tilverton City square whose step script loads ten BAR
    /// PATRONs and calls `COMBAT`. Found by walking every square of `GEO2.DAX`
    /// block 1 against the real `ECL2.DAX` scripts.
    const BAR: (u8, u8) = (6, 10);

    fn open(label: &'static str, doom_the_party: bool) -> Option<Self> {
        use crate::input::{ExtKey, InputEvent};
        let root = match std::env::var_os("GBX_DATA_DIR") {
            Some(r) => r,
            None => {
                eprintln!("SKIPPED: the M6b demo needs GBX_DATA_DIR ({label})");
                return None;
            }
        };
        let root = std::path::Path::new(&root);
        let data = load_dir(root).expect("GBX_DATA_DIR must be readable");
        let saves = load_dir(&root.join("SAVE")).expect("GBX_DATA_DIR/SAVE must be readable");
        let master = saves
            .raw_file("SAVGAMA.DAT")
            .expect("the bundled SAVGAMA.DAT must exist");
        let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n))
            .expect("the bundled slot-A save must parse");
        let mut engine = crate::import::import_original(&set, data, 0x0C0F_FEE0)
            .expect("the bundled save imports");

        if doom_the_party {
            // Demo-only pokes on the live party model (the records on disk are
            // untouched): one hit point each, an armour class every patron
            // hits, and a to-hit number that never lands. Ten BAR PATRONs then
            // reliably finish the fight, which is the only way to watch the
            // wipe ending on screen.
            for m in &mut engine.party.members {
                m.hit_point_current = 1;
                m.combat.ac = 0; // raw AC 0 == display AC 60
                m.combat.thac0_current = 0; // hitBonus 0: never hits
            }
        }

        let out_dir = std::env::var_os("RESTRIKE_M6B_DEMO_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);

        let mut run = BarBrawl {
            engine,
            out_dir,
            label,
            manual: false,
        };
        run.settle();
        eprintln!(
            "== {label} ==\n[1] imported slot A: {} at {:?} facing {:?}",
            run.engine
                .party()
                .members
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join("/"),
            run.engine.state().pos,
            run.engine.state().facing
        );
        run.dump("00-spawn");

        // The walk: a route through Tilverton's streets to the bar, driven
        // one world-menu keypress at a time (turn, turn, forward, ...) exactly
        // as a player drives it. The route is planned from the GEO's own
        // wall/door topology, so it is the streets that decide it, not a
        // hardcoded key list.
        let route = plan_route(run.engine.geo(), run.engine.state().pos, Self::BAR);
        let route = route.expect("the bar is reachable from the spawn on open streets");
        eprintln!("[2] route to the bar: {} step(s)", route.len());
        for (i, cell) in route.iter().enumerate() {
            let facing = step_facing(run.engine.state().pos, *cell);
            // Turn on the spot until we face the next cell, then step.
            for _ in 0..3 {
                if run.engine.state().facing == facing {
                    break;
                }
                run.press(InputEvent::Ext(ExtKey::Left));
            }
            assert_eq!(run.engine.state().facing, facing, "turned to face {cell:?}");
            run.press(InputEvent::Ext(ExtKey::Up));
            eprintln!(
                "    [2.{i}] at {:?} facing {:?}{}",
                run.engine.state().pos,
                run.engine.state().facing,
                if run.engine.shell().combat_host().is_some() {
                    "  <- the bar: a fight parked"
                } else {
                    ""
                }
            );
            if run.engine.shell().combat_host().is_some() {
                break;
            }
        }
        assert_eq!(
            run.engine.state().pos,
            Self::BAR,
            "the walk reached the bar square"
        );
        assert!(
            run.engine.shell().combat_host().is_some(),
            "stepping onto the bar parked a fight"
        );
        Some(run)
    }

    /// One world-menu keypress, then ticks until the walk loop settles again —
    /// or a fight parks, whichever comes first.
    fn press(&mut self, key: crate::input::InputEvent) {
        self.engine.tick(&[key]);
        self.settle();
    }

    /// Ticks to a quiet state, feeding Enter through any event text the streets
    /// print (the M2/M3 demo pattern).
    fn settle(&mut self) {
        use crate::input::InputEvent;
        use crate::shell::Shell;
        let mut last = u64::MAX;
        let mut quiet = 0u32;
        for _ in 0..1500 {
            if self.engine.shell().combat_host().is_some() {
                return;
            }
            if matches!(self.engine.shell(), Shell::WorldMenu { .. }) {
                return;
            }
            let feed: &[InputEvent] = if quiet >= 2 {
                quiet = 0;
                &[InputEvent::Enter]
            } else {
                &[]
            };
            let serial = self.engine.tick(feed).serial;
            if serial == last {
                quiet += 1;
            } else {
                quiet = 0;
                last = serial;
            }
        }
    }

    fn dump(&self, name: &str) {
        let path = self
            .out_dir
            .join(format!("restrike-{}-{name}.ppm", self.label));
        write_ppm(self.engine.framebuffer_for_demo(), &path);
        eprintln!("      frame -> {}", path.display());
    }

    /// Plays the parked fight to its end, dumping frames along the way.
    fn play(mut self) -> BrawlOutcome {
        use crate::combat_host::Stage;
        use crate::shell::Shell;
        let mut o = BrawlOutcome::default();

        self.dump("01-a-battle-begins");
        let mut announced_roster = false;

        let mut fight_ticks = 0usize;
        let mut shots = 0usize;
        let mut prints: Vec<String> = Vec::new();
        let mut menu_shots = 0usize;
        let mut award_shot = false;
        let mut treasure_keys = 0usize;
        let exp_before: i32 = self.engine.party().members.iter().map(|m| m.exp).sum();
        for tick in 0..200_000usize {
            // ★ M6c: the keys a fight needs from whoever is watching it.
            //
            // The Continue-Battle prompt is a real question now
            // (`ovr009.cs:404`) and every party turn opens the combat menu
            // unless its `quick_fight` byte says otherwise — so even the
            // QuickFight demo answers, it just answers with `Quick`.
            let keys: Vec<InputEvent> = match self.engine.shell().combat_host().map(|h| h.stage()) {
                Some(Stage::ContinuePrompt) => vec![InputEvent::Char(b'N')],
                Some(Stage::PlayerTurn) if self.manual => {
                    scripted_player_key(&self.engine).into_iter().collect()
                }
                Some(Stage::PlayerTurn) => vec![InputEvent::Char(b'Q')],
                // ★ roll-credits slice 3: the fight now ends on
                // `displayCombatResults` and the pool screen, both blocking on
                // a key exactly as the original's do.
                Some(Stage::Results) => vec![InputEvent::Enter],
                Some(Stage::Treasure) => {
                    // Share the pool out once, then leave — `E` is the only
                    // key that closes `distributeCombatTreasure`.
                    treasure_keys += 1;
                    if treasure_keys == 1 {
                        vec![InputEvent::Char(b'S')]
                    } else {
                        vec![InputEvent::Char(b'E')]
                    }
                }
                _ => Vec::new(),
            };
            // The award screen, dumped the first time it is up — this is the
            // slice's eyeball artifact.
            if !award_shot
                && matches!(
                    self.engine.shell().combat_host().map(|h| h.stage()),
                    Some(Stage::Results)
                )
            {
                award_shot = true;
                o.exp_each = self.engine.state().exp_to_add;
                o.party_exp_gained = self
                    .engine
                    .party()
                    .members
                    .iter()
                    .map(|m| m.exp)
                    .sum::<i32>()
                    - exp_before;
                self.dump("90-the-award");
                eprintln!(
                    "      award: {} xp each, pool {} gp worth, {} item(s)",
                    self.engine.state.exp_to_add,
                    self.engine.state.pooled_money.gold_worth(),
                    self.engine.state.treasure_items.len()
                );
            }
            if self.manual
                && menu_shots < 3
                && matches!(
                    self.engine.shell().combat_host().map(|h| h.stage()),
                    Some(Stage::PlayerTurn)
                )
                && self
                    .engine
                    .shell()
                    .combat_host()
                    .and_then(|h| h.manual())
                    .is_some_and(|u| {
                        matches!(
                            u.stage(),
                            crate::combat::scene::MenuStage::Main
                                | crate::combat::scene::MenuStage::Moving
                        )
                    })
            {
                menu_shots += 1;
                let prompt = self
                    .engine
                    .shell()
                    .combat_host()
                    .and_then(|h| h.scene())
                    .and_then(|s| s.prompt())
                    .unwrap_or_default()
                    .to_string();
                eprintln!("      menu on screen: {prompt:?}");
                self.dump(&format!("0{}-the-menu", menu_shots + 1));
            }
            self.engine.tick(&keys);
            for entry in self.engine.take_transcript() {
                match entry {
                    crate::vmhost::TranscriptEntry::Print { text, .. } => {
                        if text.contains("A battle begins") {
                            o.saw_announce = true;
                        }
                        prints.push(text);
                    }
                    crate::vmhost::TranscriptEntry::Request(l) => {
                        if l.starts_with("combat:") && l.contains("round(s)") {
                            o.combat_line = l;
                        } else if l.starts_with("combat:") {
                            eprintln!("      note: {l}");
                        }
                    }
                }
            }
            match self.engine.shell().combat_host().map(|h| {
                (
                    h.stage().clone(),
                    h.outcome().is_some(),
                    h.rounds() as usize,
                )
            }) {
                Some((stage, outcome_known, rounds)) => {
                    fight_ticks += 1;
                    if !announced_roster && matches!(stage, Stage::Fighting) {
                        announced_roster = true;
                        let host = self.engine.shell().combat_host().expect("fighting");
                        let roster = host.state().roster();
                        eprintln!(
                            "[3] the fight: {} combatants ({} party, {} monsters)",
                            roster.len(),
                            roster
                                .iter()
                                .filter(|c| c.team == crate::combat::Team::Party)
                                .count(),
                            roster
                                .iter()
                                .filter(|c| c.team == crate::combat::Team::Monster)
                                .count(),
                        );
                    }
                    o.rounds = o.rounds.max(rounds);
                    if outcome_known && o.outcome_known_tick.is_none() {
                        o.outcome_known_tick = Some(tick);
                        self.dump("06-outcome-known");
                    }
                    // A handful of evenly spaced mid-fight frames.
                    if matches!(stage, Stage::Fighting)
                        && fight_ticks.is_multiple_of(900)
                        && shots < 4
                    {
                        shots += 1;
                        self.dump(&format!("0{}-round-{rounds}", shots + 1));
                    }
                    if matches!(stage, Stage::Restore) {
                        self.dump("07-final-beats");
                    }
                }
                None if !o.combat_line.is_empty() => {}
                None => {}
            }
            if self.engine.state().party_killed && o.flag_tick.is_none() {
                o.flag_tick = Some(tick);
                o.party_killed = true;
                self.dump("08-screen-restored");
            }
            if matches!(self.engine.shell(), Shell::GameOver(_)) {
                o.game_over_tick = Some(tick);
                self.dump("09-game-over");
                break;
            }
            if !o.combat_line.is_empty()
                && self.engine.shell().combat_host().is_none()
                && o.game_over_tick.is_none()
                && !o.party_killed
            {
                o.resumed_to_the_walk_loop = matches!(
                    self.engine.shell(),
                    Shell::WorldMenu { .. } | Shell::Step(_)
                );
                if o.resumed_to_the_walk_loop {
                    self.dump("08-back-in-the-walk-loop");
                    break;
                }
            }
        }

        eprintln!(
            "[4] {} — {} tick(s) of fight, party at {:?}",
            o.combat_line,
            fight_ticks,
            self.engine.state().pos
        );
        let tail: Vec<&String> = prints.iter().rev().take(6).collect();
        eprintln!("[5] last combat text: {tail:?}");
        o
    }
}

/// ★ **save → quit → relaunch → load → the same H5 digest** (roll-credits
/// slice 0's end-to-end, local tier).
///
/// The multi-session property D-RC0 exists for, proved headlessly through the
/// exact code paths `frontends/desktop` uses: the desktop's own boot posture
/// (import slot A from `<data>/SAVE`), the real camp ▸ Save ▸ Save ▸ slot
/// screens emitting a `SaveLoadRequest`, `saveload_fs::fulfill` performing it,
/// then the engine **dropped entirely** — the process-exit stand-in — and a
/// fresh one built by the same boot path and handed a `Load` request. The
/// digest either matches or the session did not survive the round trip.
///
/// Saves land in a throwaway copy of the save directory, never the real one.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture a_session_survives_a_save_quit_relaunch_load`
#[test]
fn a_session_survives_a_save_quit_relaunch_load() {
    use crate::debug_log::{self, Boot};
    use crate::saveload::SaveLoadRequest;
    use crate::saveload_fs::{fulfill, scan_slot_directory};

    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (a_session_survives_a_save_quit_relaunch_load)");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let saves = debug_log::sandbox_path("session");
    debug_log::sandbox_saves(&dir.join("SAVE"), &saves).expect("saves sandbox");

    // --- session 1: boot, play, save from inside the game ---
    let mut engine = debug_log::boot(&dir, Boot::default(), 1).expect("slot A boots");
    engine.set_slot_directory(scan_slot_directory(&saves));
    for tick in 1..=300u64 {
        engine.tick(if tick % 30 == 0 {
            &[InputEvent::Enter]
        } else {
            &[]
        });
    }
    let before = engine.state_digest();

    // camp ▸ Save ▸ Save ▸ slot J (an empty slot, so this is our own `.rsav`)
    for key in *b"essJ" {
        engine.tick(&[InputEvent::Char(key)]);
        engine.tick(&[]);
    }
    let request = engine.take_io_request().expect("the screen emitted a Save");
    assert_eq!(request, SaveLoadRequest::Save('J'));
    let data = engine.game_data().clone();
    fulfill(&mut engine, request, &saves, data, 1).expect("the host writes the slot");
    assert_eq!(
        engine.state_digest(),
        before,
        "saving does not itself change engine state"
    );

    // --- quit ---
    drop(engine);

    // --- session 2: relaunch, load ---
    let mut engine = debug_log::boot(&dir, Boot::default(), 1).expect("slot A boots again");
    engine.set_slot_directory(scan_slot_directory(&saves));
    assert_eq!(
        engine.slot_directory().status('J'),
        crate::saveload::SlotStatus::RestrikeSave,
        "the relaunched session sees the slot it wrote"
    );
    // A relaunch replays the intro before the walk loop is reachable — the
    // same 300 ticks, and then the load has to overwrite all of it.
    for tick in 1..=300u64 {
        engine.tick(if tick % 30 == 0 {
            &[InputEvent::Enter]
        } else {
            &[]
        });
    }
    for key in *b"eslJ" {
        engine.tick(&[InputEvent::Char(key)]);
        engine.tick(&[]);
    }
    let request = engine.take_io_request().expect("the screen emitted a Load");
    assert_eq!(request, SaveLoadRequest::Load('J'));
    let data = engine.game_data().clone();
    fulfill(&mut engine, request, &saves, data, 1).expect("the host restores the slot");
    engine.recompose_world_screen();

    let after = engine.state_digest();
    eprintln!("  before quit: {before}");
    eprintln!("  after load:  {after}");
    assert_eq!(after, before, "the session did not survive save/quit/load");

    let _ = std::fs::remove_dir_all(&saves);
}

/// ★ The **death screen**, for eyeballing (roll-credits slice 0, G0):
/// `AfterCombatExpAndTreasure`'s wipe branch (`ovr006.cs:801-809`) — the outer
/// frame, "The monsters rejoice for the party has been destroyed" printing into
/// its `yStart=5, xStart=2` box, and `DisplayAndPause`'s "Press any key to
/// continue" on the prompt row — then the recovery load list.
///
/// Dumps three `.ppm` frames outside the repo (D10). Local-only.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture --ignored the_death_screen_and_its_recovery`
#[test]
#[ignore]
fn the_death_screen_and_its_recovery() {
    use crate::engine::Engine;
    use crate::shell::Shell;

    let Some(root) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (the_death_screen_and_its_recovery)");
        return;
    };
    let root = std::path::Path::new(&root);
    let data = load_dir(root).expect("GBX_DATA_DIR must be readable");
    let saves = load_dir(&root.join("SAVE")).expect("GBX_DATA_DIR/SAVE must be readable");
    let master = saves.raw_file("SAVGAMA.DAT").expect("slot A must exist");
    let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n))
        .expect("slot A must parse");
    let mut engine = crate::import::import_original(&set, data, 1).expect("slot A must import");
    engine.set_slot_directory(crate::saveload_fs::scan_slot_directory(&root.join("SAVE")));

    let out_dir = std::env::temp_dir();
    let dump = |engine: &mut Engine, name: &str| {
        let f = engine.tick(&[]);
        let mut fb = Framebuffer::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
            }
        }
        let path = out_dir.join(format!("restrike-{name}.ppm"));
        write_ppm(&fb, &path);
        eprintln!("  {name} -> {}", path.display());
    };

    for _ in 0..200 {
        engine.tick(&[]);
        if matches!(engine.shell(), Shell::WorldMenu { .. }) {
            break;
        }
        if engine.shell().gate_open() {
            engine.tick(&[InputEvent::Enter]);
        }
    }

    engine.state.party_killed = true;
    engine.tick(&[]);
    dump(&mut engine, "gameover-1-message");
    for _ in 0..600 {
        engine.tick(&[]);
        if engine.probe() == "game-over/press-any-key" {
            break;
        }
    }
    assert_eq!(engine.probe(), "game-over/press-any-key");
    dump(&mut engine, "gameover-2-press-any-key");

    engine.tick(&[InputEvent::Char(b' ')]);
    dump(&mut engine, "gameover-3-recovery-load-list");
    eprintln!("  recovery slots: {:?}", engine.slot_directory());
}

// --- Roll-credits slice 2: the encounter cluster on real data ---------------

/// ★ **Slice 2's acceptance drive, part 1: the Tilverton approach.**
///
/// `ECL2` block 2 `@0x8780` is the first arm of a five-way `ON GOTO` over the
/// street-encounter table (`@0x876B`), and it is the whole approach cluster in
/// nine instructions:
///
/// ```text
/// @0x8780  SAVE #0x01, [0x7EE1]        ; HeadBlockId = 1  -> the close-up is head+body
/// @0x8786  SETUP MONSTER #1, #1, #1    ; SPRIT2 block 1, max distance 1, PIC/BODY 1
/// @0x878D  PRINTCLEAR "<approach text>"
/// @0x87A9  GOSUB 0x8D04                ; a one-option HORIZONTAL MENU (press to continue)
/// @0x87AD  APPROACH                    ; distance 1 -> 0
/// @0x87AE  PRINTCLEAR "<they close>"
/// @0x87BA  SAVE #0xFF, [0x7EE1]
/// @0x87C0  CLEARMONSTERS
/// @0x87C1  LOAD MONSTER … x2
/// @0x87CF  COMBAT
/// ```
///
/// Two frames come out of it and both are dumped for eyeballing: the far band
/// (the masked `SPRIT2` sprite standing in the 3D corridor, script text under
/// it, the continue prompt on the menu line) and, one `APPROACH` later, the
/// close-up (`HEAD2`/`BODY2` block 1 filling the viewport where the sprite
/// was). Run with `GBX_DATA_DIR` set; `RESTRIKE_SLICE2_DEMO_OUT` picks the
/// output directory (default: the system temp dir). No game data enters the
/// repo — the `.ppm`s land outside it, like every other demo here.
///
/// **One artifact of entering mid-block:** the 3D corridor behind the sprite
/// is blank. `ECL2#2`'s own entry vector is what runs `LOAD FILES`/`LOAD
/// PIECES` and makes a wallset resident, and this drive deliberately starts
/// past it (the route in is the street-encounter roll, not the block's
/// arrival). The picture layer is what these frames are for, and it composes
/// over whatever the viewport holds — so the sprite and the portrait land in
/// exactly the cells they would over a painted corridor.
#[test]
fn slice2_the_tilverton_approach() {
    use crate::input::InputEvent;
    use crate::picture::Shown;

    let Some(mut engine) = crate::area_transition_tests::real_data_engine(2, 2, 1, true) else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (demo::slice2_the_tilverton_approach)");
        return;
    };
    let out_dir = std::env::var_os("RESTRIKE_SLICE2_DEMO_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);

    // Enter at the encounter arm itself. Everything upstream (`@0x86D5`'s
    // search-flag test, `@0x871E`'s day counter, the `ON GOTO`) is ordinary
    // flow this drive has no reason to re-prove.
    // Stand somewhere the approach ray can actually run: `sub_304B4` walks up
    // to two cells forward and stops at the first wall (and at the map edge),
    // so a party facing a wall gets distance 0 and the whole approach collapses
    // into its own close-up. Pick the first facing at the imported position
    // that leaves at least one band of room — which is what a party that just
    // walked down a street has by construction.
    let (px, py) = (engine.state().pos.0 as i32, engine.state().pos.1 as i32);
    let facing = [
        crate::movement::Facing::North,
        crate::movement::Facing::East,
        crate::movement::Facing::South,
        crate::movement::Facing::West,
    ]
    .into_iter()
    .find(|f| {
        crate::combat::encounter_distance(
            engine.geo(),
            crate::shell::facing_to_map_dir(*f),
            px,
            py,
            true,
        ) >= 1
    })
    .expect("Tilverton's streets must have one open direction at the spawn cell");
    engine.state.facing = facing;
    eprintln!("slice2 approach: standing at ({px},{py}) facing {facing:?}");

    engine.shell = crate::shell::boot_at_address(&mut engine.machine, 0x8780);

    let mut seen: Vec<(String, u8, u8)> = Vec::new();
    let mut last = Shown::Nothing;
    for tick in 0..600 {
        // Enter answers the one-option continue prompt at `@0x8D04`; before it
        // parks, the key is simply ignored.
        let pixels: Vec<u8> = engine.tick(&[InputEvent::Enter]).pixels.to_vec();
        let shown = engine.state().picture.shown;
        if shown != last && shown != Shown::Nothing {
            let label = match shown {
                Shown::Sprite => "sprite",
                Shown::HeadBody => "closeup",
                Shown::Pic => "pic",
                Shown::BigPic => "bigpic",
                Shown::Nothing => unreachable!(),
            };
            let path = out_dir.join(format!("restrike-slice2-approach-{label}.ppm"));
            write_ppm_pixels(&pixels, &path);
            eprintln!(
                "slice2 approach: tick {tick} -> {label} (distance {}, sprite frame {}) -> {}",
                engine.state().encounter_distance,
                engine.state().picture.sprite_frame,
                path.display()
            );
            seen.push((
                label.to_string(),
                engine.state().encounter_distance,
                engine.state().picture.sprite_frame,
            ));
            last = shown;
        }
        if engine.state().pending_combat.monsters_loaded {
            break; // the fight is next; the approach is what this drive is for
        }
    }

    eprintln!("slice2 approach: halts {:?}", engine.vm_memory().halts);
    assert!(
        engine.vm_memory().halts.is_empty(),
        "the approach must be halt-free: {:?}",
        engine.vm_memory().halts
    );
    assert_eq!(
        seen.iter().map(|(l, _, _)| l.as_str()).collect::<Vec<_>>(),
        ["sprite", "closeup"],
        "SETUP MONSTER puts the SPRIT band up; APPROACH walks it to 0 and the \
         head+body close-up takes the window (HeadBlockId was still 1)"
    );
    assert_eq!(
        seen[0].1, 1,
        "max_distance 1 clamps the ray to one band out"
    );
    assert_eq!(seen[0].2, 2, "…and Show3DSprite blits frame distance + 1");
    assert_eq!(seen[1].1, 0, "APPROACH closed the distance");
    assert_eq!(engine.state().picture.head_block, 1);
    assert_eq!(
        engine.state().picture.body_block,
        1,
        "pic_block_id doubles as the BODY id when a head is set"
    );
    assert_eq!(
        engine.state().encounter_flags,
        [true, true],
        "the SPRIT set loaded, then the close-up latched"
    );
}

/// ★ **Slice 2's acceptance drive, part 2: the ENCOUNTER MENU, live.**
///
/// `ECL4` block 32 `@0x98A9` is one of the opcode's two shipped uses. Its
/// operands are `sprite 0x22, max 0, pic 0x22, cell [0x7F79],
/// var_6 = [0,3,0,0,3], <one approach line>, "", "", 0x0C, 0x0C` — a
/// `max_distance` of 0, so the monsters are already adjacent and the fourth
/// word is PARLAY. The script then branches on the cell: `COMPARE [0x7F79],
/// #0x03 / IF <> / GOTO 0x993F` sends everything except parlay into
/// `CLEARMONSTERS; LOAD MONSTER 0x20 x8; LOAD MONSTER 0x22 x5; COMBAT`.
///
/// This drive answers COMBAT and checks the words on the menu line against the
/// original's own builder, then re-runs it answering PARLAY (slot 3, which the
/// remap resolves to slot **4** → class 3 → writes 3) and checks the cell.
///
/// The frame it dumps is the one that proves `byte_1EE95` earns its keep: the
/// distance is 0, so `sub_30580` would normally swap to the close-up — and
/// does not, because the menu flag is up (`ovr008.cs:257`). What stands in the
/// viewport under "COMBAT WAIT FLEE PARLAY" is the `SPRIT4` approach sprite,
/// which is what the original shows here too.
#[test]
fn slice2_the_encounter_menu_at_its_shipped_site() {
    use crate::input::InputEvent;

    const RESULT_CELL: u16 = 0x7F79;

    for (key, want, label) in [(b'C', 1u16, "combat"), (b'P', 3u16, "parlay")] {
        let Some(mut engine) = crate::area_transition_tests::real_data_engine(4, 32, 32, true)
        else {
            eprintln!(
                "SKIPPED: local tier needs GBX_DATA_DIR \
                 (demo::slice2_the_encounter_menu_at_its_shipped_site)"
            );
            return;
        };
        let out_dir = std::env::var_os("RESTRIKE_SLICE2_DEMO_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);

        engine.shell = crate::shell::boot_at_address(&mut engine.machine, 0x98A3);

        // Drive to the parked menu.
        let mut parked = false;
        let mut pixels: Vec<u8> = Vec::new();
        for _ in 0..400 {
            pixels = engine.tick(&[]).pixels.to_vec();
            if engine.shell.gate_open() {
                parked = true;
                break;
            }
        }
        assert!(
            parked,
            "the encounter menu never parked: {:?}",
            engine.vm_memory().halts
        );

        let words: Vec<String> = engine
            .take_transcript()
            .into_iter()
            .filter_map(|e| match e {
                crate::vmhost::TranscriptEntry::Request(l) => Some(l),
                _ => None,
            })
            .collect();
        assert!(
            words.iter().any(|w| w == "menu: COMBAT WAIT FLEE PARLAY"),
            "max_distance 0 means the monsters are adjacent, so the fourth word \
             is PARLAY: {words:?}"
        );

        let path = out_dir.join(format!("restrike-slice2-encounter-menu-{label}.ppm"));
        write_ppm_pixels(&pixels, &path);
        eprintln!("slice2 encounter menu ({label}): {}", path.display());

        engine.tick(&[InputEvent::Char(key)]);
        for _ in 0..40 {
            if engine.vm_memory().raw_word(RESULT_CELL) == Some(want) {
                break;
            }
            engine.tick(&[]);
        }
        assert_eq!(
            engine.vm_memory().raw_word(RESULT_CELL),
            Some(want),
            "var_6 = [0,3,0,0,3]: COMBAT is class 0 -> 1, PARLAY resolves to \
             slot 4 -> class 3 -> 3"
        );
    }
}

// --- Roll-credits slice 4: Vancian camp magic on the real party ------------

/// ★ **Slice 4's acceptance drive (D-S4e), on the bundled slot-A party.**
///
/// The party the GOG save ships is exactly the right fixture: SHARA is a
/// cleric with `spellCastCount[0] = [5, 5, 2, 0, 0]` and twenty-three spells in
/// her grimoire, and **every** member's `spell_list` is empty — nothing is
/// memorized, so the very first thing a real playthrough has to do is stage
/// spells and sleep on them.
///
/// The drive: boot the save, camp, open Magic ▸ Memorize on SHARA, stage from
/// the grimoire (frame dumped), then Rest (countdown frame dumped) and watch
/// the staged spells commit at the elapsed time `sub_44032` priced them at.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine --release \
///   -- --nocapture --ignored slice4_the_cleric_memorizes_and_sleeps_on_it`
#[test]
#[ignore = "local-only demo (writes frames); run explicitly"]
fn slice4_the_cleric_memorizes_and_sleeps_on_it() {
    use crate::engine::Engine;
    use crate::input::InputEvent;
    use crate::screens::Screen;
    use crate::shell::Shell;

    let Some(root) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (slice4 camp magic)");
        return;
    };
    let root = std::path::Path::new(&root);
    let data = load_dir(root).expect("GBX_DATA_DIR must be readable");
    let saves = load_dir(&root.join("SAVE")).expect("GBX_DATA_DIR/SAVE must be readable");
    let master = saves.raw_file("SAVGAMA.DAT").expect("slot A must exist");
    let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n))
        .expect("slot A must parse");
    let mut engine = crate::import::import_original(&set, data, 1).expect("slot A must import");

    let out_dir = std::env::temp_dir();
    let dump = |engine: &mut Engine, name: &str| {
        let f = engine.tick(&[]);
        let mut fb = Framebuffer::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
            }
        }
        let path = out_dir.join(format!("restrike-{name}.ppm"));
        write_ppm(&fb, &path);
        eprintln!("  {name} -> {}", path.display());
    };

    // Boot out to the walk loop.
    for _ in 0..300 {
        engine.tick(&[]);
        if matches!(engine.shell(), Shell::WorldMenu { .. }) {
            break;
        }
        if engine.shell().gate_open() {
            engine.tick(&[InputEvent::Enter]);
        }
    }
    assert!(matches!(engine.shell(), Shell::WorldMenu { .. }));

    // SHARA is slot 4 (0-based) in the shipped roster; select her, then camp.
    let shara = engine
        .party()
        .members
        .iter()
        .position(|m| m.name == "SHARA")
        .expect("the bundled party carries SHARA");
    engine.state.selected_player = shara as u8;
    let before = engine.party().members[shara].magic.clone();
    assert!(
        before.spell_list.iter().all(|&b| b == 0),
        "the shipped save memorizes nothing at all"
    );
    assert_eq!(
        before.cast_count[0],
        [5, 5, 2, 0, 0],
        "SHARA's cleric slots: 5/5/2 at levels 1-3"
    );

    let feed = |engine: &mut Engine, key: u8, budget: usize| {
        engine.tick(&[InputEvent::Char(key)]);
        for _ in 0..budget {
            engine.tick(&[]);
        }
    };

    feed(&mut engine, b'E', 4); // Encamp
    assert!(matches!(engine.shell(), Shell::Screen(Screen::Camp(_))));
    feed(&mut engine, b'M', 4); // Magic
    assert!(matches!(engine.shell(), Shell::Screen(Screen::Magic(_))));
    feed(&mut engine, b'M', 4); // Memorize — nothing staged, straight to the picker
    assert!(
        matches!(engine.shell(), Shell::Screen(Screen::Memorize(_))),
        "Memorize opened: {}",
        engine.probe()
    );
    dump(&mut engine, "slice4-1-memorize-grimoire");

    // Stage the highlighted spell three times — the picker rebuilds its list
    // and its capacity table after every pick, so the same key stages three.
    for _ in 0..3 {
        engine.tick(&[InputEvent::Enter]);
        for _ in 0..2 {
            engine.tick(&[]);
        }
    }
    dump(&mut engine, "slice4-2-memorize-after-staging");
    let staged: Vec<u8> = crate::magic::learning(&engine.party().members[shara].magic.spell_list)
        .map(|e| e.id)
        .collect();
    assert_eq!(staged.len(), 3, "three spells staged: {staged:?}");
    eprintln!(
        "  staged: {:?}",
        staged
            .iter()
            .map(|&id| crate::magic::spell_name(id))
            .collect::<Vec<_>>()
    );

    // ★ A mid-staging save round-trips: staging is record state, so it rides
    // the `.rsav` with no format change at all.
    let bytes = engine.save();
    let reload_data = load_dir(root).expect("GBX_DATA_DIR must be readable");
    let restored = Engine::restore(&bytes, reload_data).expect("a mid-staging save reloads");
    let restored_staged: Vec<u8> =
        crate::magic::learning(&restored.party().members[shara].magic.spell_list)
            .map(|e| e.id)
            .collect();
    assert_eq!(restored_staged, staged, "staging survives save/load");

    // Exit the picker; the closing review + its confirm come up.
    feed(&mut engine, b'E', 2);
    dump(&mut engine, "slice4-3-memorize-closing-review");
    feed(&mut engine, b'Y', 4); // "Memorize these spells? " — keep them
    assert!(matches!(engine.shell(), Shell::Screen(Screen::Magic(_))));

    // ★ Rest. `sub_44032` priced the three staged spells; `rest_menu` split it
    // into the countdown this screen shows.
    feed(&mut engine, b'R', 4);
    let Shell::Screen(Screen::Rest(_)) = engine.shell() else {
        panic!("Rest opened: {}", engine.probe());
    };
    dump(&mut engine, "slice4-4-rest-countdown");

    let clock_before = engine.state.clock;
    engine.tick(&[InputEvent::Char(b'R')]); // commit the countdown
                                            // Rest was opened from the MAGIC bar, so it returns there (`magic_menu`'s
                                            // own re-display loop, `ovr016.cs:636`) rather than to camp.
    for _ in 0..4000 {
        engine.tick(&[]);
        if matches!(engine.shell(), Shell::Screen(Screen::Magic(_))) {
            break;
        }
    }
    dump(&mut engine, "slice4-5-after-the-rest");
    assert!(matches!(engine.shell(), Shell::Screen(Screen::Magic(_))));

    let after = &engine.party().members[shara].magic;
    assert_eq!(
        crate::magic::learning(&after.spell_list).count(),
        0,
        "everything staged committed"
    );
    let memorized: Vec<u8> = crate::magic::learnt(&after.spell_list)
        .map(|e| e.id)
        .collect();
    assert_eq!(memorized.len(), 3, "…and is in memory: {memorized:?}");
    // ★ FD-25: rest is not a slot restoration.
    assert_eq!(
        after.cast_count, before.cast_count,
        "cast_count is capacity, untouched by rest"
    );
    // ★ Capacity reflects what is now held. The list's initial highlight is
    // not row 1: `sl_select_item` runs `index_ptr++` then
    // `menu_scroll_in_page(false, …)` before its first draw
    // (`ovr027.cs:572-573`), and `skipHeadings` walking *backwards* off the
    // leading "1st Level" heading wraps to the bottom of the visible page
    // (`:443-455`). With SHARA's 11-row Memorize box that is row 10 — the
    // first SECOND-level spell. Our `ListMenu` reproduces the arithmetic
    // exactly, so the drive stages whatever the original would have.
    let level = crate::magic::spell_level(staged[0]);
    let class_ = crate::magic::spell_class(staged[0]);
    let ch = &engine.party().members[shara];
    let left = crate::magic::how_many_spells_player_can_learn(&ch.magic, class_, level);
    let capacity = crate::magic::cast_count_at(&ch.magic, class_, level);
    eprintln!("  level-{level} slots: {left} free of {capacity}");
    assert_eq!(
        i32::from(capacity) - left,
        3,
        "the three memorized spells hold three slots at their own level"
    );
    assert!(
        engine.state.clock != clock_before,
        "the world clock advanced across the rest"
    );

    // ★ Camp exit cancels staging. Stage one more, then walk out.
    feed(&mut engine, b'M', 4); // Magic ▸ Memorize
    engine.tick(&[InputEvent::Enter]);
    for _ in 0..3 {
        engine.tick(&[]);
    }
    feed(&mut engine, b'E', 2); // out of the picker
    feed(&mut engine, b'Y', 4); // keep it staged
    feed(&mut engine, b'E', 4); // out of Magic, back to camp
    let staged_now =
        crate::magic::learning(&engine.party().members[shara].magic.spell_list).count();
    feed(&mut engine, b'E', 4); // out of camp
    assert_eq!(
        crate::magic::learning(&engine.party().members[shara].magic.spell_list).count(),
        0,
        "camp exit ran cancel_spells (staged before exit: {staged_now})"
    );
    assert_eq!(
        crate::magic::learnt(&engine.party().members[shara].magic.spell_list).count(),
        3,
        "…and the memorized spells survived it"
    );
}

// --- Roll-credits slice 5: the spell must-haves ----------------------------

/// ★ **Slice 5's Task-0 evidence (roll-credits §9.1)**: the two spell books the
/// bundled slot-A party actually carries, which is what sizes G7's must-have
/// set. Prints class levels, `spellCastCount`, the decoded grimoire
/// (`spellBook[id - 1]`, `Player.cs:363`) and the live affect chain.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture --ignored slice5_the_spell_books_that_size_the_set`
#[test]
#[ignore = "local-only evidence dump; run explicitly"]
fn slice5_the_spell_books_that_size_the_set() {
    let Some(root) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (slice5 spell books)");
        return;
    };
    let root = std::path::Path::new(&root);
    let data = load_dir(root).expect("GBX_DATA_DIR must be readable");
    let saves = load_dir(&root.join("SAVE")).expect("GBX_DATA_DIR/SAVE must be readable");
    let master = saves.raw_file("SAVGAMA.DAT").expect("slot A must exist");
    let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n))
        .expect("slot A must parse");
    let engine = crate::import::import_original(&set, data, 1).expect("slot A must import");
    let mut casters = 0;
    for m in &engine.party().members {
        eprintln!(
            "--- {} class_id={} levels={:?} race={} align={} hp={}/{}",
            m.name,
            m.class_id,
            m.class_level,
            m.race,
            m.alignment,
            m.hit_point_current,
            m.hit_point_max
        );
        eprintln!("    cast_count={:?}", m.magic.cast_count);
        // `spellBook[id - 1]` — the off-by-one §9.1 pins.
        let book: Vec<(u8, &str)> = (1u8..=0x64)
            .filter(|&id| crate::magic::knows_spell(m, id))
            .map(|id| (id, crate::magic::spell_name(id)))
            .collect();
        if !book.is_empty() {
            casters += 1;
        }
        eprintln!("    grimoire ({}) = {book:?}", book.len());
        eprintln!(
            "    affects={:?}",
            m.affects
                .iter()
                .filter_map(|r| gbx_formats::affects::AffectRecord::decode(r))
                .map(|a| (a.kind, a.minutes))
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        casters, 3,
        "SHARA, LEDERA and PHILIPPE are the only casters"
    );
}

/// ★ **Slice 5's acceptance drive (roll-credits §9.2)**: the camp Cast flow, on
/// the bundled slot-A party.
///
/// SHARA memorizes from her own grimoire, sleeps on it, and then *casts* —
/// which is the first time in this engine's life that a spell has come out of
/// camp. The drive proves the three shapes `NonCombatSpellCast` switches on:
///
/// 1. **WholeParty** — Bless lands on all six at once;
/// 2. **PartyMember** — Cure Light Wounds opens `selectAPlayer` and heals the
///    member the cursor is on (MATHEW, wounded on purpose beforehand);
/// 3. **the refusal** — a memorized Hold Person is `SpellTargets::Combat`, so
///    camp shows "can't be cast here..." and offers "Lose it?".
///
/// Frames are dumped for the picker, the target selector and the result line.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine --release \
///   -- --nocapture --ignored slice5_the_cleric_casts_in_camp`
#[test]
#[ignore = "local-only demo (writes frames); run explicitly"]
fn slice5_the_cleric_casts_in_camp() {
    use crate::engine::Engine;
    use crate::screens::Screen;
    use crate::shell::Shell;

    let Some(root) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (slice5 camp casting)");
        return;
    };
    let root = std::path::Path::new(&root);
    let data = load_dir(root).expect("GBX_DATA_DIR must be readable");
    let saves = load_dir(&root.join("SAVE")).expect("GBX_DATA_DIR/SAVE must be readable");
    let master = saves.raw_file("SAVGAMA.DAT").expect("slot A must exist");
    let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n))
        .expect("slot A must parse");
    let mut engine = crate::import::import_original(&set, data, 1).expect("slot A must import");

    let out_dir = std::env::temp_dir();
    let dump = |engine: &mut Engine, name: &str| {
        let f = engine.tick(&[]);
        let mut fb = Framebuffer::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
            }
        }
        let path = out_dir.join(format!("restrike-{name}.ppm"));
        write_ppm(&fb, &path);
        eprintln!("  {name} -> {}", path.display());
    };

    for _ in 0..300 {
        engine.tick(&[]);
        if matches!(engine.shell(), Shell::WorldMenu { .. }) {
            break;
        }
        if engine.shell().gate_open() {
            engine.tick(&[InputEvent::Enter]);
        }
    }
    assert!(matches!(engine.shell(), Shell::WorldMenu { .. }));

    let shara = engine
        .party()
        .members
        .iter()
        .position(|m| m.name == "SHARA")
        .expect("the bundled party carries SHARA");
    engine.state.selected_player = shara as u8;
    // Wound MATHEW so the cure has something to do. The memorize/rest loop is
    // slice 4's and is already proven, so the slots are seeded directly; what
    // this drive is about is that a memorized spell is now *castable*.
    engine.party.members[0].hit_point_current = 12;

    let feed = |engine: &mut Engine, key: u8, budget: usize| {
        engine.tick(&[InputEvent::Char(key)]);
        for _ in 0..budget {
            engine.tick(&[]);
        }
    };
    let settle = |engine: &mut Engine| {
        for _ in 0..3 {
            engine.tick(&[]);
        }
    };

    feed(&mut engine, b'E', 4); // Encamp
    assert!(matches!(engine.shell(), Shell::Screen(Screen::Camp(_))));

    // One spell at a time, so the list has exactly one row and the opening
    // highlight is unambiguous (§8.1's `sl_select_item` wrap makes a
    // multi-level list open on the first *second*-level row, which is a slice-4
    // finding this drive would otherwise trip over).
    let open_cast = |engine: &mut Engine, spell_id: u8| {
        crate::magic::add_learnt(&mut engine.party.members[shara].magic.spell_list, spell_id);
        // A spent Cast list empties and bounces straight back to the magic
        // menu (`cast_spell`'s own `while (spell_id != 0)` exit), so the drive
        // may already be there.
        if matches!(engine.shell(), Shell::Screen(Screen::Camp(_))) {
            engine.tick(&[InputEvent::Char(b'M')]);
            for _ in 0..4 {
                engine.tick(&[]);
            }
        }
        engine.tick(&[InputEvent::Char(b'C')]); // Cast
        for _ in 0..4 {
            engine.tick(&[]);
        }
        assert!(
            matches!(engine.shell(), Shell::Screen(Screen::Cast(_))),
            "the Cast list opened for {spell_id:#04x}: {}",
            engine.probe()
        );
    };

    // (1) **Bless** — `WholeParty`, so no selector at all: pick it and it lands.
    open_cast(&mut engine, 0x01);
    dump(&mut engine, "slice5-1-cast-picker");
    engine.tick(&[InputEvent::Enter]);
    settle(&mut engine);
    dump(&mut engine, "slice5-2-bless-on-the-whole-party");
    let blessed = engine
        .party()
        .members
        .iter()
        .filter(|m| m.has_affect(crate::spells::AFF_BLESS))
        .count();
    eprintln!("  after Bless: blessed={blessed}/6");
    assert_eq!(blessed, 6, "Bless is a WholeParty spell");

    // (2) **Cure Light Wounds** — `PartyMember`, so the cast parks on
    // `selectAPlayer` with the cursor on the caster. Walk it back to MATHEW
    // (`'G'` = Home/Kp7 is the original's own previous-player key) and commit.
    open_cast(&mut engine, 0x03);
    engine.tick(&[InputEvent::Enter]);
    settle(&mut engine);
    dump(&mut engine, "slice5-3-cast-spell-on-whom");
    for _ in 0..shara {
        engine.tick(&[InputEvent::Ext(ExtKey::Home)]);
        engine.tick(&[]);
    }
    engine.tick(&[InputEvent::Enter]);
    settle(&mut engine);
    dump(&mut engine, "slice5-4-cure-light-wounds-landed");
    let healed = engine.party().members[0].hit_point_current;
    eprintln!("  after Cure Light Wounds: MATHEW hp={healed} (was 12)");
    assert!(healed > 12, "the cure healed the member the cursor was on");

    // (3) **Hold Person** — `SpellTargets::Combat`, so camp refuses it with
    // "can't be cast here..." and offers "Lose it? ". Yes burns the slot.
    open_cast(&mut engine, 0x17);
    engine.tick(&[InputEvent::Enter]);
    settle(&mut engine);
    dump(&mut engine, "slice5-5-cant-be-cast-here");
    feed(&mut engine, b'Y', 4);
    assert!(
        crate::magic::learnt(&engine.party().members[shara].magic.spell_list)
            .next()
            .is_none(),
        "every slot was spent — including the one lost to \"Lose it?\""
    );

    // Magic ▸ Display now has something to show (slice 4's 'D' leaf, which
    // until this slice could only ever list imported racial affects).
    if !matches!(engine.shell(), Shell::Screen(Screen::Magic(_))) {
        feed(&mut engine, b'E', 4);
    }
    feed(&mut engine, b'D', 4);
    dump(&mut engine, "slice5-6-display-magic-effects");
}

/// ★ **Slice 5's in-combat casting beat** (roll-credits §9.2's acceptance),
/// rendered over the real art through the M6a scene.
///
/// A cleric blesses the line and a magic-user drops a fireball into a room full
/// of monsters, and every tick of the resulting message cascade is dumped. What
/// the frames have to show, which no test can:
///
/// - the "Casts a Spell" / spell-name pair the D-CV2 `Cast` event opens with;
/// - one `SpellTarget` highlight per combatant the blast caught — an area spell
///   lights up the *whole* list, not one icon;
/// - `AffectApplied`'s new line — "is Blessed" for each team-mate the bless
///   kept, "is affected" for the rest;
/// - the damage cascade and any deaths it causes, in the order §1.5 shows them.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture --ignored slice5_a_bless_and_a_fireball_on_screen`
/// then e.g. `ffmpeg -framerate 60 -i /tmp/restrike-slice5cast-%04d.ppm out.mp4`.
#[test]
#[ignore = "local-only demo (writes frames); run explicitly"]
fn slice5_a_bless_and_a_fireball_on_screen() {
    use crate::combat::scene::{CombatScene, CombatantIdentity, EntrySnapshot, SceneArt};
    use crate::combat::{
        ActionEvent, ActionSink, CombatMap, CombatState, Combatant, GridPos, Team,
    };
    use crate::combat_art;
    use crate::party::IconInfo;
    use crate::rng::EngineRng;
    use std::cell::RefCell;
    use std::rc::Rc;

    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (slice5 in-combat cast)");
        return;
    };
    let data = load_dir(std::path::Path::new(&dir)).expect("GBX_DATA_DIR must be readable");
    let assets = boot(&data).expect("boot must succeed against real CotAB data");
    let out_dir = std::env::temp_dir();

    // The art, exactly as the M6a reel loads it.
    let mut icons = assets.combat_icons.clone();
    let colours: [u8; 6] = [0x91, 0xA2, 0xB3, 0xC4, 0xE6, 0xF7];
    for (slot, (head, weapon)) in [(0u8, 0u8), (3, 5)].into_iter().enumerate() {
        let info = IconInfo {
            head_icon: head,
            weapon_icon: weapon,
            icon_id: slot as u8,
            icon_size: 1,
            colours,
        };
        icons.set(
            slot,
            combat_art::load_party_icon(&data, &info, true).expect("party icon"),
        );
    }
    icons.set(
        8,
        combat_art::load_monster_icon(&data, 2, 0).expect("monster icon"),
    );
    let tiles = combat_art::load_ground_tiles(&data, true).expect("dungeon ground tiles");

    // A walled room: two casters on the west wall, four monsters packed east.
    let mut map = CombatMap::uniform(0x17);
    for x in 17..31 {
        map.set_tile(GridPos::new(x, 8), 1);
        map.set_tile(GridPos::new(x, 17), 1);
    }
    for y in 8..18 {
        map.set_tile(GridPos::new(17, y), 1);
        map.set_tile(GridPos::new(30, y), 1);
    }
    let names = ["SHARA", "PHILIPPE", "THIEF", "THIEF", "THIEF", "THIEF"];
    let mut fighters: Vec<Combatant> = Vec::new();
    for (i, (team, x, y, hp)) in [
        (Team::Party, 19, 12, 29),
        (Team::Party, 19, 13, 27),
        (Team::Monster, 26, 12, 24),
        (Team::Monster, 27, 12, 24),
        (Team::Monster, 26, 13, 24),
        (Team::Monster, 27, 13, 24),
    ]
    .into_iter()
    .enumerate()
    {
        fighters.push(Combatant::new_melee(
            i,
            team,
            team == Team::Monster,
            GridPos::new(x, y),
            hp,
            5,
            16,
            12,
            (1, 6, 0),
            5,
            1,
        ));
    }
    let mut state = CombatState::new(map, fighters);
    state.fighters[0].memorized_list = vec![0x01];
    state.fighters[0].skill_level_cleric = 5;
    state.fighters[1].memorized_list = vec![0x2F];
    state.fighters[1].skill_level_magic_user = 5;

    #[derive(Clone, Default)]
    struct Batch(Rc<RefCell<Vec<ActionEvent>>>);
    struct BatchSink(Rc<RefCell<Vec<ActionEvent>>>);
    impl ActionSink for BatchSink {
        fn on_action(&mut self, event: ActionEvent) {
            self.0.borrow_mut().push(event);
        }
    }
    let batch = Batch::default();
    state.attach_action_sink(Box::new(BatchSink(Rc::clone(&batch.0))));

    let identities: Vec<CombatantIdentity> = (0..state.roster().len())
        .map(|i| CombatantIdentity::new(names[i], if i < 2 { i } else { 8 }))
        .collect();
    let mut scene = CombatScene::new(
        EntrySnapshot::from_state(&state, &identities),
        SceneArt::new(tiles, icons),
    );
    scene.refresh_panels(&state);
    scene.reconcile(&state).expect("the entry snapshot matches");

    let mut rng = EngineRng::new(0x0C0F_FEE0);
    let mut frames = 0usize;
    // Two casts, back to back, each played out to the last beat.
    for (actor, spell_id, label) in [(0usize, 0x01u8, "bless"), (1, 0x2F, "fireball")] {
        state.sub_5d2e1_demo(&mut rng, actor, spell_id);
        let events = std::mem::take(&mut *batch.0.borrow_mut());
        eprintln!("  {label}: {} events", events.len());
        scene.begin_step(&events);
        while scene.is_playing() {
            scene.tick(1);
            let mut fb = Framebuffer::new();
            scene
                .render_frame(&mut fb, &assets.symbol_sets, &assets.font)
                .expect("the cast beat must render");
            write_ppm(
                &fb,
                &out_dir.join(format!("restrike-slice5cast-{frames:04}.ppm")),
            );
            frames += 1;
        }
        scene.reconcile(&state).expect("the board reconciles");
        scene.refresh_panels(&state);
    }

    eprintln!(
        "slice5 cast beat: {frames} frames -> {}/restrike-slice5cast-*.ppm",
        out_dir.display()
    );
    eprintln!(
        "  blessed: {:?}",
        (0..2)
            .map(|i| state.roster()[i].has_affect(crate::spells::AFF_BLESS))
            .collect::<Vec<_>>()
    );
    eprintln!(
        "  monster hp after the fireball: {:?}",
        (2..6)
            .map(|i| state.roster()[i].hp_current)
            .collect::<Vec<_>>()
    );
    assert!(frames > 30, "the cascade played out");
}

/// ★ **Roll-credits slice 6 (G8): the temple, on screen, with the real party.**
///
/// The `COMBAT` opcode's non-monster branch is driven by the shipped idiom
/// verbatim — `SAVE 0xFF → 0x7EE1; SAVE 1 → 0x7EE2; CLEARMONSTERS; COMBAT`, the
/// four instructions at `ECL2#1 @0x91DF` (Tilverton). The block those bytes
/// live in is authored here rather than reached through the intro, so the run
/// is short; everything else is real — the bundled slot-A party, the real font
/// and symbol sets, the real money.
///
/// Dumps: the temple's own menu, the Heal service list, the price line, and the
/// result after a member has been raised from the dead.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine --release \
///   -- --nocapture --ignored slice6_a_visit_to_the_temple`
#[test]
#[ignore = "local-only demo (writes frames); run explicitly"]
fn slice6_a_visit_to_the_temple() {
    use crate::engine::Engine;
    use gbx_formats::game_data::GameData;

    let Some(root) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (slice6 temple)");
        return;
    };
    let root = std::path::Path::new(&root);
    let real = load_dir(root).expect("GBX_DATA_DIR must be readable");

    // The shipped temple idiom, in a block of its own. `simple_block` puts it
    // behind every vector, so boot's entry vector runs it.
    let program = crate::test_support::simple_block(|b| {
        b.op(0x09).imm_byte(0xFF).mem(0x7EE1); // SAVE 0xFF -> HeadBlockId
        b.op(0x09).imm_byte(1).mem(0x7EE2); // SAVE 1 -> EnterTemple
        b.op(0x1C); // CLEARMONSTERS
        b.op(0x24); // COMBAT
        b.op(0x11).inline_str(b"THE TEMPLE DOOR CLOSES."); // PRINT
        b.op(0x00); // EXIT
    });
    let ecl = crate::test_support::build_dax_file(&[(
        1u8,
        crate::test_support::ecl_dax_block(&program.build_bytes()),
    )]);
    let files: Vec<(String, Vec<u8>)> = real
        .file_names()
        .map(|n| {
            let bytes = if n.eq_ignore_ascii_case("ECL2.DAX") {
                ecl.clone()
            } else {
                real.raw_file(n).expect("just listed").to_vec()
            };
            (n.to_string(), bytes)
        })
        .collect();
    let data = GameData::from_files(files);

    let saves = load_dir(&root.join("SAVE")).expect("GBX_DATA_DIR/SAVE must be readable");
    let master = saves.raw_file("SAVGAMA.DAT").expect("slot A must exist");
    let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n))
        .expect("slot A must parse");
    let mut engine = crate::import::import_original(&set, data, 1).expect("slot A must import");

    // Kill MATHEW and empty everyone's purse but SHARA's, so the visit has
    // something to do and one clear payer. `kill_player` is the original's own
    // (`ovr024.cs:36-64`), so the record ends exactly as a fight would leave it.
    let shara = engine
        .party()
        .members
        .iter()
        .position(|m| m.name == "SHARA")
        .expect("the bundled party carries SHARA");
    crate::affects::kill_player(&mut engine.party.members[0], crate::rest::status::DEAD);
    engine.state.selected_player = 0; // MATHEW — the patient AND the payer
    engine.party.members[shara].money.gold = 9000;
    let mathew_before = (
        engine.party().members[0].hit_point_max,
        engine.party().members[0].stats.con.current,
    );

    let out_dir = std::env::temp_dir();
    let dump = |engine: &mut Engine, name: &str| {
        let f = engine.tick(&[]);
        let mut fb = Framebuffer::new();
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, f.pixels[y * WIDTH + x]);
            }
        }
        let path = out_dir.join(format!("restrike-{name}.ppm"));
        write_ppm(&fb, &path);
        eprintln!("  {name} -> {}", path.display());
    };

    // Boot until the temple parks.
    let mut opened = false;
    for _ in 0..2000 {
        engine.tick(&[]);
        if engine.shell().temple_host().is_some() {
            opened = true;
            break;
        }
    }
    assert!(opened, "the temple opened: {}", engine.probe());
    dump(&mut engine, "slice6-1-temple-menu");

    let feed = |engine: &mut Engine, key: InputEvent| {
        engine.tick(&[key]);
        for _ in 0..2 {
            engine.tick(&[]);
        }
    };

    // ★ MATHEW is dead and broke, so his own purse cannot pay. `poolMoney`
    // ('P') empties every PC's coins into `gbl.pooled_money`, which is
    // `buy_cure`'s second source (`ovr005.cs:43-46`) — the fallback the whole
    // party-purse model exists for.
    feed(&mut engine, InputEvent::Char(b'P'));
    feed(&mut engine, InputEvent::Char(b'H')); // Heal
    dump(&mut engine, "slice6-2-heal-services");

    // Walk to Raise Dead (row 7) with the original's own in-page step key.
    for _ in 0..7 {
        feed(&mut engine, InputEvent::Ext(ExtKey::End));
    }
    feed(&mut engine, InputEvent::Enter); // commit the row
    dump(&mut engine, "slice6-3-raise-dead-price");
    feed(&mut engine, InputEvent::Enter); // acknowledge the price
    feed(&mut engine, InputEvent::Char(b'Y')); // pay for cure
    dump(&mut engine, "slice6-4-raised");

    let m = &engine.party().members[0];
    eprintln!(
        "  {}: status={} hp={}/{} con={} (was hp_max {} con {})",
        m.name,
        m.status.health_status,
        m.hit_point_current,
        m.hit_point_max,
        m.stats.con.current,
        mathew_before.0,
        mathew_before.1
    );
    eprintln!(
        "  SHARA's purse after pooling: {} gp",
        engine.party().members[shara].money.gold
    );
    assert_eq!(m.status.health_status, crate::rest::status::OKEY);
    assert_eq!(m.hit_point_current, 1);
    assert_eq!(m.stats.con.current, mathew_before.1 - 1, "a point of CON");
    let pooled = engine.state().pooled_money.gold_worth();
    eprintln!("  pool left: {pooled} gp worth");

    // Leave, and prove the script resumed on the instruction after COMBAT.
    feed(&mut engine, InputEvent::Char(b'E')); // out of the Heal list
    feed(&mut engine, InputEvent::Char(b'E')); // out of the temple
                                               // ★ There is still money in the pool, so the priest stops them on the way
                                               // out (`ovr005.cs:449-467`). No, keep the change.
    dump(&mut engine, "slice6-5-the-priest-notices-the-money");
    feed(&mut engine, InputEvent::Char(b'N'));
    let mut resumed = false;
    for _ in 0..2000 {
        engine.tick(&[]);
        if engine.take_transcript().iter().any(|e| {
            matches!(e, crate::vmhost::TranscriptEntry::Print { text, .. }
                if text.contains("THE TEMPLE DOOR CLOSES"))
        }) {
            resumed = true;
            break;
        }
    }
    dump(&mut engine, "slice6-6-back-outside");
    assert!(resumed, "the script resumed after the temple closed");
}

/// ★ **Roll-credits slice 7's acceptance drive** (`roll-credits.md` §11): the
/// overland, live, from the edge of HAP to a fight with black dragons on a
/// wilderness floor.
///
/// Every beat is the shipped script's — the drive presses letters and Enter
/// and nothing else. `ECL5#48`'s overland exit hands the party to `ECL1#80`,
/// whose entry vector puts BIGPIC1 block `0x79` (the Dalelands) on screen and
/// falls into its own travel menu.
///
/// Dumps:
/// 1. the edge menu over the map, with the `MapCursor` DARK (the first 300 ms
///    of `displayInput`'s blink);
/// 2. the same frame with the cursor LIT at HAP — cell `(0x1F, 0x0F)`;
/// 3. the JOURNEY destination list (a VERTICAL MENU, which by the original's
///    own asymmetry does NOT blink the cursor);
/// 4. the travel-mode prompt;
/// 5. the encounter's text as it pages;
/// 6. the fight itself, on `SetupWildernessFloor`'s terrain and WILDCOM's art.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine --release \
///   -- --nocapture --ignored slice7_the_overland_and_a_wilderness_fight`
#[test]
#[ignore = "local-only demo (writes frames); run explicitly"]
fn slice7_the_overland_and_a_wilderness_fight() {
    use crate::engine::Engine;

    let Some(mut engine) = crate::area_transition_tests::real_data_engine(5, 48, 50, true) else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (slice7 overland)");
        return;
    };
    let out_dir = std::env::temp_dir();
    let dump = |engine: &mut Engine, name: &str| {
        let f = engine.tick(&[]);
        let path = out_dir.join(format!("restrike-{name}.ppm"));
        write_ppm_pixels(f.pixels, &path);
        eprintln!("  {name} -> {}", path.display());
    };
    let line = |engine: &Engine| {
        engine
            .shell()
            .parked_widget_for_tests()
            .and_then(|w| w.display_line())
            .unwrap_or_default()
    };

    engine.shell = crate::shell::boot_at_address(&mut engine.machine, 0x8086);
    let mut reached = false;
    for _ in 0..4000 {
        engine.tick(&[]);
        if line(&engine).to_ascii_uppercase().contains("JOURNEY ON") {
            reached = true;
            break;
        }
    }
    assert!(reached, "the edge menu never parked: {:?}", engine.probe());
    eprintln!(
        "  at the edge of city {} — menu: {:?}",
        engine.vm_memory().current_city(),
        line(&engine)
    );

    // (1) The blink's dark lead-in, then (2) the cursor lit. 300 ms is 18
    // ticks; the `+ 6` lands well inside the 500 ms lit window.
    dump(&mut engine, "slice7-1-the-dalelands-cursor-dark");
    for _ in 0..(18 + 6) {
        engine.tick(&[]);
    }
    dump(&mut engine, "slice7-2-the-dalelands-cursor-lit");
    let (cx, cy) = crate::mapcursor::position(engine.vm_memory().current_city()).unwrap();
    eprintln!("  cursor cell: ({cx:#04X}, {cy:#04X})");

    // (3) JOURNEY ON -> the destination list.
    engine.tick(&[InputEvent::Char(b'J')]);
    for _ in 0..200 {
        engine.tick(&[]);
        if engine.shell().gate_open() {
            break;
        }
    }
    dump(&mut engine, "slice7-3-the-destination-list");

    // (4) Commit the destination, then the travel-mode prompt.
    engine.tick(&[InputEvent::Enter]);
    for _ in 0..200 {
        engine.tick(&[]);
        if line(&engine).to_ascii_uppercase().contains("WILDERNESS") {
            break;
        }
    }
    eprintln!("  travel modes: {:?}", line(&engine));
    dump(&mut engine, "slice7-4-how-will-you-get-there");

    // (5)-(6) Answer everything until the fight is on screen.
    let mut in_fight = false;
    let mut dumped_text = false;
    for _ in 0..20_000 {
        engine.tick(&[InputEvent::Enter]);
        if !dumped_text && engine.shell().gate_open() && engine.shell().combat_host().is_none() {
            dumped_text = true;
            dump(&mut engine, "slice7-5-the-encounter");
        }
        if engine.shell().combat_host().is_some() {
            in_fight = true;
            break;
        }
    }
    assert!(
        in_fight,
        "the journey never reached its fight: {:?}",
        engine.probe()
    );
    for _ in 0..30 {
        engine.tick(&[]);
    }
    dump(&mut engine, "slice7-6-a-wilderness-fight");
    // The bundled save ships `quick_fight = 0`, so the first PC's turn parks on
    // the manual combat menu. `Q` (Quick) hands the fight to the AI, which is
    // what makes the later frames show anything moving.
    for _ in 0..400 {
        engine.tick(&[InputEvent::Char(b'Q')]);
    }
    dump(&mut engine, "slice7-7-the-fight-runs");
    for _ in 0..1500 {
        engine.tick(&[InputEvent::Char(b'Q')]);
    }
    // ★ And out the other side: the journey finishes, `ECL1#80`'s vector 1
    // runs again, and the edge menu comes back — at ESSEMBRA now, with the
    // cursor on ITS cell. Getting here needed `LoadPic`'s `WildernessMap` arm
    // (`ovr025.cs:1443-1448`): the combat restore used to paint the dungeon
    // frame over the map and never re-arm `can_draw_bigpic`, so the overland
    // came back as an empty box.
    dump(&mut engine, "slice7-8-back-on-the-map-at-essembra");

    assert_eq!(
        engine.state().game_state,
        crate::shell::GameState::WildernessMap
    );
    assert!(!engine.vm_memory().in_dungeon());
    eprintln!("  halts: {:?}", engine.vm_memory().halts);
}

/// ★ **Roll-credits slice 8's acceptance drive** (`roll-credits.md` §12): the
/// Items screen and a Use beat, on the bundled slot-A party.
///
/// ★ **The slot-A party carries nothing.** `~/goldbox-data/cotab/SAVE/` has
/// `CHRDATA1..6.SAV` and **no `CHRDATA*.SWG` at all** — the GOG-bundled save's
/// six characters have empty inventories, so there is no shipped item on them
/// to Ready or drink. The drive therefore hands them real records the same way
/// the game would: `ITEM2.DAX` block 2 is the authored treasure block whose ten
/// records include the Potion of Extra Healing, two MU scrolls and a Banded
/// Mail +1, and `CMD_Treasure`'s table arm (`ovr003.cs:1083-1099`) is exactly
/// "every `Item.StructSize` record in that block". Nothing is synthesized; the
/// bytes are the user's own game data.
///
/// Dumps:
/// 1. the character sheet with `Items` on its bar;
/// 2. the Items list — generated names, the Yes/No readied column, and the verb
///    bar with the highlight on `Use` (`sl_select_item`'s `menuSelectedWord = 1`);
/// 3. the same list after Ready, with the sheet's numbers moved;
/// 4. the wand's `"is a combat-only item..."` / `"Use it? "` beat;
/// 5. the potion's heal.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine --release \
///   -- --nocapture --ignored slice8_the_items_screen_and_a_use_beat`
#[test]
#[ignore = "local-only demo (writes frames); run explicitly"]
fn slice8_the_items_screen_and_a_use_beat() {
    use crate::engine::Engine;
    use crate::screens::Screen;
    use crate::shell::Shell;

    let Some(root) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: needs GBX_DATA_DIR (slice8 items)");
        return;
    };
    let root = std::path::Path::new(&root);
    let data = load_dir(root).expect("GBX_DATA_DIR must be readable");
    let saves = load_dir(&root.join("SAVE")).expect("GBX_DATA_DIR/SAVE must be readable");
    let master = saves.raw_file("SAVGAMA.DAT").expect("slot A must exist");
    let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n))
        .expect("slot A must parse");

    // The census the drive is built on: what does the bundled party carry?
    let carried: usize = set.chars.iter().map(|c| c.items.len()).sum();
    eprintln!(
        "  slot-A party: {} members, {carried} items",
        set.chars.len()
    );

    // `ITEM2.DAX` block 2 — the authored treasure block (`CMD_Treasure`'s own
    // table arm). Ten records; the drive uses three of them.
    let block = data.block("ITEM2.DAX", 2).expect("ITEM2.DAX block 2");
    let treasure: Vec<Vec<u8>> = block
        .chunks_exact(gbx_formats::save_orig::ITEM_RECORD_SIZE)
        .map(<[u8]>::to_vec)
        .collect();
    eprintln!("  ITEM2.DAX#2 holds {} authored records:", treasure.len());
    for r in &treasure {
        eprintln!(
            "    {:34} type={:3} aff1={:3} spell={:#04x}",
            crate::items::display_name(r, false, false),
            gbx_formats::save_orig::item_type(r),
            gbx_formats::save_orig::item_affect(r, 1),
            gbx_formats::save_orig::item_affect(r, 2),
        );
    }

    // ★ The Wand of **Fireballs** (`ITEM5.DAX` block 49), read before the data
    // set moves into the engine. Four of the six shipped wands carry spell ids
    // that are still tripwired (§12.4), and a tripwired id is refused by
    // `spell_entry` *before* the combat-only branch can fire — so the beat that
    // shows the charge-burn has to be one of the two whose row exists. Fireball
    // (`0x2F`) is §9.1's; Lightning Bolt (`0x33`) is not.
    let wand = data
        .block("ITEM5.DAX", 49)
        .expect("ITEM5.DAX block 49")
        .chunks_exact(gbx_formats::save_orig::ITEM_RECORD_SIZE)
        .find(|r| {
            matches!(gbx_formats::save_orig::item_type(r), 78 | 79)
                && gbx_formats::save_orig::item_affect(r, 2) & 0x7F == 0x2F
        })
        .expect("the Wand of Fireballs")
        .to_vec();

    let mut engine = crate::import::import_original(&set, data, 1).expect("slot A must import");
    let out_dir = std::env::temp_dir();
    let dump = |engine: &mut Engine, name: &str| {
        let f = engine.tick(&[]);
        let path = out_dir.join(format!("restrike-{name}.ppm"));
        write_ppm_pixels(f.pixels, &path);
        eprintln!("  {name} -> {}", path.display());
    };

    for _ in 0..300 {
        engine.tick(&[]);
        if matches!(engine.shell(), Shell::WorldMenu { .. }) {
            break;
        }
        if engine.shell().gate_open() {
            engine.tick(&[InputEvent::Enter]);
        }
    }
    assert!(matches!(engine.shell(), Shell::WorldMenu { .. }));

    // Hand the first member the Banded Mail +1, the Potion of Extra Healing and
    // one of the block's MU scrolls, plus a wand from `ITEM3.DAX` block 17 (the
    // Wand of Lightning) for the combat-only beat.
    let find = |pred: &dyn Fn(&[u8]) -> bool| {
        treasure
            .iter()
            .find(|r| pred(r))
            .cloned()
            .expect("the block carries it")
    };
    let mail = find(&|r| gbx_formats::save_orig::item_type(r) == 57);
    let potion = find(&|r| {
        gbx_formats::save_orig::item_type(r) == 71
            && gbx_formats::save_orig::item_affect(r, 2) == 0x63
    });
    let scroll = find(&|r| gbx_formats::save_orig::item_type(r) == 61);
    engine.party.members[0].items = vec![mail, potion, scroll, wand];
    engine.party.members[0].hit_point_current = 10;
    engine.state.selected_player = 0;
    let name = engine.party().members[0].name.clone();
    eprintln!("  {name} now carries:");
    for r in &engine.party().members[0].items {
        eprintln!("    {}", crate::items::display_name(r, false, true));
    }

    engine.open_party_view();
    dump(&mut engine, "slice8-1-character-sheet");
    let bar = crate::charsheet::sheet_view(&engine.party().members[0]).command_bar;
    eprintln!("  sheet bar: {bar:?}");
    assert!(bar.starts_with("Items"), "the sheet offers Items");

    engine.tick(&[InputEvent::Char(b'I')]);
    dump(&mut engine, "slice8-2-items-list");
    assert!(matches!(engine.shell(), Shell::Screen(Screen::Items(_))));

    // (1) **Ready** the mail (row 0) and watch AC move.
    let ac_before = engine.party().members[0].combat.ac;
    engine.tick(&[InputEvent::Char(b'R')]);
    dump(&mut engine, "slice8-3-readied");
    let ac_after = engine.party().members[0].combat.ac;
    eprintln!(
        "  AC (stored) {ac_before} -> {ac_after}  (display {} -> {})",
        0x3C - ac_before as i32,
        0x3C - ac_after as i32
    );

    // (2) **The wand** — row 3, readied, then Use: a combat-only item out of
    // combat, which offers to burn a charge for nothing.
    for _ in 0..3 {
        engine.tick(&[InputEvent::Ext(ExtKey::End)]);
        engine.tick(&[]);
    }
    engine.tick(&[InputEvent::Char(b'R')]);
    engine.tick(&[]);
    engine.tick(&[InputEvent::Char(b'U')]);
    dump(&mut engine, "slice8-4-combat-only-item");
    let charges_before =
        gbx_formats::save_orig::item_affect(&engine.party().members[0].items[3], 1);
    engine.tick(&[InputEvent::Char(b'Y')]);
    engine.tick(&[]);
    let charges_after = gbx_formats::save_orig::item_affect(&engine.party().members[0].items[3], 1);
    eprintln!("  wand charges {charges_before} -> {charges_after} (spent for nothing)");
    assert_eq!(charges_after + 1, charges_before);

    // (3) **The potion** — row 1: ready it, drink it, and watch the record heal.
    for _ in 0..2 {
        engine.tick(&[InputEvent::Ext(ExtKey::Home)]);
        engine.tick(&[]);
    }
    engine.tick(&[InputEvent::Char(b'R')]);
    engine.tick(&[]);
    let hp_before = engine.party().members[0].hit_point_current;
    engine.tick(&[InputEvent::Char(b'U')]);
    dump(&mut engine, "slice8-5-potion-drunk");
    let hp_after = engine.party().members[0].hit_point_current;
    eprintln!("  {name} hp {hp_before} -> {hp_after} (Potion of Extra Healing, 2d4+2)");
    assert!(hp_after > hp_before, "the potion healed through the record");
}
