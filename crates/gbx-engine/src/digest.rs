//! ★ **H5 state digests** (`docs/design/roll-credits.md` D-RC3): a
//! deterministic hash of the engine state a playthrough checkpoint cares
//! about.
//!
//! The M6 arc proved why this cannot be a frame hash. Every slice that touched
//! a pixel — the menu highlight, the roster panel, the focus box — invalidated
//! frame goldens wholesale, and the roll-credits slices 2/4/7 will do it again.
//! A checkpoint that survives renderer work has to hash *state*, and state that
//! can tell quest progress apart, not just position: hence the ScriptMemory
//! windows, where every `SAVE`/`GETTABLE` flag in the game's most-used opcode
//! lives.
//!
//! ## The field order is append-only, forever
//!
//! **Changing the order of these fields, or the encoding of any one of them,
//! invalidates every H5 trace ever recorded.** There is no version negotiation
//! and deliberately so: a digest is a bare hash in a text file, and a trace
//! whose digests silently mean something else is worse than one that fails.
//! New state joins at the END, never in the middle, and [`DIGEST_LAYOUT`] gets
//! a new line. If a field's *meaning* changes (slice 1 turns `game_area` from a
//! constant into real state — which is exactly why it is hashed here already,
//! ahead of being state), the slot stays where it is.
//!
//! The one composite field, the ScriptMemory windows, is hashed through
//! D-SAVE1's own canonical encoding — `postcard` over
//! [`WindowsSnapshot`](crate::vmhost::WindowsSnapshot), whose collections are
//! all `BTreeMap`s precisely so that encoding is deterministic. This module
//! invents no second serialization of anything.

use crate::engine::Engine;
use sha2::{Digest, Sha256};

/// The digest's field order, in words — kept next to the code that feeds it so
/// the two cannot drift, and quotable in a trace file's header.
///
/// Append only. Never reorder. Never remove.
pub const DIGEST_LAYOUT: &[&str] = &[
    "position (x, y)",
    "facing",
    "game_area",
    "resident ECL block id",
    "clock (total units)",
    "party (count, then per member: current/max HP, XP, the eight class levels, item count, affect count)",
    "PRNG state",
    "search_flags",
    "ScriptMemory windows (WindowsSnapshot, postcard — D-SAVE1's canonical encoding)",
];

/// A running digest built from little-endian primitives, so the hash does not
/// depend on the host's byte order.
struct Fields(Sha256);

impl Fields {
    fn new() -> Self {
        Fields(Sha256::new())
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.update([v]);
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.update(v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.update(v.to_le_bytes());
        self
    }
    fn bytes(&mut self, v: &[u8]) -> &mut Self {
        // Length-prefixed: two adjacent variable-length fields must not be
        // able to alias each other by shifting the boundary between them.
        self.0.update((v.len() as u64).to_le_bytes());
        self.0.update(v);
        self
    }
    fn finish(self) -> String {
        self.0
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// Hashes `engine` per [`DIGEST_LAYOUT`]. See the module doc before changing
/// anything here.
pub(crate) fn state_digest(engine: &Engine) -> String {
    use crate::movement::Facing;

    let state = engine.state();
    let mut f = Fields::new();

    // 1. position
    f.u8(state.pos.0).u8(state.pos.1);
    // 2. facing — the original's own `mapDirection` encoding (0/2/4/6), not
    //    the Rust discriminant, so the number in the hash means something.
    f.u8(match state.facing {
        Facing::North => 0,
        Facing::East => 2,
        Facing::South => 4,
        Facing::West => 6,
    });
    // 3. game_area. A const today (`engine::GAME_AREA`); slice 1 moves it into
    //    `EngineState`. Hashed now so that promotion does not reshape digests.
    f.u8(crate::engine::GAME_AREA);
    // 4. resident ECL block id
    f.u8(state.ecl_block_id);
    // 5. clock
    f.u32(state.clock.total_units);
    // 6. party
    let party = engine.party();
    f.u32(party.members.len() as u32);
    for m in &party.members {
        f.u8(m.hit_point_current).u8(m.hit_point_max);
        f.i32(m.exp);
        for lvl in m.class_level {
            f.u8(lvl);
        }
        f.u32(m.items.len() as u32).u32(m.affects.len() as u32);
    }
    // 7. PRNG state
    f.u32(engine.prng_state());
    // 8. search_flags
    f.u8(state.search_flags);
    // 9. the ScriptMemory windows, through D-SAVE1's canonical encoding
    let windows = postcard::to_allocvec(&engine.vm_memory().snapshot())
        .expect("WindowsSnapshot is postcard-serializable — it is the .rsav payload");
    f.bytes(&windows);

    f.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        crate::save_roundtrip_tests::imported_engine()
    }

    /// Writes one raw ScriptMemory word — a `SAVE` opcode's landing site —
    /// through the same snapshot/restore pair `.rsav` uses.
    fn set_quest_word(e: &mut Engine, addr: u16, value: u16) {
        let mut snapshot = e.vm_memory().snapshot();
        snapshot.raw_words.insert(addr, value);
        e.vm_memory.restore_windows(snapshot);
    }

    #[test]
    fn a_digest_is_stable_for_an_unchanged_engine() {
        let e = engine();
        assert_eq!(e.state_digest(), e.state_digest());
        assert_eq!(e.state_digest().len(), 64, "hex SHA-256");
    }

    #[test]
    fn two_engines_built_the_same_way_digest_the_same() {
        let (mut a, mut b) = (engine(), engine());
        for _ in 0..30 {
            a.tick(&[]);
            b.tick(&[]);
        }
        assert_eq!(a.state_digest(), b.state_digest());
    }

    /// Every field in [`DIGEST_LAYOUT`] actually reaches the hash — a field
    /// that is documented but not fed is the failure mode this catches.
    #[test]
    fn every_documented_field_moves_the_digest() {
        let base = engine();
        let base_digest = base.state_digest();

        let mut e = engine();
        e.state.pos.0 = e.state.pos.0.wrapping_add(1);
        assert_ne!(e.state_digest(), base_digest, "position");

        let mut e = engine();
        e.state.facing = e.state.facing.turn_left();
        assert_ne!(e.state_digest(), base_digest, "facing");

        let mut e = engine();
        e.state.ecl_block_id = e.state.ecl_block_id.wrapping_add(1);
        assert_ne!(e.state_digest(), base_digest, "ecl block id");

        let mut e = engine();
        e.state.clock.total_units += 1;
        assert_ne!(e.state_digest(), base_digest, "clock");

        let mut e = engine();
        e.state.search_flags ^= 1;
        assert_ne!(e.state_digest(), base_digest, "search_flags");

        let mut e = engine();
        e.party.members[0].hit_point_current = e.party.members[0].hit_point_current.wrapping_sub(1);
        assert_ne!(e.state_digest(), base_digest, "party HP");

        let mut e = engine();
        e.party.members[0].exp += 1;
        assert_ne!(e.state_digest(), base_digest, "party XP");

        let mut e = engine();
        e.party.members[0].class_level[0] = e.party.members[0].class_level[0].wrapping_add(1);
        assert_ne!(e.state_digest(), base_digest, "party levels");

        let mut e = engine();
        e.party.members[0].items.push(vec![0u8; 4]);
        assert_ne!(e.state_digest(), base_digest, "inventory count");

        let mut e = engine();
        let bumped = e.prng_state().wrapping_add(1);
        e.rng.set_state(bumped);
        assert_ne!(e.state_digest(), base_digest, "PRNG state");

        // The quest flags: a `SAVE` opcode's landing site is a raw window word.
        let mut e = engine();
        set_quest_word(&mut e, 0x4321, 7);
        assert_ne!(e.state_digest(), base_digest, "ScriptMemory windows");
    }

    /// Two engines whose *only* difference is a quest flag digest apart —
    /// D-RC3's whole reason for hashing the windows rather than position.
    #[test]
    fn a_quest_flag_alone_separates_two_otherwise_identical_engines() {
        let a = engine();
        let mut b = engine();
        assert_eq!(a.state_digest(), b.state_digest());
        set_quest_word(&mut b, 0x7000, 1);
        assert_ne!(
            a.state_digest(),
            b.state_digest(),
            "a set quest flag must be visible to a checkpoint"
        );
    }
}
