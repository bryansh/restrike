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

use std::path::{Path, PathBuf};

use gbx_engine::combat::DEFAULT_NO_ACTION_LIMIT;
use gbx_engine::combat::{combat_state_from_records, CombatMap, RecordCombatant, Team};
use gbx_engine::rng::{EngineRng, RngDraw, RngSink};
use gbx_oracle::Trace;
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::pack::RuleSet;
use std::cell::RefCell;
use std::rc::Rc;

mod common;

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

/// Open-floor fallback tile (`0x17` = passable floor, move_cost 1) — used only
/// when the capture predates the `combat_entry.terrain` field. Terrain is
/// load-bearing for movement (doc §14), so a modern capture's replay always
/// builds its map from the captured ground grid.
const FLOOR: u8 = 0x17;

/// Decode the `combat_entry.terrain` lowercase-hex ground grid.
fn decode_terrain(hex: &str) -> Vec<u8> {
    let b = hex.as_bytes();
    let val = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    };
    b.chunks_exact(2)
        .map(|p| (val(p[0]) << 4) | val(p[1]))
        .collect()
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

/// The capture's combat draws: `(before, after, operand)` per `rng` event that
/// appears **after** the `combat_entry` snapshot, in file order. `operand` is
/// `ss_sp_words[3]` (the draw's `Random(n)` argument, diagnostic only) — an
/// unknown field to the typed reader, so it is pulled from the raw JSON here.
struct CaptureDraw {
    before: u32,
    after: u32,
    operand: Option<u16>,
}

/// Pull the capture's post-`combat_entry` draws straight from the raw JSONL, so
/// the diagnostic operand (`ss_sp_words[3]`) is available alongside
/// `(before, after)`. Ordering matches the typed reader's event order.
fn capture_combat_draws(text: &str) -> Vec<CaptureDraw> {
    let mut out = Vec::new();
    let mut seen_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("e").and_then(|e| e.as_str()) {
            Some("combat_entry") => seen_entry = true,
            Some("rng") if seen_entry => {
                let before = v["before"].as_u64().unwrap() as u32;
                let after = v["after"].as_u64().unwrap() as u32;
                let operand = v
                    .get("ss_sp_words")
                    .and_then(|w| w.as_array())
                    .and_then(|w| w.get(3))
                    .and_then(|n| n.as_u64())
                    .map(|n| n as u16);
                out.push(CaptureDraw {
                    before,
                    after,
                    operand,
                });
            }
            _ => {}
        }
    }
    out
}

/// Infer which combat mechanic drew, from the `Random(n)` operand — the honest
/// die tells the mechanic (§2/§4/§9 draw map).
fn mechanic_for(operand: Option<u16>) -> &'static str {
    match operand {
        Some(6) => "initiative d6 (CalculateInitiative)",
        Some(100) => "d100 (FindNextCombatant selection, or FleeCheck/advance morale)",
        Some(20) => "d20 (to-hit PC_CanHitTarget, or a saving throw)",
        Some(7) => "d7 (QuickFight AI mode-gate / wand-scan / spell-priority)",
        Some(n) => match n {
            0 => "random(0) edge draw",
            _ => "damage die (weapon/monster attack dice)",
        },
        None => "unknown (operand not recorded)",
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

    // Build the replay roster in the captured order, at the captured positions.
    let entries: Vec<RecordCombatant> = entry
        .combatants
        .iter()
        .map(|c| RecordCombatant {
            team: match c.team {
                0 => Team::Party,
                1 => Team::Monster,
                other => panic!("combat_entry has an unknown team byte {other}"),
            },
            pos: gbx_engine::combat::GridPos::new(c.x as i32, c.y as i32),
            record: &c.record,
        })
        .collect();

    let n_combatants = entries.len();
    let (party, monsters) = entries.iter().fold((0, 0), |(p, m), e| match e.team {
        Team::Party => (p + 1, m),
        Team::Monster => (p, m + 1),
    });

    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    // Real terrain when the capture carries it (§14: load-bearing), else the
    // documented open-floor fallback for pre-terrain captures.
    let map = match entry.terrain.as_deref() {
        Some(hex) => CombatMap::from_ground(decode_terrain(hex)),
        None => CombatMap::uniform(FLOOR),
    };
    let mut state = combat_state_from_records(&entries, map, &flavor).expect("records decode");
    // `area2.field_58C` — the faithful FleeCheck_001 gate-2 morale threshold
    // (doc §28). Captures that carry it (the rout capture pokes 50) use it;
    // legacy captures without the field default to 99 (the measured bar value,
    // under which the natural rout is mathematically impossible → the four
    // closed captures stay closed).
    state.area_field_58c = entry.area2_field_58c.map(|v| v as i32).unwrap_or(99);
    // `gbl.mapDirection` (the party's world facing, half-encoded {0 N, 2 E, 4 S,
    // 6 W} per coab `Gbl.cs:354`, `byte_1D53B`) — the flee HEADING input
    // (`moralFailureEscape`, `sub_359D1` @`ovr010:0B14`). Precedence:
    // `RESTRIKE_MAP_DIR` (explicit trial override) > the capture's emitted
    // `map_direction` (hooks from 8ab275e on; the armed/caster staging captures
    // carry it, closing the §29 TODO) > the provisional geometry-matched
    // default 2 (E — the heading whose bar-rout positions match, §29/§30, now
    // capture-CONFIRMED by armed-bar's emitted md=2 from the same room).
    state.map_direction = std::env::var("RESTRIKE_MAP_DIR")
        .ok()
        .and_then(|s| s.parse::<u8>().ok())
        .or(entry.map_direction)
        .unwrap_or(2);
    // `gbl.AutoPCsCastMagic` (`byte_1D904`) — `BattleSetup` resets it false
    // (`ovr011.cs:1186`); the '2' key toggles it mid-fight. Not in the capture
    // snapshot (staging-hook TODO), so `RESTRIKE_AUTO_CAST=1` arms it for fights
    // where the player did (caster-bar: pressed in round 1 BEFORE the first
    // caster turn — "on from entry" is draw-equivalent for that capture, doc
    // §33). Default false = the faithful entry state.
    state.auto_pcs_cast_magic = std::env::var("RESTRIKE_AUTO_CAST")
        .map(|v| v == "1")
        .unwrap_or(false);
    // Mid-fight presses (doc §38): `RESTRIKE_AUTO_CAST_TOGGLES=16` (comma
    // list of 0-based global turn ordinals) flips the flag at each listed
    // turn's head — the flip-window model for captures where the '2' press
    // landed mid-fight (caster-bar: between PHILIPPE's round-1 and round-2
    // turns, so ordinal 16 = his round-2 turn head).
    if let Ok(v) = std::env::var("RESTRIKE_AUTO_CAST_TOGGLES") {
        state.auto_cast_toggles = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    }
    // §34.1: the ITEMS table + per-capture ranged loadouts (one shared place,
    // `common`). `None` loadouts leave a combatant melee-identical; armed-bar
    // arms MATHEW/TRAVIS's bows.
    let capture_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A ranged capture cannot replay without the ITEMS table — without this
    // loud skip a missing game file surfaces as a baffling divergence at draw
    // ~58 (the guard already skips the same way).
    let item_data = common::load_item_data();
    if common::capture_has_loadout(&capture_name) && item_data.is_none() {
        eprintln!(
            "SKIPPED (ITEMS absent, D10): {capture_name} carries a ranged loadout \
             and needs the local game data (~/goldbox-data/cotab/ITEMS or GBX_ITEMS_FILE)"
        );
        return;
    }
    let records: Vec<&[u8]> = entries.iter().map(|e| e.record).collect();
    common::apply_loadouts(&mut state, &capture_name, &records, item_data);

    // Seed gbx-prng with the snapshot's rng_state and tap every draw.
    let tap = DrawTap::default();
    let draws = tap.draws.clone();
    let mut rng = EngineRng::new(entry.rng_state);
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
    let mut rounds: Vec<(u16, usize, usize)> = Vec::new();
    let outcome = state.run_combat_observed(&mut rng, DEFAULT_NO_ACTION_LIMIT, |s, r| {
        let (p, m) =
            s.roster()
                .iter()
                .filter(|f| f.in_combat)
                .fold((0usize, 0usize), |(p, m), f| match f.team {
                    Team::Party => (p + 1, m),
                    Team::Monster => (p, m + 1),
                });
        rounds.push((r, p, m));
    });

    // The two draw streams.
    let ours = draws.borrow();
    let capture = capture_combat_draws(&text);

    eprintln!(
        "H4 replay: {n_combatants} combatants ({party} party, {monsters} monster), \
         seed {:#010x}; our fight = {} draws ({:?}), capture = {} draws",
        entry.rng_state,
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
                let operand_ok = match (o.n, c.operand) {
                    (Some(a), Some(b)) => a == b,
                    // One side lacks a recorded operand → fall back to
                    // (before, after) only for this draw.
                    _ => true,
                };
                if o.before == c.before && o.after == c.after && operand_ok {
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
