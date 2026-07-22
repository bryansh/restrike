//! **H4 divergence localizer** — replays the `combat4` live capture (which carries
//! real `terrain`, per-`round_snapshot` and per-`turn_snapshot` board state) and
//! diffs our engine against it three ways:
//!
//! 1. **Draw stream** under *uniform floor* vs *real terrain* — how far each
//!    matches the capture's `rng` events, and whether terrain helps.
//! 2. **Per-round board** (cadence-robust): our board at each `RoundStarted`
//!    vs the capture's `round_snapshot`s — first divergent round + combatant +
//!    field (`pos` ⇒ movement, `hp` ⇒ targeting/damage).
//! 3. **Per-turn board**: our post-`Turn` board vs the capture's `turn_snapshot`s,
//!    reporting the first `target`/`pos`/`hp` disagreement.
//!
//! This is a **diagnostic**, not a milestone assert — it prints its findings and
//! passes (so a plain `cargo test` in CI, which lacks the local capture, skips it;
//! D10: the capture holds real record bytes and is local-only). Point it at the
//! capture with `GBX_H4_TURNDIFF=/path/to/combat4.gbxtrace`, else it uses the
//! `~/goldbox-data/traces/combat4.gbxtrace` default and skips if absent.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gbx_engine::combat::{
    combat_state_from_records, CombatMap, CombatStep, GridPos, RecordCombatant, Team,
};
use gbx_engine::rng::{EngineRng, RngDraw, RngSink};
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::pack::RuleSet;

const DEFAULT_CAPTURE: &str = "goldbox-data/traces/combat4.gbxtrace";

fn capture_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("GBX_H4_TURNDIFF") {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(DEFAULT_CAPTURE))
}

fn hex_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let val = |c: u8| -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 0,
        }
    };
    let mut i = 0;
    while i + 1 < b.len() {
        out.push((val(b[i]) << 4) | val(b[i + 1]));
        i += 2;
    }
    out
}

/// A combatant as the capture recorded it at entry.
struct CapEntry {
    team: u8,
    x: i32,
    y: i32,
    record: Vec<u8>,
}

/// One `(before, after, operand)` PRNG draw.
type Draw = (u32, u32, Option<u16>);
/// A per-round board row: `(team, x, y, hp)`.
type RoundRow = (u8, u8, u8, u8);
/// A per-turn board row: `(team, x, y, hp, target)`.
type TurnRow = (u8, u8, u8, u8, u8);

/// Everything the capture holds, parsed straight from the raw JSONL.
struct Capture {
    rng_state: u32,
    terrain: Vec<u8>,
    /// `area2.field_58C` (the faithful FleeCheck_001 gate-2 threshold, doc §28);
    /// legacy captures without the field default to 99 (the measured bar value).
    field_58c: i32,
    entry: Vec<CapEntry>,
    /// post-`combat_entry` `rng` events.
    draws: Vec<Draw>,
    /// `round_snapshot`s: per round, one board.
    rounds: Vec<(u16, Vec<RoundRow>)>,
    /// `turn_snapshot`s: per seq, one board.
    turns: Vec<Vec<TurnRow>>,
}

fn parse_capture(text: &str) -> Capture {
    let mut rng_state = 0u32;
    let mut terrain = Vec::new();
    let mut field_58c: i32 = 99; // §28 default for pre-field_58C captures
    let mut entry = Vec::new();
    let mut draws = Vec::new();
    let mut rounds = Vec::new();
    let mut turns = Vec::new();
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
            Some("combat_entry") => {
                seen_entry = true;
                rng_state = v["rng_state"].as_u64().unwrap() as u32;
                if let Some(t) = v.get("terrain").and_then(|t| t.as_str()) {
                    terrain = hex_decode(t);
                }
                if let Some(f) = v.get("area2_field_58c").and_then(|f| f.as_u64()) {
                    field_58c = f as i32;
                }
                for c in v["combatants"].as_array().unwrap() {
                    entry.push(CapEntry {
                        team: c["team"].as_u64().unwrap() as u8,
                        x: c["x"].as_u64().unwrap() as i32,
                        y: c["y"].as_u64().unwrap() as i32,
                        record: hex_decode(c["record"].as_str().unwrap()),
                    });
                }
            }
            Some("rng") if seen_entry => {
                let operand = v
                    .get("ss_sp_words")
                    .and_then(|w| w.as_array())
                    .and_then(|w| w.get(3))
                    .and_then(|n| n.as_u64())
                    .map(|n| n as u16);
                draws.push((
                    v["before"].as_u64().unwrap() as u32,
                    v["after"].as_u64().unwrap() as u32,
                    operand,
                ));
            }
            Some("round_snapshot") => {
                let round = v["round"].as_u64().unwrap() as u16;
                let board = v["combatants"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| {
                        (
                            c["team"].as_u64().unwrap() as u8,
                            c["x"].as_u64().unwrap() as u8,
                            c["y"].as_u64().unwrap() as u8,
                            c["hp"].as_u64().unwrap() as u8,
                        )
                    })
                    .collect();
                rounds.push((round, board));
            }
            Some("turn_snapshot") => {
                let board = v["combatants"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| {
                        (
                            c["team"].as_u64().unwrap() as u8,
                            c["x"].as_u64().unwrap() as u8,
                            c["y"].as_u64().unwrap() as u8,
                            c["hp"].as_u64().unwrap() as u8,
                            c["target"].as_u64().unwrap() as u8,
                        )
                    })
                    .collect();
                turns.push(board);
            }
            _ => {}
        }
    }

    Capture {
        rng_state,
        terrain,
        field_58c,
        entry,
        draws,
        rounds,
        turns,
    }
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

fn team_of(b: u8) -> Team {
    match b {
        0 => Team::Party,
        1 => Team::Monster,
        other => panic!("unknown team byte {other}"),
    }
}

/// Our board: (team, x, y, hp) per fighter, roster order.
fn board_ph(state: &gbx_engine::combat::CombatState) -> Vec<RoundRow> {
    state
        .roster()
        .iter()
        .map(|f| {
            let team = match f.team {
                Team::Party => 0,
                Team::Monster => 1,
            };
            (
                team,
                f.pos.x as u8,
                f.pos.y as u8,
                f.hp_current.max(0) as u8,
            )
        })
        .collect()
}

/// Our board with target: (team, x, y, hp, target-index-or-255) per fighter.
fn board_pht(state: &gbx_engine::combat::CombatState) -> Vec<TurnRow> {
    state
        .roster()
        .iter()
        .map(|f| {
            let team = match f.team {
                Team::Party => 0,
                Team::Monster => 1,
            };
            let target = f.target.map(|t| t as u8).unwrap_or(255);
            (
                team,
                f.pos.x as u8,
                f.pos.y as u8,
                f.hp_current.max(0) as u8,
                target,
            )
        })
        .collect()
}

struct RunResult {
    draws: Vec<Draw>,
    round_boards: Vec<(u16, Vec<RoundRow>)>,
    turn_boards: Vec<Vec<TurnRow>>,
}

fn run(cap: &Capture, map: CombatMap) -> RunResult {
    let entries: Vec<RecordCombatant> = cap
        .entry
        .iter()
        .map(|c| RecordCombatant {
            team: team_of(c.team),
            pos: GridPos::new(c.x, c.y),
            record: &c.record,
        })
        .collect();

    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    let mut state = combat_state_from_records(&entries, map, &flavor).expect("records decode");
    // The faithful FleeCheck_001 gate-2 morale threshold (doc §28).
    state.area_field_58c = cap.field_58c;
    // Flee HEADING input (`map_direction`, doc §28/§29) — the capture does not
    // carry it; provisional default 2 (E — the geometry-matching heading, §29),
    // `RESTRIKE_MAP_DIR` overrides for the trial.
    state.map_direction = std::env::var("RESTRIKE_MAP_DIR")
        .ok()
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(2);
    // `AutoPCsCastMagic` knob, same contract as `h4_replay` (doc §33):
    // `RESTRIKE_AUTO_CAST=1` arms PC casting for captures where the player did.
    state.auto_pcs_cast_magic = std::env::var("RESTRIKE_AUTO_CAST")
        .map(|v| v == "1")
        .unwrap_or(false);

    let tap = DrawTap::default();
    let draws = tap.draws.clone();
    let mut rng = EngineRng::new(cap.rng_state);
    rng.attach_sink(Box::new(tap));

    let mut round_boards = Vec::new();
    let mut turn_boards = Vec::new();
    let mut guard = 0usize;
    loop {
        guard += 1;
        assert!(guard < 1_000_000, "step loop runaway");
        match state.step(&mut rng) {
            CombatStep::RoundStarted { round } => {
                round_boards.push((round, board_ph(&state)));
            }
            CombatStep::Turn { .. } => {
                turn_boards.push(board_pht(&state));
            }
            CombatStep::RoundEnded { battle_over, .. } => {
                if battle_over {
                    break;
                }
            }
            CombatStep::Ended => break,
        }
    }

    let ours: Vec<Draw> = draws
        .borrow()
        .iter()
        .map(|d| (d.before, d.after, d.n))
        .collect();
    RunResult {
        draws: ours,
        round_boards,
        turn_boards,
    }
}

/// How many leading draws share the same `(before, after)` — count-only (a pure
/// LCG makes this trivially equal until the counts desync).
fn draw_match_len(ours: &[(u32, u32, Option<u16>)], cap: &[(u32, u32, Option<u16>)]) -> usize {
    let n = ours.len().min(cap.len());
    (0..n)
        .take_while(|&i| ours[i].0 == cap[i].0 && ours[i].1 == cap[i].1)
        .count()
}

/// The real diagnostic: how many leading draws share the same **operand** (die
/// size). The first mismatch is the exact draw where our mechanic drew a
/// different die than the game — naming where a turn's logic forks.
fn operand_match_len(ours: &[(u32, u32, Option<u16>)], cap: &[(u32, u32, Option<u16>)]) -> usize {
    let n = ours.len().min(cap.len());
    (0..n).take_while(|&i| ours[i].2 == cap[i].2).count()
}

/// Demonstrates the metric difference behind the "2995 vs 153" question:
/// the count-only `(before,after)` match (what h4_replay used) vs the operand
/// (die-size) match. Point it at any capture with `GBX_H4_TURNDIFF`.
#[test]
fn h4_count_vs_operand_metric() {
    let Some(path) = capture_path() else {
        eprintln!("SKIPPED");
        return;
    };
    if !path.exists() {
        eprintln!("SKIPPED: {} absent", path.display());
        return;
    }
    let text = std::fs::read_to_string(&path).expect("readable");
    let cap = parse_capture(&text);
    // Uniform floor so it works on terrain-less captures too.
    let ours = run(&cap, CombatMap::uniform(0x17));
    let count = draw_match_len(&ours.draws, &cap.draws);
    let operand = operand_match_len(&ours.draws, &cap.draws);
    eprintln!(
        "capture {} : {} combatants, {} capture draws, our {} draws",
        path.file_name().unwrap().to_string_lossy(),
        cap.entry.len(),
        cap.draws.len(),
        ours.draws.len(),
    );
    eprintln!(
        "  COUNT-only (before,after) match: {count}/{}   <- what h4_replay measured (LCG-trivial)",
        cap.draws.len()
    );
    eprintln!(
        "  OPERAND (die-size) match:        {operand}/{}   <- the real mechanic-level test",
        cap.draws.len()
    );
}

fn die_label(n: Option<u16>) -> &'static str {
    match n {
        Some(6) => "d6 init",
        Some(100) => "d100 select/morale",
        Some(20) => "d20 to-hit/save",
        Some(7) => "d7 behavior-gate",
        Some(2) => "d2 damage",
        Some(0) => "rand(0)",
        Some(_) => "d? damage",
        None => "?",
    }
}

#[test]
fn dump_random_wrapper_bytes() {
    let Some(home) = std::env::var_os("HOME") else {
        eprintln!("SKIPPED");
        return;
    };
    let path = PathBuf::from(home).join("goldbox-data/cotab/START.EXE");
    if !path.exists() {
        eprintln!("SKIPPED: START.EXE absent at {}", path.display());
        return;
    }
    let packed = std::fs::read(&path).expect("readable");
    let image = gbx_formats::exepack::decode(&packed).expect("exepack decode");
    // The Random(N) wrapper is image 0xa55a; RandNext at 0xa5a9.
    for (label, start, len) in [
        ("Random(N) @0xa55a", 0xa55a, 0x50usize),
        ("RandNext @0xa5a9", 0xa5a9, 0x40),
    ] {
        let bytes = &image[start..start + len];
        eprint!("{label}:\n  ");
        for (i, b) in bytes.iter().enumerate() {
            eprint!("{b:02x} ");
            if (i + 1) % 16 == 0 {
                eprint!("\n  ");
            }
        }
        eprintln!();
    }
}

/// A merged timeline of draws and action events, so a turn's draws are labeled
/// by the mechanic that emitted them.
#[derive(Clone)]
enum Tl {
    Draw(Option<u16>),
    Ev(String),
}

#[test]
fn h4_first_turn_trace() {
    let Some(path) = capture_path() else {
        eprintln!("SKIPPED");
        return;
    };
    if !path.exists() {
        eprintln!("SKIPPED: capture absent");
        return;
    }
    let text = std::fs::read_to_string(&path).expect("readable");
    let cap = parse_capture(&text);

    let timeline: Rc<RefCell<Vec<Tl>>> = Rc::new(RefCell::new(Vec::new()));

    struct DrawTl(Rc<RefCell<Vec<Tl>>>);
    impl RngSink for DrawTl {
        fn on_draw(&mut self, d: RngDraw) {
            self.0.borrow_mut().push(Tl::Draw(d.n));
        }
    }
    struct ActTl(Rc<RefCell<Vec<Tl>>>);
    impl gbx_engine::combat::ActionSink for ActTl {
        fn on_action(&mut self, e: gbx_engine::combat::ActionEvent) {
            self.0.borrow_mut().push(Tl::Ev(format!("{e:?}")));
        }
    }

    let records: Vec<Vec<u8>> = cap.entry.iter().map(|c| c.record.clone()).collect();
    let entries: Vec<RecordCombatant> = cap
        .entry
        .iter()
        .zip(&records)
        .map(|(c, rec)| RecordCombatant {
            team: team_of(c.team),
            pos: GridPos::new(c.x, c.y),
            record: rec,
        })
        .collect();
    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    let mut state = combat_state_from_records(
        &entries,
        CombatMap::from_ground(cap.terrain.clone()),
        &flavor,
    )
    .expect("decode");
    state.area_field_58c = cap.field_58c;
    state.attach_action_sink(Box::new(ActTl(timeline.clone())));
    let mut rng = EngineRng::new(cap.rng_state);
    rng.attach_sink(Box::new(DrawTl(timeline.clone())));

    // Drive far enough to cover the draw-153 region.
    let mut steps = 0;
    loop {
        steps += 1;
        if steps > 2000 {
            break;
        }
        match state.step(&mut rng) {
            CombatStep::Ended => break,
            CombatStep::RoundEnded {
                battle_over: true, ..
            } => break,
            _ => {}
        }
        if timeline.borrow().len() > 900 {
            break;
        }
    }

    eprintln!("=== OUR merged timeline around draw 145-160 (ours) ===");
    let tl = timeline.borrow();
    let mut draw_i = 0usize;
    for entry in tl.iter() {
        let show = (145..=160).contains(&draw_i);
        match entry {
            Tl::Draw(n) => {
                if show {
                    eprintln!(
                        "  draw {:3}: d{}  ({})",
                        draw_i,
                        n.map(|x| x as i32).unwrap_or(-1),
                        die_label(*n)
                    );
                }
                draw_i += 1;
            }
            Tl::Ev(s) => {
                if show
                    && (s.starts_with("Pick")
                        || s.starts_with("Ai")
                        || s.starts_with("Attack")
                        || s.starts_with("Dmg")
                        || s.starts_with("Move"))
                {
                    eprintln!("           >>> {s}");
                }
            }
        }
        if draw_i > 372 {
            break;
        }
    }

    eprintln!("\n=== CAPTURE operands 145-160 for reference ===");
    for i in 350..371.min(cap.draws.len()) {
        eprintln!(
            "  draw {i:3}: d{}",
            cap.draws[i].2.map(|x| x as i32).unwrap_or(-1)
        );
    }
}

#[test]
fn h4_philippe_near_list() {
    let Some(path) = capture_path() else {
        eprintln!("SKIPPED");
        return;
    };
    if !path.exists() {
        eprintln!("SKIPPED: capture absent");
        return;
    }
    let text = std::fs::read_to_string(&path).expect("readable");
    let cap = parse_capture(&text);

    use gbx_engine::combat::{
        build_near_targets, can_reach, find_combatant_direction, reach_ray, RangeCombatant,
    };

    let map = CombatMap::from_ground(cap.terrain.clone());
    let combatants: Vec<RangeCombatant> = cap
        .entry
        .iter()
        .map(|c| RangeCombatant {
            pos: GridPos::new(c.x, c.y),
            size: 1,
            team: team_of(c.team),
        })
        .collect();

    let actor = std::env::var("GBX_ACTOR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5usize); // PHILIPPE by default; GBX_ACTOR=4 for SHARA
    let ap = combatants[actor].pos;
    eprintln!("PHILIPPE (actor {actor}) at ({},{})", ap.x, ap.y);
    eprintln!("\n=== per-monster raw reach from PHILIPPE (real terrain) ===");
    eprintln!(" idx  pos      grid_dist  reach_ray(steps,reach)  can_reach(0xff)  dir");
    for (i, c) in combatants.iter().enumerate() {
        if c.team != Team::Monster {
            continue;
        }
        let tp = c.pos;
        let gd = (tp.x - ap.x).abs().max((tp.y - ap.y).abs());
        let rr = reach_ray(&map, ap, tp, false);
        let cr = can_reach(&map, ap, tp, 0xff, false);
        let dir = find_combatant_direction(tp, ap);
        eprintln!(
            "  {i:2}  ({:2},{:2})   {gd:3}       ({:3},{})            {:?}           {dir}",
            tp.x, tp.y, rr.steps, rr.reach, cr
        );
    }

    let near = build_near_targets(&map, &combatants, actor, 0xff, false);
    eprintln!(
        "\n=== our sorted near-list ({} entries) — find_target picks near[5] ===",
        near.len()
    );
    for (rank, nt) in near.iter().enumerate() {
        let dir = find_combatant_direction(nt.pos, ap);
        let mark = if rank == 5 {
            "  <-- near[5] (our pick)"
        } else {
            ""
        };
        eprintln!(
            "  rank {rank}: monster idx {:2} at ({:2},{:2}) steps={} dir={dir}{mark}",
            nt.idx, nt.pos.x, nt.pos.y, nt.steps
        );
    }
    eprintln!("\n(capture picks monster 11; if near[5] != 11 here, the sort order diverges)");
}

#[test]
fn h4_round0_moves() {
    let Some(path) = capture_path() else {
        eprintln!("SKIPPED");
        return;
    };
    if !path.exists() {
        eprintln!("SKIPPED: capture absent");
        return;
    }
    let text = std::fs::read_to_string(&path).expect("readable");
    let cap = parse_capture(&text);

    // Collect Move + Pick events with an action sink.
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    struct S(Rc<RefCell<Vec<String>>>);
    impl gbx_engine::combat::ActionSink for S {
        fn on_action(&mut self, e: gbx_engine::combat::ActionEvent) {
            self.0.borrow_mut().push(format!("{e:?}"));
        }
    }

    let records: Vec<Vec<u8>> = cap.entry.iter().map(|c| c.record.clone()).collect();
    let entries: Vec<RecordCombatant> = cap
        .entry
        .iter()
        .zip(&records)
        .map(|(c, rec)| RecordCombatant {
            team: team_of(c.team),
            pos: GridPos::new(c.x, c.y),
            record: rec,
        })
        .collect();
    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    let mut state = combat_state_from_records(
        &entries,
        CombatMap::from_ground(cap.terrain.clone()),
        &flavor,
    )
    .expect("decode");
    state.area_field_58c = cap.field_58c;
    state.attach_action_sink(Box::new(S(log.clone())));
    let mut rng = EngineRng::new(cap.rng_state);
    rng.attach_sink(Box::new(DrawTap::default()));

    // Run through round 0 (stop at the first RoundEnded).
    loop {
        match state.step(&mut rng) {
            CombatStep::RoundEnded { .. } => break,
            CombatStep::Ended => break,
            _ => {}
        }
    }

    let names = ["MATHEW", "MARK", "TRAVIS", "LEDERA", "SHARA", "PHILIPPE"];
    eprintln!("=== OUR round-0 Pick order + moves (vs capture) ===");
    eprintln!("capture: SHARA(4)->(32,13) MARK(1)->(33,14) LEDERA(3)->(31,12) MATHEW(0)->(31,11) TRAVIS(2)->(32,14)");
    for line in log.borrow().iter() {
        if line.starts_with("Pick") {
            eprintln!("{line}");
        } else if line.starts_with("Move") {
            eprintln!("   {line}");
        }
    }
    eprintln!("\n=== our round-0 final positions ===");
    for (i, f) in state.roster().iter().enumerate() {
        let nm = if i < 6 { names[i] } else { "PATRON" };
        eprintln!("  {nm}({i}): ({},{})", f.pos.x, f.pos.y);
    }
}

#[test]
fn h4_locate_draw() {
    let target: usize = std::env::var("GBX_DRAW")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(358);
    let Some(path) = capture_path() else {
        eprintln!("SKIPPED");
        return;
    };
    if !path.exists() {
        eprintln!("SKIPPED");
        return;
    }
    let text = std::fs::read_to_string(&path).expect("readable");
    let cap = parse_capture(&text);

    let count = Rc::new(RefCell::new(0usize));
    let log: Rc<RefCell<Vec<(usize, String)>>> = Rc::new(RefCell::new(Vec::new()));
    struct Ctr(Rc<RefCell<usize>>);
    impl RngSink for Ctr {
        fn on_draw(&mut self, _d: RngDraw) {
            *self.0.borrow_mut() += 1;
        }
    }
    struct Rec(Rc<RefCell<usize>>, Rc<RefCell<Vec<(usize, String)>>>);
    impl gbx_engine::combat::ActionSink for Rec {
        fn on_action(&mut self, e: gbx_engine::combat::ActionEvent) {
            self.1
                .borrow_mut()
                .push((*self.0.borrow(), format!("{e:?}")));
        }
    }
    let records: Vec<Vec<u8>> = cap.entry.iter().map(|c| c.record.clone()).collect();
    let entries: Vec<RecordCombatant> = cap
        .entry
        .iter()
        .zip(&records)
        .map(|(c, rec)| RecordCombatant {
            team: team_of(c.team),
            pos: GridPos::new(c.x, c.y),
            record: rec,
        })
        .collect();
    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    let mut state = combat_state_from_records(
        &entries,
        CombatMap::from_ground(cap.terrain.clone()),
        &flavor,
    )
    .expect("decode");
    state.area_field_58c = cap.field_58c;
    // Same flee-heading knob as `run()` (§29) — without it this diagnostic
    // replays a DIFFERENT fight (md=0 → NW flight) than the localize/replay
    // paths and misleads the peel.
    state.map_direction = std::env::var("RESTRIKE_MAP_DIR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    // Same `AutoPCsCastMagic` knob as `run()` (doc §33).
    state.auto_pcs_cast_magic = std::env::var("RESTRIKE_AUTO_CAST")
        .map(|v| v == "1")
        .unwrap_or(false);
    state.attach_action_sink(Box::new(Rec(count.clone(), log.clone())));
    let mut rng = EngineRng::new(cap.rng_state);
    rng.attach_sink(Box::new(Ctr(count.clone())));
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 1_000_000 || *count.borrow() > target + 30 {
            break;
        }
        match state.step(&mut rng) {
            CombatStep::Ended => break,
            CombatStep::RoundEnded { battle_over, .. } if battle_over => break,
            _ => {}
        }
    }
    eprintln!("=== our events near draw {target} (draw#: event) ===");
    for (dc, e) in log.borrow().iter() {
        if *dc + 20 >= target
            && *dc <= target + 20
            && (e.starts_with("Pick")
                || e.starts_with("Ai")
                || e.starts_with("Attack")
                || e.starts_with("Dmg")
                || e.starts_with("Move"))
        {
            eprintln!("  d{dc}: {e}");
        }
    }
    eprintln!(
        "\ncapture operands {}-{}: {:?}",
        target.saturating_sub(6),
        target + 6,
        (target.saturating_sub(6)..=target + 6)
            .filter_map(|i| cap.draws.get(i).map(|d| d.2))
            .collect::<Vec<_>>()
    );
}

#[test]
fn h4_decode_party_records() {
    let Some(path) = capture_path() else {
        eprintln!("SKIPPED: no capture path");
        return;
    };
    if !path.exists() {
        eprintln!("SKIPPED: capture absent (D10 local-only)");
        return;
    }
    let text = std::fs::read_to_string(&path).expect("readable");
    let cap = parse_capture(&text);
    eprintln!("=== decoded records (class/weapon/attack profile) ===");
    for (i, c) in cap.entry.iter().enumerate() {
        let rec = gbx_formats::save_orig::decode_char_record(&c.record).expect("decode");
        eprintln!(
            "[{i:2}] {:<8} team={} class={} race={} hd={} mv={} basemv={} wpn_icon={} qf={} ctrl_morale={:#04x}\n       atk_base={:?}\n       atk_cur ={:?}  hp={}/{}",
            rec.name,
            c.team,
            rec.class,
            rec.race,
            rec.hit_dice,
            rec.movement,
            rec.base_movement,
            rec.weapon_icon,
            rec.quick_fight,
            rec.control_morale,
            rec.attack_profile_base,
            rec.attack_profile_current,
            rec.hit_point_current,
            rec.hit_point_max,
        );
    }
}

#[test]
fn h4_turndiff_localize() {
    let Some(path) = capture_path() else {
        eprintln!("SKIPPED: no HOME/GBX_H4_TURNDIFF to locate the combat4 capture");
        return;
    };
    if !path.exists() {
        eprintln!(
            "SKIPPED: local-tier combat4 capture absent at {} (D10 local-only)",
            path.display()
        );
        return;
    }
    let text = std::fs::read_to_string(&path).expect("capture readable");
    let cap = parse_capture(&text);

    eprintln!(
        "capture: seed {:#010x}, {} combatants, terrain {} cells, {} draws, {} round_snapshots, {} turn_snapshots",
        cap.rng_state,
        cap.entry.len(),
        cap.terrain.len(),
        cap.draws.len(),
        cap.rounds.len(),
        cap.turns.len(),
    );

    // ---- 1. draw stream: uniform vs real terrain ----
    let uniform = run(&cap, CombatMap::uniform(0x17));
    let mu = draw_match_len(&uniform.draws, &cap.draws);
    eprintln!(
        "\n[uniform floor] our draws {} vs capture {} — matched {}/{} before divergence",
        uniform.draws.len(),
        cap.draws.len(),
        mu,
        cap.draws.len()
    );

    assert_eq!(cap.terrain.len(), 1250, "terrain must be a 50x25 grid");
    let terrain = run(&cap, CombatMap::from_ground(cap.terrain.clone()));
    let mt = draw_match_len(&terrain.draws, &cap.draws);
    eprintln!(
        "[real terrain ] our draws {} vs capture {} — matched {}/{} before divergence",
        terrain.draws.len(),
        cap.draws.len(),
        mt,
        cap.draws.len()
    );
    if mt == cap.draws.len() && terrain.draws.len() == cap.draws.len() {
        eprintln!("\n*** H4 MELEE CLOSED under real terrain: full draw-stream equality ***");
    }

    // ---- 1b. OPERAND divergence (the real localizer) ----
    // (before,after) is count-only; the operand (die size) names the mechanic.
    for (rl, res) in [("uniform floor", &uniform), ("real terrain", &terrain)] {
        let om = operand_match_len(&res.draws, &cap.draws);
        eprintln!(
            "\n[{rl}] first OPERAND divergence at draw #{om} (of {} ours / {} capture)",
            res.draws.len(),
            cap.draws.len()
        );
        if om < res.draws.len() && om < cap.draws.len() {
            let lo = om.saturating_sub(6);
            let hi = (om + 12).min(res.draws.len()).min(cap.draws.len());
            eprintln!("   idx | ours op | capture op");
            for i in lo..hi {
                let mark = if res.draws[i].2 != cap.draws[i].2 {
                    "  <-- FIRST DIVERGENCE"
                } else {
                    ""
                };
                eprintln!(
                    "  {i:4} | {:>4} {:<18} | {:>4} {:<18}{}",
                    res.draws[i].2.map(|n| n as i32).unwrap_or(-1),
                    die_label(res.draws[i].2),
                    cap.draws[i].2.map(|n| n as i32).unwrap_or(-1),
                    die_label(cap.draws[i].2),
                    mark
                );
            }
        }
    }

    // Use the better run for the board diffs.
    let (label, res) = if mt >= mu {
        ("real terrain", &terrain)
    } else {
        ("uniform floor", &uniform)
    };
    eprintln!("\nusing [{label}] run for board diffs (matched more/equal draws)");

    // ---- 2. per-round board diff (cadence-robust) ----
    eprintln!("\n=== per-round board diff (team,x,y,hp) ===");
    eprintln!(
        "  our RoundStarted rounds: {:?}",
        res.round_boards.iter().map(|(r, _)| *r).collect::<Vec<_>>()
    );
    eprintln!(
        "  capture round_snapshot rounds: {:?}",
        cap.rounds.iter().map(|(r, _)| *r).collect::<Vec<_>>()
    );
    // Align by index (both sequences start at the first round the engine reports).
    let n_rounds = res.round_boards.len().min(cap.rounds.len());
    let mut first_round_div = None;
    'rounds: for i in 0..n_rounds {
        let (our_r, our_b) = &res.round_boards[i];
        let (cap_r, cap_b) = &cap.rounds[i];
        for (ci, (o, c)) in our_b.iter().zip(cap_b.iter()).enumerate() {
            if o != c {
                first_round_div = Some((i, *our_r, *cap_r, ci, *o, *c));
                break 'rounds;
            }
        }
    }
    match first_round_div {
        None => eprintln!("  all {n_rounds} aligned rounds match board-for-board (pos+hp)"),
        Some((i, our_r, cap_r, ci, o, c)) => {
            let field = if o.1 != c.1 || o.2 != c.2 {
                "pos (MOVEMENT)"
            } else if o.3 != c.3 {
                "hp (TARGETING/DAMAGE)"
            } else {
                "team"
            };
            eprintln!(
                "  FIRST DIVERGENT ROUND at seq-index {i} (our round {our_r}, capture round {cap_r}):"
            );
            eprintln!(
                "    combatant [{ci}]: ours (t{},x{},y{},hp{}) vs capture (t{},x{},y{},hp{})  <-- {field}",
                o.0, o.1, o.2, o.3, c.0, c.1, c.2, c.3
            );
            // Dump the full board at that round for context.
            let (_, our_b) = &res.round_boards[i];
            let (_, cap_b) = &cap.rounds[i];
            eprintln!("    full board at this round (idx: ours | capture, * = differs):");
            for (k, (o, c)) in our_b.iter().zip(cap_b.iter()).enumerate() {
                let mark = if o != c { " *" } else { "" };
                eprintln!(
                    "      [{k:2}] ({},{},{},hp{}) | ({},{},{},hp{}){}",
                    o.0, o.1, o.2, o.3, c.0, c.1, c.2, c.3, mark
                );
            }
        }
    }

    // ---- 3. per-turn board diff (adds target) ----
    eprintln!("\n=== per-turn board diff (team,x,y,hp,target) ===");
    eprintln!(
        "  our post-Turn snapshots: {}, capture turn_snapshots: {}",
        res.turn_boards.len(),
        cap.turns.len()
    );
    let n_turns = res.turn_boards.len().min(cap.turns.len());
    let mut first_turn_div = None;
    'turns: for i in 0..n_turns {
        let ob = &res.turn_boards[i];
        let cb = &cap.turns[i];
        for (ci, (o, c)) in ob.iter().zip(cb.iter()).enumerate() {
            if o != c {
                first_turn_div = Some((i, ci, *o, *c));
                break 'turns;
            }
        }
    }
    match first_turn_div {
        None => eprintln!(
            "  all {n_turns} index-aligned turn snapshots match (note: capture emits-on-change, ours emits-per-turn — index alignment is only meaningful up to the first cadence split)"
        ),
        Some((i, ci, o, c)) => {
            let field = if o.4 != c.4 && o.1 == c.1 && o.2 == c.2 {
                "target (SORT-TIE / TARGETING) — positions match"
            } else if o.1 != c.1 || o.2 != c.2 {
                "pos (MOVEMENT)"
            } else if o.3 != c.3 {
                "hp (DAMAGE)"
            } else {
                "target"
            };
            eprintln!("  FIRST DIVERGENT TURN snapshot at index {i}:");
            eprintln!(
                "    combatant [{ci}]: ours (t{},x{},y{},hp{},tgt{}) vs capture (t{},x{},y{},hp{},tgt{})  <-- {field}",
                o.0, o.1, o.2, o.3, o.4, c.0, c.1, c.2, c.3, c.4
            );
        }
    }

    // ---- 3b. per-turn POSITION-only divergence (isolate the movement cascade) ----
    // The residual after §15's #1/#2/#4 is a round-0 movement cascade: a mover
    // lands one cell off, which is draw-free but shifts draw-free targeting later.
    // The per-turn diff above stops on the first *any-field* change (often a
    // target/hp cadence artifact), so hunt the first (x,y) disagreement explicitly.
    // CAVEAT: per-turn snapshots have the cadence split noted above, so a large
    // apparent jump here can be a *misalignment* (capture snapshot i ≠ our
    // snapshot i's actor), not a real 6-cell move. The cadence-robust,
    // authoritative movement signal is the per-round board diff above (§2): after
    // §15 #1/#2/#4 it shows the round-0 cascade as combatants 0/1/2/4 + monster 13
    // landing ~1 cell off while PHILIPPE (5) and LEDERA (3) match the capture.
    eprintln!("\n=== first per-turn POSITION divergence (movement cascade; cadence-caveated) ===");
    let mut first_pos_div = None;
    'pos: for i in 0..n_turns {
        for (ci, (o, c)) in res.turn_boards[i]
            .iter()
            .zip(cap.turns[i].iter())
            .enumerate()
        {
            if o.1 != c.1 || o.2 != c.2 {
                first_pos_div = Some((i, ci));
                break 'pos;
            }
        }
    }
    match first_pos_div {
        None => eprintln!("  no per-turn position divergence within {n_turns} aligned snapshots"),
        Some((i, ci)) => {
            eprintln!("  FIRST POSITION divergence at turn snapshot {i}, combatant [{ci}]:");
            for (k, (o, c)) in res.turn_boards[i]
                .iter()
                .zip(cap.turns[i].iter())
                .enumerate()
            {
                let mark = if o.1 != c.1 || o.2 != c.2 {
                    "  <-- pos differs"
                } else {
                    ""
                };
                eprintln!(
                    "    [{k:2}] ours ({},{}) | capture ({},{}){}",
                    o.1, o.2, c.1, c.2, mark
                );
            }
        }
    }
}
