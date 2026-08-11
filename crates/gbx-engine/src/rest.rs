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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

// ===========================================================================
// ★ The rest clock and the learning schedule (roll-credits §8, D-S4c's Rest)
// ===========================================================================
//
// Everything below is `gbl.timeToRest` and its loop — a **separate** clock from
// the world clock. `resting` counts `timeToRest` DOWN five minutes at a time
// while pushing the world clock UP by the same `step_game_time(1, 5)`.

/// `timeScales` / `word_1A13C` (`ovr021.cs:8`) — slot `i` carries into slot
/// `i+1` at `TIME_SCALES[i]`.
///
/// The slot identities follow from the scales plus `display_resting_time`'s own
/// labels (`ovr021.cs:220-247`, and `resting_time_menu`'s Days/Hours/Mins word
/// mapping at `:314-324`):
///
/// | slot | is | carries at |
/// |---|---|---|
/// | 0 | tenths of a minute | 10 |
/// | 1 | minutes, ones | 10 |
/// | 2 | minutes, tens | 6 |
/// | 3 | hours | 24 |
/// | 4 | days | 30 |
/// | 5 | months | 12 |
/// | 6 | years | 256 |
///
/// ★ **A naming correction, banked here with its evidence.** coab calls slot 5
/// `time_year` (`Classes/Area1.cs:58-59`, record offset `0x196`) and leaves
/// slot 6 as `field_198`. The scales say otherwise: slot 4 (days) carries at
/// **30** and slot 5 carries at **12**, so slot 5 is *months* and slot 6 is the
/// year — which is also why `NormalizeClock`'s slot-6 arm ages the party rather
/// than carrying (`ovr021.cs:121-127`). This module names them correctly;
/// `crate::movement::GameClock`'s own cell mapping still carries coab's names
/// and its documented placeholder minutes-per-unit, and is left alone here
/// (correcting it moves the walk goldens and the ScriptMemory clock cells —
/// its own change, not this door's).
pub const TIME_SCALES: [i32; 7] = [10, 10, 6, 24, 30, 12, 0x100];

/// `Classes/RestTime.cs` — the seven-slot countdown `gbl.timeToRest` holds,
/// indexable exactly like coab's `this[int index]` (`:54-100`).
///
/// Not `EngineState` and not serialized by the original's `SaveGame`: it is a
/// `gbl` scratch value that `MakeCamp` clears on entry (`ovr016.cs:1086`) and
/// `rest_menu` clears again after every rest (`ovr016.cs:293`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RestTime {
    pub slots: [i32; 7],
}

/// The four slots `resting`'s loop condition and `rest_time_subtract`'s guard
/// test (`ovr021.cs:177-180`, `:548-552`): minutes-ones, minutes-tens, hours,
/// days. Slot 0 and the month/year slots are deliberately not in it — the same
/// four fields, in the original's own order.
const COUNTDOWN_SLOTS: [usize; 4] = [4, 3, 2, 1];

impl RestTime {
    /// `RestTime.Clear()` (`:43-52`).
    pub fn clear(&mut self) {
        self.slots = [0; 7];
    }

    /// True while any of the four countdown slots is non-zero — `resting`'s
    /// loop condition (`ovr021.cs:548-552`).
    pub fn any_time_left(&self) -> bool {
        COUNTDOWN_SLOTS.iter().any(|&i| self.slots[i] > 0)
    }

    /// The displayed triple `display_resting_time` renders (`ovr021.cs:234-246`):
    /// days, hours, and minutes as `tens * 10 + ones`.
    pub fn display_parts(&self) -> (i32, i32, i32) {
        (
            self.slots[4],
            self.slots[3],
            self.slots[2] * 10 + self.slots[1],
        )
    }
}

/// `NormalizeClock` / `sub_58317` (`ovr021.cs:110-130`) — one carry pass, low
/// slot to high. Returns `true` when the slot-6 arm fired, which in the
/// original ages **every party member by one year** and, pointedly, does *not*
/// subtract (`:121-127`). Unreachable for a rest countdown (256 years); the
/// return value exists so the world-clock caller can honour it if this ever
/// gets wired there.
pub fn normalize_clock(t: &mut RestTime) -> bool {
    let mut aged = false;
    for (i, scale) in TIME_SCALES.iter().enumerate() {
        if t.slots[i] >= *scale {
            if i != 6 {
                t.slots[i + 1] += 1;
                t.slots[i] -= *scale;
            } else {
                aged = true;
            }
        }
    }
    aged
}

/// `clock_583C8` / `sub_583C8` (`ovr021.cs:133-148`): normalize, then flatten
/// any whole month back down into days (`30 * months`) and **cap days at 99**.
///
/// The cap is why the rest menu can never ask for more than 99 days: `Add` on
/// the Days field runs through here every time (`:336`).
pub fn clock_583c8(t: &mut RestTime) {
    normalize_clock(t);
    if t.slots[5] > 0 {
        t.slots[4] += TIME_SCALES[4] * t.slots[5];
        t.slots[5] = 0;
        if t.slots[4] > 99 {
            t.slots[4] = 99;
        }
    }
}

/// `rest_time_5849F` / `sub_5849F` (`ovr021.cs:175-211`) — subtract `amount`
/// from slot `time_index`, borrowing from higher slots when it does not fit.
///
/// Three details worth keeping:
/// - The whole body is skipped when all four countdown slots are already zero
///   (`:177-180`), so subtracting from an exhausted clock is a no-op.
/// - The borrow scan stops at slot 5: when nothing below the month slot has
///   anything left, the clock is **cleared outright** and the subtraction
///   becomes zero (`:192-196`) — that is how the last partial five minutes of a
///   rest disappear rather than going negative.
/// - Every call ends in [`clock_583c8`] (`:209`).
pub fn rest_time_subtract(t: &mut RestTime, time_index: usize, amount: i32) {
    if !t.any_time_left() {
        return;
    }
    let mut amount = amount;
    while amount > t.slots[time_index] {
        let mut donor = time_index + 1;
        while donor < 5 && t.slots[donor] == 0 {
            donor += 1;
        }
        if donor == 5 {
            t.clear();
            amount = 0;
        } else {
            for i in (time_index + 1..=donor).rev() {
                t.slots[i] -= 1;
                t.slots[i - 1] += TIME_SCALES[i - 1];
            }
        }
    }
    t.slots[time_index] -= amount;
    clock_583c8(t);
}

// ---------------------------------------------------------------------------
// How long the party must rest — `sub_44032` and `rest_menu`'s split
// ---------------------------------------------------------------------------

/// ★ `sub_44032` (`ovr016.cs:8-64`) — one character's required rest, **in
/// minutes**, and the `spell_to_learn_count` it sets as a side effect.
///
/// ```text
/// walk the staged (Learning) spells:  max_spell_level, total_spell_level
/// walk every scroll's three affects with the high bit set:
///                                     max_scribe_level, total_scribe_level
/// count = 0
/// if total_spell_level > 0 || total_scribe_level > 0   -> count = 4
/// if max_spell_level  > 2 || max_scribe_level  > 2     -> count = 6
/// player.spell_to_learn_count = count
/// return count * 0x3C + total_scribe_level * 0x0F + total_spell_level * 0x0F
/// ```
///
/// So: **four hours of study, six if anything is level 3 or higher**, plus
/// **fifteen minutes per spell level** across everything being learned. The two
/// `if`s are sequential, not `else if` — a level-3 spell overwrites the 4 with
/// a 6 (`:55-58`).
///
/// `count` is *hours*: [`SpellLearning::tick_study_hour`] decrements it once per
/// twelve loop iterations, and twelve iterations is sixty minutes.
///
/// The scroll scan uses `> 0x7f` (`:34`), unlike `rest_scribe`'s `> 0x80`
/// (`ovr021.cs:425`) — see [`SpellLearning::scribe_next`].
pub fn required_rest_minutes(
    ch: &mut crate::party::Character,
    scrolls: &crate::magic::ScrollLookup,
) -> i32 {
    let mut max_spell_level = 0;
    let mut total_spell_level = 0;
    for e in crate::magic::learning(&ch.magic.spell_list) {
        let lvl = i32::from(crate::magic::spell_level(e.id));
        max_spell_level = max_spell_level.max(lvl);
        total_spell_level += lvl;
    }

    let mut max_scribe_level = 0;
    let mut total_scribe_level = 0;
    for item in &ch.items {
        if !scrolls.is_scroll(item) {
            continue;
        }
        for sp in crate::magic::scroll_spells(item).filter(|s| s.scribing) {
            let lvl = i32::from(crate::magic::spell_level(sp.id));
            max_scribe_level = max_scribe_level.max(lvl);
            total_scribe_level += lvl;
        }
    }

    let mut count = 0u8;
    if total_spell_level > 0 || total_scribe_level > 0 {
        count = 4;
    }
    if max_spell_level > 2 || max_scribe_level > 2 {
        count = 6;
    }
    ch.magic.spell_to_learn_count = count;

    i32::from(count) * 0x3C + total_scribe_level * 0x0F + total_spell_level * 0x0F
}

/// `rest_menu`'s own head (`ovr016.cs:274-289`): the party's **maximum**
/// required time, split into the countdown's day/hour/minute slots.
///
/// The split is the original's, verbatim (`:287-289`) — hours = `minutes / 60`,
/// tens-of-minutes = `(minutes - hours*60) / 10`, ones = `minutes % 10`. It
/// never fills the day slot: a party can need at most six hours plus change, so
/// `minutes / 60` is the whole story.
///
/// Note it calls `sub_44032` for **every** member, so every member's
/// `spell_to_learn_count` is (re)armed here — including the members who need
/// no study at all, who get `0`.
pub fn rest_menu_time(
    party: &mut crate::party::Party,
    scrolls: &crate::magic::ScrollLookup,
) -> RestTime {
    let mut max_rest_time = 0;
    for member in &mut party.members {
        max_rest_time = max_rest_time.max(required_rest_minutes(member, scrolls));
    }
    let mut t = RestTime::default();
    t.slots[3] = max_rest_time / 60;
    t.slots[2] = (max_rest_time - t.slots[3] * 60) / 10;
    t.slots[1] = max_rest_time % 10;
    t
}

// ---------------------------------------------------------------------------
// Healing — `heal_player`, and camp Fix's arithmetic around it
// ---------------------------------------------------------------------------

/// `Status` (`Classes/Enums.cs:7-18`).
pub mod status {
    pub const OKEY: u8 = 0x0;
    pub const ANIMATED: u8 = 0x1;
    pub const UNCONSCIOUS: u8 = 0x4;
    pub const DYING: u8 = 0x5;
}

/// `heal_player(arg_0, amount, player)` (`ovr024.cs:1335-1370`), the
/// out-of-combat half.
///
/// The status gate is `okey || animated || unconscious || dying` (`:1337-1340`)
/// — a dead, stoned or gone character is not healed by anything here. The
/// inner gate (`:1342-1344`) lets a full-HP character through only when
/// `arg_0 == 0`, which is how the caller distinguishes "top up" from "cure",
/// and every camp caller passes `0`.
///
/// The `dying → unconscious` promotion runs only for a character who is not
/// `in_combat` (`:1353-1358`); the further `unconscious →` affect removal
/// (`CallAffectTable(Remove, …, affect_4e)`, `:1360-1364`) needs the
/// out-of-combat affect system, which this engine does not have — named here
/// rather than silently skipped.
///
/// Returns whether anything changed (coab's `true` return drives a
/// `PartySummary` repaint).
pub fn heal_player(arg_0: u8, amount: i32, ch: &mut crate::party::Character) -> bool {
    let st = ch.status.health_status;
    if !matches!(
        st,
        status::OKEY | status::ANIMATED | status::UNCONSCIOUS | status::DYING
    ) {
        return false;
    }
    if !(ch.hit_point_current < ch.hit_point_max
        || (ch.hit_point_current >= ch.hit_point_max && arg_0 == 0))
    {
        return false;
    }
    let healed = (i32::from(ch.hit_point_current) + amount).clamp(0, i32::from(ch.hit_point_max));
    ch.hit_point_current = healed as u8;
    if !ch.status.in_combat && ch.status.health_status == status::DYING {
        ch.status.health_status = status::UNCONSCIOUS;
        // TODO(G8/affects): `unconscious` additionally clears `affect_4e`
        // (`ovr024.cs:1360-1364`) — no out-of-combat affect table yet.
    }
    true
}

// ---------------------------------------------------------------------------
// The learning schedule — `spellLaernTimeout`, `CheckForSpellLearning`,
// `sub_58C03`, `rest_memorize`, `rest_scribe`
// ---------------------------------------------------------------------------

/// What one rest iteration did to somebody's spells, for the message line
/// (`DisplayCaseSpellText`, `ovr023.cs:3114-3134`: "<name> has memorized" /
/// "has scribed", then the spell name on the next row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestEvent {
    /// `rest_memorize` committed a staged spell (`ovr021.cs:403`, `MarkLearnt`).
    Memorized { member: usize, spell_id: u8 },
    /// `rest_scribe` copied a scroll spell into the grimoire (`ovr021.cs:434`).
    Scribed { member: usize, spell_id: u8 },
    /// The scroll ran out of charges and was destroyed
    /// (`remove_spell_from_scroll` → `lose_item`, `ovr023.cs:3109`).
    ScrollConsumed { member: usize },
    /// `rest_heal`'s eight-hour tick healed the party one hit point
    /// (`ovr021.cs:362-388`) — "The Whole Party Is Healed".
    PartyHealed,
    /// The rest-encounter check fired (`ovr021.cs:594-602`).
    Interrupted,
}

/// `spellLaernTimeout` (`seg600:758D`, `ovr021.cs:452`) plus `resting`'s own
/// `var_C` study counter — the whole of the learning schedule's state.
///
/// Nine slots for at most eight party members because the original indexes them
/// **from 1** (`ovr021.cs:457`, `:491`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SpellLearning {
    timeouts: [i32; 9],
    /// `resting`'s `var_C` (`ovr021.cs:528`) — counts iterations to twelve,
    /// i.e. to one hour.
    study_counter: i32,
}

impl SpellLearning {
    /// `resting`'s opening `System.Array.Clear(spellLaernTimeout, 0,
    /// gbl.TeamList.Count)` (`ovr021.cs:521`), replicated **exactly**.
    ///
    /// ★ It is off by one against its own indexing: the players occupy slots
    /// `1..=Count` but the clear covers `0..Count-1`, so the LAST member's
    /// timeout survives from one rest into the next. Almost always harmless
    /// (a completed rest leaves every timeout at 0); it bites only when a rest
    /// is interrupted with that member mid-spell, whose next rest then starts
    /// the countdown already part-way through. Kept because the shape —
    /// a 9-slot array indexed from 1, cleared for `party_size` entries from
    /// the base — is exactly what a `memset(base, 0, party_size * 2)` in the
    /// original would produce. Flagged for the listing to settle.
    pub fn reset(&mut self, party_count: usize) {
        for slot in self.timeouts.iter_mut().take(party_count.min(9)) {
            *slot = 0;
        }
        self.study_counter = 0;
    }

    /// The timeout slot for party index `i` (0-based here, 1-based in the
    /// original).
    pub fn timeout(&self, member: usize) -> i32 {
        self.timeouts.get(member + 1).copied().unwrap_or(0)
    }

    /// `rest_memorize` (`ovr021.cs:393-413`).
    ///
    /// With `find_next == true` it only **peeks**: the level of the first
    /// staged spell, nothing committed. With `false` it commits that spell
    /// (`MarkLearnt`, `:403`), flips `find_next`, and keeps walking — so the
    /// returned level belongs to the spell *after* the one just learned, and is
    /// `0` when there is no next.
    fn memorize_next(
        find_next: &mut bool,
        member: usize,
        ch: &mut crate::party::Character,
        events: &mut Vec<RestEvent>,
    ) -> i32 {
        let staged: Vec<u8> = crate::magic::learning(&ch.magic.spell_list)
            .map(|e| e.id)
            .collect();
        for id in staged {
            if *find_next {
                return i32::from(crate::magic::spell_level(id));
            }
            crate::magic::mark_learnt(&mut ch.magic.spell_list, id);
            events.push(RestEvent::Memorized {
                member,
                spell_id: id,
            });
            *find_next = true;
        }
        0
    }

    /// `rest_scribe` (`ovr021.cs:416-450`), with `remove_spell_from_scroll`
    /// (`ovr023.cs:3090-3112`) folded in at its one call site.
    ///
    /// ★ **The gate here is `> 0x80`, strictly** (`ovr021.cs:425`), while
    /// `sub_44032`'s own scroll scan uses `> 0x7f` (`ovr016.cs:34`). The two
    /// disagree about exactly one byte value — `0x80`, i.e. spell id 0 staged —
    /// which no real scroll can carry (id 0 is the empty-affect marker). Both
    /// transcribed as written rather than reconciled.
    ///
    /// `remove_spell_from_scroll` picks the **LAST** matching affect index
    /// (`ovr023.cs:3094-3100` assigns without breaking), zeroes it, decrements
    /// `namenum2`, and destroys the scroll once that falls below `0xD2`.
    fn scribe_next(
        find_next: &mut bool,
        member: usize,
        ch: &mut crate::party::Character,
        scrolls: &crate::magic::ScrollLookup,
        events: &mut Vec<RestEvent>,
    ) -> i32 {
        let mut next_scribe_lvl = 0;
        let mut consumed: Option<usize> = None;
        for item_idx in 0..ch.items.len() {
            if !scrolls.is_scroll(&ch.items[item_idx]) {
                continue;
            }
            for affect_index in 1..4 {
                if next_scribe_lvl != 0 {
                    break;
                }
                let raw = gbx_formats::save_orig::item_affect(&ch.items[item_idx], affect_index);
                if raw <= 0x80 {
                    continue;
                }
                let id = raw & 0x7F;
                if *find_next {
                    next_scribe_lvl = i32::from(crate::magic::spell_level(id));
                } else {
                    // `player.LearnSpell(spell)` — `spellBook[id - 1] = 1`
                    // (`Player.cs:364`).
                    if id >= 1 {
                        let slot = id as usize - 1;
                        if ch.magic.spell_book.len() <= slot {
                            ch.magic.spell_book.resize(slot + 1, 0);
                        }
                        ch.magic.spell_book[slot] = 1;
                    }
                    if remove_spell_from_scroll(&mut ch.items[item_idx], id) {
                        consumed = Some(item_idx);
                    }
                    events.push(RestEvent::Scribed {
                        member,
                        spell_id: id,
                    });
                    *find_next = true;
                }
            }
            if next_scribe_lvl != 0 {
                break;
            }
        }
        if let Some(idx) = consumed {
            ch.items.remove(idx);
            ch.readied_items = ch
                .readied_items
                .iter()
                .filter(|&&i| i != idx)
                .map(|&i| if i > idx { i - 1 } else { i })
                .collect();
            events.push(RestEvent::ScrollConsumed { member });
        }
        next_scribe_lvl
    }

    /// `CheckForSpellLearning` / `sub_58B4D` (`ovr021.cs:454-480`) — run once
    /// per rest iteration, i.e. once per five minutes.
    ///
    /// Decrement first, **then** test for zero, so a timeout of 1 fires in the
    /// same iteration. A member still studying (`spell_to_learn_count > 0`)
    /// never learns here — that is [`Self::tick_study_hour`]'s job. The spell
    /// that lands sets the next one's timeout to `level * 3` iterations, i.e.
    /// fifteen minutes per level, which is exactly the rate
    /// [`required_rest_minutes`] budgets.
    pub fn check_for_spell_learning(
        &mut self,
        party: &mut crate::party::Party,
        scrolls: &crate::magic::ScrollLookup,
        events: &mut Vec<RestEvent>,
    ) {
        for member in 0..party.members.len() {
            let slot = member + 1;
            if slot >= self.timeouts.len() {
                break;
            }
            if self.timeouts[slot] > 0 {
                self.timeouts[slot] -= 1;
            }
            if self.timeouts[slot] == 0 && party.members[member].magic.spell_to_learn_count == 0 {
                let mut find_next = false;
                let ch = &mut party.members[member];
                let mut next = Self::scribe_next(&mut find_next, member, ch, scrolls, events);
                if next == 0 {
                    next = Self::memorize_next(&mut find_next, member, ch, events);
                }
                self.timeouts[slot] = next * 3;
            }
        }
    }

    /// `sub_58C03` (`ovr021.cs:483-511`) — the study hour.
    ///
    /// Twelve iterations (twelve × five minutes = one hour) tick every
    /// still-studying member's `spell_to_learn_count` down by one. The member
    /// whose count reaches **zero** peeks the first pending spell (`find_next`
    /// starts `true`, so nothing is learned yet) and arms its timeout at
    /// `level * 2` iterations — ten minutes per level, not the fifteen every
    /// subsequent spell gets. That asymmetry is the original's; it is why the
    /// first spell of a rest always lands a little early.
    pub fn tick_study_hour(
        &mut self,
        party: &mut crate::party::Party,
        scrolls: &crate::magic::ScrollLookup,
        events: &mut Vec<RestEvent>,
    ) {
        self.study_counter += 1;
        if self.study_counter < 12 {
            return;
        }
        self.study_counter = 0;
        for member in 0..party.members.len() {
            let slot = member + 1;
            if slot >= self.timeouts.len() {
                break;
            }
            let count = party.members[member].magic.spell_to_learn_count;
            if count == 0 {
                continue;
            }
            let next_count = count - 1;
            party.members[member].magic.spell_to_learn_count = next_count;
            if next_count != 0 {
                continue;
            }
            let mut find_next = true;
            let ch = &mut party.members[member];
            let mut next = Self::scribe_next(&mut find_next, member, ch, scrolls, events);
            if next == 0 {
                next = Self::memorize_next(&mut find_next, member, ch, events);
            }
            self.timeouts[slot] = next * 2;
        }
    }
}

/// `remove_spell_from_scroll` (`sub_623FF`, `ovr023.cs:3090-3112`). Returns
/// whether the scroll should now be dropped (`lose_item`, `:3109`).
fn remove_spell_from_scroll(item: &mut [u8], spell_id: u8) -> bool {
    let mut affect_index = 0;
    for index in 1..=3 {
        if gbx_formats::save_orig::item_affect(item, index) & 0x7F == spell_id {
            affect_index = index; // LAST match wins — the original never breaks
        }
    }
    if affect_index == 0 {
        return false;
    }
    gbx_formats::save_orig::set_item_affect(item, affect_index, 0);
    let namenum2 = gbx_formats::save_orig::item_namenum2(item).wrapping_sub(1);
    gbx_formats::save_orig::set_item_namenum2(item, namenum2);
    namenum2 < 0xD2
}

// ---------------------------------------------------------------------------
// ★ The rest loop itself — `resting` (`ovr021.cs:516-612`), as a stepper
// ---------------------------------------------------------------------------

/// `resting`'s loop state, one iteration at a time.
///
/// The original blocks in a `while`; this engine is immediate-mode (D-UI1), so
/// the loop is inverted into [`RestSession::step`] and the camp screen calls it
/// per tick. Nothing else changes: the order of operations inside one iteration
/// is `ovr021.cs:571-604` verbatim, and one iteration is still five minutes.
///
/// **Not saved by the original** (`SaveGame` serializes neither `timeToRest`,
/// `rest_10_seconds`, `spellLaernTimeout` nor `rest_incounter_count`), so this
/// lives in the camp screen's own state rather than in `EngineState` — which is
/// also why this slice needs no `SAVE_FORMAT_VERSION` bump.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RestSession {
    pub time_to_rest: RestTime,
    /// `gbl.rest_10_seconds` (`ovr016.cs:1084`, `ovr021.cs:360`).
    pub rest_10_seconds: i32,
    pub learning: SpellLearning,
    pub schedule: RestEncounterSchedule,
    /// `display_counter` (`ovr021.cs:529`) — the interactive redraw every fifth
    /// iteration (`:574-579`).
    pub display_counter: i32,
    /// `resting_intetrupted` (`:519`) — the value `MakeCamp` propagates and
    /// `TryEncamp` turns into the ECL header's vector 3.
    pub interrupted: bool,
    /// `stop_resting` (`:518`).
    pub stopped: bool,
}

/// `rest_heal`'s threshold (`ovr021.cs:362`): `8 * 36` iterations. At five
/// minutes each that is a full day of game time per party-wide hit point —
/// the name `rest_10_seconds` is vestigial, the counter counts loop passes.
pub const REST_HEAL_PERIOD: i32 = 8 * 36;

/// `rest_heal`'s own line (`ovr021.cs:379`).
pub const PARTY_HEALED_TEXT: &str = "The Whole Party Is Healed";

impl RestSession {
    /// `resting`'s preamble (`ovr021.cs:518-529`): clear the learning
    /// timeouts for the party, zero the counters, and take the countdown the
    /// caller computed.
    ///
    /// `gbl.affects_timed_out[0..0x48] = true` (`:523-526`) has no counterpart
    /// here: `CheckAffectsTimingOut` is the out-of-combat affect ticker
    /// (`ovr021.cs:11-107`), and this engine has no out-of-combat affect
    /// system yet (G7's tail). Named, not silently dropped.
    pub fn start(time_to_rest: RestTime, party_count: usize) -> Self {
        let mut learning = SpellLearning::default();
        learning.reset(party_count);
        RestSession {
            time_to_rest,
            rest_10_seconds: 0,
            learning,
            schedule: RestEncounterSchedule::default(),
            display_counter: 0,
            interrupted: false,
            stopped: false,
        }
    }

    /// `resting`'s loop condition (`ovr021.cs:548-552`).
    pub fn resting(&self) -> bool {
        !self.stopped && self.time_to_rest.any_time_left()
    }

    /// The player answering "Stop Resting? Y" (`ovr021.cs:559-562`).
    pub fn stop(&mut self) {
        self.stopped = true;
    }

    /// ★ One iteration of `resting`'s body (`ovr021.cs:569-605`), in the
    /// original's order:
    ///
    /// 1. `rest_time_5849F(1, 5)` — five minutes off the countdown
    /// 2. `display_counter++`
    /// 3. `step_game_time(1, 5)` — five minutes onto the world clock
    /// 4. `rest_heal`
    /// 5. `CheckForSpellLearning`
    /// 6. `sub_58C03` (the study hour)
    /// 7. the rest-encounter check
    ///
    /// ★ **FD-44 is the seventh step**: the schedule transcribed in this
    /// module finally has its caller. `period`/`percentage` come from the live
    /// Party-window cells ([`schedule_cells`]), and an [`RestCheck::Interrupted`]
    /// sets both `stopped` and `interrupted` exactly as `:599-600` does.
    ///
    /// The draw is still unreachable from any captured fight: this is the only
    /// caller, its only caller is the camp screen, and a capture replays a
    /// combat entry-state snapshot straight into `CombatState` — it never
    /// camps. `interactive` only gates the redraw cadence, never a draw.
    pub fn step(
        &mut self,
        party: &mut crate::party::Party,
        clock: &mut crate::movement::GameClock,
        scrolls: &crate::magic::ScrollLookup,
        rng: &mut crate::rng::EngineRng,
        period: i16,
        percentage: i16,
    ) -> Vec<RestEvent> {
        let mut events = Vec::new();
        if !self.resting() {
            return events;
        }
        // 1-2
        rest_time_subtract(&mut self.time_to_rest, 1, 5);
        self.display_counter += 1;
        if self.display_counter >= 5 {
            self.display_counter = 0;
        }
        // 3. `step_game_time(1, 5)` — the world clock, through the engine's
        // existing slot/amount API (`crate::movement::GameClock::step`, the
        // ECL CLOCK opcode's own entry point). That type's minutes-per-unit is
        // a documented placeholder predating this slice; the arguments here are
        // the original's exactly.
        clock.step(1, 5);
        // 4
        self.rest_10_seconds += 1;
        if self.rest_10_seconds >= REST_HEAL_PERIOD {
            let mut any = false;
            for member in &mut party.members {
                any |= heal_player(0, 1, member);
            }
            if any {
                events.push(RestEvent::PartyHealed);
            }
            self.rest_10_seconds = 0;
        }
        // 5-6
        self.learning
            .check_for_spell_learning(party, scrolls, &mut events);
        self.learning.tick_study_hour(party, scrolls, &mut events);
        // 7 — FD-44
        if let RestCheck::Interrupted { .. } = self.schedule.check(period, percentage, rng) {
            self.stopped = true;
            self.interrupted = true;
            events.push(RestEvent::Interrupted);
        }
        events
    }
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

    // -----------------------------------------------------------------
    // The rest clock, the required-time formula, and the learning schedule
    // -----------------------------------------------------------------

    use crate::magic;
    use crate::party::{Character, Party};

    fn blank_char() -> Character {
        let rec = gbx_formats::save_orig::decode_char_record(&vec![
            0u8;
            gbx_formats::save_orig::CHAR_RECORD_SIZE
        ])
        .unwrap();
        let mut ch = crate::party::character_from_record(&rec, vec![], vec![]);
        ch.magic.spell_list = vec![0u8; magic::SPELL_LIST_SIZE];
        ch.magic.spell_book = vec![0u8; 100];
        ch.hit_point_max = 20;
        ch.hit_point_current = 20;
        ch
    }

    /// The scales, and the slot identities they force (see [`TIME_SCALES`]'s
    /// own doc): 60 minutes make an hour, 24 hours a day, 30 days a month.
    #[test]
    fn the_time_scales_carry_minutes_into_hours_into_days() {
        let mut t = RestTime::default();
        t.slots[1] = 9;
        normalize_clock(&mut t);
        assert_eq!(t.slots[1], 9, "nine ones do not carry");
        t.slots[1] = 10;
        normalize_clock(&mut t);
        assert_eq!((t.slots[1], t.slots[2]), (0, 1), "ten ones make one ten");
        t.slots[2] = 6;
        normalize_clock(&mut t);
        assert_eq!((t.slots[2], t.slots[3]), (0, 1), "six tens make one hour");
        t.slots[3] = 24;
        normalize_clock(&mut t);
        assert_eq!((t.slots[3], t.slots[4]), (0, 1), "24 hours make one day");
        t.slots[4] = 30;
        normalize_clock(&mut t);
        assert_eq!(
            (t.slots[4], t.slots[5]),
            (0, 1),
            "30 days make one month — which is why slot 5 is months, not years"
        );
    }

    /// `clock_583C8` flattens months back into days and caps at 99
    /// (`ovr021.cs:137-147`).
    #[test]
    fn the_rest_clock_flattens_months_into_days_and_caps_at_99() {
        let mut t = RestTime::default();
        t.slots[5] = 2;
        clock_583c8(&mut t);
        assert_eq!((t.slots[5], t.slots[4]), (0, 60), "two months = 60 days");
        t.slots[5] = 4;
        clock_583c8(&mut t);
        assert_eq!(t.slots[4], 99, "180 days clamps to the 99-day ceiling");
    }

    /// `rest_time_5849F` borrows down the slots, and clears the clock outright
    /// when nothing below the month slot can pay (`ovr021.cs:192-196`).
    #[test]
    fn subtracting_five_minutes_borrows_and_then_clears() {
        // One hour on the clock; five minutes off it borrows twice.
        let mut t = RestTime::default();
        t.slots[3] = 1;
        rest_time_subtract(&mut t, 1, 5);
        assert_eq!(t.display_parts(), (0, 0, 55));

        // Exactly five minutes left: the subtraction empties it.
        let mut t = RestTime::default();
        t.slots[1] = 5;
        rest_time_subtract(&mut t, 1, 5);
        assert!(!t.any_time_left());

        // Three minutes left and five asked for: nothing above can pay, so the
        // whole clock is cleared rather than going negative.
        let mut t = RestTime::default();
        t.slots[1] = 3;
        rest_time_subtract(&mut t, 1, 5);
        assert_eq!(t.slots, [0; 7]);

        // An exhausted clock is a no-op (`:177-180`).
        let mut t = RestTime::default();
        rest_time_subtract(&mut t, 1, 5);
        assert_eq!(t.slots, [0; 7]);
    }

    /// ★ `sub_44032`: four hours of study, six if anything is level 3+, plus
    /// fifteen minutes per spell level.
    #[test]
    fn required_rest_is_four_or_six_hours_plus_fifteen_minutes_per_level() {
        let scrolls = magic::ScrollLookup::default();
        let mut ch = blank_char();
        assert_eq!(required_rest_minutes(&mut ch, &scrolls), 0);
        assert_eq!(ch.magic.spell_to_learn_count, 0);

        // One level-1 spell: 4*60 + 15 = 255.
        magic::add_learn(&mut ch.magic.spell_list, 0x0F);
        assert_eq!(required_rest_minutes(&mut ch, &scrolls), 255);
        assert_eq!(ch.magic.spell_to_learn_count, 4);

        // Add a level-2 (0x1E Invisibility): still four hours, +30.
        magic::add_learn(&mut ch.magic.spell_list, 0x1E);
        assert_eq!(required_rest_minutes(&mut ch, &scrolls), 4 * 60 + 45);
        assert_eq!(ch.magic.spell_to_learn_count, 4);

        // Add a level-3 (0x2F Fireball): the study period jumps to six hours.
        magic::add_learn(&mut ch.magic.spell_list, 0x2F);
        assert_eq!(required_rest_minutes(&mut ch, &scrolls), 6 * 60 + 90);
        assert_eq!(ch.magic.spell_to_learn_count, 6);

        // Memorized (not staged) spells cost nothing — only `LearningList`
        // counts (`ovr016.cs:13`).
        let mut done = blank_char();
        magic::add_learnt(&mut done.magic.spell_list, 0x2F);
        assert_eq!(required_rest_minutes(&mut done, &scrolls), 0);
    }

    /// `rest_menu`'s split (`ovr016.cs:287-289`): the party's max, in
    /// days/hours/minutes, and every member's study counter armed.
    #[test]
    fn rest_menu_time_takes_the_partys_maximum_and_arms_every_member() {
        let scrolls = magic::ScrollLookup::default();
        let mut party = Party::default();
        let mut a = blank_char();
        magic::add_learn(&mut a.magic.spell_list, 0x0F); // 255 minutes
        let mut b = blank_char();
        magic::add_learn(&mut b.magic.spell_list, 0x2F); // 6*60 + 45 = 405
        party.members.push(a);
        party.members.push(b);

        let t = rest_menu_time(&mut party, &scrolls);
        assert_eq!(t.display_parts(), (0, 6, 45), "the slower caster decides");
        assert_eq!(party.members[0].magic.spell_to_learn_count, 4);
        assert_eq!(party.members[1].magic.spell_to_learn_count, 6);
    }

    fn rest_to_completion(
        party: &mut Party,
        session: &mut RestSession,
        scrolls: &magic::ScrollLookup,
    ) -> Vec<RestEvent> {
        let mut clock = crate::movement::GameClock::default();
        let mut r = EngineRng::new(1);
        let mut all = Vec::new();
        let mut guard = 0;
        while session.resting() {
            all.extend(session.step(party, &mut clock, scrolls, &mut r, 0, 0));
            guard += 1;
            assert!(guard < 10_000, "rest loop must terminate");
        }
        all
    }

    /// ★ End to end: stage two spells, rest the computed time, and watch them
    /// commit — the first at `level * 2` iterations after the study hours, the
    /// second `level * 3` after that.
    #[test]
    fn a_full_rest_commits_every_staged_spell() {
        let scrolls = magic::ScrollLookup::default();
        let mut party = Party::default();
        let mut ch = blank_char();
        ch.magic.cast_count[2][0] = 2;
        magic::add_learn(&mut ch.magic.spell_list, 0x0F); // Magic Missile, MU 1
        magic::add_learn(&mut ch.magic.spell_list, 0x15); // Sleep, MU 1
        party.members.push(ch);

        let t = rest_menu_time(&mut party, &scrolls);
        assert_eq!(t.display_parts(), (0, 4, 30), "4h study + 2 x 15 minutes");
        let mut session = RestSession::start(t, party.members.len());
        let events = rest_to_completion(&mut party, &mut session, &scrolls);

        let memorized: Vec<u8> = events
            .iter()
            .filter_map(|e| match e {
                RestEvent::Memorized { spell_id, .. } => Some(*spell_id),
                _ => None,
            })
            .collect();
        assert_eq!(
            memorized,
            vec![0x15, 0x0F],
            "ascending slot order — the most recently staged spell commits first"
        );
        assert_eq!(
            magic::learning(&party.members[0].magic.spell_list).count(),
            0,
            "nothing is left staged"
        );
        let learnt: Vec<u8> = magic::learnt(&party.members[0].magic.spell_list)
            .map(|e| e.id)
            .collect();
        assert_eq!(learnt, vec![0x15, 0x0F]);
        assert!(!session.interrupted);
    }

    /// ★ **FD-25, re-pinned.** Rest never resets `spellCastCount`. It is a
    /// fixed capacity the character record carries; the M5-era framing of rest
    /// as "spell-slot restoration" does not match the original at all. Nothing
    /// in `resting`, `CheckForSpellLearning`, `sub_58C03`, `rest_memorize` or
    /// `rest_scribe` writes it.
    #[test]
    fn rest_never_resets_cast_count() {
        let scrolls = magic::ScrollLookup::default();
        let mut party = Party::default();
        let mut ch = blank_char();
        ch.magic.cast_count = [[5, 5, 2, 0, 0], [0; 5], [4, 2, 1, 0, 0]];
        magic::add_learn(&mut ch.magic.spell_list, 0x03);
        party.members.push(ch);
        let before = party.members[0].magic.cast_count;

        let t = rest_menu_time(&mut party, &scrolls);
        let mut session = RestSession::start(t, party.members.len());
        rest_to_completion(&mut party, &mut session, &scrolls);

        assert_eq!(
            party.members[0].magic.cast_count, before,
            "cast_count is capacity, not a per-rest pool"
        );
    }

    /// The study clock: twelve iterations to the hour, `spell_to_learn_count`
    /// hours before anything is learned (`sub_58C03`).
    #[test]
    fn nothing_is_learned_until_the_study_hours_have_elapsed() {
        let scrolls = magic::ScrollLookup::default();
        let mut party = Party::default();
        let mut ch = blank_char();
        magic::add_learn(&mut ch.magic.spell_list, 0x0F);
        party.members.push(ch);
        let t = rest_menu_time(&mut party, &scrolls);
        let mut session = RestSession::start(t, party.members.len());

        let mut clock = crate::movement::GameClock::default();
        let mut r = EngineRng::new(1);
        // Four hours = 48 iterations; the 48th zeroes the count and arms the
        // first spell's `level * 2` timeout, so nothing has landed yet.
        for _ in 0..48 {
            let ev = session.step(&mut party, &mut clock, &scrolls, &mut r, 0, 0);
            assert!(!ev.iter().any(|e| matches!(e, RestEvent::Memorized { .. })));
        }
        assert_eq!(party.members[0].magic.spell_to_learn_count, 0);
        // `level * 2` = 2 more iterations.
        let mut landed = false;
        for _ in 0..2 {
            landed |= session
                .step(&mut party, &mut clock, &scrolls, &mut r, 0, 0)
                .iter()
                .any(|e| matches!(e, RestEvent::Memorized { .. }));
        }
        assert!(landed, "the first spell lands ten minutes per level later");
    }

    /// ★ FD-44: the schedule finally has a caller, and an interruption stops
    /// the rest and raises the flag `MakeCamp` propagates
    /// (`ovr021.cs:594-602`).
    #[test]
    fn a_fired_encounter_check_interrupts_the_rest() {
        let scrolls = magic::ScrollLookup::default();
        let mut party = Party::default();
        party.members.push(blank_char());
        let mut t = RestTime::default();
        t.slots[3] = 8; // eight hours
        let mut session = RestSession::start(t, 1);
        let mut clock = crate::movement::GameClock::default();
        let mut r = EngineRng::new(1);

        // period 1 / percentage 100: the very first iteration fires.
        let ev = session.step(&mut party, &mut clock, &scrolls, &mut r, 1, 100);
        assert!(ev.contains(&RestEvent::Interrupted));
        assert!(session.interrupted && session.stopped);
        assert!(!session.resting(), "the loop breaks");
        assert!(
            session.time_to_rest.any_time_left(),
            "and the unslept time stays on the clock"
        );
    }

    /// A disarmed schedule (the post-`vm_init_ecl` default) burns no draw
    /// through the loop either — the draw-neutrality argument, exercised
    /// through the real caller rather than the unit above.
    #[test]
    fn the_rest_loop_takes_no_draw_when_the_schedule_is_disarmed() {
        let scrolls = magic::ScrollLookup::default();
        let mut party = Party::default();
        party.members.push(blank_char());
        let mut t = RestTime::default();
        t.slots[3] = 8;
        let mut session = RestSession::start(t, 1);
        let mut clock = crate::movement::GameClock::default();
        let mut r = EngineRng::new(0xDEAD_BEEF);
        let before = r.state();
        while session.resting() {
            session.step(&mut party, &mut clock, &scrolls, &mut r, 0, 0);
        }
        assert_eq!(r.state(), before, "period 0 never rolls");
    }

    /// `rest_heal` (`ovr021.cs:358-388`): one party-wide hit point every
    /// `8 * 36` iterations, and `heal_player`'s status gate.
    #[test]
    fn rest_heals_one_hit_point_every_eight_by_thirtysix_iterations() {
        let scrolls = magic::ScrollLookup::default();
        let mut party = Party::default();
        let mut ch = blank_char();
        ch.hit_point_current = 10;
        party.members.push(ch);
        let mut t = RestTime::default();
        t.slots[4] = 2; // two days — enough for two heal ticks
        let mut session = RestSession::start(t, 1);
        let mut clock = crate::movement::GameClock::default();
        let mut r = EngineRng::new(1);
        let mut heals = 0;
        while session.resting() {
            heals += session
                .step(&mut party, &mut clock, &scrolls, &mut r, 0, 0)
                .iter()
                .filter(|e| **e == RestEvent::PartyHealed)
                .count();
        }
        assert_eq!(heals, 2, "2 days / (288 x 5 minutes) = 2");
        assert_eq!(party.members[0].hit_point_current, 12);
    }

    /// `heal_player`'s status gate and its `dying → unconscious` promotion
    /// (`ovr024.cs:1337-1358`).
    #[test]
    fn heal_player_gates_on_status_and_promotes_the_dying() {
        let mut dead = blank_char();
        dead.status.health_status = 6; // Status.dead
        dead.hit_point_current = 0;
        assert!(!heal_player(0, 5, &mut dead));
        assert_eq!(dead.hit_point_current, 0);

        let mut dying = blank_char();
        dying.status.health_status = status::DYING;
        dying.hit_point_current = 0;
        assert!(heal_player(0, 5, &mut dying));
        assert_eq!(dying.hit_point_current, 5);
        assert_eq!(dying.status.health_status, status::UNCONSCIOUS);

        let mut full = blank_char();
        assert!(
            heal_player(0, 5, &mut full),
            "arg_0 == 0 lets a full character through"
        );
        assert_eq!(full.hit_point_current, full.hit_point_max, "and caps");
    }

    /// ★ The scribe half: a staged scroll spell lands in the grimoire, the
    /// affect byte is cleared, `namenum2` is decremented, and the scroll is
    /// destroyed once that falls below `0xD2`
    /// (`remove_spell_from_scroll`, `ovr023.cs:3090-3112`).
    #[test]
    fn a_rested_scribe_learns_the_spell_and_consumes_the_scroll() {
        let mut item = vec![0u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
        item[0x2E] = 1; // an ITEMS type our synthetic table calls a scroll
        gbx_formats::save_orig::set_item_affect(&mut item, 1, 0x0F | 0x80);
        gbx_formats::save_orig::set_item_namenum2(&mut item, 0xD2);

        let mut ch = blank_char();
        ch.items.push(item);
        let scrolls = scroll_table_where_type_one_is_a_scroll();

        let mut party = Party::default();
        party.members.push(ch);
        let t = rest_menu_time(&mut party, &scrolls);
        assert_eq!(t.display_parts(), (0, 4, 15), "one level-1 scroll spell");

        let mut session = RestSession::start(t, 1);
        let events = rest_to_completion(&mut party, &mut session, &scrolls);
        assert!(events.contains(&RestEvent::Scribed {
            member: 0,
            spell_id: 0x0F
        }));
        assert!(events.contains(&RestEvent::ScrollConsumed { member: 0 }));
        assert_eq!(
            party.members[0].magic.spell_book[0x0F - 1],
            1,
            "the spell is in the grimoire"
        );
        assert!(party.members[0].items.is_empty(), "the scroll is gone");
    }

    /// A synthetic `ITEMS` table whose entry 1 is a scroll (`item_slot` 11).
    fn scroll_table_where_type_one_is_a_scroll() -> magic::ScrollLookup {
        // The file's own layout (`gbx_formats::items`): a 2-byte header then
        // 16-byte entries whose first byte is `item_slot`.
        let mut bytes = vec![0u8; gbx_formats::items::ITEMS_HEADER_SIZE];
        bytes.extend_from_slice(&[0u8; gbx_formats::items::ITEM_ENTRY_SIZE]); // type 0
        let mut scroll = [0u8; gbx_formats::items::ITEM_ENTRY_SIZE];
        scroll[0] = 11; // ItemSlot.slot_11 — `Item.IsScroll`'s lower bound
        bytes.extend_from_slice(&scroll);
        let data = gbx_formats::game_data::GameData::from_files(vec![(
            crate::combat_host::ITEMS_FILE.to_string(),
            bytes,
        )]);
        magic::ScrollLookup::load(&data)
    }

    /// `cancel_spells` at camp entry AND exit (`ovr016.cs:1095`/`:1154`):
    /// staged memorizations and staged scribes both evaporate; memorized
    /// spells and un-staged scroll spells do not.
    #[test]
    fn cancel_spells_drops_staging_on_both_sides_of_camp() {
        let scrolls = scroll_table_where_type_one_is_a_scroll();
        let mut item = vec![0u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
        item[0x2E] = 1;
        gbx_formats::save_orig::set_item_affect(&mut item, 1, 0x0F | 0x80); // staged
        gbx_formats::save_orig::set_item_affect(&mut item, 2, 0x15); // not staged

        let mut ch = blank_char();
        ch.items.push(item);
        magic::add_learnt(&mut ch.magic.spell_list, 0x03); // in memory
        magic::add_learn(&mut ch.magic.spell_list, 0x0F); // staged
        ch.magic.spell_to_learn_count = 4;

        let mut party = Party::default();
        party.members.push(ch);
        magic::cancel_spells(&mut party, &scrolls);

        let left: Vec<u8> = magic::entries(&party.members[0].magic.spell_list)
            .map(|e| e.id)
            .collect();
        assert_eq!(left, vec![0x03], "only the memorized spell survives");
        assert_eq!(party.members[0].magic.spell_to_learn_count, 0);
        assert_eq!(
            gbx_formats::save_orig::item_affect(&party.members[0].items[0], 1),
            0x0F,
            "the scroll keeps its spell, minus the staging bit"
        );
        assert_eq!(
            gbx_formats::save_orig::item_affect(&party.members[0].items[0], 2),
            0x15
        );
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
