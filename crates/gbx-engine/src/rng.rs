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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EngineRng {
    inner: Prng,
}

impl EngineRng {
    pub fn new(seed: u32) -> Self {
        EngineRng {
            inner: Prng::new(seed),
        }
    }

    /// The binary's integer `Random(n)` wrapper — exclusive `0..n`, always
    /// draws (including `n == 0`). Inherent method so concrete-`EngineRng`
    /// callers (training, vmhost) need not import `VmRng`; the trait impl
    /// delegates here.
    pub fn random(&mut self, n: u16) -> u16 {
        self.inner.random(n)
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

impl VmRng for EngineRng {
    fn random(&mut self, n: u16) -> u16 {
        self.inner.random(n)
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
}
