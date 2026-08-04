//! **The H4 melee-closure milestone** (D-OR5(b)): replay a *live* combat
//! entry-state capture through our engine and assert our PRNG draw stream equals
//! the original's, **draw-for-draw**.
//!
//! This is the measurement, not a combat-mechanic fix. It seeds `gbx-prng` from
//! the capture's `rng_state`, builds a `CombatState` from the captured roster
//! (order + positions from the snapshot, records decoded by
//! `gbx_engine::combat::combat_state_from_records`), runs the unified tick engine
//! to `Ended` with an `RngSink`, and compares the resulting `(before, after)`
//! draw stream to the capture's `rng` events.
//!
//! - **Full match** ⇒ H4 melee closes (a clear `H4 MELEE CLOSED` line + assert).
//! - **Divergence** ⇒ the **first** divergent draw is printed in full (index,
//!   both sides' `(before, after, operand)`, the draw before it, and the inferred
//!   mechanic), and the test fails with that diagnostic. **We do not fix combat
//!   here** — the divergence is the finding that scopes the next session.
//!
//! **D10:** the capture holds real character/monster record bytes and is
//! **local-only** — never in the repo/CI. The test gates on its presence and
//! loud-skips when absent, like every local-tier test.
//!
//! **Assembly** (M6a, `combat-visualizer.md` §4): the capture → `CombatState`
//! path is the library's — `gbx_oracle::replay::reel_input` →
//! `gbx_engine::combat::reel::build_state` — shared verbatim with the frontier
//! guard and the M6a reel. Knob precedence is unchanged: the `RESTRIKE_*` trial
//! overrides win, then whatever the staging hook emitted, then the capture's
//! committed sidecar row, then the documented defaults (heading 2, 58C 99,
//! 6E4 0). An unpinned capture named through `GBX_H4_CAPTURE` therefore behaves
//! exactly as it did before the sidecar existed.

use std::path::{Path, PathBuf};

use gbx_engine::combat::reel::{self, draws_agree, mechanic_for, ExpectedDraw};
use gbx_engine::combat::Team;
use gbx_engine::combat::DEFAULT_NO_ACTION_LIMIT;
use gbx_engine::rng::{EngineRng, RngDraw, RngSink};
use gbx_oracle::replay;
use gbx_oracle::Trace;
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::pack::RuleSet;
use std::cell::RefCell;
use std::rc::Rc;

/// The canonical local-only capture (D10): the `combat4` bar brawl (16
/// combatants, seed `0x80ee4cee`, 3,075 draws, real terrain + board snapshots).
/// Overridable with `GBX_H4_CAPTURE`; otherwise the `~/goldbox-data/traces/`
/// sibling of `GBX_DATA_DIR`.
const CAPTURE_NAME: &str = "combat4.gbxtrace";

/// Resolve the capture path, or `None` when the **local tier is not active**.
/// The local tier is active when either `GBX_H4_CAPTURE` (explicit override) or
/// `GBX_DATA_DIR` (the project-wide local-data signal the demos gate on) is set —
/// so a plain `cargo test` (the CI gate, neither var set) **skips** this
/// milestone test exactly as it skips the `GBX_DATA_DIR` demos (D10).
fn capture_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("GBX_H4_CAPTURE") {
        return Some(PathBuf::from(p));
    }
    // Only auto-discover the default path when the local tier is explicitly on.
    std::env::var_os("GBX_DATA_DIR")?;
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join("goldbox-data/traces")
            .join(CAPTURE_NAME),
    )
}

/// A draw tap recording every `(before, after, n)` at the engine seam.
#[derive(Clone, Default)]
struct DrawTap {
    draws: Rc<RefCell<Vec<RngDraw>>>,
}
impl RngSink for DrawTap {
    fn on_draw(&mut self, draw: RngDraw) {
        self.draws.borrow_mut().push(draw);
    }
}

#[test]
fn h4_melee_replays_the_bar_brawl_capture_draw_for_draw() {
    let Some(path) = capture_path() else {
        eprintln!("SKIPPED: no HOME/GBX_H4_CAPTURE to locate the H4 capture");
        return;
    };
    if !path.exists() {
        eprintln!(
            "SKIPPED: local-tier H4 capture absent at {} \
             (set GBX_H4_CAPTURE; real record bytes are local-only, D10)",
            path.display()
        );
        return;
    }

    let text = std::fs::read_to_string(&path).expect("H4 capture must be readable");

    // The reader extension (D1) parses the combat_entry snapshot + the rng stream.
    let trace = Trace::parse(&text).expect("H4 capture parses");
    let entry = trace
        .combat_entry()
        .expect("the capture carries a combat_entry snapshot");

    let capture_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    // The whole assembly (roster, terrain, knobs, kits, the ITEMS table) is
    // the library's — the same call the frontier guard and the M6a reel make.
    // `sidecar_or_default` gives an unpinned capture the documented defaults
    // (heading 2, magic off, no schedules), which is exactly what this harness
    // applied before the sidecar table existed; the `RESTRIKE_*` trial
    // overrides then sit on top of everything.
    let sidecar = replay::sidecar_or_default(&capture_name);
    let item_data = replay::load_item_data();
    // A ranged capture cannot replay without the ITEMS table — without this
    // loud skip a missing game file surfaces as a baffling divergence at draw
    // ~58 (the guard already skips the same way).
    if replay::capture_has_loadout(&capture_name) && item_data.is_none() {
        eprintln!(
            "SKIPPED (ITEMS absent, D10): {capture_name} carries a ranged loadout \
             and needs the local game data (~/goldbox-data/cotab/ITEMS or GBX_ITEMS_FILE)"
        );
        return;
    }
    let mut input = replay::reel_input(&capture_name, entry, &sidecar, item_data)
        .unwrap_or_else(|e| panic!("{capture_name}: {e}"));
    // The investigator's knobs (`RESTRIKE_MAP_DIR`, `RESTRIKE_AUTO_CAST`,
    // `RESTRIKE_AUTO_CAST_TOGGLES`, `RESTRIKE_CONTINUE_BATTLE`,
    // `RESTRIKE_AREA_6E4`) — an explicit trial override for a session probing
    // an open frontier. The guard deliberately does NOT read these.
    replay::apply_env_overrides(&mut input.knobs);

    let n_combatants = input.combatants.len();
    let (party, monsters) = input
        .combatants
        .iter()
        .fold((0, 0), |(p, m), e| match e.team {
            Team::Party => (p + 1, m),
            Team::Monster => (p, m + 1),
        });

    // The 6E0/6E2 per-team to-hit pair is parse-only (unmodeled — the gate
    // needs listing verification before engine wiring, the #21 lesson). A
    // capture carrying nonzero values WILL diverge at its first hit test;
    // say so up front instead of surfacing as a baffling d20 fork.
    if entry.area2_field_6e0.unwrap_or(0) != 0 || entry.area2_field_6e2.unwrap_or(0) != 0 {
        eprintln!(
            "WARNING: capture carries a nonzero area2 to-hit pair (6e0={:?}, 6e2={:?}) — \
             not modeled; expect hit-test divergence (doc §46)",
            entry.area2_field_6e0, entry.area2_field_6e2
        );
    }

    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    let mut state = reel::build_state(&input, &flavor).expect("records decode");

    // Seed gbx-prng with the snapshot's rng_state and tap every draw.
    let tap = DrawTap::default();
    let draws = tap.draws.clone();
    let mut rng = EngineRng::new(input.rng_state);
    rng.attach_sink(Box::new(tap.clone()));

    // Stub tripwires (doc §24): collect every `StubTripped` with the draw index
    // it fired at, so a capture that reaches unmodeled territory (downed PC,
    // memorized spells, 0-HD sweep, the surrender branch) NAMES itself — before
    // any divergence diagnostic, and even when the stream still matches.
    /// One trip: `(draw index when it fired, combatant, stub name)`.
    type Trip = (usize, usize, &'static str);
    struct StubTap {
        draws: Rc<RefCell<Vec<RngDraw>>>,
        trips: Rc<RefCell<Vec<Trip>>>,
    }
    impl gbx_engine::combat::ActionSink for StubTap {
        fn on_action(&mut self, e: gbx_engine::combat::ActionEvent) {
            if let gbx_engine::combat::ActionEvent::StubTripped { combatant_id, stub } = e {
                self.trips
                    .borrow_mut()
                    .push((self.draws.borrow().len(), combatant_id, stub));
            }
        }
    }
    let trips: Rc<RefCell<Vec<Trip>>> = Rc::new(RefCell::new(Vec::new()));
    state.attach_action_sink(Box::new(StubTap {
        draws: tap.draws.clone(),
        trips: trips.clone(),
    }));

    // Record the per-round survivor trajectory (draw-free observation) so a
    // length divergence names the round our fight ended vs the capture's.
    // `run_scripted` (§9.6) answers D-CV5's suspensions from the sidecar's
    // manual-turn schedule; with an empty schedule it is `run_combat_observed`
    // line for line.
    let mut rounds: Vec<(u16, usize, usize)> = Vec::new();
    let outcome = reel::run_scripted(
        &mut state,
        &mut rng,
        DEFAULT_NO_ACTION_LIMIT,
        &input.manual_script,
        |s, r| {
            let (p, m) = s.roster().iter().filter(|f| f.in_combat).fold(
                (0usize, 0usize),
                |(p, m), f| match f.team {
                    Team::Party => (p + 1, m),
                    Team::Monster => (p, m + 1),
                },
            );
            rounds.push((r, p, m));
        },
    );

    // The two draw streams.
    let ours = draws.borrow();
    let capture: Vec<ExpectedDraw> = replay::capture_draws(&text);

    eprintln!(
        "H4 replay: {n_combatants} combatants ({party} party, {monsters} monster), \
         seed {:#010x}; our fight = {} draws ({:?}), capture = {} draws",
        input.rng_state,
        ours.len(),
        outcome,
        capture.len()
    );
    eprintln!(
        "  our per-round survivors (round: party/monsters at round end): {:?}",
        rounds
    );
    if !trips.borrow().is_empty() {
        eprintln!("\n  ⚠ STUBBED MECHANICS REACHED (unproven territory from the first trip on):");
        for (draw, id, stub) in trips.borrow().iter() {
            eprintln!("    draw ~#{draw}: combatant {id} tripped `{stub}`");
        }
    }

    // Draw-for-draw comparison over the equality surface. `(before, after)`
    // alone is only draw-COUNT equality for a pure LCG (the §14/§28 lesson: the
    // chain advances identically whatever die is asked for), so the surface is
    // ALSO the **operand**: when both sides carry one (`n` vs `ss_sp_words[3]`),
    // a mismatch is a divergence — the same stricter metric the localizer uses.
    let max = ours.len().max(capture.len());
    for i in 0..max {
        match (ours.get(i), capture.get(i)) {
            (Some(o), Some(c)) => {
                // `draws_agree` is the shared surface: `(before, after)` always,
                // plus the operand when BOTH sides carry one (one side lacking a
                // recorded operand falls back to the chain for that draw).
                if draws_agree(o, c) {
                    continue;
                }
                // First divergence — print it in full and stop.
                eprintln!("\n=== H4 REPLAY DIVERGENCE at draw #{i} ===");
                if i > 0 {
                    let po = &ours[i - 1];
                    let pc = &capture[i - 1];
                    eprintln!(
                        "  draw #{} (context, matched): ours ({:#010x}->{:#010x}, n={:?}) | \
                         capture ({:#010x}->{:#010x}, op={:?})",
                        i - 1,
                        po.before,
                        po.after,
                        po.n,
                        pc.before,
                        pc.after,
                        pc.operand
                    );
                }
                eprintln!(
                    "  ours   : before={:#010x} after={:#010x} n={:?}",
                    o.before, o.after, o.n
                );
                eprintln!(
                    "  capture: before={:#010x} after={:#010x} op={:?}",
                    c.before, c.after, c.operand
                );
                let which = if o.before != c.before {
                    "before"
                } else if o.after != c.after {
                    "after"
                } else {
                    "operand"
                };
                eprintln!(
                    "  field `{which}` differs; inferred mechanic (ours): {} | (capture): {}",
                    mechanic_for(o.n),
                    mechanic_for(c.operand)
                );
                eprintln!("  {}/{} draws matched before divergence.", i, max);
                panic!(
                    "H4 replay diverged at draw #{i} on `{which}`: \
                     ours ({:#010x}->{:#010x}, n={:?}) vs capture ({:#010x}->{:#010x}, op={:?}); \
                     inferred mechanic {} — this scopes the next fix session (do NOT fix combat in the harness).",
                    o.before, o.after, o.n, c.before, c.after, c.operand, mechanic_for(c.operand)
                );
            }
            (Some(o), None) => {
                panic!(
                    "H4 replay diverged at draw #{i} on `length`: our fight drew MORE \
                     ({} draws) than the capture ({}). First extra draw: ({:#010x}->{:#010x}, n={:?}), \
                     mechanic {}. {} draws matched.",
                    ours.len(),
                    capture.len(),
                    o.before,
                    o.after,
                    o.n,
                    mechanic_for(o.n),
                    capture.len()
                );
            }
            (None, Some(c)) => {
                panic!(
                    "H4 replay diverged at draw #{i} on `length`: our fight ENDED EARLY \
                     ({} draws) vs the capture ({}). First missing capture draw: ({:#010x}->{:#010x}, op={:?}), \
                     mechanic {}. {} draws matched.",
                    ours.len(),
                    capture.len(),
                    c.before,
                    c.after,
                    c.operand,
                    mechanic_for(c.operand),
                    ours.len()
                );
            }
            (None, None) => unreachable!("i < max(len)"),
        }
    }

    // Every draw matched and the lengths are equal — H4 melee closes.
    if trips.borrow().is_empty() {
        eprintln!(
            "\nH4 MELEE CLOSED: {} draws matched draw-for-draw against the live bar-brawl capture.",
            ours.len()
        );
    } else {
        eprintln!(
            "\nH4 replay MATCHED {} draws draw-for-draw — but stubbed mechanics were reached \
             (see the trip list above): the stream is proven, the mechanics behind those trips \
             are not.",
            ours.len()
        );
    }
    assert_eq!(
        ours.len(),
        capture.len(),
        "full draw-stream equality (checked above; this pins the count)"
    );
}
