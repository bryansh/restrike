//! **D-CV3 — time**: the original's wall-clock sleeps as whole ticks.
//!
//! coab paces combat with `SysDelay(ms)`; we have no clock (D9) and no
//! blocking loop (D8), so every §1.4 quantity converts once, here, through
//! D-UI1's rule — `ticks = max(1, round(ms · 60 / 1000))` — and the timeline
//! then counts whole ticks. Nothing in the scene ever reads a clock or sleeps.
//!
//! **Determinism is per-tick.** `game_speed_var` scales only the beats that
//! coab scales (`GameDelay` and the magic-stars repeat count); it never
//! changes *what* a frame contains, only how long it is held. That is what
//! lets a host run the reel at N×, and what lets the timeline's fast-drain
//! (D-CV3's open speed/skip door) collapse a step to zero ticks without any
//! downstream observer being able to tell.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/seg041.cs:335-339` (`GameDelay`) — `SysDelay(game_speed_var *
//!   100)`, and `engine/seg001.cs:274` — the default `game_speed_var` of 4.
//! - `engine/ovr009.cs:672-704` (the Speed menu) — the player-settable 0–9
//!   range, which is why speed 0 exists and has to mean "one tick", not zero.
//! - `engine/ovr014.cs:940` — `SysDelay(100)`, the attack pose hold.
//! - `engine/ovr014.cs:1590-1671` (`DrawRangedAttack`) — the per-weapon-class
//!   missile step delays (see [`super::missile`] for the class table itself).
//! - `engine/ovr023.cs:762` / `:2052` — the spell projectile's `0x1E` and the
//!   lightning bolt's `0x32`.
//! - `engine/ovr025.cs:1152-1163` (`MagicAttackDisplay`) — `SysDelay(70)` per
//!   burst frame, four frames, `game_speed_var + 1` repeats for the stars.
//! - `engine/ovr033.cs:562-575` (`CombatantKilled`) — nine `SysDelay(10)`
//!   alternations, then a `GameDelay`.
//! - `engine/ovr009.cs:181,274` — `SysDelay(0x0C8)`, the quick-fight engage.
//! - `engine/ovr014.cs:251-321` (`sub_3E748`) — a movement step has **no**
//!   delay at all; it is a redraw and a sound.

/// Ticks per second — D-UI1's presentation clock.
pub const TICKS_PER_SECOND: u32 = 60;

/// `game_speed_var`'s boot value (`seg001.cs:274`).
pub const DEFAULT_GAME_SPEED: u8 = 4;

/// The Speed menu's range (`ovr009.cs:672-704`).
pub const MAX_GAME_SPEED: u8 = 9;

/// D-UI1's millisecond → tick rule: `max(1, round(ms · 60 / 1000))`.
///
/// Rounded half **up** in integer arithmetic (`(ms·60 + 500) / 1000`). No §1.4
/// quantity lands on a tie — the first one would be 25 ms — so the tie rule is
/// a statement of intent rather than a decision any beat depends on.
///
/// The `max(1, …)` floor is load-bearing at the bottom of the Speed menu:
/// `GameDelay` at speed 0 is `SysDelay(0)`, which on the original is "no
/// delay". A zero-tick beat would make the whole message vanish inside one
/// frame, so it holds for exactly one — the shortest thing a 60 Hz presenter
/// can show.
pub const fn ms_to_ticks(ms: u32) -> u32 {
    let ticks = (ms * TICKS_PER_SECOND + 500) / 1000;
    if ticks == 0 {
        1
    } else {
        ticks
    }
}

/// The attack pose hold — `SysDelay(100)` after the attacker's Attack frame
/// goes up and before the missile flies (`ovr014.cs:940`). 6 ticks.
pub const ATTACK_POSE_MS: u32 = 100;

/// One on-target burst frame — `SysDelay(70)` (`ovr025.cs:1160`). 4 ticks.
pub const BURST_FRAME_MS: u32 = 70;

/// Frames in one burst pass (`for frame = 0; frame <= 3`, `ovr025.cs:1154`).
pub const BURST_FRAMES: u32 = 4;

/// One death-flash alternation — `SysDelay(10)` (`ovr033.cs:574`). 1 tick.
pub const DEATH_FLASH_MS: u32 = 10;

/// Death-flash alternations (`for var_3 = 0; var_3 <= 8`, `ovr033.cs:561`).
pub const DEATH_FLASH_FRAMES: u32 = 9;

/// The quick-fight engage pause — `SysDelay(0x0C8)` (`ovr009.cs:181,274`).
/// 12 ticks. Not reached by the AI-driven reel; it is the manual menu's 'Q'
/// and the party-wide `0x10` key (M6c).
pub const QUICK_FIGHT_ENGAGE_MS: u32 = 200;

/// The clock one fight is paced by: `game_speed_var` and the beats derived
/// from it. Everything else in §1.4 is a fixed millisecond count and lives as
/// a `const` above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeatClock {
    game_speed: u8,
}

impl Default for BeatClock {
    fn default() -> Self {
        BeatClock::new(DEFAULT_GAME_SPEED)
    }
}

impl BeatClock {
    /// Clamped to the Speed menu's own 0–9 range.
    pub fn new(game_speed: u8) -> Self {
        BeatClock {
            game_speed: game_speed.min(MAX_GAME_SPEED),
        }
    }

    pub fn game_speed(&self) -> u8 {
        self.game_speed
    }

    pub fn set_game_speed(&mut self, game_speed: u8) {
        self.game_speed = game_speed.min(MAX_GAME_SPEED);
    }

    /// `GameDelay()` (`seg041.cs:335-339`) — `speed × 100` ms. The message
    /// beat: at the default 4 that is 400 ms, 24 ticks.
    pub fn game_delay(&self) -> u32 {
        ms_to_ticks(self.game_speed as u32 * 100)
    }

    /// `MagicAttackDisplay`'s star repeat count (`ovr025.cs:1152`): `loops =
    /// game_speed_var`, and the loop is `for loop = 0; loop <= loops`, so the
    /// four-frame pass runs `speed + 1` times. The no-stars variant passes
    /// `loops = 0` — one pass, followed by a `GameDelay`.
    pub fn star_burst_passes(&self) -> u32 {
        self.game_speed as u32 + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_d_ui1_rule_matches_the_hand_computed_table() {
        // Every §1.4 quantity, converted by hand from `round(ms·60/1000)`.
        assert_eq!(ms_to_ticks(400), 24, "GameDelay at the default speed 4");
        assert_eq!(ms_to_ticks(100), 6, "attack pose hold");
        assert_eq!(ms_to_ticks(70), 4, "burst frame");
        assert_eq!(ms_to_ticks(50), 3, "axe/club/oil and lightning step");
        assert_eq!(ms_to_ticks(30), 2, "spell projectile step");
        assert_eq!(ms_to_ticks(20), 1, "the default weapon class's step");
        assert_eq!(ms_to_ticks(10), 1, "arrow/sling step, death flash");
        assert_eq!(ms_to_ticks(200), 12, "quick-fight engage");
    }

    #[test]
    fn a_zero_millisecond_delay_still_holds_one_tick() {
        // Speed 0's `GameDelay` is `SysDelay(0)`. A zero-tick beat would never
        // be seen at all, so the D-UI1 floor gives it the shortest frame.
        assert_eq!(ms_to_ticks(0), 1);
        assert_eq!(BeatClock::new(0).game_delay(), 1);
    }

    #[test]
    fn game_delay_tracks_the_speed_menu_end_to_end() {
        assert_eq!(BeatClock::new(0).game_delay(), 1);
        assert_eq!(BeatClock::default().game_delay(), 24);
        assert_eq!(BeatClock::new(9).game_delay(), 54);
        // Past the menu's own range the clock clamps rather than extrapolating.
        assert_eq!(BeatClock::new(200).game_speed(), MAX_GAME_SPEED);
    }

    #[test]
    fn stars_repeat_speed_plus_one_times() {
        assert_eq!(BeatClock::new(0).star_burst_passes(), 1);
        assert_eq!(BeatClock::default().star_burst_passes(), 5);
        assert_eq!(BeatClock::new(9).star_burst_passes(), 10);
    }
}
