//! ★ **The draw-parity invariant** (`docs/design/combat-visualizer.md` D-CV8)
//! — the single most load-bearing test in M6.
//!
//! D-CV2's hard invariant is that *presentation never touches the PRNG and
//! never perturbs the draw stream*. Three proof obligations back it: the
//! frontier guard stays 15/15 (every commit), the M6a reel asserts capture
//! draw-equality while rendering (slice 5), and **this** — one synthetic fight
//! run twice, headless and through a fully-driven [`CombatScene`], asserting
//! the two produce identical `RngDraw` streams and identical final rosters.
//!
//! What "fully driven" means here is the point. The scene side does everything
//! a real host does and then some:
//!
//! - attaches an [`ActionSink`] and buffers each `step()`'s batch (D-CV2);
//! - composes and **plays that batch to completion, one tick at a time**,
//!   before calling `step()` again (the lockstep invariant);
//! - reconciles the presented board and refreshes the panel cache against the
//!   real `CombatState` at every step boundary;
//! - runs the whole thing at three different `game_speed_var` settings, and
//!   once with a fast-drain instead of ticks, so neither pacing nor skipping
//!   can be what keeps the streams equal.
//!
//! A regression this catches that nothing else does: a scene that reads the
//! live roster mid-playback would still *look* right, and the frontier guard
//! would still pass (the guard never constructs a scene) — but any future beat
//! that rolls a die, or any reconciliation that "fixes" state by writing back
//! into `CombatState`, shows up here as a diverged stream.

use super::{CombatScene, CombatantIdentity, EntrySnapshot, SceneArt};
use crate::combat::{
    ActionEvent, ActionSink, CombatMap, CombatState, CombatStep, Combatant, GridPos, Team,
    TurnDriver,
};
use crate::rng::{EngineRng, RngDraw, RngSink};
use std::cell::RefCell;
use std::rc::Rc;

const SEED: u32 = 0x0C0F_FEE0;
const FLOOR: u8 = 0x17;

#[derive(Clone, Default)]
struct Draws(Rc<RefCell<Vec<RngDraw>>>);
struct DrawTap(Rc<RefCell<Vec<RngDraw>>>);
impl RngSink for DrawTap {
    fn on_draw(&mut self, draw: RngDraw) {
        self.0.borrow_mut().push(draw);
    }
}
impl Draws {
    fn sink(&self) -> Box<dyn RngSink> {
        Box::new(DrawTap(Rc::clone(&self.0)))
    }
    fn taken(&self) -> Vec<RngDraw> {
        self.0.borrow().clone()
    }
}

/// Buffers one `step()`'s events, D-CV2's render feed.
#[derive(Clone, Default)]
struct Batch(Rc<RefCell<Vec<ActionEvent>>>);
struct BatchSink(Rc<RefCell<Vec<ActionEvent>>>);
impl ActionSink for BatchSink {
    fn on_action(&mut self, event: ActionEvent) {
        self.0.borrow_mut().push(event);
    }
}
impl Batch {
    fn sink(&self) -> Box<dyn ActionSink> {
        Box::new(BatchSink(Rc::clone(&self.0)))
    }
    fn take(&self) -> Vec<ActionEvent> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

/// The fight both runs drive: a party of three against three monsters on open
/// floor, close enough to close and trade, far enough that some turns are
/// spent walking. Hand-authored (D10) — no game data anywhere near it.
fn synthetic_fight() -> CombatState {
    let mut fighters: Vec<Combatant> = Vec::new();
    for (i, (team, x, y, hp, ac, hit, dice)) in [
        (Team::Party, 20, 11, 18, 5, 16, (1, 8, 1)),
        (Team::Party, 20, 12, 22, 4, 15, (2, 4, 0)),
        (Team::Party, 20, 13, 14, 6, 17, (1, 6, 0)),
        (Team::Monster, 26, 11, 12, 7, 18, (1, 6, 0)),
        (Team::Monster, 26, 12, 10, 7, 18, (1, 4, 1)),
        (Team::Monster, 26, 13, 11, 6, 17, (1, 6, 0)),
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
    let mut state = CombatState::new(CombatMap::uniform(FLOOR), fighters);
    state.turn = TurnDriver::MeleeAi;
    state
}

/// One combatant's final state, presentation-independent.
type Fingerprint = (usize, GridPos, i32, bool, u8);
/// A whole run's result: the draws it took and where the roster ended.
type RunResult = (Vec<RngDraw>, Vec<Fingerprint>);

/// A final-state fingerprint that does not depend on presentation at all.
fn roster_fingerprint(state: &CombatState) -> Vec<Fingerprint> {
    state
        .roster()
        .iter()
        .map(|c| {
            (
                c.id,
                c.pos,
                c.hp_current,
                c.in_combat,
                c.health_status as u8,
            )
        })
        .collect()
}

/// The headless reference — exactly what `run_combat` does today.
fn run_headless() -> RunResult {
    let draws = Draws::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(draws.sink());
    let mut state = synthetic_fight();
    let mut steps = 0;
    while state.step(&mut rng) != CombatStep::Ended {
        steps += 1;
        assert!(steps < 5_000, "the synthetic fight must end");
    }
    (draws.taken(), roster_fingerprint(&state))
}

/// How the scene run consumes each step's playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drive {
    /// One tick at a time, to completion — the faithful host.
    Ticks,
    /// The D-CV3 fast drain, on every step.
    Skip,
}

/// The scene-driven run: the same fight, with a scene composed, played and
/// reconciled around every single `step()`.
fn run_scene(speed: u8, drive: Drive) -> RunResult {
    let draws = Draws::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(draws.sink());

    let batch = Batch::default();
    let mut state = synthetic_fight();
    state.attach_action_sink(batch.sink());

    // D-CV2's entry snapshot is read after the FIRST step (the camera
    // initializes lazily inside `combat_setup` on that call).
    let first = state.step(&mut rng);
    assert_ne!(first, CombatStep::Ended);
    let identities: Vec<CombatantIdentity> = (0..state.roster().len())
        .map(|i| CombatantIdentity::new(format!("C{i}"), i))
        .collect();
    let mut scene = CombatScene::new(
        EntrySnapshot::from_state(&state, &identities),
        SceneArt::default(),
    );
    scene.set_game_speed(speed);
    // The art store is empty, which is fine and deliberate: an unloaded icon
    // slot draws nothing (`ovr034.cs:92`), so the whole schedule still plays —
    // this test is about the draw *stream*, not about pixels.
    scene.refresh_panels(&state);
    scene.reconcile(&state).expect("the entry snapshot matches");

    let mut sounds = 0usize;
    let mut steps = 0;
    loop {
        // Present step N to completion BEFORE computing N+1 (the lockstep
        // invariant). Nothing here may touch `rng`.
        scene.begin_step(&batch.take());
        match drive {
            Drive::Ticks => {
                let mut ticks = 0;
                while scene.is_playing() {
                    sounds += scene.tick(1).len();
                    ticks += 1;
                    assert!(ticks < 100_000, "a step's playback must drain");
                }
            }
            Drive::Skip => sounds += scene.skip().len(),
        }
        // Boundary reads, the only two the scene is allowed.
        scene
            .reconcile(&state)
            .unwrap_or_else(|e| panic!("presented board drifted at step {steps}: {e:?}"));
        scene.refresh_panels(&state);

        steps += 1;
        assert!(steps < 5_000, "the synthetic fight must end");
        if state.step(&mut rng) == CombatStep::Ended {
            break;
        }
    }
    // Drain the final step's batch so the scene ends where the fight does.
    scene.begin_step(&batch.take());
    scene.skip();
    scene.reconcile(&state).expect("the final board reconciles");

    assert!(sounds > 0, "the fight produced sound cues");
    (draws.taken(), roster_fingerprint(&state))
}

#[test]
fn the_scene_driven_fight_draws_exactly_what_the_headless_one_draws() {
    let (headless_draws, headless_roster) = run_headless();
    assert!(
        headless_draws.len() > 100,
        "the fixture fight is substantial: {} draws",
        headless_draws.len()
    );

    for speed in [0u8, 4, 9] {
        let (scene_draws, scene_roster) = run_scene(speed, Drive::Ticks);
        assert_eq!(
            scene_draws.len(),
            headless_draws.len(),
            "speed {speed}: the scene run drew a different NUMBER of dice"
        );
        // Compared draw-for-draw so a divergence names its index rather than
        // just failing on the vectors.
        for (i, (a, b)) in headless_draws.iter().zip(&scene_draws).enumerate() {
            assert_eq!(a, b, "speed {speed}: draw {i} diverged");
        }
        assert_eq!(
            scene_roster, headless_roster,
            "speed {speed}: the fight ended differently"
        );
    }
}

#[test]
fn a_fast_drained_fight_draws_the_same_stream_too() {
    // D-CV3's skip door: collapsing every step's playback to zero ticks is
    // still a complete playback, so the invariant has to survive it.
    let (headless_draws, headless_roster) = run_headless();
    let (scene_draws, scene_roster) = run_scene(4, Drive::Skip);
    assert_eq!(scene_draws, headless_draws);
    assert_eq!(scene_roster, headless_roster);
}

#[test]
fn attaching_a_scene_changes_nothing_about_the_outcome() {
    // The weaker, faster sibling: the same fight with the action sink attached
    // but no scene at all. Isolates "buffering events costs nothing" from
    // "playing them back costs nothing", so a failure in the test above points
    // at the timeline rather than at the sink.
    let (headless_draws, headless_roster) = run_headless();

    let draws = Draws::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(draws.sink());
    let batch = Batch::default();
    let mut state = synthetic_fight();
    state.attach_action_sink(batch.sink());
    let mut events = 0;
    while state.step(&mut rng) != CombatStep::Ended {
        events += batch.take().len();
    }
    events += batch.take().len();

    assert!(events > 50, "the fight really did emit a batch: {events}");
    assert_eq!(draws.taken(), headless_draws);
    assert_eq!(roster_fingerprint(&state), headless_roster);
}
