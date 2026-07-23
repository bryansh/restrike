//! **The frontier-pin regression guard.** A committed manifest of the exact H4
//! replay outcome for every local capture, so the open frontiers
//! (`combat+terrain4` @368, `armed-bar` @58 — the ranged slice's driver,
//! `caster-bar` @453 — the spell subsystem's driver) cannot silently drift and
//! the five closed captures (incl. the `bar-rout-58c50` rout 3521/3521 — doc
//! §32 — and `bar-fists-2` 3811/3811 — doc §33) cannot silently regress.
//!
//! ## The exact-pin rule (read before editing [`PINS`])
//!
//! Each entry is either [`Expect::Closed`] (our engine must replay the capture
//! operand-exact, draw-for-draw, equal length, with **zero** stub trips) or
//! [`Expect::Frontier`]`(N)` — the replay must diverge at **exactly** draw `N`
//! (not `>= N`, not `<= N`). A frontier moves **only** via a deliberate edit to
//! this manifest, made **in the same commit** as the engine fix that earned the
//! move. Both a regression (frontier shrinks / a closed capture diverges) and an
//! unexplained *forward* drift (frontier grows without a manifest edit) fail
//! loudly here. This is the tripwire that keeps "operand-exact" honest across
//! sessions.
//!
//! D10: the manifest holds only capture **basenames** and **draw indices** — no
//! record bytes, no `~/goldbox-data` content, ever. Like [`h4_replay`], the test
//! is local-tier: it loud-skips per-capture when a file is absent, so plain CI
//! (no `GBX_DATA_DIR`) stays green.
//!
//! The replay+compare here mirrors `h4_replay`'s milestone machinery deliberately
//! (a compact copy — factoring the milestone test into a shared module would churn
//! it for little gain); the equality surface is identical: `(before, after)` plus
//! the `Random(n)` **operand** whenever both sides carry one.

use std::path::{Path, PathBuf};

mod common;

use gbx_engine::combat::{
    combat_state_from_records, CombatMap, GridPos, RecordCombatant, Team, DEFAULT_NO_ACTION_LIMIT,
};
use gbx_engine::rng::{EngineRng, RngDraw, RngSink};
use gbx_oracle::Trace;
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::pack::RuleSet;
use std::cell::RefCell;
use std::rc::Rc;

/// The pinned outcome for one capture.
enum Expect {
    /// Replays operand-exact, draw-for-draw, equal length, zero stub trips.
    Closed,
    /// Diverges at **exactly** this draw index (operand or `(before,after)`).
    Frontier(usize),
}

/// One manifest row: capture basename, its pinned outcome, the flee heading
/// (`map_direction`) to apply in-process, and the `AutoPCsCastMagic` input
/// state — the entry value (`auto_cast`, doc §33) plus any mid-fight '2'
/// presses as a toggle schedule (`auto_cast_toggles`, global turn ordinals,
/// doc §38). Only `bar-rout-58c50` routs, so the heading is load-bearing there
/// (md=2, the geometry-matched value, doc §29); for the non-routing captures
/// it is inert but set uniformly for clarity. The magic flag today feeds only
/// the `memorized-spells` wire (no draws), but the manifest carries each
/// fight's true input state so the caster peel inherits it.
struct Pin {
    capture: &'static str,
    expect: Expect,
    map_direction: u8,
    auto_cast: bool,
    auto_cast_toggles: &'static [u32],
}

/// **The manifest.** Current truth (doc §29). Edit ONLY alongside the fix that
/// changes a frontier — see the module doc's exact-pin rule.
const PINS: &[Pin] = &[
    Pin {
        capture: "combat4.gbxtrace",
        expect: Expect::Closed,
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
    },
    Pin {
        capture: "combat3+terrain4.gbxtrace",
        expect: Expect::Closed,
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
    },
    Pin {
        capture: "combat2+terrain4.gbxtrace",
        expect: Expect::Closed,
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
    },
    Pin {
        // Pre-existing operand divergence in the oldest capture (no board
        // snapshots, grafted terrain, field_58C=99 so unrelated to flee); it only
        // ever count-matched (doc §29 finding 3).
        capture: "combat+terrain4.gbxtrace",
        expect: Expect::Frontier(368),
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
    },
    Pin {
        // ★ CLOSED 3521/3521 (doc §32): the rout capture replays operand-exact
        // end to end — FleeCheck ladder, behind-AC, target restore (§31 bug
        // #14), and the cross-round guard (§32 bug #15) all landed.
        capture: "bar-rout-58c50.gbxtrace",
        expect: Expect::Closed, // was Frontier(2895); guarding-survives-initiative (§32 bug #15)
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
    },
    Pin {
        // The armed-slice driver (doc §25 runbook item 3): MATHEW's first turn
        // fires two long-bow shots from range (d20/d6/d20) where our range=1
        // stub (FD-29) walked him across the bar. §34.4/34.5 landed the cornered
        // weapon swap + ammo depletion (TRAVIS's 10-arrow quiver empties → he
        // punches), carrying the frontier to 2019. The facing subsystem (§36)
        // closed the rest: flanking (ovr014:16AD-16E9) landed the swarmed-target
        // behind-AC hits to 2517, and backstab (CanBackStabTarget, ovr014:28D7)
        // landed TRAVIS's cornered-punch kill at 2517 (ac_behind−4, ×3 damage).
        // CLOSED, operand-exact — the last H4 bar-fight frontier.
        capture: "armed-bar.gbxtrace",
        expect: Expect::Closed,
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
    },
    Pin {
        // The caster driver. sub_3560B's selection loop is now faithful (doc
        // §41.1): PHILIPPE's round-2 turn draws 3× roll_dice(1) (d7 bound rolled
        // 1 → one priority-7 pass, MM pri 4 rejects ×3) and his round-3 turn
        // draws 10× roll_dice(1) (3+3+3 rejects at pri 7/6/5, accept at pri 4)
        // before the accept — carrying the frontier from @453 to @1026, the
        // cast's first draw (the `find_target` targeting d10). The cast body
        // lands in the next commit. §38's flip-window ruling stands: entry-false
        // + a toggle at turn ordinal 16 (his round-2 turn head).
        capture: "caster-bar.gbxtrace",
        expect: Expect::Frontier(1026),
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[16],
    },
    Pin {
        // A third independent fist seed, free from the caster staging (doc
        // §33): two memorized-but-inert slots (magic off) — also the
        // capture-proof that the gated memorized-spells wire stays silent.
        capture: "bar-fists-2.gbxtrace",
        expect: Expect::Closed,
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
    },
];

/// Open-floor fallback tile (matches `h4_replay`) for pre-terrain captures.
const FLOOR: u8 = 0x17;

/// Resolve the traces directory, or `None` when the local tier is not active
/// (neither `GBX_TRACES_DIR` nor `GBX_DATA_DIR` set → plain CI skips, D10).
fn traces_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("GBX_TRACES_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("GBX_DATA_DIR")?;
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("goldbox-data/traces"))
}

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

#[derive(Clone, Default)]
struct DrawTap {
    draws: Rc<RefCell<Vec<RngDraw>>>,
}
impl RngSink for DrawTap {
    fn on_draw(&mut self, draw: RngDraw) {
        self.draws.borrow_mut().push(draw);
    }
}

/// The capture's post-`combat_entry` draws with the diagnostic operand
/// (`ss_sp_words[3]`), pulled from the raw JSONL (mirrors `h4_replay`).
fn capture_draws(text: &str) -> Vec<(u32, u32, Option<u16>)> {
    let mut out = Vec::new();
    let mut seen = false;
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
            Some("combat_entry") => seen = true,
            Some("rng") if seen => {
                let before = v["before"].as_u64().unwrap() as u32;
                let after = v["after"].as_u64().unwrap() as u32;
                let operand = v
                    .get("ss_sp_words")
                    .and_then(|w| w.as_array())
                    .and_then(|w| w.get(3))
                    .and_then(|n| n.as_u64())
                    .map(|n| n as u16);
                out.push((before, after, operand));
            }
            _ => {}
        }
    }
    out
}

/// Replay a capture and return `(first_divergence_index, trip_count)`.
/// `None` divergence == closed (all draws equal on `(before, after, operand)` and
/// equal length). The comparison is `h4_replay`'s: `(before, after)` always, plus
/// the operand when both sides carry one.
fn replay(
    path: &Path,
    capture: &str,
    map_direction: u8,
    auto_cast: bool,
    auto_cast_toggles: &[u32],
) -> (Option<usize>, usize) {
    let text = std::fs::read_to_string(path).expect("capture readable");
    let trace = Trace::parse(&text).expect("capture parses");
    let entry = trace
        .combat_entry()
        .expect("capture carries a combat_entry snapshot");

    let entries: Vec<RecordCombatant> = entry
        .combatants
        .iter()
        .map(|c| RecordCombatant {
            team: match c.team {
                0 => Team::Party,
                1 => Team::Monster,
                other => panic!("unknown team byte {other}"),
            },
            pos: GridPos::new(c.x as i32, c.y as i32),
            record: &c.record,
        })
        .collect();

    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    let map = match entry.terrain.as_deref() {
        Some(hex) => CombatMap::from_ground(decode_terrain(hex)),
        None => CombatMap::uniform(FLOOR),
    };
    let mut state = combat_state_from_records(&entries, map, &flavor).expect("records decode");
    state.area_field_58c = entry.area2_field_58c.map(|v| v as i32).unwrap_or(99);
    // The capture's own emitted heading wins (hooks from 8ab275e on); the pin's
    // value covers legacy captures without the field.
    state.map_direction = entry.map_direction.unwrap_or(map_direction);
    state.auto_pcs_cast_magic = auto_cast;
    state.auto_cast_toggles = auto_cast_toggles.to_vec();
    // §34.1: the ITEMS table + per-capture ranged loadouts (one shared place,
    // `common`). `None` loadouts leave a combatant melee-identical, so the six
    // non-armed pins are unshifted; armed-bar arms MATHEW/TRAVIS's bows.
    let records: Vec<&[u8]> = entries.iter().map(|e| e.record).collect();
    common::apply_loadouts(&mut state, capture, &records, common::load_item_data());

    let tap = DrawTap::default();
    let draws = tap.draws.clone();
    let mut rng = EngineRng::new(entry.rng_state);
    rng.attach_sink(Box::new(tap));

    // Count stub trips (a closed capture must fire none).
    let trips: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
    struct TripTap(Rc<RefCell<usize>>);
    impl gbx_engine::combat::ActionSink for TripTap {
        fn on_action(&mut self, e: gbx_engine::combat::ActionEvent) {
            if matches!(e, gbx_engine::combat::ActionEvent::StubTripped { .. }) {
                *self.0.borrow_mut() += 1;
            }
        }
    }
    state.attach_action_sink(Box::new(TripTap(trips.clone())));

    state.run_combat_observed(&mut rng, DEFAULT_NO_ACTION_LIMIT, |_, _| {});

    let trip_count = *trips.borrow();
    let ours = draws.borrow();
    let cap = capture_draws(&text);
    let max = ours.len().max(cap.len());
    let mut frontier = None;
    for i in 0..max {
        match (ours.get(i), cap.get(i)) {
            (Some(o), Some(c)) => {
                let operand_ok = match (o.n, c.2) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                };
                if !(o.before == c.0 && o.after == c.1 && operand_ok) {
                    frontier = Some(i);
                    break;
                }
            }
            _ => {
                frontier = Some(i);
                break;
            }
        }
    }
    (frontier, trip_count)
}

#[test]
fn h4_frontier_pins_hold() {
    let Some(dir) = traces_dir() else {
        eprintln!(
            "SKIPPED: frontier guard needs the local traces dir \
             (set GBX_DATA_DIR or GBX_TRACES_DIR; captures are local-only, D10)"
        );
        return;
    };

    let mut checked = 0;
    for pin in PINS {
        let path = dir.join(pin.capture);
        if !path.exists() {
            eprintln!("SKIPPED (absent, D10): {}", pin.capture);
            continue;
        }
        // A ranged capture needs the ITEMS table; without it (D10) the replay
        // would fall back to melee and the pin cannot hold — skip loudly.
        if common::capture_has_loadout(pin.capture) && common::load_item_data().is_none() {
            eprintln!("SKIPPED (ITEMS absent, D10): {}", pin.capture);
            continue;
        }
        checked += 1;
        let (frontier, trips) = replay(
            &path,
            pin.capture,
            pin.map_direction,
            pin.auto_cast,
            pin.auto_cast_toggles,
        );
        match pin.expect {
            Expect::Closed => {
                assert_eq!(
                    frontier, None,
                    "{}: pinned CLOSED but diverged at draw {frontier:?} \
                     (regression — do NOT edit the pin without the fix that earned it)",
                    pin.capture
                );
                assert_eq!(
                    trips, 0,
                    "{}: pinned CLOSED but {trips} stub trip(s) fired",
                    pin.capture
                );
                eprintln!("OK  {} — CLOSED (operand-exact, 0 trips)", pin.capture);
            }
            Expect::Frontier(n) => {
                assert_eq!(
                    frontier,
                    Some(n),
                    "{}: pinned frontier {n} but diverged at {frontier:?} \
                     (drift — a frontier moves ONLY via a manifest edit in the same \
                     commit as the fix that earned it)",
                    pin.capture
                );
                eprintln!("OK  {} — frontier @{n} (exact)", pin.capture);
            }
        }
    }

    if checked == 0 {
        eprintln!(
            "SKIPPED: no pinned captures present under {}",
            dir.display()
        );
    } else {
        eprintln!("frontier guard: {checked}/{} pins held", PINS.len());
    }
}
