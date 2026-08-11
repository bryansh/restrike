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
/// ★ **Roll-credits slice 6**: the `CallAffectTable(Remove)` dispatch fires
/// here now, gated on the record's own `callAffectTable` flag exactly as the
/// original gates it (`ovr024.cs:75-79`). Slice 5 named its absence as the
/// reason `AffectSlowPoison`'s kill-on-timeout never fired.
///
/// The `CalcStatBonuses` recompute for `friends`/`enlarge`/`strength`/
/// `strength_spell` (`:83-94`) is still unmodelled — no row any camp path
/// plants is in that set (see [`STAT_RECOMPUTE_KINDS`]).
pub fn remove_affect(ch: &mut Character, kind: u8) -> bool {
    remove_affect_inner(ch, kind, false)
}

/// `remove_affect` under **`gbl.cureSpell`** — the flag every cure brackets its
/// removals with (`ovr005.cs:97/178/253`, `ovr023.cs:1606/2257/2349`).
///
/// It does not suppress the handler; it suppresses the handler's **re-plant**
/// (`ovr013.addAffect`, `:25-36`, returns `false` and adds nothing when the
/// flag is up). That one indirection is what makes Neutralize Poison a cure
/// rather than a slow, expensive way to poison somebody again.
pub fn cure_remove(ch: &mut Character, kind: u8) -> bool {
    remove_affect_inner(ch, kind, true)
}

fn remove_affect_inner(ch: &mut Character, kind: u8, cure_spell: bool) -> bool {
    let Some(i) = index_of(ch, kind) else {
        return false;
    };
    // `:75-79` — the handler runs BEFORE the record leaves the chain, so
    // `AffectSlowPoison`'s own `HasAffect(poisoned)` test still sees the
    // whole chain including the affect being removed.
    let fires = AffectRecord::decode(&ch.affects[i]).is_some_and(|a| a.call_affect_table);
    if fires {
        call_affect_table_remove(ch, kind, cure_spell);
    }
    // The handler may have shuffled the chain (`AffectSlowPoison` removes
    // `poison_damage`), so the index is re-resolved rather than reused.
    match index_of(ch, kind) {
        Some(i) => {
            ch.affects.remove(i);
            true
        }
        None => true,
    }
}

/// ★ **`CallAffectTable(Effect.Remove, …)` out of combat** (`ovr013.cs:1939-1951`)
/// — the seam slice 5 named and this slice wires.
///
/// The original's table has ~100 rows, but only an affect **planted with
/// `callAffectTable == true`** ever reaches one, and outside combat exactly
/// three such affects exist on a party member: `slow_poison` and
/// `poison_damage` (planted by Slow Poison, `ovr023.cs:1307-1309`) and
/// `highConRegen` (planted by the CON recompute). The first two are the poison
/// arc; the third's handler is a heal tick that only runs on the Add side.
/// Every other row is a no-op here **because it cannot be reached**, not
/// because it is unimplemented — which is why this is a two-arm match rather
/// than a tripwire.
fn call_affect_table_remove(ch: &mut Character, kind: u8, cure_spell: bool) {
    match kind {
        AFF_SLOW_POISON => affect_slow_poison(ch),
        AFF_POISON_DAMAGE => affect_poison_damage(ch, cure_spell),
        _ => {}
    }
}

/// `Affects.poison_damage` (`Classes/Affect.cs:22`).
pub const AFF_POISON_DAMAGE: u8 = 0x0F;
/// `Affects.slow_poison` (`:31`).
pub const AFF_SLOW_POISON: u8 = 0x16;
/// `Affects.poisoned` (`:64`).
pub const AFF_POISONED: u8 = 0x37;

/// ★ **`AffectSlowPoison`** (`sub_3A517`, `ovr013.cs:304-317`) — the clock
/// running out.
///
/// It ignores its `Effect` argument entirely, so it fires on the way in *and*
/// on the way out; what makes it the poison arc's ending is that the only way
/// the affect leaves a chain out of combat is by timing out
/// ([`check_affects_timing_out`]).
///
/// ```text
/// if (player.HasAffect(poisoned))  KillPlayer("dies from poison", dead, player);
/// cureSpell = true;  remove_affect(poison_damage);  cureSpell = false;
/// ```
///
/// ★ **The ordering inside Neutralize Poison is therefore load-bearing.** The
/// cure removes `poisoned` **first**, then `slow_poison`
/// (`ovr023.cs:2259-2261`, `ovr005.cs:255-257`): reverse those two lines and
/// the cure kills the patient, because this handler would still find the
/// poison on the chain.
fn affect_slow_poison(ch: &mut Character) {
    if ch.has_affect(AFF_POISONED) {
        kill_player(ch, crate::rest::status::DEAD);
    }
    // `cureSpell = true` around this one, so `AffectPoisonDamage` does not
    // re-plant the tick it is about to lose.
    cure_remove(ch, AFF_POISON_DAMAGE);
}

/// ★ **`AffectPoisonDamage`** (`sub_3A3BC`, `ovr013.cs:235-251`) — the tick.
///
/// Also ignores its `Effect` argument, and that is the whole trick: when the
/// ten-minute affect times out, the remove handler **re-plants it for another
/// ten minutes** and takes one hit point off anybody above 1. So a slow-poisoned
/// character bleeds a point every ten minutes of camp time until either the
/// `slow_poison` affect lapses (and [`affect_slow_poison`] kills them) or a cure
/// arrives — and a cure works precisely because `gbl.cureSpell` makes
/// `addAffect` return `false` without adding (`ovr013.cs:25-36`).
///
/// The `hit_point_current > 1` guard means the tick alone can never kill; only
/// the lapse does.
fn affect_poison_damage(ch: &mut Character, cure_spell: bool) {
    if cure_spell {
        return; // `addAffect` returned false — no re-plant, no damage.
    }
    let data = find_affect(ch, AFF_POISON_DAMAGE)
        .map(|a| a.data)
        .unwrap_or(0xFF);
    add_affect(ch, AFF_POISON_DAMAGE, 10, data, true);
    if ch.hit_point_current > 1 {
        ch.hit_point_current -= 1;
    }
}

/// ★ **`KillPlayer(text, status, player)`** (`ovr024.cs:36-64`), out of combat.
///
/// Its guard is the reason `stoned` and `gone` had to become real rungs
/// (deliverable B): a member already `stoned`, `dead` or `gone` is **not**
/// killed again, and the function returns without touching a single field.
/// Otherwise the status lands, `in_combat` clears, hit points go to zero and
/// `RemoveCombatAffects` strips the combat-scoped chain — the same nineteen-row
/// table combat's own [`crate::combat`] side strips
/// (`unk_16D41`, `ovr024.cs:661-691`).
///
/// `CheckAffectsEffect(player, CheckType.Death)` (`:47`) is the combat death
/// dispatch and is not reachable here: its only non-trivial rows are monster
/// handlers (`troll_fire_or_acid`'s 3d6 and friends), none of which a party
/// member carries. Named rather than silently skipped.
pub fn kill_player(ch: &mut Character, status: u8) {
    if crate::rest::status::is_terminal(ch.status.health_status) {
        return; // `:39-42`
    }
    ch.status.health_status = status;
    ch.status.in_combat = false;
    ch.hit_point_current = 0;
    remove_combat_affects(ch);
}

/// `RemoveCombatAffects(player)` (`sub_645AB`, `ovr024.cs:661-691`) on a roster
/// record. The table is the same nineteen ids the combat side strips
/// (`unk_16D41[1..19]` @`seg600:0A32-0A44`).
pub fn remove_combat_affects(ch: &mut Character) {
    for &kind in STRIP_COMBAT_KINDS {
        remove_affect(ch, kind);
    }
}

/// `unk_16D41[1..19]` (`seg600:0A32-0A44`) — `RemoveCombatAffects`' strip
/// table, byte for byte the list `crate::combat::affects` carries.
const STRIP_COMBAT_KINDS: &[u8] = &[
    0x07, 0x0B, 0x0D, 0x15, 0x17, 0x1E, 0x1F, 0x20, 0x33, 0x34, 0x35, 0x3A, 0x3B, 0x5F, 0x62, 0x88,
    0x89, 0x8B, 0x90,
];

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
            // `:60-77` — the decrement pass builds a `removeList` rather than
            // removing in place, because `remove_affect` is what has to run
            // over it afterwards.
            let mut keep: Vec<Vec<u8>> = Vec::with_capacity(member.affects.len());
            let mut remove_list: Vec<u8> = Vec::new();
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
                    // `:75` — into removeList, but the record stays on the
                    // chain until `remove_affect` takes it off, which is what
                    // lets its own handler still see it (`AffectSlowPoison`
                    // reads `HasAffect(poisoned)` from the live chain).
                    keep.push(raw);
                    remove_list.push(a.kind);
                }
            }
            member.affects = keep;
            // ★ `:79-81` — `ovr024.remove_affect(remove, remove.type, player)`
            // per expired affect, which is the **`CallAffectTable(Remove)`
            // dispatch** slice 5 named as missing: this is where a lapsed
            // `slow_poison` kills, and where a lapsed `poison_damage`
            // re-plants itself and takes a hit point.
            for kind in remove_list {
                remove_affect(member, kind);
                expired += 1;
            }
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
    use crate::party::{character_from_record, Character, Party};

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

    // --- roll-credits slice 6: the poison clock ---------------------------

    /// ★ The whole arc, in one test (`AffectSlowPoison`, `ovr013.cs:304-317`;
    /// `AffectPoisonDamage`, `:235-251`): a poisoned member under Slow Poison
    /// bleeds a point every ten minutes of camp time, and **dies the moment
    /// the Slow Poison lapses**.
    #[test]
    fn the_poison_clock_ticks_and_then_kills() {
        let mut party = Party {
            members: vec![ch("BITTEN")],
        };
        let m = &mut party.members[0];
        m.hit_point_max = 20;
        m.hit_point_current = 20;
        m.status.health_status = crate::rest::status::OKEY;
        m.status.in_combat = true;
        // What a spider bite leaves (`PoisonAttack`, `ovr013.cs:848-860`) plus
        // what Slow Poison plants on top (`is_affected2`, `ovr023.cs:1291-1310`).
        add_affect(m, AFF_POISONED, 0, 0xFF, false); // permanent marker
        add_affect(m, AFF_SLOW_POISON, 30, 0xFF, true); // 30 minutes of grace
        add_affect(m, AFF_POISON_DAMAGE, 10, 0xFF, true); // the ten-minute tick

        let mut dirty = vec![true];
        // Ten minutes: the tick lapses, re-plants itself, and takes a point.
        check_affects_timing_out(&mut party, &mut dirty, true, 1, 10);
        assert_eq!(party.members[0].hit_point_current, 19, "one point per tick");
        assert!(
            party.members[0].has_affect(AFF_POISON_DAMAGE),
            "the tick re-planted itself for another ten minutes"
        );
        assert_eq!(
            party.members[0].status.health_status,
            crate::rest::status::OKEY
        );

        // Another ten: still ticking, still alive.
        check_affects_timing_out(&mut party, &mut dirty, true, 1, 10);
        assert_eq!(party.members[0].hit_point_current, 18);
        assert_eq!(
            party.members[0].status.health_status,
            crate::rest::status::OKEY
        );

        // ...and the thirtieth minute is the last: `slow_poison` lapses, its
        // handler finds `poisoned` still on the chain, and kills. (The sweep
        // runs the remove list in CHAIN order, so the tick's own point lands
        // first and is then annihilated by the kill's `hit_point_current = 0`.)
        check_affects_timing_out(&mut party, &mut dirty, true, 1, 10);
        assert_eq!(
            party.members[0].status.health_status,
            crate::rest::status::DEAD,
            "the poison ran out of patience"
        );
        assert_eq!(party.members[0].hit_point_current, 0);
        assert!(!party.members[0].status.in_combat);
        assert!(
            !party.members[0].has_affect(AFF_POISON_DAMAGE),
            "`AffectSlowPoison` clears the tick under cureSpell"
        );
    }

    /// ★ And the cure: Neutralize Poison arrives before the lapse, so nothing
    /// re-plants and nothing kills. The removal ORDER is what makes it work —
    /// `poisoned` leaves the chain first.
    #[test]
    fn neutralize_poison_saves_the_patient_because_of_the_order() {
        let mut party = Party {
            members: vec![ch("SAVED")],
        };
        let m = &mut party.members[0];
        m.hit_point_max = 20;
        m.hit_point_current = 18;
        add_affect(m, AFF_POISONED, 0, 0xFF, false);
        add_affect(m, AFF_SLOW_POISON, 30, 0xFF, true);
        add_affect(m, AFF_POISON_DAMAGE, 10, 0xFF, true);

        // `SpellNeutralizePoison`'s three removals, in the original's order.
        cure_remove(m, AFF_POISONED);
        cure_remove(m, AFF_SLOW_POISON);
        cure_remove(m, AFF_POISON_DAMAGE);

        assert_eq!(m.status.health_status, crate::rest::status::OKEY, "alive");
        assert_eq!(m.hit_point_current, 18, "and unhurt by the cure");
        assert!(m.affects.is_empty());

        // The clock now finds nothing to run.
        let mut dirty = vec![true];
        check_affects_timing_out(&mut party, &mut dirty, true, 3, 24);
        assert_eq!(
            party.members[0].status.health_status,
            crate::rest::status::OKEY
        );
    }

    /// ★ Reverse those two lines and the "cure" kills — the reason the order
    /// is called out in the code rather than tidied.
    #[test]
    fn curing_in_the_wrong_order_would_kill_the_patient() {
        let mut c = ch("UNLUCKY");
        c.hit_point_max = 20;
        c.hit_point_current = 18;
        add_affect(&mut c, AFF_POISONED, 0, 0xFF, false);
        add_affect(&mut c, AFF_SLOW_POISON, 30, 0xFF, true);

        cure_remove(&mut c, AFF_SLOW_POISON); // the wrong one first
        assert_eq!(
            c.status.health_status,
            crate::rest::status::DEAD,
            "`AffectSlowPoison` found the poison still on the chain"
        );
    }

    /// The tick alone can never kill: `AffectPoisonDamage`'s own guard is
    /// `hit_point_current > 1` (`ovr013.cs:240`).
    #[test]
    fn the_tick_alone_never_kills() {
        let mut party = Party {
            members: vec![ch("CLINGING")],
        };
        let m = &mut party.members[0];
        m.hit_point_max = 20;
        m.hit_point_current = 1;
        add_affect(m, AFF_POISONED, 0, 0xFF, false);
        add_affect(m, AFF_SLOW_POISON, 0, 0xFF, true); // permanent: never lapses
        add_affect(m, AFF_POISON_DAMAGE, 10, 0xFF, true);

        let mut dirty = vec![true];
        for _ in 0..20 {
            check_affects_timing_out(&mut party, &mut dirty, true, 1, 10);
        }
        assert_eq!(party.members[0].hit_point_current, 1);
        assert_eq!(
            party.members[0].status.health_status,
            crate::rest::status::OKEY
        );
    }

    /// ★ `KillPlayer`'s guard (`ovr024.cs:39-42`): a statue is not killed
    /// again — status, hit points and `in_combat` are all left alone, which is
    /// what lets a petrified member be carried to a temple rather than
    /// downgraded to a corpse by the next thing that happens to them.
    #[test]
    fn a_petrified_member_is_never_killed_again() {
        let mut c = ch("STATUE");
        c.status.health_status = crate::rest::status::STONED;
        c.status.in_combat = false;
        c.hit_point_current = 7;
        add_affect(&mut c, 0x01, 30, 5, false); // a bless it was carrying

        kill_player(&mut c, crate::rest::status::DEAD);

        assert_eq!(c.status.health_status, crate::rest::status::STONED);
        assert_eq!(c.hit_point_current, 7, "not even the hit points move");
        assert!(c.has_affect(0x01), "and nothing is stripped");
    }

    /// `KillPlayer` on a living member runs the whole body, `RemoveCombatAffects`
    /// included (`ovr024.cs:44-46`): a combat-scoped affect on the strip table
    /// goes, one off it stays.
    #[test]
    fn kill_player_strips_the_combat_scoped_chain() {
        let mut c = ch("FALLEN");
        c.status.in_combat = true;
        c.hit_point_current = 12;
        add_affect(&mut c, 0x35, 20, 0xFF, false); // sleep — on the strip table
        add_affect(&mut c, 0x08, 0, 0xFF, false); // protection from evil — not

        kill_player(&mut c, crate::rest::status::DEAD);

        assert_eq!(c.status.health_status, crate::rest::status::DEAD);
        assert_eq!(c.hit_point_current, 0);
        assert!(!c.status.in_combat);
        assert!(!c.has_affect(0x35));
        assert!(c.has_affect(0x08));
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
