//! The engine's one PRNG — CotAB's **binary-exact** random number generator.
//!
//! This is not a stand-in. It is the generator recovered from the CotAB v1.3
//! GOG `START.EXE` (EXEPACK-decompressed via `gbx_formats::exepack::decode`),
//! re-derived and adversarially re-verified in `docs/design/oracle-rig.md` §1.
//! coab is **not** the spec here: `seg051.cs` swaps in C# `System.Random`; the
//! binary is a Borland Turbo Pascal 5.x LCG. Where they disagree, the binary
//! wins, and this crate implements the binary.
//!
//! Recovered facts (oracle-rig §1 — do not re-derive):
//! - `RandNext` (image `0xa5a9`, cs `0x8F7:0x1639`): `state = state × 0x08088405
//!   + 1` (mod 2^32); the live state dword lives at `DS:0x47F0`; the new state
//!   is returned in `DX:AX`.
//! - integer `Random(N)` wrapper (image `0xa55a`, cs `0x8F7:0x15EA`): calls
//!   `RandNext` **first** (the draw is always consumed); then, if `N == 0`,
//!   returns 0 *after having drawn*; else `hi16(new_state) DIV N`, remainder
//!   returned in `AX` — i.e. `hi16(new_state) mod N`. TP 5.x `div bx`, **not**
//!   TP6+'s scaled high word (v1's refuted claim).
//! - float `Random` (image `0xa570`): `new_state / 2^32` as a TP 6-byte real.
//!   **M5 scope** — see [`Prng::random_real`], deliberately unimplemented.
//!
//! D-OR1: this is the *only* RNG in the engine. After the M4-step-1 migration,
//! all game randomness flows through this crate and nothing else. The state is
//! serialized into `SaveState` (hence the `serde` dependency and the u32-narrow
//! seed at the engine API), and it is what the M4 oracle rig pokes and replays.
//!
//! Pure, wasm-clean leaf: zero runtime dependencies beyond `serde`.

/// CotAB's PRNG state and operations. `state` is the live LCG state — the
/// in-memory equivalent of the original's `DS:0x47F0` dword.
///
/// There is no "oracle mode": oracle mode *is* play mode (D-OR1). Seeding for
/// the oracle rig and for `.rsav` restore is done through [`Prng::set_state`];
/// there is no second constructor and no alternate draw path.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Prng {
    state: u32,
}

impl Prng {
    /// The Turbo Pascal LCG multiplier, `0x08088405` — stored in the binary as
    /// a code-segment data word `0x8405` at `cs:[0x166F]` (image `0xa5df`) plus
    /// shift-add contributions for the high half, which is why naive 32-bit
    /// constant scans of the image find nothing (oracle-rig §1).
    const MULTIPLIER: u32 = 0x0808_8405;

    /// Seeds the state directly (`state = seed`). The original seeds the same
    /// dword from `Randomize`'s DOS wall-clock read at boot; we never read a
    /// clock — the seed is supplied so replays are reproducible from
    /// `(data fingerprint, seed)` alone (PLAN.md D9).
    pub fn new(seed: u32) -> Self {
        Prng { state: seed }
    }

    /// One full LCG step: `state = state × 0x08088405 + 1` (mod 2^32), returning
    /// the **new** state. This is `RandNext` (image `0xa5a9`), which returns the
    /// post-update state in `DX:AX`.
    ///
    /// The name is the D-OR1 API spec; it is not the `Iterator::next` this crate
    /// deliberately does not implement (a PRNG is not an iterator — it never
    /// ends, and `random()` is the real consumer entry point).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(Self::MULTIPLIER).wrapping_add(1);
        self.state
    }

    /// The integer `Random(N)` wrapper (image `0xa55a`), implemented **exactly**:
    ///
    /// - The state is **always** advanced first — even when `n == 0`. The binary
    ///   `call RandNext` precedes the `N == 0` test (`33 c0`/`0b db`/`74 04` sit
    ///   *after* the `e8 4c 00` call). So `random(0)` consumes a draw and then
    ///   returns 0. There is no short-circuit; this is the whole point of the
    ///   draw-parity contract (D-OR1(b)) — coab's `seg051.cs:33-40` returns 0
    ///   *without* drawing, a one-draw desync hazard the binary does not have.
    /// - Otherwise the result is `hi16(new_state) mod n` — TP 5.x `div bx` with
    ///   the remainder returned in `AX`. The bound is **exclusive**: `random(n)`
    ///   ranges over `0..n`. (This is the exclusive primitive the migration
    ///   maps every call site onto; `random(1)` is always 0 and still draws.)
    pub fn random(&mut self, n: u16) -> u16 {
        let new_state = self.next();
        if n == 0 {
            0
        } else {
            ((new_state >> 16) as u16) % n
        }
    }

    /// The live LCG state (the `DS:0x47F0` dword). Used by `.rsav` serialization
    /// and by the M4 oracle rig to read a synchronization point.
    pub fn state(&self) -> u32 {
        self.state
    }

    /// Overwrites the live state. Used by `.rsav` restore and by the oracle
    /// rig's one-shot seed poke (D-OR1: oracle mode == play mode).
    pub fn set_state(&mut self, state: u32) {
        self.state = state;
    }

    // The float `Random` path (image `0xa570`: `new_state / 2^32` as a Turbo
    // Pascal 6-byte real) is intentionally **not implemented**. It lands in M5,
    // when `ovr019`'s four `Random__Real` call sites are reached (oracle-rig §1,
    // caller census). Do not add it before then.
    //
    // pub fn random_real(&mut self) -> ... {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE most important test in this crate: `random(0)` **advances the state
    /// and returns 0** — it does *not* short-circuit. The binary `call RandNext`
    /// runs before the `N == 0` test, so a draw is consumed even when the caller
    /// passes 0. If this ever regresses to an early return, every call site that
    /// can pass 0 (e.g. `roll_dice(size, count)` with `size == 0`) desyncs from
    /// the original by exactly one draw — silently. This is the sharpest edge of
    /// the D-OR1(b) draw-parity contract.
    #[test]
    fn random_zero_still_advances_the_state_and_returns_zero() {
        let mut a = Prng::new(0);
        assert_eq!(a.random(0), 0, "random(0) returns 0");
        assert_eq!(
            a.state(),
            0x1,
            "random(0) must have DRAWN: state(0) stepped to 0*0x08088405+1 = 1"
        );

        // And it keeps advancing on every subsequent call, 0 or not.
        let mut b = Prng::new(0);
        let mut c = Prng::new(0);
        b.random(0);
        c.next();
        assert_eq!(
            b.state(),
            c.state(),
            "random(0) advances exactly like next()"
        );
    }

    /// The LCG algebra, pinned to values computed independently of this code
    /// (`state = state*0x08088405 + 1 mod 2^32`). If the multiplier, increment,
    /// or wrap ever drift, these fail.
    #[test]
    fn lcg_state_sequence_matches_hand_computed_pins() {
        let mut p = Prng::new(0);
        let seed0 = [0x1u32, 0x0808_8406, 0xdc6d_ac1f, 0x33dc_589c, 0x45de_2b0d];
        for (i, want) in seed0.iter().enumerate() {
            assert_eq!(p.next(), *want, "seed 0, step {}", i + 1);
        }

        let mut q = Prng::new(0x1234_5678);
        let seed_a = [
            0xcb5b_9059u32,
            0x79ff_b5be,
            0xd9a4_84b7,
            0xf25c_f394,
            0xe609_11e5,
        ];
        for (i, want) in seed_a.iter().enumerate() {
            assert_eq!(q.next(), *want, "seed 0x12345678, step {}", i + 1);
        }
    }

    /// `random(n) = hi16(new_state) mod n`, pinned for two moduli from seed 0.
    /// The hi16 sequence from seed 0 is [0x0000, 0x0808, 0xdc6d, 0x33dc, ...];
    /// these expected outputs were computed externally, not from this code.
    #[test]
    fn random_results_match_hand_computed_pins() {
        let mut p = Prng::new(0);
        let want_mod6 = [0u16, 4, 5, 4, 0, 1, 5, 1];
        for (i, want) in want_mod6.iter().enumerate() {
            assert_eq!(p.random(6), *want, "random(6) draw {i}");
        }

        let mut q = Prng::new(0);
        let want_mod100 = [0u16, 56, 29, 76, 86, 17, 85, 3];
        for (i, want) in want_mod100.iter().enumerate() {
            assert_eq!(q.random(100), *want, "random(100) draw {i}");
        }
    }

    /// `random(1)` is always 0 — but it still consumes a draw every call.
    #[test]
    fn random_one_is_always_zero_and_still_draws() {
        let mut p = Prng::new(0xdead_beef);
        let mut mirror = Prng::new(0xdead_beef);
        for _ in 0..64 {
            assert_eq!(p.random(1), 0, "random(1) is always 0");
            mirror.next();
            assert_eq!(p.state(), mirror.state(), "random(1) draws like next()");
        }
    }

    /// `random(n)` stays in `0..n` (exclusive) for a range of moduli.
    #[test]
    fn random_stays_in_exclusive_range() {
        let mut p = Prng::new(12345);
        for &n in &[2u16, 3, 6, 7, 20, 100, 255, 256, 1000] {
            for _ in 0..2000 {
                let v = p.random(n);
                assert!(v < n, "random({n}) returned {v}, out of 0..{n}");
            }
        }
    }

    /// `state`/`set_state` round-trip, and two instances at the same state
    /// produce identical streams (the oracle-rig replay property).
    #[test]
    fn set_state_round_trips_and_reproduces_streams() {
        let mut a = Prng::new(1);
        for _ in 0..500 {
            a.next();
        }
        let checkpoint = a.state();

        let mut b = Prng::new(999);
        b.set_state(checkpoint);
        assert_eq!(b.state(), checkpoint, "set_state/state round-trip");

        // From the same state, a and b must draw identically.
        for i in 0..500 {
            assert_eq!(a.random(37), b.random(37), "divergence at draw {i}");
        }
    }
}

/// Local-tier (GBX_DATA_DIR-gated) hash pin. The `random`/`next` semantics in
/// this crate were derived from one specific decompressed image; this test
/// re-derives the RNG-cluster pin **from the user's own binary** and loud-fails
/// on mismatch, so a different game version is *re-pinned, never trusted*.
///
/// Follows the local-tier pattern of
/// `gbx-formats/src/exepack.rs`'s `decodes_real_start_exe_and_matches_known_anchors`
/// (env gate, loud `SKIPPED:` line, `.expect` messages). `gbx-formats` + `sha2`
/// are dev-dependencies so the crate stays a pure leaf.
#[cfg(test)]
mod pin {
    use sha2::{Digest, Sha256};

    /// The `[0xa55a, 0xa5ee)` range — one contiguous span covering the integer
    /// wrapper, float entry, `RandNext`, the `0x8405` multiplier word, and
    /// `Randomize` (oracle-rig §1's verification pin).
    const PIN_SHA256: &str = "0f770ce01cc999eb8ca75406d57de94ffd7c01e7438c0647395b26a668bea68b";

    #[test]
    fn rng_cluster_pin_matches_the_users_binary() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (gbx_prng::pin::rng_cluster_pin_matches_the_users_binary)");
            return;
        };
        let path = std::path::Path::new(&dir).join("START.EXE");
        let packed = std::fs::read(&path).expect("GBX_DATA_DIR/START.EXE must be readable");

        let image = gbx_formats::exepack::decode(&packed)
            .expect("real START.EXE must EXEPACK-decode cleanly");

        let cluster = image
            .get(0xa55a..0xa5ee)
            .expect("decompressed image must contain the RNG cluster range [0xa55a,0xa5ee)");

        let hash: String = Sha256::digest(cluster)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();

        assert_eq!(
            hash, PIN_SHA256,
            "RNG-cluster pin MISMATCH: the binary at GBX_DATA_DIR is not the image gbx-prng's \
             semantics were derived from. `random`/`next` here (multiplier, mod-vs-scale, \
             draw-always) are no longer known to apply to this binary — re-derive and re-pin \
             per oracle-rig §1 before trusting any roll. Do NOT just update this hash."
        );
    }
}
