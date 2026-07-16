//! The engine's one seedable PRNG (PLAN.md D9: "single seedable PRNG, no
//! wall clock, replayable input traces"). As of M4 step 1 this is the
//! **binary-exact** generator: a thin engine-local wrapper over
//! [`gbx_prng::Prng`] (CotAB's Turbo Pascal LCG, recovered and adversarially
//! re-derived in `docs/design/oracle-rig.md` §1). The splitmix64 placeholder
//! this replaced is gone; all game randomness now flows through `gbx-prng` and
//! nothing else (D-OR1).
//!
//! `EngineRng` exists (rather than using `gbx_prng::Prng` directly) for two
//! reasons: it is the local type that carries the `gbx_vm::VmRng` impl (neither
//! `VmRng` nor `Prng` is local to `gbx-engine`, so the impl must hang off a
//! type that is), and it is the field type persisted in `SaveState.prng`.
//!
//! Bound convention: [`EngineRng::random`] is the binary's `Random(n)` — an
//! **exclusive** `0..n` draw that always advances the state, including `n == 0`
//! (returns 0 *after* drawing). The old `roll_uniform`'s inclusive bound and
//! its `== 0` short-circuit are both gone; see the M4 migration ledger in
//! `docs/design/oracle-rig.md` §6.

use gbx_prng::Prng;
use gbx_vm::VmRng;

/// One PRNG draw, as observed at the [`EngineRng::random`] seam: the state
/// dword (`DS:0x47F0`) before and after the LCG step, plus the wrapper operand
/// `n` and its returned `result`. This is exactly the D-OR3 `prng`-profile
/// event surface (`docs/design/oracle-rig.md` D-OR3) — `(before, after)` is
/// the equality core, `(n, result)` the operand extension. `before`/`after`
/// are the two independent values the tier-1 staging hook also captures, so a
/// restrike trace and an oracle trace are directly comparable draw-for-draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RngDraw {
    pub before: u32,
    pub after: u32,
    /// The wrapper operand — `Some` for every engine-side draw (the engine
    /// always knows `n`); `Option` because the format permits state-only draws
    /// from emitters that don't (the staging hook observes `RandNext` directly,
    /// below the `Random(n)` wrapper, so it may not know `n`).
    pub n: Option<u16>,
    /// The value the `Random(n)` wrapper returned. `Some` alongside `n`.
    pub result: Option<u16>,
}

/// The engine's trace seam (D-OR3, task deliverable 3): the *core* stays pure
/// (no trace/format types leak into normal play), and an observer is attached
/// only when a differential run wants one. The trait lives here, on the engine
/// side; `gbx-oracle` provides the `.gbxtrace`-writing implementation — so
/// `gbx-engine` never depends on `gbx-oracle` (only the reverse). Default is
/// zero-cost and inert: with no sink attached, [`EngineRng::random`] pays a
/// single `Option::is_some` branch and nothing else, and neither `Engine::save`
/// nor the committed `.rsav` golden changes (the sink is never serialized).
pub trait RngSink {
    /// Called once per draw, *after* the state has advanced. Same-draw ordering
    /// is the natural call order of the draw sites (D-OR3: emission order is
    /// part of the format contract).
    fn on_draw(&mut self, draw: RngDraw);
}

/// The engine's one PRNG — a thin wrapper over [`gbx_prng::Prng`] carrying the
/// `gbx_vm::VmRng` impl and the `.rsav`-persisted state (module doc). It also
/// carries an *optional, never-serialized* [`RngSink`]: the differential-trace
/// seam (D-OR3). The sink is excluded from every serialized/compared surface —
/// only [`inner`](Self::inner) (the `u32` LCG state) is `.rsav` state — so
/// `Debug`/`Clone`/`PartialEq`/`Eq`/`Serialize`/`Deserialize` are all defined
/// (by hand where the trait object forbids a derive) to behave exactly as they
/// did before the sink existed. A cloned `EngineRng` (e.g. the one `save()`
/// snapshots) never carries a sink.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct EngineRng {
    inner: Prng,
    /// The differential-trace observer. `#[serde(skip)]` keeps it out of the
    /// `.rsav` payload entirely (restore fills it with `None` via `Default`),
    /// so the serialized bytes — and the committed golden hash — are identical
    /// to the pre-sink single-field struct.
    #[serde(skip)]
    sink: Option<Box<dyn RngSink>>,
}

impl EngineRng {
    pub fn new(seed: u32) -> Self {
        EngineRng {
            inner: Prng::new(seed),
            sink: None,
        }
    }

    /// The binary's integer `Random(n)` wrapper — exclusive `0..n`, always
    /// draws (including `n == 0`). Inherent method so concrete-`EngineRng`
    /// callers (training, vmhost) need not import `VmRng`; the trait impl
    /// delegates here. This is the single seam every §6-ledger draw site flows
    /// through, so it is where the [`RngSink`] observation happens: capture the
    /// state before and after the step, and — only if a sink is attached —
    /// forward the `(before, after, n, result)` draw. With no sink this is one
    /// branch over the underlying [`gbx_prng::Prng::random`].
    pub fn random(&mut self, n: u16) -> u16 {
        let before = self.inner.state();
        let result = self.inner.random(n);
        if let Some(sink) = self.sink.as_mut() {
            let after = self.inner.state();
            sink.on_draw(RngDraw {
                before,
                after,
                n: Some(n),
                result: Some(result),
            });
        }
        result
    }

    /// Attaches a differential-trace observer (D-OR3). Replaces any existing
    /// sink and returns it. Attaching a sink changes only what is *observed*,
    /// never the draw stream: `random` produces the same values and advances
    /// the same state whether or not a sink is present.
    pub fn attach_sink(&mut self, sink: Box<dyn RngSink>) -> Option<Box<dyn RngSink>> {
        self.sink.replace(sink)
    }

    /// Detaches and returns the current observer, if any (end of a capture).
    pub fn take_sink(&mut self) -> Option<Box<dyn RngSink>> {
        self.sink.take()
    }

    /// The live LCG state (`DS:0x47F0`). Read into `.rsav` and by the oracle
    /// rig at a synchronization point.
    pub fn state(&self) -> u32 {
        self.inner.state()
    }

    /// Overwrites the live state — `.rsav` restore and the oracle rig's seed
    /// poke (D-OR1: oracle mode == play mode).
    pub fn set_state(&mut self, state: u32) {
        self.inner.set_state(state);
    }
}

// --- Hand-written trait impls (the `Box<dyn RngSink>` field forbids deriving
// Clone/PartialEq/Eq/Debug, and would drag the sink into equality/formatting
// if it could). Every one operates on `inner` *only*, so `EngineRng`'s
// observable identity, formatting, and — crucially — equality are exactly the
// pre-sink single-field behavior. `Serialize`/`Deserialize` stay derived (the
// sink is `#[serde(skip)]`), so the `.rsav` bytes are unchanged. ---

impl Clone for EngineRng {
    /// Clones the PRNG state; the clone carries **no** sink. `Engine::save`
    /// snapshots the live PRNG via this clone, so a save never accidentally
    /// captures (or requires) an observer.
    fn clone(&self) -> Self {
        EngineRng {
            inner: self.inner.clone(),
            sink: None,
        }
    }
}

impl PartialEq for EngineRng {
    /// State equality only — two `EngineRng`s are equal iff their LCG state is,
    /// exactly as the pre-sink `#[derive(PartialEq)]` compared. The sink is a
    /// runtime observer, not identity.
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for EngineRng {}

impl std::fmt::Debug for EngineRng {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineRng")
            .field("inner", &self.inner)
            .field("sink", &self.sink.as_ref().map(|_| "<attached>"))
            .finish()
    }
}

impl VmRng for EngineRng {
    fn random(&mut self, n: u16) -> u16 {
        // Delegate to the inherent method so trait-object draws (the VM's
        // `op_random`, service `roll`/`roll_dice`) are observed too.
        EngineRng::random(self, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_reproduces_the_same_sequence() {
        let mut a = EngineRng::new(42);
        let mut b = EngineRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.random(1000), b.random(1000));
        }
    }

    #[test]
    fn wraps_gbx_prng_exactly() {
        // EngineRng is a pass-through: same seed, same draws as the crate.
        let mut e = EngineRng::new(0);
        let mut p = Prng::new(0);
        for _ in 0..200 {
            assert_eq!(e.random(97), p.random(97));
        }
    }

    #[test]
    fn random_zero_still_draws() {
        let mut e = EngineRng::new(0);
        assert_eq!(e.random(0), 0);
        assert_eq!(e.state(), 0x1, "random(0) advanced the state");
    }

    #[test]
    fn state_round_trips() {
        let mut a = EngineRng::new(7);
        for _ in 0..50 {
            a.random(13);
        }
        let s = a.state();
        let mut b = EngineRng::new(0);
        b.set_state(s);
        assert_eq!(a.state(), b.state());
        assert_eq!(a.random(13), b.random(13));
    }

    /// A minimal in-crate sink that records every observed draw — proves the
    /// seam fires with the right `(before, after, n, result)` and, critically,
    /// that attaching it does **not** perturb the draw stream (an inert vs.
    /// observed `EngineRng` produce identical values and states).
    #[derive(Default)]
    struct RecordingSink {
        draws: std::rc::Rc<std::cell::RefCell<Vec<RngDraw>>>,
    }

    #[test]
    fn sink_observes_before_after_n_result_without_changing_the_stream() {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut observed = EngineRng::new(0x1234_5678);
        observed.attach_sink(Box::new(RecordingSink { draws: log.clone() }));

        let mut inert = EngineRng::new(0x1234_5678);

        let ns = [6u16, 100, 0, 1, 20, 255];
        for &n in &ns {
            let want_before = inert.state();
            let want = inert.random(n);
            let want_after = inert.state();

            let got = observed.random(n);
            assert_eq!(got, want, "attaching a sink must not change the result");
            assert_eq!(
                observed.state(),
                want_after,
                "attaching a sink must not change the state"
            );

            let recorded = *log.borrow().last().unwrap();
            assert_eq!(recorded.before, want_before);
            assert_eq!(recorded.after, want_after);
            assert_eq!(recorded.n, Some(n));
            assert_eq!(recorded.result, Some(want));
        }
        assert_eq!(log.borrow().len(), ns.len(), "one draw recorded per call");
    }

    /// The `n == 0` draw-always contract is visible through the sink: a draw is
    /// recorded and the state advances, even though the result is 0.
    #[test]
    fn sink_records_the_n_zero_draw() {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut rng = EngineRng::new(0);
        rng.attach_sink(Box::new(RecordingSink { draws: log.clone() }));
        assert_eq!(rng.random(0), 0);
        let d = *log.borrow().last().unwrap();
        assert_eq!(d.before, 0);
        assert_eq!(d.after, 0x1, "n==0 still advanced the state");
        assert_eq!(d.n, Some(0));
        assert_eq!(d.result, Some(0));
    }

    impl RngSink for RecordingSink {
        fn on_draw(&mut self, draw: RngDraw) {
            self.draws.borrow_mut().push(draw);
        }
    }

    /// The sink is never serialized: an `EngineRng` with an attached sink
    /// serializes to the same bytes as one without (the `.rsav` invariant the
    /// committed golden depends on), and equality ignores the sink.
    #[test]
    fn sink_is_excluded_from_serialization_and_equality() {
        let plain = EngineRng::new(999);
        let mut with_sink = EngineRng::new(999);
        with_sink.attach_sink(Box::new(RecordingSink::default()));

        assert_eq!(plain, with_sink, "equality ignores the sink");

        let plain_bytes = postcard::to_allocvec(&plain).unwrap();
        let sink_bytes = postcard::to_allocvec(&with_sink).unwrap();
        assert_eq!(
            plain_bytes, sink_bytes,
            "the sink must not appear in the serialized form"
        );

        // And it round-trips back to a sink-less value.
        let restored: EngineRng = postcard::from_bytes(&sink_bytes).unwrap();
        assert_eq!(restored, plain);
    }
}
