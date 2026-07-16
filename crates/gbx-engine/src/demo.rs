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
    // CORRECTION (M2 step 8, post-DIVIDE): this assertion originally
    // expected the bash to land the party at (7,11), true only because the
    // per-step script (vector 1) halted on DIVIDE before this door's real
    // logic ever ran (the FIRST FIELD FINDING that opened step 8 — see
    // `docs/fidelity-docket.md` FD-9). With DIVIDE/OR/ON GOSUB implemented,
    // vector 1 runs to completion and this specific door turns out to be a
    // real *area transition* trigger, not a plain GEO-door state flip: the
    // service log shows `Load3dMap { block_id: 1 }` (a different resident
    // area), zero halts. M2's engine deliberately doesn't wire cross-area
    // GEO-block swapping (`engine.rs`'s doc comment: block *selection* is
    // step 5+/M3+ scope) — position lands at the raw-store default (0,0)
    // rather than a real new-area spawn point, a known consequence of that
    // gap, not a new bug. Docketed as FD-19 rather than silently "fixed" by
    // rewriting the assertion to whatever the engine happens to do.
    assert_eq!(
        engine.state.pos,
        (0, 0),
        "bashing this door triggers a real (unimplemented) area transition, not a simple move"
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

// --- M4 combat #2 deliverable 6: the watchable fight demo ---

/// A synthetic combatant for the fight demo. Carries the HP / AC / weapon the
/// initiative-slice `Combatant` doesn't (initiative reads only `in_combat` +
/// reaction adjustment); the demo owns the full record so it can resolve real
/// attacks and track HP.
///
/// **All stats are hand-built, D10-clean — deliberately NOT real CotAB numbers**
/// (a "goblin" here is invented, not a decoded `MON2CHA` record). `ac`/`hit_bonus`
/// use the original's raw encoding: display AC = `0x3C - ac`, and `hit_bonus` is a
/// THAC0-derived to-hit number (higher = better), so a hit needs
/// `d20 + hit_bonus >= raw_ac` (the `>=` weapon path).
#[cfg(test)]
struct DemoFighter {
    id: usize,
    name: &'static str,
    team: crate::combat::Team,
    hp: i32,
    max_hp: i32,
    ac: u8,
    hit_bonus: i32,
    dice_size: u8,
    dice_count: u8,
    damage_bonus: u8,
    reaction_adj: i8,
}

#[cfg(test)]
impl DemoFighter {
    fn alive(&self) -> bool {
        self.hp > 0
    }
    /// Display AC (`0x3C - ac`), for the transcript.
    fn display_ac(&self) -> i32 {
        0x3C - self.ac as i32
    }
}

/// The watchable fight demo (local-tier, `GBX_DATA_DIR`-gated like the other
/// demos so it stays out of CI — it uses **no game data**, only synthetic
/// D10-clean stats; the gate is purely to place it in the local run tier).
///
/// A synthetic party-vs-goblins encounter run to completion through the **real**
/// combat subsystem: `CombatState` drives the round loop + initiative + selection
/// draw order (the slice-1 subsystem), and `resolve_attack` resolves each swing
/// (the slice-2 to-hit + damage). The transcript prints the round, each
/// combatant's initiative `delay`, every attack (who, the raw d20, hit/miss, the
/// damage), HP dropping, deaths, and the victor.
///
/// **The target picker is an explicit demo-only placeholder** — "first living
/// enemy", consuming ZERO draws. This is NOT faithful AI: real target selection
/// (`find_target`, the next slice) consumes draws and would change the stream, so
/// this demo is a *demonstration, not a parity artifact*. The engine's real turn
/// slot stays a zero-draw stub (`CombatState` yields `Turn` and the demo composes
/// `resolve_attack` + the placeholder picker itself). Because dead combatants stay
/// in `CombatState`'s roster (no death model in the slice), they still get picked;
/// the demo skips a downed actor's turn. Everything shares one `EngineRng` stream
/// (D9), but the picker + dead-member handling make the interleaving non-faithful
/// by construction — hence demo-only.
///
/// Run: `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine \
///   -- --nocapture watch_a_fight`
#[test]
fn watch_a_fight() {
    use crate::combat::{resolve_attack, AttackProfile, CombatState, CombatStep, Combatant, Team};
    use crate::rng::EngineRng;

    if std::env::var_os("GBX_DATA_DIR").is_none() {
        eprintln!("SKIPPED: fight demo runs in the local tier (GBX_DATA_DIR) — watch_a_fight");
        return;
    }

    // A synthetic party of three vs five goblins. Raw AC / hit_bonus per the
    // encoding note on DemoFighter; weapons are plain dice. Ids are the roster
    // index (0..8), party first (TeamList order).
    let mut fighters = vec![
        DemoFighter {
            id: 0,
            name: "Kethra",
            team: Team::Party,
            hp: 26,
            max_hp: 26,
            ac: 54, // display AC 6
            hit_bonus: 45,
            dice_size: 8,
            dice_count: 1,
            damage_bonus: 2, // longsword 1d8+2
            reaction_adj: 2,
        },
        DemoFighter {
            id: 1,
            name: "Dolan",
            team: Team::Party,
            hp: 22,
            max_hp: 22,
            ac: 52, // display AC 8
            hit_bonus: 44,
            dice_size: 10,
            dice_count: 1,
            damage_bonus: 1, // bastard sword 1d10+1
            reaction_adj: 0,
        },
        DemoFighter {
            id: 2,
            name: "Sable",
            team: Team::Party,
            hp: 18,
            max_hp: 18,
            ac: 50, // display AC 10
            hit_bonus: 43,
            dice_size: 6,
            dice_count: 1,
            damage_bonus: 3, // short sword 1d6+3
            reaction_adj: 3,
        },
    ];
    for i in 0..5 {
        fighters.push(DemoFighter {
            id: 3 + i,
            name: ["Snik", "Grub", "Yark", "Mool", "Zeth"][i],
            team: Team::Monster,
            hp: 7,
            max_hp: 7,
            ac: 48, // display AC 12
            hit_bonus: 41,
            dice_size: 6,
            dice_count: 1,
            damage_bonus: 0, // spear 1d6
            reaction_adj: 0,
        });
    }

    let seed = 0x0C0F_FEE0u32;
    let mut rng = EngineRng::new(seed);

    let roster: Vec<Combatant> = fighters
        .iter()
        .map(|f| Combatant::new(f.id, f.team, f.reaction_adj, true))
        .collect();
    let mut state = CombatState::new(roster);

    eprintln!("== A FIGHT ==  (seed {seed:#010x}; synthetic, D10-clean)");
    eprintln!("Party:");
    for f in fighters.iter().filter(|f| f.team == Team::Party) {
        eprintln!(
            "  {:<7} AC {:>2}  HP {:>2}  THAC0-hit +{}  {}d{}+{}",
            f.name,
            f.display_ac(),
            f.hp,
            f.hit_bonus,
            f.dice_count,
            f.dice_size,
            f.damage_bonus
        );
    }
    eprintln!("Goblins:");
    for f in fighters.iter().filter(|f| f.team == Team::Monster) {
        eprintln!("  {:<7} AC {:>2}  HP {:>2}", f.name, f.display_ac(), f.hp);
    }

    /// The DEMO-ONLY target picker — first living enemy of the opposite team,
    /// zero draws. NOT faithful AI (see the fn doc).
    fn first_living_enemy(fighters: &[DemoFighter], team: Team) -> Option<usize> {
        fighters.iter().position(|f| f.team != team && f.alive())
    }

    fn side_alive(fighters: &[DemoFighter], team: Team) -> bool {
        fighters.iter().any(|f| f.team == team && f.alive())
    }

    let victor = loop {
        match state.step(&mut rng) {
            CombatStep::RoundStarted { round } => {
                eprintln!("\n── Round {} ──", round + 1);
                // Initiative values this round (the slice-1 d6 + reaction, clamped).
                let inits: Vec<String> = state
                    .roster()
                    .iter()
                    .filter(|c| fighters[c.id].alive())
                    .map(|c| format!("{}={}", fighters[c.id].name, c.delay))
                    .collect();
                eprintln!("   initiative: {}", inits.join("  "));
            }
            CombatStep::Turn { combatant_id } => {
                let attacker_idx = combatant_id; // id == roster index
                if !fighters[attacker_idx].alive() {
                    continue; // a downed combatant still gets picked; skip its turn
                }
                let team = fighters[attacker_idx].team;
                let Some(target_idx) = first_living_enemy(&fighters, team) else {
                    continue; // no enemies left; the round-end check ends it
                };

                let (name, prof) = {
                    let a = &fighters[attacker_idx];
                    (
                        a.name,
                        AttackProfile {
                            attacker_id: a.id,
                            target_id: fighters[target_idx].id,
                            target_ac: fighters[target_idx].ac,
                            hit_bonus: a.hit_bonus,
                            team_bonus: 0,
                            dice_size: a.dice_size,
                            dice_count: a.dice_count,
                            damage_bonus: a.damage_bonus,
                            backstab: None,
                        },
                    )
                };
                let target_name = fighters[target_idx].name;
                let hp_before = fighters[target_idx].hp;

                let out = resolve_attack(&mut rng, prof, None);

                if let Some(dmg) = out.damage {
                    fighters[target_idx].hp -= dmg.amount;
                    let hp_after = fighters[target_idx].hp.max(0);
                    let dead = fighters[target_idx].hp <= 0;
                    eprintln!(
                        "   {name} → {target_name}: d20 {:>2}  HIT for {:>2}  ({target_name} {hp_before}→{hp_after}){}",
                        out.to_hit.d20,
                        dmg.amount,
                        if dead { "  ✝ DOWN" } else { "" }
                    );
                } else {
                    eprintln!("   {name} → {target_name}: d20 {:>2}  miss", out.to_hit.d20);
                }

                if !side_alive(&fighters, Team::Party) {
                    break Team::Monster;
                }
                if !side_alive(&fighters, Team::Monster) {
                    break Team::Party;
                }
            }
            CombatStep::RoundEnded { battle_over, .. } => {
                let alive: Vec<&str> = fighters
                    .iter()
                    .filter(|f| f.team == Team::Party && f.alive())
                    .map(|f| f.name)
                    .collect();
                let foes = fighters
                    .iter()
                    .filter(|f| f.team == Team::Monster && f.alive())
                    .count();
                eprintln!(
                    "   end of round: party [{}], {foes} goblin(s) left",
                    alive.join(", ")
                );
                if battle_over {
                    // The stalemate cap (15 rounds) — neither side finished.
                    break if side_alive(&fighters, Team::Party) {
                        Team::Party
                    } else {
                        Team::Monster
                    };
                }
            }
            CombatStep::Ended => {
                break if side_alive(&fighters, Team::Party) {
                    Team::Party
                } else {
                    Team::Monster
                };
            }
        }
    };

    eprintln!(
        "\n== VICTOR: {} ==",
        match victor {
            Team::Party => "the party",
            Team::Monster => "the goblins",
        }
    );
    for f in fighters.iter().filter(|f| f.team == Team::Party) {
        eprintln!(
            "  {:<7} {}",
            f.name,
            if f.alive() {
                format!("HP {}/{}", f.hp, f.max_hp)
            } else {
                "DOWN".to_string()
            }
        );
    }

    // The demo is a demonstration, not a parity artifact — the only invariant it
    // asserts is that a fight runs to a decision.
    assert!(
        !side_alive(&fighters, Team::Party) || !side_alive(&fighters, Team::Monster),
        "the fight resolved to one side standing"
    );
}
