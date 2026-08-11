//! ★ The **out-of-combat** affect system (roll-credits §9, G7's Task 2).
//!
//! Derived by reading coab for behavior (D11, never copied).
//!
//! Combat has carried the affect substrate since M5 doc §39
//! (`crate::combat::affects`): a [`Combatant`](crate::combat::Combatant) holds
//! a decoded `Vec<AffectRecord>` and the 24-case `CheckAffectsEffect` dispatch
//! runs over it. Out of combat there was nothing at all — a
//! [`Character`](crate::party::Character) carries the raw `.fx` chain as
//! opaque 9-byte records (§5.5) and only [`Character::has_affect`] ever looked
//! inside one.
//!
//! This module is the missing half: the same four operations
//! (`FindAffect`/`add_affect`/`remove_affect`/`cure_affect`, `ovr025.cs:1175`,
//! `ovr024.cs:609`/`:67`/`:705`) over the raw chain, plus the only clock the
//! original ever runs them on — [`check_affects_timing_out`].
//!
//! ## Why the raw chain stays raw
//!
//! Exactly the argument `crate::magic` made for `spell_list`: the array *is*
//! the state. `Character::affects: Vec<Vec<u8>>` already round-trips through
//! `.rsav` and through the original's `.fx` files, so operating on it in place
//! costs **no `SAVE_FORMAT_VERSION` bump** and no golden regeneration. Every
//! function here decodes on the way past, exactly as
//! [`Character::has_affect`] already did.
//!
//! ## The clock, and the quirk it hides
//!
//! `CheckAffectsTimingOut` (`sub_5801E`, `ovr021.cs:11-104`) is called from
//! **one** place: `step_game_time` (`:171`). Its first branch is the whole
//! story — when `game_state != Camping` it does not decrement anything; it just
//! marks every slot of `affects_timed_out` dirty and returns. So **a buff's
//! minutes only ever tick down while the party is camping.** Bless cast in
//! camp survives an arbitrary amount of walking; it expires during the *next*
//! rest. That is the original's behavior, not a simplification, and
//! [`check_affects_timing_out`] reproduces it including the dirty-flag
//! bookkeeping (which is what makes the early-out cheap when nothing on the
//! party is timed).
//!
//! ## Draws
//!
//! Everything in this module is **PRNG-free**. `remove_affect`'s
//! `CallAffectTable(Remove)` side effects are the same cited-and-tripwired set
//! combat carries (`crate::combat::affects`), and the one draw-bearing affect
//! handler in the game (`troll_fire_or_acid`'s 3d6) hangs off the combat Death
//! dispatch, which camp never reaches.

use crate::party::{Character, Party};
use gbx_formats::affects::AffectRecord;

/// `timeScales` (`word_1A13C`, `ovr021.cs:8`) — the per-slot carry divisors the
/// clock normalizes on. [`check_affects_timing_out`] uses them to convert a
/// `(slot, steps)` pair into minutes.
pub const TIME_SCALES: [i32; 7] = [10, 10, 6, 24, 30, 12, 0x100];

/// Every decoded affect on a character, in chain order (order is observable:
/// `FindAffect` is find-**first**, doc §39.2).
pub fn decoded(ch: &Character) -> impl Iterator<Item = AffectRecord> + '_ {
    ch.affects
        .iter()
        .filter_map(|raw| AffectRecord::decode(raw))
}

/// `FindAffect(out affect, kind, player)` (`ovr025.cs:1175-1180`): the **first**
/// affect of `kind`, or `None`.
pub fn find_affect(ch: &Character, kind: u8) -> Option<AffectRecord> {
    decoded(ch).find(|a| a.kind == kind)
}

/// The chain index of the first affect of `kind` — what the mutating
/// operations need.
fn index_of(ch: &Character, kind: u8) -> Option<usize> {
    ch.affects
        .iter()
        .position(|raw| AffectRecord::decode(raw).is_some_and(|a| a.kind == kind))
}

/// `add_affect(call_spell_jump_list, data, minutes, type, player)`
/// (`ovr024.cs:609-615`): construct the affect and **append it at the tail**
/// (`player.affects.Add`; the binary walks the `next` chain to the end).
///
/// The add-side `CallAffectTable(Add)` handler is not fired here. Out of
/// combat the only must-have row whose add-handler does anything is
/// `slow_poison` (`AffectSlowPoison`, `ovr013.cs:305-317`), and that one is
/// driven explicitly by its caster — see [`crate::camp_cast`].
pub fn add_affect(ch: &mut Character, kind: u8, minutes: u16, data: u8, call_affect_table: bool) {
    ch.affects.push(
        AffectRecord {
            kind,
            minutes,
            data,
            call_affect_table,
        }
        .encode()
        .to_vec(),
    );
}

/// `remove_affect(null, kind, player)` (`ovr024.cs:67-95`) — drop the **first**
/// matching instance, not all of them. Returns whether one was removed.
///
/// The two cited side effects (the `CallAffectTable(Remove)` when the record
/// carries `call_affect_table`, and the `CalcStatBonuses` recompute for
/// `friends`/`enlarge`/`strength`/`strength_spell`) are the same ones combat
/// tripwires as `affect-remove-side`; no must-have row plants an affect in
/// either set out of combat, so nothing is silently dropped here — see
/// [`STAT_RECOMPUTE_KINDS`].
pub fn remove_affect(ch: &mut Character, kind: u8) -> bool {
    match index_of(ch, kind) {
        Some(i) => {
            ch.affects.remove(i);
            true
        }
        None => false,
    }
}

/// Remove **every** instance of `kind` (the `while FindAffect` idiom
/// `remove_invisibility` uses, `ovr024.cs:650-658`).
pub fn remove_all(ch: &mut Character, kind: u8) -> usize {
    let mut n = 0;
    while remove_affect(ch, kind) {
        n += 1;
    }
    n
}

/// `cure_affect(affectId, player)` (`is_cured`, `ovr024.cs:705-718`): if the
/// affect is present, show `"<name> is Cured"` and remove it. Returns whether
/// anything was cured — which is exactly what the cure spells branch on.
pub fn cure_affect(ch: &mut Character, kind: u8) -> bool {
    if find_affect(ch, kind).is_none() {
        return false;
    }
    remove_affect(ch, kind);
    true
}

/// The affect kinds whose removal triggers a `CalcStatBonuses` recompute
/// (`ovr024.cs:0222-0245` in the listing; **CHA on `friends` 0x0E** — coab
/// wrote `resist_fire`, a coab≠binary bug the M5 peel caught — and **STR on
/// `enlarge` 0x0C / `strength` 0x26 / `strength_spell` 0x92**). None of §9.1's
/// must-have rows plants one, which is why this module can leave the recompute
/// unmodelled and still be complete for the set it implements.
pub const STAT_RECOMPUTE_KINDS: [u8; 4] = [0x0E, 0x0C, 0x26, 0x92];

/// `CheckAffectsTimingOut(timeSlot, timeSteps)` (`sub_5801E`, `ovr021.cs:11-104`)
/// — the ONLY place an affect's `minutes` ever decreases.
///
/// `camping` is `gbl.game_state == GameState.Camping`. When it is false the
/// original marks every `affects_timed_out[i]` dirty and returns without
/// touching a single record (`:13-19`), so this function does the same and the
/// party's buffs are frozen for the whole walk. `dirty` is the caller's
/// `affects_timed_out` array — a per-member "might still have timed affects"
/// flag that the original keeps across calls and uses as an early-out.
///
/// The conversion (`:41-46`) folds `timeSlot` down to minutes through
/// [`TIME_SCALES`], and the tick runs in **ten-minute** chunks (`:50-52`), each
/// chunk sweeping the whole party: an affect with `minutes > chunk` loses the
/// chunk and stays dirty, one with `minutes <= chunk` is removed. The second
/// re-scan at `:82-89` ("not sure why we are doing this again") is the
/// original's and is kept: a member whose *other* affects still have time left
/// re-arms the flag the first pass cleared.
///
/// Returns the number of affects that expired. Draw-free.
pub fn check_affects_timing_out(
    party: &mut Party,
    dirty: &mut Vec<bool>,
    camping: bool,
    time_slot: usize,
    time_steps: i32,
) -> usize {
    if dirty.len() < party.members.len() {
        dirty.resize(party.members.len(), true);
    }
    if !camping {
        // `:13-19` — every slot goes dirty, nothing is decremented.
        for d in dirty.iter_mut() {
            *d = true;
        }
        return 0;
    }
    // `:21-38` — if nobody is flagged there is nothing timed to tick.
    if !dirty.iter().take(party.members.len()).any(|&d| d) {
        return 0;
    }

    // `:41-46` — fold the slot down to minutes.
    let mut remaining = time_steps;
    let mut slot = time_slot;
    while slot > 1 {
        remaining *= TIME_SCALES[slot - 1];
        slot -= 1;
    }

    let mut expired = 0;
    while remaining > 0 {
        let chunk = remaining.min(10) as u16;
        for (i, member) in party.members.iter_mut().enumerate() {
            if !dirty[i] {
                continue;
            }
            dirty[i] = false;
            let mut keep: Vec<Vec<u8>> = Vec::with_capacity(member.affects.len());
            for raw in std::mem::take(&mut member.affects) {
                let Some(mut a) = AffectRecord::decode(&raw) else {
                    keep.push(raw); // undecodable: not state we understand, kept
                    continue;
                };
                if a.minutes == 0 {
                    keep.push(raw); // permanent (`:66-69`)
                } else if chunk < a.minutes {
                    a.minutes -= chunk;
                    keep.push(a.encode().to_vec());
                    dirty[i] = true;
                } else {
                    expired += 1; // `:75` — into removeList
                }
            }
            member.affects = keep;
            // `:82-89` — the original's second scan, verbatim.
            if decoded(member).any(|a| a.minutes > 0) {
                dirty[i] = true;
            }
        }
        remaining = if remaining > 10 { remaining - 10 } else { 0 };
    }
    expired
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::{character_from_record, Character};

    fn ch(name: &str) -> Character {
        let rec = vec![0u8; gbx_formats::save_orig::CHAR_RECORD_SIZE];
        let record = gbx_formats::save_orig::decode_char_record(&rec).unwrap();
        let mut c = character_from_record(&record, Vec::new(), Vec::new());
        c.name = name.to_string();
        c
    }

    #[test]
    fn add_find_remove_round_trip_through_the_raw_chain() {
        let mut c = ch("SHARA");
        assert!(find_affect(&c, 0x01).is_none());
        add_affect(&mut c, 0x01, 6, 5, false);
        // The chain really is raw bytes — nine of them, as the `.fx` file has.
        assert_eq!(c.affects.len(), 1);
        assert_eq!(
            c.affects[0].len(),
            gbx_formats::save_orig::AFFECT_RECORD_SIZE
        );
        let found = find_affect(&c, 0x01).expect("bless is on the chain");
        assert_eq!((found.minutes, found.data), (6, 5));
        assert!(c.has_affect(0x01), "the pre-existing reader agrees");
        assert!(remove_affect(&mut c, 0x01));
        assert!(!remove_affect(&mut c, 0x01));
        assert!(c.affects.is_empty());
    }

    /// `FindAffect` is find-FIRST and `remove_affect` removes ONE — the pair
    /// that makes chain order observable (doc §39.2).
    #[test]
    fn find_is_first_and_remove_takes_only_one() {
        let mut c = ch("MARK");
        add_affect(&mut c, 0x08, 10, 1, false);
        add_affect(&mut c, 0x08, 20, 2, false);
        assert_eq!(find_affect(&c, 0x08).unwrap().minutes, 10);
        assert!(remove_affect(&mut c, 0x08));
        assert_eq!(find_affect(&c, 0x08).unwrap().minutes, 20);
        assert_eq!(remove_all(&mut c, 0x08), 1);
        assert!(find_affect(&c, 0x08).is_none());
    }

    #[test]
    fn cure_affect_reports_whether_it_found_anything() {
        let mut c = ch("SHARA");
        assert!(!cure_affect(&mut c, 0x21), "nothing to cure");
        add_affect(&mut c, 0x21, 0, 0xFF, false); // blinded, permanent
        assert!(cure_affect(&mut c, 0x21));
        assert!(!c.has_affect(0x21));
    }

    /// ★ The quirk, pinned: outside camp `CheckAffectsTimingOut` decrements
    /// nothing at all (`ovr021.cs:13-19`).
    #[test]
    fn walking_never_ages_a_buff() {
        let mut party = Party {
            members: vec![ch("SHARA")],
        };
        add_affect(&mut party.members[0], 0x01, 6, 5, false);
        let mut dirty = vec![true];
        // A whole day of slot-4 steps, out of camp.
        let expired = check_affects_timing_out(&mut party, &mut dirty, false, 4, 30);
        assert_eq!(expired, 0);
        assert_eq!(find_affect(&party.members[0], 0x01).unwrap().minutes, 6);
        assert!(dirty[0], "everything goes dirty instead");
    }

    /// In camp the clock runs, in ten-minute chunks, and an affect whose
    /// minutes fall to or below the chunk is removed.
    #[test]
    fn camping_ages_and_expires_a_buff() {
        let mut party = Party {
            members: vec![ch("SHARA")],
        };
        add_affect(&mut party.members[0], 0x08, 15, 1, false); // prot evil, 15 min
        let mut dirty = vec![true];
        // Slot 1 is minutes (`display_resting_time`'s own mapping): ten steps.
        let expired = check_affects_timing_out(&mut party, &mut dirty, true, 1, 10);
        assert_eq!(expired, 0, "15 > 10 — it survives, shortened");
        assert_eq!(find_affect(&party.members[0], 0x08).unwrap().minutes, 5);
        let expired = check_affects_timing_out(&mut party, &mut dirty, true, 1, 10);
        assert_eq!(expired, 1, "5 <= 10 — gone");
        assert!(find_affect(&party.members[0], 0x08).is_none());
        assert!(!dirty[0], "nothing timed is left, so the early-out arms");
    }

    /// `minutes == 0` is permanent (doc §39.1) — the paladins' imported
    /// protection-from-evil, and every racial affect, must never time out.
    #[test]
    fn a_permanent_affect_never_expires() {
        let mut party = Party {
            members: vec![ch("MATHEW")],
        };
        add_affect(&mut party.members[0], 0x08, 0, 0xFF, false);
        let mut dirty = vec![true];
        for _ in 0..50 {
            check_affects_timing_out(&mut party, &mut dirty, true, 2, 24);
        }
        assert!(party.members[0].has_affect(0x08));
    }

    /// The dirty-flag early-out is the original's, and it is observable: with
    /// every flag clear the function returns without reading a record.
    #[test]
    fn the_dirty_flag_short_circuits_the_sweep() {
        let mut party = Party {
            members: vec![ch("SHARA")],
        };
        add_affect(&mut party.members[0], 0x01, 6, 5, false);
        let mut dirty = vec![false];
        assert_eq!(
            check_affects_timing_out(&mut party, &mut dirty, true, 1, 60),
            0
        );
        assert_eq!(
            find_affect(&party.members[0], 0x01).unwrap().minutes,
            6,
            "untouched — the flag said there was nothing to do"
        );
    }

    /// The slot fold: slot 2 is ten minutes a step (`TIME_SCALES[1]`), so three
    /// steps age a 30-minute affect out exactly.
    #[test]
    fn the_slot_fold_matches_time_scales() {
        let mut party = Party {
            members: vec![ch("SHARA")],
        };
        add_affect(&mut party.members[0], 0x13, 30, 0xFF, false); // find traps
        let mut dirty = vec![true];
        check_affects_timing_out(&mut party, &mut dirty, true, 2, 2);
        assert_eq!(
            find_affect(&party.members[0], 0x13).unwrap().minutes,
            10,
            "two slot-2 steps = 20 minutes"
        );
        assert_eq!(
            check_affects_timing_out(&mut party, &mut dirty, true, 2, 1),
            1
        );
    }
}
