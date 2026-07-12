//! The engine's one seedable PRNG (PLAN.md D9: "single seedable PRNG, no
//! wall clock, replayable input traces"). This is a placeholder generator,
//! not the original's bit-exact algorithm — recovering that is H3/M4 scope
//! (PLAN.md §3). Everything that needs randomness before then (the fade
//! recolor dither, `EngineServices::roll`/`roll_dice`, this session's stub
//! VM) draws from this one generator, never a second one, so replays stay
//! reproducible from `(data fingerprint, seed)` alone.
//!
//! splitmix64 (Vigna/Steele, public domain) — chosen only for its
//! well-known, easily-reimplemented-from-the-published-constants shape;
//! nothing about its output needs to match the original engine.

use gbx_vm::VmRng;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EngineRng {
    state: u64,
}

impl EngineRng {
    pub fn new(seed: u64) -> Self {
        EngineRng { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

impl VmRng for EngineRng {
    /// Uniform in `0..=inclusive_max`. `inclusive_max == 0` always returns 0
    /// (never divides by zero).
    fn roll_uniform(&mut self, inclusive_max: u16) -> u16 {
        if inclusive_max == 0 {
            return 0;
        }
        (self.next_u64() % (inclusive_max as u64 + 1)) as u16
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
            assert_eq!(a.roll_uniform(1000), b.roll_uniform(1000));
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = EngineRng::new(1);
        let mut b = EngineRng::new(2);
        let seq_a: Vec<u16> = (0..20).map(|_| a.roll_uniform(u16::MAX)).collect();
        let seq_b: Vec<u16> = (0..20).map(|_| b.roll_uniform(u16::MAX)).collect();
        assert_ne!(seq_a, seq_b);
    }

    #[test]
    fn roll_uniform_stays_within_bounds() {
        let mut rng = EngineRng::new(7);
        for _ in 0..1000 {
            let v = rng.roll_uniform(5);
            assert!(v <= 5);
        }
    }

    #[test]
    fn roll_uniform_zero_max_never_panics() {
        let mut rng = EngineRng::new(7);
        assert_eq!(rng.roll_uniform(0), 0);
    }
}
