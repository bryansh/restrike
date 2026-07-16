//! The engine sink implementation (D-OR3, task deliverable 3): the concrete
//! [`gbx_engine::rng::RngSink`] that turns live PRNG draws into `prng`-profile
//! [`TraceEvent`]s.
//!
//! The trait lives in `gbx-engine` (the core stays pure); this is the
//! `.gbxtrace`-producing side, so the dependency runs `gbx-oracle -> gbx-engine`
//! only. A [`TraceCollector`] hands the engine a boxed sink that shares its
//! buffer, so after a capture the caller reads the events back **without**
//! downcasting or file I/O — then stamps a header on with [`TraceCollector::into_trace`].
//!
//! Usage — `no_run` rather than `ignore` so the example is **compile-checked**
//! against the real API (an `ignore`d example is never built, so it rots
//! silently; building an `Engine` needs game data, so it must not *run* in CI):
//! ```no_run
//! # use gbx_oracle::{TraceCollector, TraceHeader};
//! # fn capture(engine: &mut gbx_engine::engine::Engine, header: TraceHeader) {
//! let collector = TraceCollector::new();
//! engine.attach_rng_sink(collector.sink());
//! // … drive the engine (ticks, a curated encounter) …
//! let trace = collector.into_trace(header);          // ready to compare/write
//! # }
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use gbx_engine::combat::{ActionEvent, ActionSink};
use gbx_engine::rng::{RngDraw, RngSink};

use crate::format::{InitEvent, PickEvent, RngEvent, Trace, TraceEvent, TraceHeader};

/// A shared draw buffer. Cheap to clone (an `Rc` bump); every clone — including
/// the boxed sink handed to the engine — appends to the same `Vec`, in draw
/// order (D-OR3: emission order is the format contract).
#[derive(Clone, Default)]
pub struct TraceCollector {
    events: Rc<RefCell<Vec<TraceEvent>>>,
}

impl TraceCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// A boxed sink sharing this collector's buffer — pass to
    /// `Engine::attach_rng_sink` / `EngineRng::attach_sink`.
    pub fn sink(&self) -> Box<dyn RngSink> {
        Box::new(CollectorSink {
            events: Rc::clone(&self.events),
        })
    }

    /// A boxed **action** sink sharing this collector's buffer — pass to
    /// `CombatState::attach_action_sink`. Feeding the same collector as
    /// [`sink`](Self::sink) interleaves `init`/`pick` events with the draws in
    /// emission order (D-OR3's same-tick contract), so a combined-order trace is
    /// index-alignable across the two profiles.
    pub fn action_sink(&self) -> Box<dyn ActionSink> {
        Box::new(CollectorActionSink {
            events: Rc::clone(&self.events),
        })
    }

    /// Snapshot of the events captured so far, in draw order.
    pub fn events(&self) -> Vec<TraceEvent> {
        self.events.borrow().clone()
    }

    /// Number of events captured so far.
    pub fn len(&self) -> usize {
        self.events.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.borrow().is_empty()
    }

    /// Stamps `header` onto the captured events, producing a finished [`Trace`]
    /// ready to compare, chain-check, or write canonically.
    pub fn into_trace(&self, header: TraceHeader) -> Trace {
        Trace::new(header, self.events())
    }
}

/// The boxed observer the engine actually holds. Each draw becomes one
/// `prng`-profile `rng` event carrying `(before, after, n, result)` — restrike
/// is a full-operand emitter, so `n`/`result` are always present. `caller` is
/// left `None`: restrike's synthetic call-site tags are diagnostic-only and not
/// needed for equality (D-OR3), and a combat session can add them if useful.
struct CollectorSink {
    events: Rc<RefCell<Vec<TraceEvent>>>,
}

impl RngSink for CollectorSink {
    fn on_draw(&mut self, draw: RngDraw) {
        self.events.borrow_mut().push(TraceEvent::Rng(RngEvent {
            before: draw.before,
            after: draw.after,
            n: draw.n,
            result: draw.result,
            caller: None,
        }));
    }
}

/// The boxed action observer the combat state holds. Translates each engine
/// [`ActionEvent`] into its canonical `.gbxtrace` form ([`InitEvent`] /
/// [`PickEvent`], field order pinned in [`crate::format`]) — the engine emits
/// engine-local plain data, this side owns the on-disk vocabulary, so
/// `gbx-engine` never depends on `gbx-oracle`. `surprise` becomes the `0`/`1`
/// integer the integers-only encoding requires.
struct CollectorActionSink {
    events: Rc<RefCell<Vec<TraceEvent>>>,
}

impl ActionSink for CollectorActionSink {
    fn on_action(&mut self, event: ActionEvent) {
        let translated = match event {
            ActionEvent::Init {
                combatant_id,
                delay,
                dex_adj,
                surprise,
            } => TraceEvent::Init(InitEvent {
                combatant_id: combatant_id as u32,
                delay: delay as i16,
                dex_adj: dex_adj as i16,
                surprise: surprise as u8,
            }),
            ActionEvent::Pick {
                pass,
                combatant_id,
                delay,
                roll,
            } => TraceEvent::Pick(PickEvent {
                pass,
                combatant_id: combatant_id as u32,
                delay: delay as i16,
                roll,
            }),
        };
        self.events.borrow_mut().push(translated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::check_chain;
    use crate::format::Profile;
    use gbx_engine::rng::EngineRng;

    fn header(seed: u32) -> TraceHeader {
        TraceHeader {
            gbxtrace: 1,
            profile: Profile::Prng,
            game: "cotab-v1.3".to_string(),
            seed,
            encounter: "sink-test".to_string(),
            source: "restrike".to_string(),
            notes: None,
        }
    }

    /// The engine sink emits a genuine, chain-continuous trace whose per-draw
    /// values match an independent `gbx_prng` replay — proving the seam carries
    /// the true `(before, after, n, result)`.
    #[test]
    fn engine_sink_emits_a_genuine_chain_matching_a_prng_replay() {
        let seed = 0x0cab_1234u32; // arbitrary constant
        let collector = TraceCollector::new();
        let mut rng = EngineRng::new(seed);
        rng.attach_sink(collector.sink());

        let ns = [6u16, 100, 0, 1, 20, 8, 255];
        let mut expected = Vec::new();
        for &n in &ns {
            expected.push(rng.random(n));
        }

        let trace = collector.into_trace(header(seed));
        // Self-consistent: after == step(before), draws link.
        assert_eq!(check_chain(&trace), Ok(()));

        // Values match an independent replay.
        let mut replay = gbx_prng::Prng::new(seed);
        let drawn: Vec<_> = trace.rng_events().collect();
        assert_eq!(drawn.len(), ns.len());
        for (i, (ev, &n)) in drawn.iter().zip(ns.iter()).enumerate() {
            let before = replay.state();
            let result = replay.random(n);
            let after = replay.state();
            assert_eq!(ev.before, before, "draw {i} before");
            assert_eq!(ev.after, after, "draw {i} after");
            assert_eq!(ev.n, Some(n), "draw {i} n");
            assert_eq!(ev.result, Some(result), "draw {i} result");
            assert_eq!(ev.result, Some(expected[i]), "draw {i} result vs engine");
        }
    }

    /// The action sink captures a real combat round's `init`/`pick` events, and
    /// feeding the *same* collector as the RNG sink interleaves them with the
    /// draws in emission order — the prng chain still checks over the draw subset
    /// (init/pick are skipped), and every draw is a d6 (initiative) or d100
    /// (selection).
    #[test]
    fn action_sink_captures_a_real_combat_round_interleaved_with_draws() {
        use gbx_engine::combat::{CombatState, CombatStep, Combatant, Team};

        let seed = 0x0c0f_fee0u32;
        let collector = TraceCollector::new();
        let mut rng = EngineRng::new(seed);
        rng.attach_sink(collector.sink());

        let roster = vec![
            Combatant::new(0, Team::Party, 0, true),
            Combatant::new(1, Team::Party, 0, true),
            Combatant::new(2, Team::Monster, 0, true),
        ];
        let mut state = CombatState::new(roster);
        state.attach_action_sink(collector.action_sink());

        // Drive exactly one round.
        loop {
            match state.step(&mut rng) {
                CombatStep::RoundEnded { .. } => break,
                CombatStep::Ended => panic!("ended mid-round"),
                _ => {}
            }
        }

        let events = collector.events();
        let inits = events
            .iter()
            .filter(|e| matches!(e, TraceEvent::Init(_)))
            .count();
        let picks = events
            .iter()
            .filter(|e| matches!(e, TraceEvent::Pick(_)))
            .count();
        assert_eq!(inits, 3, "one init per combatant");
        assert_eq!(picks, 3, "one pick per selection (all three act)");

        // The prng subset still forms a continuous chain (init/pick skipped).
        let trace = collector.into_trace(header(seed));
        assert_eq!(check_chain(&trace), Ok(()));

        // Every draw is a d6 or a d100 — pure initiative, no other consumers.
        for ev in trace.rng_events() {
            assert!(matches!(ev.n, Some(6) | Some(100)), "n was {:?}", ev.n);
        }

        // Emission order: the very first event is the first combatant's d6 draw,
        // immediately followed by its `init` (each init brackets its d6). The
        // first `pick` only appears after every `init` (selection follows
        // initiative).
        assert!(matches!(
            events[0],
            TraceEvent::Rng(RngEvent { n: Some(6), .. })
        ));
        assert!(matches!(events[1], TraceEvent::Init(_)));
        let first_pick = events.iter().position(|e| matches!(e, TraceEvent::Pick(_)));
        let last_init = events
            .iter()
            .rposition(|e| matches!(e, TraceEvent::Init(_)));
        assert!(
            first_pick.unwrap() > last_init.unwrap(),
            "all inits precede any pick"
        );
    }

    #[test]
    fn detaching_the_sink_stops_capture() {
        let collector = TraceCollector::new();
        let mut rng = EngineRng::new(1);
        rng.attach_sink(collector.sink());
        rng.random(6);
        rng.random(6);
        rng.take_sink();
        rng.random(6); // not observed
        assert_eq!(collector.len(), 2);
    }
}
