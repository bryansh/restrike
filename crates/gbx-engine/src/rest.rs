//! ★ The rest-encounter schedule — `rest_incounter_period` /
//! `rest_incounter_percentage`, and the one place the original consumes them.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/ovr021.cs` `resting` (`:516-612`) — the rest loop, and its
//!   encounter check at `:586-604`.
//! - `engine/ovr024.cs` `roll_dice` (`:586-598`) — the d100.
//! - `Classes/Area2.cs:56-59` — the two cells, `DataOffset` `0x5A4`/`0x5A6`,
//!   both **signed** words.
//! - `Classes/Gbl.cs:440` — `rest_incounter_count`, the companion counter.
//! - `engine/ovr008.cs:111-112` — `vm_init_ecl` zeroes the period/percentage
//!   pair at every block entry, and pointedly does **not** touch the counter.
//!
//! ## What this is, and what it is not
//!
//! It is **rest-only**. The walk loop has no engine-side encounter roll at all:
//! `MovePartyForward` (`ovr015.cs:318-346`) and `MovePositionForward`
//! (`ovr008.cs:1256-1279`) update position and the clock and nothing else.
//! Wandering monsters in CotAB are entirely script-driven — `RANDOM` (0x08)
//! into an ECL variable, then `IF`/`COMBAT` — which is why the census finds 53
//! `RANDOM` uses and no engine hook.
//!
//! The check fires from exactly two call sites, both inside camp: `rest_menu`
//! (`ovr016.cs:291`, `resting(true)`) and `FixTeam` (`ovr016.cs:1057`,
//! `resting(false)` — and when *that* one is interrupted the queued healing is
//! discarded, `ovr016.cs:1059-1068`). The interruption is not signalled to the
//! script by a flag: `resting` merely returns "interrupted", `MakeCamp`
//! propagates it, and `TryEncamp` then runs the ECL header's **vector 3**,
//! `CampInterruptedAddr` (`ovr003.cs:1920`) — the block's own camp-ambush
//! script.
//!
//! ## Draw neutrality
//!
//! [`RestEncounterSchedule::check`] is the only PRNG draw in this module, and
//! it cannot be reached from any captured fight: its sole caller is the camp
//! rest loop, and a capture is a combat entry-state snapshot replayed straight
//! into `CombatState` — no camp, no rest, no walk. The rest loop itself does
//! not exist in this engine yet (roll-credits G3/slice 4 owns Rest's commit,
//! clock and healing), so today the draw is unreachable from *every* path; when
//! slice 4 lands it, the argument narrows to "captures never rest" and stays
//! true.

use crate::rng::EngineRng;

/// The two `Area2` cells' Party-window addresses (`DataOffset = (addr -
/// 0x7C00) * 2`, so `0x5A4`/`0x5A6` → these), named in `crate::vmhost` and
/// zeroed by `vm_init_ecl`. Real content arms them: `ECL4#37 @0x822E`/`@0x8234`
/// writes percentage `0x0A` and period `0x1E` immediately after its overland
/// `NEWECL`.
pub const REST_INCOUNTER_PERIOD_ADDR: u16 = 0x7ED2;
pub const REST_INCOUNTER_PERCENTAGE_ADDR: u16 = 0x7ED3;

/// `gbl.rest_incounter_count` (`Classes/Gbl.cs:440`) — the iteration counter
/// the check runs on.
///
/// It lives in `gbl`, **not** in `Area2`, and that has two consequences worth
/// preserving rather than tidying away:
/// 1. `SaveGame` never writes it (`ovr017.cs:1109-1156` serializes
///    `game_area`, `area_ptr`, `area2_ptr`, `stru_1B2CA`, `ecl_ptr`, the
///    position bytes, the game states, `setBlocks` and the party) — so a
///    reloaded game starts its count at 0, exactly as this type's `Default`
///    does.
/// 2. `vm_init_ecl` zeroes the *period* and *percentage* at every block entry
///    (`ovr008.cs:111-112`) but leaves the count alone, so a stale count
///    carried across a transition can make the first check in a newly-armed
///    area fire early. Replicated, not corrected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RestEncounterSchedule {
    count: i16,
}

/// What one rest-loop iteration's encounter check decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestCheck {
    /// The period is disabled (`<= 0`), or the counter has not reached it yet.
    /// No PRNG draw was taken.
    Quiet,
    /// The counter reached the period, so the d100 was rolled and missed. The
    /// counter is back at 0.
    Rolled { roll: u16 },
    /// The roll landed inside the percentage: `stop_resting = true`,
    /// `resting_intetrupted = true`, and the loop breaks with
    /// "Your repose is suddenly interrupted!" on screen.
    Interrupted { roll: u16 },
}

/// `resting`'s own interruption line (`ovr021.cs:598`), printed at row `0x13`
/// column 1 in colour 15.
pub const INTERRUPTED_TEXT: &str = "Your repose is suddenly interrupted!";

impl RestEncounterSchedule {
    /// ★ One rest-loop iteration's encounter check (`ovr021.cs:586-604`),
    /// transcribed:
    ///
    /// ```text
    /// if (rest_incounter_period > 0) {
    ///     rest_incounter_count++;
    ///     if (rest_incounter_count >= rest_incounter_period) {
    ///         rest_incounter_count = 0;
    ///         if (roll_dice(100, 1) <= rest_incounter_percentage) { … }
    ///     }
    /// }
    /// ```
    ///
    /// Four details the shape hides:
    /// - **The unit of `period` is a loop iteration, not a minute.** Each
    ///   iteration advances the clock by `step_game_time(1, 5)`
    ///   (`ovr021.cs:581`), i.e. five units of slot 1 — so `period` counts
    ///   five-tick blocks.
    /// - **Pre-increment, `>=`, then a hard reset to 0.** There is no carry, so
    ///   the check fires on exactly every `period`-th iteration.
    /// - **The roll happens only on a firing iteration**, so the PRNG advances
    ///   `⌈iterations / period⌉` times, not once per iteration. That matters to
    ///   anyone reproducing a stream.
    /// - **`roll_dice(100, 1)` is `Random(100) + 1`** (`ovr024.cs:591`), i.e.
    ///   `1..=100` inclusive, compared with `<=`. So percentage `P` fires with
    ///   probability `P/100` for `1 <= P <= 100`; `P <= 0` never fires but
    ///   **still burns the draw**, and `P >= 100` always fires.
    ///
    /// Both cells are **signed** in the original (`DataType.SWord`), and the
    /// guard is `> 0` — so a script writing `0x8000..=0xFFFF` to `0x7ED2`
    /// silently disables the schedule rather than arming a huge one.
    pub fn check(&mut self, period: i16, percentage: i16, rng: &mut EngineRng) -> RestCheck {
        if period <= 0 {
            return RestCheck::Quiet; // `:586`
        }
        self.count = self.count.wrapping_add(1); // `:588`
        if self.count < period {
            return RestCheck::Quiet; // `:590`
        }
        self.count = 0; // `:592`
        let roll = 1 + rng.random(100); // `roll_dice(100, 1)`, `:594`
        if roll as i16 <= percentage {
            RestCheck::Interrupted { roll }
        } else {
            RestCheck::Rolled { roll }
        }
    }

    /// The live counter, for the inspector and for tests.
    pub fn count(&self) -> i16 {
        self.count
    }

    /// `seg001.cs:268`/`:362`'s process-init reset — the only place outside a
    /// firing check that touches it.
    pub fn reset(&mut self) {
        self.count = 0;
    }
}

/// Reads the two cells out of the raw Party-window store as the original's
/// signed shorts. Kept next to [`RestEncounterSchedule::check`] so a caller
/// cannot accidentally pass them unsigned.
pub fn schedule_cells(vm: &crate::vmhost::VmMemoryState) -> (i16, i16) {
    (
        vm.raw_word(REST_INCOUNTER_PERIOD_ADDR).unwrap_or(0) as i16,
        vm.raw_word(REST_INCOUNTER_PERCENTAGE_ADDR).unwrap_or(0) as i16,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng() -> EngineRng {
        EngineRng::new(0x1234_5678)
    }

    /// `period <= 0` is the post-`vm_init_ecl` default and the disabled state:
    /// no count, no draw, ever.
    #[test]
    fn a_zero_period_never_fires_and_never_draws() {
        let mut s = RestEncounterSchedule::default();
        let mut r = rng();
        let before = r.state();
        for _ in 0..1000 {
            assert_eq!(s.check(0, 100, &mut r), RestCheck::Quiet);
        }
        assert_eq!(s.count(), 0, "the counter does not even advance");
        assert_eq!(r.state(), before, "and the PRNG never moves");
    }

    /// A negative period — reachable, because the cell is a signed short and
    /// the ECL write path is unsigned — disables it just as thoroughly.
    #[test]
    fn a_negative_period_disables_the_schedule() {
        let mut s = RestEncounterSchedule::default();
        let mut r = rng();
        let before = r.state();
        assert_eq!(s.check(-1, 100, &mut r), RestCheck::Quiet);
        assert_eq!(r.state(), before);
    }

    /// The counter is pre-incremented and compared with `>=`, then reset — so
    /// the check fires on exactly every `period`-th iteration, and the PRNG
    /// advances once per firing, not once per iteration.
    #[test]
    fn the_check_fires_every_period_iterations_with_one_draw_each() {
        let mut s = RestEncounterSchedule::default();
        let mut r = rng();
        let mut draws = 0;
        let mut fired = Vec::new();
        for i in 1..=30 {
            // percentage 0: the roll is still taken, and always misses
            // (`1..=100 <= 0` is false).
            match s.check(6, 0, &mut r) {
                RestCheck::Quiet => {}
                RestCheck::Rolled { .. } => {
                    draws += 1;
                    fired.push(i);
                }
                RestCheck::Interrupted { .. } => panic!("percentage 0 can never fire"),
            }
        }
        assert_eq!(fired, [6, 12, 18, 24, 30], "no carry, no drift");
        assert_eq!(draws, 5, "one draw per firing, five in thirty iterations");
    }

    /// `roll_dice(100, 1)` is `Random(100) + 1` — the inclusive `1..=100` the
    /// `<=` comparison is written against. A percentage of 100 always fires; a
    /// percentage of 0 never does, but still draws.
    #[test]
    fn the_roll_is_one_to_one_hundred_inclusive() {
        let mut r = rng();
        let mut lo = u16::MAX;
        let mut hi = 0;
        for _ in 0..4000 {
            let mut s = RestEncounterSchedule::default();
            match s.check(1, 100, &mut r) {
                RestCheck::Interrupted { roll } => {
                    lo = lo.min(roll);
                    hi = hi.max(roll);
                }
                other => panic!("percentage 100 must always fire, got {other:?}"),
            }
        }
        assert_eq!(lo, 1, "the +1 makes 0 unreachable");
        assert_eq!(hi, 100, "…and 100 reachable");
    }

    /// The interrupting arm resets the counter like the missing one does — the
    /// reset is above the roll, not inside its `if`.
    #[test]
    fn a_firing_check_resets_the_counter_whether_or_not_it_interrupts() {
        let mut r = rng();
        let mut hit = RestEncounterSchedule::default();
        assert!(matches!(
            hit.check(1, 100, &mut r),
            RestCheck::Interrupted { .. }
        ));
        assert_eq!(hit.count(), 0);

        let mut miss = RestEncounterSchedule::default();
        assert!(matches!(miss.check(1, 0, &mut r), RestCheck::Rolled { .. }));
        assert_eq!(miss.count(), 0);
    }

    /// The quirk `vm_init_ecl` leaves behind: the period/percentage pair is
    /// zeroed at every block entry (`ovr008.cs:111-112`) but the counter is
    /// not, so a count accumulated in one area can make the very first check in
    /// a newly-armed one fire immediately.
    #[test]
    fn a_stale_counter_survives_a_block_change_and_can_fire_early() {
        let mut s = RestEncounterSchedule::default();
        let mut r = rng();
        // Area A: period 10, five iterations in.
        for _ in 0..5 {
            assert_eq!(s.check(10, 50, &mut r), RestCheck::Quiet);
        }
        assert_eq!(s.count(), 5);
        // NEWECL: the cells are wiped, the counter is not — and the new area
        // arms a period of 6, so the very next iteration fires.
        assert!(!matches!(s.check(6, 100, &mut r), RestCheck::Quiet));
    }

    /// The cells are read out of the raw Party-window store as signed shorts.
    #[test]
    fn schedule_cells_reads_the_pair_as_signed_shorts() {
        let mut vm = crate::vmhost::VmMemoryState::new();
        vm.poke_raw(REST_INCOUNTER_PERIOD_ADDR, 0x001E);
        vm.poke_raw(REST_INCOUNTER_PERCENTAGE_ADDR, 0x000A);
        assert_eq!(schedule_cells(&vm), (30, 10), "ECL4#37's own arming");

        vm.poke_raw(REST_INCOUNTER_PERIOD_ADDR, 0xFFFF);
        assert_eq!(schedule_cells(&vm).0, -1);
    }
}
