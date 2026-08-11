//! ★ **The slice-6 test surface** (`docs/design/combat-visualizer.md` §8.5) —
//! the Shell combat flow's state chart, proven through the real
//! `Engine::tick` loop.
//!
//! Four obligations, one module:
//!
//! 1. **Park/resume in every flow kind** (§8.3 rule 1): a fight parks inside
//!    Boot, Step, Look and a chain round, and each flow resumes into the
//!    identical pre-fight cursor — proven by the stages that run *after* the
//!    fight, which only a surviving cursor can reach.
//! 2. **The GameOver deferral** (§8.2's MUST): a wiped party's final beats all
//!    play before `party_killed` unwinds the shell. This is
//!    `shell.rs::party_killed_unwinds_to_game_over_and_resets_the_flag`'s
//!    rendered twin.
//! 3. **A parked fight round-trips** through serde and keeps fighting (D-CV7).
//! 4. ★ **The shell-path draw-parity invariant** (§8.3 rule 4): a scripted
//!    fight driven through the parked shell — scene composed, played tick by
//!    tick, reconciled at every boundary — produces the identical `RngDraw`
//!    stream to the same fight run headlessly.
//!
//! Everything is D10 synthetic (the `combat_wiring` fixture set), so it all
//! runs in CI.

#![cfg(test)]

use crate::combat::{CombatState, CombatStep, GridPos, HealthStatus};
use crate::combat_host::Stage;
use crate::combat_wiring::{
    char_record, combat_game_data, engine_with_program, load_then_combat_program, open_geo,
    party_member, synthetic_font, synthetic_set4,
};
use crate::engine::{Engine, GAME_AREA, INITIAL_ECL_BLOCK};
use crate::rng::{EngineRng, RngDraw, RngSink};
use crate::shell::Shell;
use crate::test_support::{build_dax_file, ecl_dax_block};
use crate::vmhost::TranscriptEntry;
use gbx_formats::game_data::GameData;
use gbx_vm::test_support::EclBuilder;
use std::cell::RefCell;
use std::rc::Rc;

/// A cap that a real fixture fight never approaches — every loop here is
/// bounded so a hang shows up as a failure, not a stuck suite.
const MAX_TICKS: usize = 20_000;

// --- taps -----------------------------------------------------------------

#[derive(Clone, Default)]
struct Draws(Rc<RefCell<Vec<RngDraw>>>);
struct DrawTap(Rc<RefCell<Vec<RngDraw>>>);
impl RngSink for DrawTap {
    fn on_draw(&mut self, d: RngDraw) {
        self.0.borrow_mut().push(d);
    }
}
impl Draws {
    fn sink(&self) -> Box<dyn RngSink> {
        Box::new(DrawTap(Rc::clone(&self.0)))
    }
    fn len(&self) -> usize {
        self.0.borrow().len()
    }
    fn taken(&self) -> Vec<RngDraw> {
        self.0.borrow().clone()
    }
}

// --- fixtures -------------------------------------------------------------

fn two_pcs() -> Vec<crate::party::Character> {
    vec![
        party_member("Ravd", 40, 54, 50),
        party_member("Ilma", 38, 52, 48),
    ]
}

/// A party that cannot win: 1 HP each, hopeless AC, hopeless to-hit.
fn doomed_pcs() -> Vec<crate::party::Character> {
    vec![
        party_member("Meek", 1, 0, 0),
        party_member("Frail", 1, 0, 0),
    ]
}

/// A `GameData` whose `ECL{area}.DAX` block 1 has one program per vector slot,
/// so a test can put the `LOAD MONSTER; COMBAT; PRINT` sequence behind whichever
/// vector the flow it wants to exercise fires:
///
/// | vector | fired by |
/// |---|---|
/// | 0 | `StepFlow`'s first stage (`RUN_ADDR_1`) |
/// | 1 | `LookFlow`, and `StepFlow`'s stage after the door interaction |
/// | 4 | `BootFlow`'s entry vector, and every chained block's |
fn vectored_game_data(vectors: [&str; 5], body: impl Fn(&mut EclBuilder)) -> GameData {
    let mut b = EclBuilder::new();
    for name in vectors {
        b.raw(&[0]);
        b.imm_word_label(name);
    }
    body(&mut b);
    let ecl = build_dax_file(&[(INITIAL_ECL_BLOCK, ecl_dax_block(&b.build_bytes()))]);
    let goblin = char_record(b"GOBLIN", 3, 10, 20, 6, true);
    let mon = build_dax_file(&[(0u8, goblin)]);
    GameData::from_files([
        (format!("ECL{GAME_AREA}.DAX"), ecl),
        (format!("MON{GAME_AREA}CHA.DAX"), mon),
    ])
}

/// `LOAD MONSTER 0, copies, 1; COMBAT; PRINT marker; EXIT` at `label`.
fn fight_at(b: &mut EclBuilder, label: &str, copies: u8, marker: &[u8]) {
    b.label(label);
    b.op(0x0B).imm_byte(0).imm_byte(copies).imm_byte(1);
    b.op(0x24); // COMBAT
    b.op(0x11).inline_str(marker); // PRINT — only runs if the vector resumed
    b.op(0x00); // EXIT
}

fn exit_at(b: &mut EclBuilder, label: &str) {
    b.label(label);
    b.op(0x00);
}

fn engine_with(data: GameData, party: Vec<crate::party::Character>) -> Engine {
    let mut sets = crate::symbols::SymbolSets::new();
    sets.load(4, synthetic_set4());
    let mut e = Engine::new_fixture(synthetic_font(), sets, open_geo(), data, 1);
    e.party = crate::party::Party { members: party };
    // Interior of the map: the faithful floor projects off-grid squares as
    // walls, so a fight staged at the spawn corner is walled off from its own
    // monsters (see `combat_wiring::engine_with_program`).
    e.state.pos = (8, 8);
    e
}

/// ★ **M6c**: the keys a test with no player at the keyboard still owes.
///
/// A won fight with two members standing ends on `yes_no("Continue Battle:")`
/// (`ovr009.cs:404-410`) — a real question, and since D-CV5 a real suspension.
/// These tests are about the shell flow, so they answer it the way an operator
/// who is done fighting does: `N`.
fn auto_keys(e: &Engine) -> Vec<crate::input::InputEvent> {
    match e.shell().combat_host().map(|h| h.stage()) {
        Some(Stage::ContinuePrompt) => vec![crate::input::InputEvent::Char(b'N')],
        // ★ roll-credits slice 3: the fight now ends on `displayCombatResults`
        // and the pool screen, both of which block on a key exactly as the
        // original's do. Acknowledge the results, then Exit the pool.
        Some(Stage::Results) => vec![crate::input::InputEvent::Enter],
        Some(Stage::Treasure) => vec![crate::input::InputEvent::Char(b'E')],
        _ => Vec::new(),
    }
}

/// [`Engine::tick`] with [`auto_keys`] fed in.
fn tick<'a>(e: &'a mut Engine) -> crate::engine::Frame<'a> {
    let keys = auto_keys(e);
    e.tick(&keys)
}

/// What one driven tick observed.
#[derive(Default)]
struct Observed {
    transcript: Vec<String>,
    prints: Vec<String>,
}

fn drain(e: &mut Engine, into: &mut Observed) {
    for entry in e.take_transcript() {
        match entry {
            TranscriptEntry::Request(l) => into.transcript.push(l),
            TranscriptEntry::Print { text, .. } => into.prints.push(text),
        }
    }
}

/// The fight's **closing** line — the one `tick_combat` writes at ExitStage
/// completion. Matched on `round(s)` because the host also emits `combat:`
/// diagnostics mid-fight (a dropped key, a deferred wilderness floor).
fn combat_line(o: &Observed) -> Option<&String> {
    o.transcript
        .iter()
        .find(|l| l.starts_with("combat:") && l.contains("round(s)"))
}

// === 1. park/resume in every flow kind (§8.3 rule 1) =======================

/// Drives `e` until the vector's post-`COMBAT` PRINT lands, asserting the whole
/// way that the fight really parked (the shell variant never changes, and no
/// vector pumps while it is on screen). Returns what was observed.
fn park_and_resume(e: &mut Engine, marker: &str, expect: fn(&Shell) -> bool) -> Observed {
    let mut o = Observed::default();
    let mut ticks_parked = 0usize;
    let mut saw_park = false;
    for _ in 0..MAX_TICKS {
        tick(e);
        drain(e, &mut o);
        if e.shell().combat_host().is_some() {
            saw_park = true;
            ticks_parked += 1;
            assert!(
                expect(e.shell()),
                "the flow that owns the fight must stay exactly where it was"
            );
            assert!(
                e.shell().gate_open(),
                "no vector may pump while a fight is parked (D-UI7)"
            );
        }
        if o.prints.iter().any(|p| p.contains(marker)) {
            break;
        }
    }
    assert!(saw_park, "the fight parked in the vector run");
    assert!(
        ticks_parked > 30,
        "the fight was on screen for real time, not one tick: {ticks_parked}"
    );
    assert!(
        o.prints.iter().any(|p| p.contains(marker)),
        "the vector resumed after the fight (its post-COMBAT PRINT ran)"
    );
    o
}

#[test]
fn a_fight_parks_and_resumes_inside_the_boot_flow() {
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTER-BOOT"), two_pcs());
    let o = park_and_resume(&mut e, "AFTER-BOOT", |s| matches!(s, Shell::Boot(_)));
    assert!(combat_line(&o).is_some());
    // The cursor survived: Boot's remaining stage (the post-chain resume) still
    // ran, so the shell reached the world menu rather than stalling.
    for _ in 0..MAX_TICKS {
        if matches!(e.shell(), Shell::WorldMenu { .. }) {
            return;
        }
        tick(&mut e);
    }
    panic!("Boot never completed after the fight");
}

#[test]
fn a_fight_parks_and_resumes_inside_the_step_flow() {
    // Vector 0 fights; vector 1 (which `StepFlow` fires *after* the fight, past
    // the door interaction) prints its own marker. Seeing BOTH markers is the
    // cursor-survival proof: `StepStage::RunVector2` is only reachable from the
    // stage the fight suspended.
    let data = vectored_game_data(["step", "search", "unused", "unused", "boot"], |b| {
        fight_at(b, "step", 3, b"AFTER-STEP");
        b.label("search");
        b.op(0x11).inline_str(b"SEARCHED");
        b.op(0x00);
        exit_at(b, "unused");
        exit_at(b, "boot");
    });
    let mut e = engine_with(data, two_pcs());

    // Boot to the world menu, then step forward.
    for _ in 0..50 {
        tick(&mut e);
        if matches!(e.shell(), Shell::WorldMenu { .. }) {
            break;
        }
    }
    e.take_transcript();
    e.tick(&[crate::input::InputEvent::Ext(crate::input::ExtKey::Up)]);

    let o = park_and_resume(&mut e, "AFTER-STEP", |s| matches!(s, Shell::Step(_)));
    assert!(combat_line(&o).is_some());

    let mut o = o;
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        drain(&mut e, &mut o);
        if matches!(e.shell(), Shell::WorldMenu { .. }) {
            break;
        }
    }
    assert!(
        o.prints.iter().any(|p| p.contains("SEARCHED")),
        "StepFlow resumed at its own next stage and fired vector 1 — the \
         pre-fight cursor survived"
    );
}

#[test]
fn a_fight_parks_and_resumes_inside_the_look_flow() {
    let data = vectored_game_data(["step", "search", "unused", "unused", "boot"], |b| {
        exit_at(b, "step");
        fight_at(b, "search", 3, b"AFTER-LOOK");
        exit_at(b, "unused");
        exit_at(b, "boot");
    });
    let mut e = engine_with(data, two_pcs());
    for _ in 0..50 {
        tick(&mut e);
        if matches!(e.shell(), Shell::WorldMenu { .. }) {
            break;
        }
    }
    e.take_transcript();
    // `search_flags |= 2` is set by the 'L' handler and restored by LookFlow's
    // own last stage — which only runs if the cursor survived the fight.
    e.tick(&[crate::input::InputEvent::Char(b'l')]);

    let o = park_and_resume(&mut e, "AFTER-LOOK", |s| matches!(s, Shell::Look(_)));
    assert!(combat_line(&o).is_some());
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        if matches!(e.shell(), Shell::WorldMenu { .. }) {
            break;
        }
    }
    assert_eq!(
        e.state().search_flags,
        0,
        "LookFlow's search-flag restore ran after the fight (rule 1)"
    );
}

#[test]
fn a_fight_parks_and_resumes_inside_a_chain_round() {
    // Block 1's entry vector chains to block 9, whose entry vector fights. The
    // fight therefore parks inside a `ChainRunner`'s vector run, the one flow
    // shape that is not a plain `BootFlow`/`StepFlow`/`LookFlow` run.
    let mut chain = EclBuilder::new();
    for _ in 0..5 {
        chain.raw(&[0]);
        chain.imm_word_label("entry");
    }
    chain.label("entry");
    chain.op(0x20).imm_byte(9); // NEWECL block 9

    let mut fight = EclBuilder::new();
    for _ in 0..5 {
        fight.raw(&[0]);
        fight.imm_word_label("entry");
    }
    fight_at(&mut fight, "entry", 3, b"AFTER-CHAIN");

    let ecl = build_dax_file(&[
        (INITIAL_ECL_BLOCK, ecl_dax_block(&chain.build_bytes())),
        (9, ecl_dax_block(&fight.build_bytes())),
    ]);
    let goblin = char_record(b"GOBLIN", 3, 10, 20, 6, true);
    let data = GameData::from_files([
        (format!("ECL{GAME_AREA}.DAX"), ecl),
        (
            format!("MON{GAME_AREA}CHA.DAX"),
            build_dax_file(&[(0u8, goblin)]),
        ),
    ]);
    let mut e = engine_with(data, two_pcs());

    let o = park_and_resume(&mut e, "AFTER-CHAIN", |s| matches!(s, Shell::Boot(_)));
    assert!(combat_line(&o).is_some());
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        if matches!(e.shell(), Shell::WorldMenu { .. }) {
            break;
        }
    }
    assert!(
        !e.state().chained,
        "the chain runner finished after the fight — its cursor survived"
    );
    assert_eq!(e.state().ecl_block_id, 9);
}

// === 2. the GameOver deferral (§8.2's MUST) ================================

#[test]
fn a_wiped_partys_final_beats_all_play_before_the_game_over_unwind() {
    // ★ The rendered twin of `shell.rs`'s
    // `party_killed_unwinds_to_game_over_and_resets_the_flag`.
    //
    // `Shell::tick` replaces the shell with `GameOver` at top-of-tick the
    // instant `party_killed` is set. So the ordering §8.2 demands is: the
    // outcome becomes known inside `Fighting`, the fight keeps presenting
    // (ExitStage's beats + the screen restore), and only when ExitStage
    // COMPLETES does `party_killed` go up — one whole tick before the unwind.
    let mut e = engine_with_program(load_then_combat_program(4, b"UNREACHED"), doomed_pcs());

    let mut outcome_known_at: Option<usize> = None;
    let mut flag_set_at: Option<usize> = None;
    let mut game_over_at: Option<usize> = None;
    let mut stages_after_outcome: Vec<Stage> = Vec::new();

    for tick_no in 0..MAX_TICKS {
        tick(&mut e);
        if let Some(host) = e.shell().combat_host() {
            if host.outcome().is_some() {
                if outcome_known_at.is_none() {
                    outcome_known_at = Some(tick_no);
                    assert_eq!(
                        host.outcome(),
                        Some(crate::combat::CombatOutcome::MonstersWin)
                    );
                }
                stages_after_outcome.push(host.stage().clone());
                assert!(
                    !e.state().party_killed,
                    "tick {tick_no}: party_killed must stay down while the fight is \
                     still presenting — setting it here annihilates the final beats"
                );
            }
        }
        if e.state().party_killed && flag_set_at.is_none() {
            flag_set_at = Some(tick_no);
        }
        if matches!(e.shell(), Shell::GameOver(_)) {
            game_over_at = Some(tick_no);
            break;
        }
    }

    let outcome_known_at = outcome_known_at.expect("the party was wiped");
    let flag_set_at = flag_set_at.expect("a wipe sets party_killed");
    let game_over_at = game_over_at.expect("party_killed unwinds to GameOver");

    assert!(
        flag_set_at > outcome_known_at,
        "the flag went up on the very tick the outcome was known ({flag_set_at} \
         vs {outcome_known_at}) — the deferral is not in place"
    );
    assert!(
        game_over_at > flag_set_at,
        "GameOver must arrive on a LATER tick than the write ({game_over_at} vs \
         {flag_set_at}), i.e. after the restored screen was presented"
    );
    // Every ExitStage beat was reached, in order, with the outcome already known.
    assert!(
        stages_after_outcome
            .iter()
            .any(|s| matches!(s, Stage::FinalBeats { .. })),
        "the final-beats hold played: {stages_after_outcome:?}"
    );
    assert!(
        stages_after_outcome.contains(&Stage::Restore),
        "the screen restore ran before the writes: {stages_after_outcome:?}"
    );
    assert!(
        stages_after_outcome.len() > 10,
        "the ending was on screen for real time, not one tick: {}",
        stages_after_outcome.len()
    );
}

// === 3. a parked fight round-trips (D-CV7) =================================

#[test]
fn a_parked_fight_round_trips_through_serde_and_keeps_fighting() {
    // D-CV7 by construction: a fight parked in `VmPhase::Combat` is inside the
    // serde-derived `Shell`, so it must encode. The scene and the event buffer
    // are `#[serde(skip)]` and rebuilt on the next tick — which this proves by
    // running the *restored* fight to its end and getting the same outcome, and
    // the same draw stream, as the run that was never interrupted.
    let control = run_fight_to_the_end(None);
    let restored = run_fight_to_the_end(Some(40));

    assert_eq!(
        restored.combat_line, control.combat_line,
        "the restored fight ended the same way"
    );
    assert_eq!(
        restored.draws, control.draws,
        "a snapshot/restore mid-fight perturbs nothing: the PRNG lives on the \
         engine, and rebuilding the scene draws no dice"
    );
    assert!(
        restored.round_tripped,
        "the mid-fight snapshot really happened"
    );
}

struct FightRun {
    combat_line: String,
    draws: Vec<RngDraw>,
    round_tripped: bool,
}

/// Runs the fixture fight to its end. With `snapshot_after` set, the shell is
/// serialized and restored once, that many ticks into the fight.
fn run_fight_to_the_end(snapshot_after: Option<usize>) -> FightRun {
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTERWARD"), two_pcs());
    let draws = Draws::default();
    e.attach_rng_sink(draws.sink());

    let mut fighting_ticks = 0usize;
    let mut round_tripped = false;
    let mut o = Observed::default();
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        drain(&mut e, &mut o);
        if e.shell().combat_host().is_some() {
            fighting_ticks += 1;
            if snapshot_after == Some(fighting_ticks) {
                let json = serde_json::to_string(e.shell()).expect("a parked fight serializes");
                let back: Shell = serde_json::from_str(&json).expect("and deserializes");
                assert!(
                    back.combat_host().is_some(),
                    "the fight survived the round trip"
                );
                assert!(
                    back.combat_host().unwrap().scene().is_none(),
                    "the scene is skipped, to be rebuilt on the next tick"
                );
                e.shell = back;
                round_tripped = true;
            }
        }
        if let Some(line) = combat_line(&o) {
            return FightRun {
                combat_line: line.clone(),
                draws: draws.taken(),
                round_tripped,
            };
        }
    }
    panic!("the fixture fight never ended");
}

#[test]
fn a_fight_parked_on_a_players_turn_round_trips_with_its_menus() {
    // ★ **M6c's half of D-CV7**: the state a *suspended manual turn* adds —
    // the open `ManualTurn` in the core, the `ManualUi` in the host — is inside
    // the same serde-derived `Shell`, so a fight waiting on a keypress
    // snapshots by construction. (Nothing player-facing can reach a save from
    // here: the combat menu builds no Save word, `ovr009.cs:313-360`. The
    // obligation is the type system's, and the inspector's debug pane.)
    let mut e = engine_with_program(load_then_combat_program(1, b"AFTERWARD"), manual_pcs());
    for _ in 0..MAX_TICKS {
        e.tick(&[]);
        if e.shell()
            .combat_host()
            .is_some_and(|h| matches!(h.stage(), Stage::PlayerTurn))
        {
            break;
        }
    }
    let host = e.shell().combat_host().expect("a manual turn opened");
    let actor = host.manual().expect("with menus").actor();
    let prompt = host
        .scene()
        .and_then(|s| s.prompt())
        .expect("the menu is on the prompt row")
        .to_string();

    let json = serde_json::to_string(e.shell()).expect("a suspended turn serializes");
    e.shell = serde_json::from_str(&json).expect("and deserializes");

    let back = e.shell().combat_host().expect("the fight survived");
    assert!(
        matches!(back.stage(), Stage::PlayerTurn),
        "still the player's"
    );
    assert_eq!(
        back.manual().map(|u| u.actor()),
        Some(actor),
        "and still the same player's"
    );
    assert_eq!(
        back.state().manual_turn().map(|m| m.actor()),
        Some(actor),
        "the core's own suspension came back too"
    );
    assert!(back.state().is_interactive());

    // The next tick rebuilds the scene and puts the menu back on the row.
    e.tick(&[]);
    assert_eq!(
        e.shell()
            .combat_host()
            .and_then(|h| h.scene())
            .and_then(|s| s.prompt()),
        Some(prompt.as_str()),
        "the same words, redrawn from the restored state"
    );
    // And it still plays: hand the turn to the AI and let the fight finish.
    let mut o = Observed::default();
    for _ in 0..MAX_TICKS {
        let keys: Vec<crate::input::InputEvent> = match e.shell().combat_host().map(|h| h.stage()) {
            Some(Stage::PlayerTurn) => vec![crate::input::InputEvent::Char(b'Q')],
            Some(Stage::ContinuePrompt) => vec![crate::input::InputEvent::Char(b'N')],
            // The post-fight award screens (roll-credits slice 3).
            Some(Stage::Results) => vec![crate::input::InputEvent::Enter],
            Some(Stage::Treasure) => vec![crate::input::InputEvent::Char(b'E')],
            _ => Vec::new(),
        };
        e.tick(&keys);
        drain(&mut e, &mut o);
        if combat_line(&o).is_some() {
            break;
        }
    }
    assert!(
        combat_line(&o).is_some(),
        "the restored fight played on to its end"
    );
}

#[test]
fn a_restored_fight_rebuilds_its_scene_and_renders_again() {
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTERWARD"), two_pcs());
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        if e.shell()
            .combat_host()
            .is_some_and(|h| matches!(h.stage(), Stage::Fighting))
        {
            break;
        }
    }
    let json = serde_json::to_string(e.shell()).unwrap();
    e.shell = serde_json::from_str(&json).unwrap();
    assert!(e.shell().combat_host().unwrap().scene().is_none());
    tick(&mut e);
    assert!(
        e.shell().combat_host().is_some_and(|h| h.scene().is_some()),
        "the first tick after a restore rebuilds the presenter"
    );
}

// === 4. ★ the shell-path draw-parity invariant (§8.3 rule 4) ===============

#[test]
fn the_shell_driven_fight_draws_exactly_what_the_headless_one_draws() {
    // ★ The state chart's own parity test. D-CV8's invariant says a scene-driven
    // fight and a headless one produce identical `RngDraw` streams; §8.3 rule 4
    // extends it to the *shell* path, where the fight is additionally wrapped in
    // a suspended `VectorRun`, a tick clock, live input drains, and a full
    // screen repaint every frame.
    //
    // The fork point is the moment the host first reaches `Fighting`: the fight
    // is assembled (floor dice spent, first `step()` taken), so cloning the
    // `CombatState` and the engine's PRNG state there gives a headless twin of
    // exactly the remaining fight.
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTERWARD"), two_pcs());
    let draws = Draws::default();
    e.attach_rng_sink(draws.sink());

    let mut fork: Option<(CombatState, u32, usize)> = None;
    let mut o = Observed::default();
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        drain(&mut e, &mut o);
        if fork.is_none() {
            if let Some(host) = e.shell().combat_host() {
                if matches!(host.stage(), Stage::Fighting) {
                    // A boundary read: the first `Fighting` tick happens before
                    // any further `step()`, so this is a step boundary.
                    fork = Some((host.state().clone(), e.prng_state(), draws.len()));
                }
            }
        }
        if combat_line(&o).is_some() {
            break;
        }
    }

    let (forked_state, forked_rng, forked_at) = fork.expect("the fight reached its Fighting stage");
    let shell_draws = draws.taken();
    assert!(
        shell_draws.len() - forked_at > 100,
        "the fight after the fork is substantial: {} draws",
        shell_draws.len() - forked_at
    );

    // The headless twin: the same state, the same PRNG, `while step != Ended`.
    let headless = Draws::default();
    let mut rng = EngineRng::new(0);
    rng.set_state(forked_rng);
    rng.attach_sink(headless.sink());
    let mut state = forked_state;
    // ★ M6c: the fork carries the host's interactive flag, and a headless
    // driver cannot answer D-CV5's suspensions. Turning it off is not a
    // difference between the twins — it swaps the *source* of the
    // Continue-Battle answer from the keyboard ('N', which `auto_keys` presses)
    // to the empty schedule (also 'N'). Every draw either side of it is the
    // same, which is exactly what this test then proves.
    state.set_interactive(false);
    let mut steps = 0;
    while state.step(&mut rng) != CombatStep::Ended {
        steps += 1;
        assert!(steps < 10_000, "the headless twin must end");
    }
    let headless_draws = headless.taken();

    let shell_tail = &shell_draws[forked_at..];
    assert_eq!(
        shell_tail.len(),
        headless_draws.len(),
        "the shell-driven fight drew a different NUMBER of dice than the \
         headless one — presentation touched the PRNG"
    );
    for (i, (a, b)) in shell_tail.iter().zip(&headless_draws).enumerate() {
        assert_eq!(
            a, b,
            "draw {i} after the fork diverged: shell {a:?} vs headless {b:?}"
        );
    }

    // ...and the fights ended the same way.
    assert_eq!(
        state.outcome(),
        crate::combat::CombatOutcome::PartyWins,
        "the headless twin reached the same verdict the transcript recorded"
    );
    assert!(combat_line(&o).unwrap().contains("party wins"));
}

#[test]
fn the_floor_dice_are_the_only_draws_before_the_fights_first_step() {
    // The other half of the parity story: what the shell path spends BEFORE the
    // fight proper. Since M6 slice 6 that is `SetupDungeonFloor`'s furniture
    // d10s and nothing else — placement, encounter distance and the kit
    // derivation are all draw-free, so the §2 initiative fingerprint still
    // follows immediately.
    //
    // The fixture's GEO is open, so its floor spends zero dice and the very
    // first draw is an initiative d6 (`combat_wiring` asserts that shape). Here
    // the same program runs over a GEO with a walled, roofed room at the party's
    // square — the floor now rolls, and the fingerprint still starts right after.
    let mut geo = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
    let (x, y) = (8usize, 8usize);
    geo[2 + x + 16 * y] = (1 << 4) | 1; // north + east walls
    geo[2 + 256 + x + 16 * y] = (1 << 4) | 1; // south + west walls
    geo[2 + 2 * 256 + x + 16 * y] = 0x40; // the x2 furniture bit
    let geo = gbx_formats::geo::GeoBlock::parse(&geo).unwrap();

    let mut sets = crate::symbols::SymbolSets::new();
    sets.load(4, synthetic_set4());
    let data = combat_game_data(load_then_combat_program(3, b"AFTERWARD"));
    let mut e = Engine::new_fixture(synthetic_font(), sets, geo, data, 1);
    e.party = crate::party::Party { members: two_pcs() };
    e.state.pos = (x as u8, y as u8);

    let draws = Draws::default();
    e.attach_rng_sink(draws.sink());
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        if e.shell()
            .combat_host()
            .is_some_and(|h| matches!(h.stage(), Stage::Fighting))
        {
            break;
        }
    }
    // The first `Fighting` tick has taken the round-1 initiative step only; a
    // few more ticks reach the first selection pass.
    for _ in 0..600 {
        tick(&mut e);
        if draws.len() > 16 {
            break;
        }
    }
    let taken = draws.taken();
    let floor_dice = taken.iter().take_while(|d| d.n == Some(10)).count();
    assert!(
        floor_dice > 0,
        "a walled, roofed room rolls furniture dice: {:?}",
        taken.iter().take(8).map(|d| d.n).collect::<Vec<_>>()
    );
    // 2 party + 3 monsters = 5 initiative d6s, then the d100 selection pass —
    // the §2 fingerprint, intact, immediately after the floor.
    for (i, d) in taken[floor_dice..floor_dice + 5].iter().enumerate() {
        assert_eq!(
            d.n,
            Some(6),
            "draw #{i} after the floor must be an initiative d6, got {:?}",
            d.n
        );
    }
    assert_eq!(
        taken[floor_dice + 5].n,
        Some(100),
        "the d100 selection pass follows: {:?}",
        taken.iter().map(|d| d.n).collect::<Vec<_>>()
    );
}

// === the M6b done-condition, in miniature =================================

#[test]
fn the_fight_reaches_the_screen_and_gives_it_back() {
    // What "on screen" means mechanically: the combat scene paints the
    // framebuffer while the fight runs (the frame changes from the exploration
    // view), and the exploration view is restored before the VM resumes.
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTERWARD"), two_pcs());
    let mut before_fight = None;
    let mut during_fight = None;
    let mut after_fight = None;

    let mut o = Observed::default();
    for _ in 0..MAX_TICKS {
        let hash = tick(&mut e).hash_hex();
        drain(&mut e, &mut o);
        match e.shell().combat_host().map(|h| h.stage().clone()) {
            None if before_fight.is_none() => before_fight = Some(hash),
            Some(Stage::Fighting) => during_fight = Some(hash),
            None if combat_line(&o).is_some() && after_fight.is_none() => after_fight = Some(hash),
            _ => {}
        }
        if after_fight.is_some() {
            break;
        }
    }

    let before = before_fight.expect("a pre-fight frame");
    let during = during_fight.expect("a mid-fight frame");
    let after = after_fight.expect("a post-fight frame");
    assert_ne!(
        before, during,
        "the combat screen replaced the exploration one"
    );
    assert_ne!(during, after, "and the exploration screen came back");
}

/// A party that fights **manually** — `quick_fight` cleared, the way a save
/// whose player never pressed Quick carries it.
fn manual_pcs() -> Vec<crate::party::Character> {
    two_pcs()
        .into_iter()
        .map(|mut c| {
            c.status.quick_fight = 0;
            c
        })
        .collect()
}

#[test]
fn a_manual_fight_is_played_from_the_menus_and_won() {
    // ★ **M6c's done-condition, in CI form** (§4): every party turn opens the
    // combat menu, a scripted player aims and commits from it, and the fight is
    // won by hand — through the same `attack_target` the AI swings with.
    let mut e = engine_with_program(load_then_combat_program(1, b"AFTERWARD"), manual_pcs());
    let mut o = Observed::default();
    let mut menus_opened = 0usize;
    let mut last_stage: Option<Stage> = None;
    for _ in 0..MAX_TICKS {
        let stage = e.shell().combat_host().map(|h| h.stage().clone());
        if matches!(stage, Some(Stage::PlayerTurn)) && last_stage != stage {
            menus_opened += 1;
        }
        last_stage = stage.clone();
        let keys: Vec<crate::input::InputEvent> = match stage {
            Some(Stage::PlayerTurn) => crate::demo::scripted_player_key(&e).into_iter().collect(),
            Some(Stage::ContinuePrompt) => vec![crate::input::InputEvent::Char(b'N')],
            // The post-fight award screens (roll-credits slice 3).
            Some(Stage::Results) => vec![crate::input::InputEvent::Enter],
            Some(Stage::Treasure) => vec![crate::input::InputEvent::Char(b'E')],
            _ => Vec::new(),
        };
        e.tick(&keys);
        drain(&mut e, &mut o);
        if combat_line(&o).is_some() {
            break;
        }
    }
    assert!(menus_opened > 0, "the party's turns opened the combat menu");
    let line = combat_line(&o).expect("the fight ended and reported itself");
    assert!(
        line.contains("party wins"),
        "a hand-played fight was won: {line:?}"
    );
    assert!(
        !o.transcript.iter().any(|l| l.contains("refused")),
        "no command the menus offered was refused by the core: {:?}",
        o.transcript
    );
}

/// ★ Regression, found live 2026-08-08 while wiring the ally prompt: pressing
/// `A` opened Aim with an **empty** scan list. `ManualUi::key` builds the
/// `AimState` but only `refresh` fills `aim.list` from `copy_sorted_players`
/// (`ovr014.cs:2073`), and the host refreshed only after a `MenuAction::Issue`
/// — so `Next`/`Prev` had nothing to walk, the cursor sat on the actor
/// forever, and §9.4's whole list half was unusable from the keyboard. The
/// unit tests passed throughout because they call `refresh` themselves.
#[test]
fn opening_aim_fills_the_scan_list() {
    let mut e = engine_with_program(load_then_combat_program(1, b"AFTERWARD"), manual_pcs());
    let mut opened = false;
    for _ in 0..MAX_TICKS {
        if matches!(
            e.shell().combat_host().map(|h| h.stage()),
            Some(Stage::PlayerTurn)
        ) {
            opened = true;
            break;
        }
        e.tick(&[]);
    }
    assert!(opened, "a manual party turn opened");

    // The key may wait a tick or two while the last batch finishes playing
    // (the D-CV2 lockstep rule); the point is that it never needs a *command*
    // to load the list.
    e.tick(&[crate::input::InputEvent::Char(b'A')]);
    for _ in 0..8 {
        e.tick(&[]);
    }
    let host = e.shell().combat_host().expect("still parked on the fight");
    let ui = host.manual().expect("the menus are open");
    assert!(
        ui.aim_target().is_some(),
        "Aim must open with copy_sorted_players' list already loaded"
    );
}

#[test]
fn space_during_a_fight_hands_the_next_turn_to_the_player() {
    // ★ M6c: SPACE is a real key at last (`process_input_in_monsters_turn`,
    // `ovr010.cs:729-743`). Pressed while the AI fights, it revokes auto-fight
    // for every player-controlled combatant, and the next party turn opens the
    // combat menu instead of running itself.
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTERWARD"), two_pcs());
    let mut o = Observed::default();
    let mut pressed = false;
    let mut saw_menu = false;
    for _ in 0..MAX_TICKS {
        let input: Vec<crate::input::InputEvent> = match e.shell().combat_host().map(|h| h.stage())
        {
            Some(Stage::Fighting) if !pressed => {
                pressed = true;
                vec![crate::input::InputEvent::Char(b' ')]
            }
            // The menu is open: its words are on the prompt row. `Quick`
            // hands this turn — and every later one — back to the AI
            // (`SetPlayerQuickFight` + `PlayerQuickFight`, `ovr009.cs:175`),
            // which is how a player who pressed SPACE by accident recovers.
            Some(Stage::PlayerTurn) => {
                if !saw_menu {
                    let host = e.shell().combat_host().expect("parked");
                    let prompt = host
                        .scene()
                        .and_then(|s| s.prompt())
                        .unwrap_or_default()
                        .to_string();
                    assert!(
                        prompt.contains("View Aim") && prompt.ends_with("Quick Done"),
                        "the combat menu is on the prompt row: {prompt:?}"
                    );
                    saw_menu = true;
                }
                vec![crate::input::InputEvent::Char(b'Q')]
            }
            Some(Stage::ContinuePrompt) => vec![crate::input::InputEvent::Char(b'N')],
            // The post-fight award screens (roll-credits slice 3).
            Some(Stage::Results) => vec![crate::input::InputEvent::Enter],
            Some(Stage::Treasure) => vec![crate::input::InputEvent::Char(b'E')],
            _ => Vec::new(),
        };
        e.tick(&input);
        drain(&mut e, &mut o);
        if combat_line(&o).is_some() {
            break;
        }
    }
    assert!(pressed, "SPACE was pressed during an AI turn");
    assert!(saw_menu, "the next party turn opened the combat menu");
    assert!(
        combat_line(&o).is_some_and(|l| !l.contains("dropped")),
        "and nothing was dropped: {:?}",
        combat_line(&o)
    );
}

#[test]
fn the_two_key_toggles_auto_magic_at_a_step_head() {
    // '2' (`ovr010.cs:718-730`) works from M6b: a flag flip the next turn's
    // spell gate reads. It lands at step heads only (the D-CV2 lockstep rule).
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTERWARD"), two_pcs());
    let mut o = Observed::default();
    let mut pressed = 0usize;
    for _ in 0..MAX_TICKS {
        let input: &[crate::input::InputEvent] = if e.shell().combat_host().is_some() {
            &[crate::input::InputEvent::Char(b'2')]
        } else {
            &[]
        };
        e.tick(input);
        drain(&mut e, &mut o);
        pressed += usize::from(o.prints.iter().any(|p| p == "Magic On" || p == "Magic Off"));
        if combat_line(&o).is_some() {
            break;
        }
    }
    assert!(pressed > 0, "the toggle reported itself: {:?}", o.prints);
}

// === the roster the fight actually assembles ==============================

#[test]
fn the_live_party_fights_with_its_own_record_not_a_placeholder_die() {
    // D-CV6 item 2, end to end through the shell: the party's combatants come
    // from its real records (`combat::kits`), so the damage dice are the
    // record's — not the retired `DEFAULT_PARTY_WEAPON_DIE` 1d8.
    let mut party = two_pcs();
    for c in &mut party {
        c.combat.attacks.current[2] = 3; // 3d4+1, a die no placeholder ever had
        c.combat.attacks.current[4] = 4;
        c.combat.attacks.current[6] = 1;
    }
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTERWARD"), party);
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        if let Some(host) = e.shell().combat_host() {
            if matches!(host.stage(), Stage::Fighting) {
                let roster = host.state().roster();
                assert_eq!(roster[0].name_dice(), (3, 4, 1));
                assert_eq!(roster[1].name_dice(), (3, 4, 1));
                assert!(roster[0].pos.in_bounds());
                assert!(roster.len() >= 5, "party + monsters");
                assert_eq!(roster[0].health_status, HealthStatus::Okey);
                return;
            }
        }
    }
    panic!("the fight never started");
}

impl crate::combat::Combatant {
    /// Test-local shorthand for the attack-1 profile triple.
    fn name_dice(&self) -> (u8, u8, u8) {
        (self.dice_count, self.dice_size, self.damage_bonus)
    }
}

#[test]
fn the_monsters_carry_the_icon_slot_and_cpic_block_loadmonster_gave_them() {
    // The identities the scene draws from: names off the records, icon slots
    // off `gbl.monster_icon_id`, CPIC blocks off LOAD MONSTER's third operand.
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTERWARD"), two_pcs());
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        if let Some(host) = e.shell().combat_host() {
            if matches!(host.stage(), Stage::Fighting) {
                let roster = host.state().roster();
                assert_eq!(roster.len(), 5);
                // Positions are `PlaceCombatants`' — everyone found a cell.
                for c in roster {
                    assert!(c.pos.in_bounds(), "combatant {} placed", c.id);
                }
                return;
            }
        }
    }
    panic!("the fight never started");
}

#[test]
fn a_fight_with_no_living_party_reports_itself_and_still_replies() {
    // The degenerate case a live game can reach (every member at 0 HP): the
    // fight refuses, says so, and the VM still gets its `Reply::Combat` so the
    // script goes on rather than hanging.
    let mut dead = two_pcs();
    for c in &mut dead {
        c.hit_point_current = 0;
    }
    let mut e = engine_with_program(load_then_combat_program(3, b"AFTERWARD"), dead);
    let mut o = Observed::default();
    for _ in 0..MAX_TICKS {
        tick(&mut e);
        drain(&mut e, &mut o);
        if o.prints.iter().any(|p| p.contains("AFTERWARD")) {
            break;
        }
    }
    assert!(
        o.transcript.iter().any(|l| l.contains("no living party")),
        "the refusal names itself: {:?}",
        o.transcript
    );
    assert!(
        o.prints.iter().any(|p| p.contains("AFTERWARD")),
        "and the script still resumed"
    );
}

#[test]
fn placement_uses_the_areas_real_walls() {
    // `place_combatants`' area-wall hook stopped being stubbed open-ground in
    // slice 6. Proof that the real flags reach it: the same encounter staged at
    // the map's edge — where every off-grid probe reads as a wall — deploys the
    // roster differently from one staged inland.
    fn first_monster_cell(pos: (u8, u8)) -> GridPos {
        let mut sets = crate::symbols::SymbolSets::new();
        sets.load(4, synthetic_set4());
        let data = combat_game_data(load_then_combat_program(3, b"AFTERWARD"));
        let mut e = Engine::new_fixture(synthetic_font(), sets, open_geo(), data, 1);
        e.party = crate::party::Party { members: two_pcs() };
        e.state.pos = pos;
        // As `SETUP MONSTER` would have armed it — see `engine_with_program`.
        e.state.encounter_distance = 2;
        for _ in 0..MAX_TICKS {
            tick(&mut e);
            if let Some(host) = e.shell().combat_host() {
                if matches!(host.stage(), Stage::Fighting) {
                    return host.state().roster()[2].pos;
                }
            }
        }
        panic!("the fight never started at {pos:?}");
    }
    assert_ne!(
        first_monster_cell((0, 0)),
        first_monster_cell((8, 8)),
        "the fan-out saw the map edge"
    );
}
