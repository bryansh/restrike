//! ★ **The temple** (`ovr005.cs`) — roll-credits slice 6 / G8, the death-recovery
//! half of D-RC6.
//!
//! `CMD_Combat`'s non-monster branch (`ovr003.cs:971-1000`) is a three-way
//! dispatch on two `Area2` flags a script has just set:
//!
//! ```text
//! monstersLoaded == false && combat_type == normal
//!     EnterShop  == 1  →  ovr007.CityShop()
//!     EnterTemple == 1  →  ovr005.temple_shop()
//!     otherwise         →  ovr006.AfterCombatExpAndTreasure()
//! ```
//!
//! So a temple is *a `COMBAT` opcode with a flag set*, which is why the census
//! never showed one: the flag write is a plain `SAVE 1 → 0x7EE2`. The four
//! shipped sites are listed in [`crate::shell`]'s dispatch.
//!
//! ## The service table, transcribed
//!
//! `temple_heal` (`ovr005.cs:285-370`) builds its list from `temple_sl`
//! (`:13`) — and builds **ten** rows from an eleven-entry array (`for i in
//! 0..10`), so the trailing `"Exit"` is never a list row. Leaving is `E`/Esc on
//! the `"Heal Exit"` prompt, which `sl_select_item` answers with `'\0'`
//! (`ovr027.cs:652-657`); the `case 10` in the dispatch switch is dead code.
//!
//! | # | service | price (gp) | effect |
//! |---|---|---|---|
//! | 0 | Cure Blindness | 1000 | remove `blinded` (0x21) |
//! | 1 | Cure Disease | 1000 | remove all six [`DISEASE_TYPES`] |
//! | 2 | Cure Light Wounds | 100 | heal 1d8 |
//! | 3 | Cure Serious Wounds | 350 | heal 2d8+1 |
//! | 4 | Cure Critical Wounds | 600 | heal 3d8+3 |
//! | 5 | Heal | 5000 | to full **minus 1d4**, + blindness, disease and `feeblemind` |
//! | 6 | Neutralize Poison | 1000 | remove `poisoned`/`slow_poison`/`poison_damage` |
//! | 7 | Raise Dead | 5500 | see [`raise_dead`] |
//! | 8 | Remove Curse | 3500 | `SpellRemoveCurse` on the member |
//! | 9 | Stone to Flesh | 2000 | ★ `stoned` → `okey`, 1 hit point |
//!
//! ★ **Stone to Flesh is delivered here and only here.** Slice 5 proved the
//! *spell* does not exist in CotAB (the `Spells` enum runs `0x01..0x65` with no
//! such row, §9.1), so the temple is the whole answer to a medusa.
//!
//! ## The money, and the one thing everybody misremembers
//!
//! `buy_cure` (`:28-59`) charges the **selected player's own purse first** and
//! falls back to the **pooled money**, never to another member's purse:
//!
//! ```text
//! cost <= SelectedPlayer.Money.GetGoldWorth()  →  SelectedPlayer pays
//! else cost <= pooled_money.GetGoldWorth()     →  the pool pays
//! else                                         →  "Not enough money."
//! ```
//!
//! And it prints its price **before** asking (`press_any_key("… will only cost
//! N gold pieces.")` then `yes_no("pay for cure ")`), so the player always sees
//! the number. `temple_shop` itself **clears the pool on entry**
//! (`gbl.pooled_money.ClearAll()`, `:406`) — a party that walks in with
//! unpooled coins has nothing in the pool until it presses `P`.
//!
//! ## Draws
//!
//! Three services roll: the three cure-wounds rows and `Heal`'s 1d4. Every
//! other service is arithmetic. Nothing here is reachable from a capture (no
//! replay runs a shop), and the temple cannot be entered from combat.

use crate::affects;
use crate::money::MoneySet;
use crate::party::Character;
use crate::rest::{heal_player, roll_dice, status};
use crate::rng::EngineRng;
use gbx_rules::pack::RuleSet;

/// `Affects` ids the temple's services touch (`Classes/Affect.cs`).
mod aff {
    pub const POISON_DAMAGE: u8 = 0x0F;
    pub const SLOW_POISON: u8 = 0x16;
    pub const HELPLESS: u8 = 0x1F;
    pub const ANIMATE_DEAD: u8 = 0x20;
    pub const BLINDED: u8 = 0x21;
    pub const CAUSE_DISEASE_1: u8 = 0x22;
    pub const BESTOW_CURSE: u8 = 0x24;
    pub const WEAKEN: u8 = 0x2B;
    pub const CAUSE_DISEASE_2: u8 = 0x2C;
    pub const POISONED: u8 = 0x37;
    pub const AFFECT_39: u8 = 0x39;
    pub const FEEBLEMIND: u8 = 0x44;
    pub const HIGH_CON_REGEN: u8 = 0x3E;
}

/// `ovr005.disease_types` (`ovr005.cs:9-11`) — the six affects Cure Disease
/// (and `Heal`) strip, in the original's own array order.
pub const DISEASE_TYPES: [u8; 6] = [
    aff::HELPLESS,
    aff::CAUSE_DISEASE_1,
    aff::WEAKEN,
    aff::CAUSE_DISEASE_2,
    aff::ANIMATE_DEAD,
    aff::AFFECT_39,
];

/// `Race.elf` (`Classes/Enums.cs`) — the *spell* Raise Dead's own exclusion.
pub const RACE_ELF: u8 = 2;

/// One row of `temple_sl` (`ovr005.cs:13`) with the price its handler charges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Service {
    pub name: &'static str,
    pub cost: i64,
}

/// The ten rows `temple_heal` actually lists. `temple_sl` has an eleventh
/// entry (`"Exit"`) that the `for (i = 0; i < 10; i++)` build loop never adds.
pub const SERVICES: [Service; 10] = [
    Service {
        name: "Cure Blindness",
        cost: 1000,
    },
    Service {
        name: "Cure Disease",
        cost: 1000,
    },
    Service {
        name: "Cure Light Wounds",
        cost: 100,
    },
    Service {
        name: "Cure Serious Wounds",
        cost: 350,
    },
    Service {
        name: "Cure Critical Wounds",
        cost: 600,
    },
    Service {
        name: "Heal",
        cost: 5000,
    },
    Service {
        name: "Neutralize Poison",
        cost: 1000,
    },
    Service {
        name: "Raise Dead",
        cost: 5500,
    },
    Service {
        name: "Remove Curse",
        cost: 3500,
    },
    Service {
        name: "Stone to Flesh",
        cost: 2000,
    },
];

/// Whether the member visibly needs service `index`, and the line the temple
/// says when they do not — `CastCureAnyway`'s argument (`ovr005.cs:17-25`).
///
/// A `None` return means the service asks nothing first (the three cure-wounds
/// rows and `Heal` never do: a full-health character can buy healing and waste
/// it).
pub fn not_needed_line(index: usize, ch: &Character) -> Option<&'static str> {
    match index {
        0 => (!ch.has_affect(aff::BLINDED)).then_some("is not blind."),
        1 => (!DISEASE_TYPES.iter().any(|&a| ch.has_affect(a))).then_some("is not diseased."),
        6 => (!ch.has_affect(aff::POISONED)).then_some("is not poisoned."),
        // `raise_dead` (`:161-169`): `dead` OR `animated` both count as dead.
        7 => (!matches!(ch.status.health_status, status::DEAD | status::ANIMATED))
            .then_some("is not dead."),
        // `remove_curse` (`:263-270`): a cursed ITEM counts too.
        8 => {
            (!ch.has_affect(aff::BESTOW_CURSE) && !has_cursed_item(ch)).then_some("is not cursed.")
        }
        9 => (ch.status.health_status != status::STONED).then_some("is not stoned."),
        _ => None,
    }
}

fn has_cursed_item(ch: &Character) -> bool {
    ch.items
        .iter()
        .any(|i| gbx_formats::save_orig::item_is_cursed(i))
}

/// What [`pay`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payment {
    /// The member's own purse covered it (`ovr005.cs:39-42`).
    OwnPurse,
    /// The pooled money covered it (`:43-46`).
    Pool,
    /// `"Not enough money."` (`:49`).
    NotEnough,
}

/// `buy_cure`'s money half (`ovr005.cs:28-59`), after the player has said Yes.
/// The order is load-bearing: **own purse first**, pool second, and never
/// another member's coins.
pub fn pay(ch: &mut Character, pool: &mut MoneySet, cost: i64, rules: &RuleSet) -> Payment {
    if crate::money::can_afford(&ch.money, cost, rules) {
        crate::money::subtract_gold_worth(&mut ch.money, cost, rules);
        return Payment::OwnPurse;
    }
    if pool.gold_worth() >= cost {
        pool.subtract_gold_worth(cost);
        return Payment::Pool;
    }
    Payment::NotEnough
}

/// `buy_cure`'s price line (`ovr005.cs:30`).
pub fn price_line(index: usize) -> String {
    let s = SERVICES[index];
    format!("{} will only cost {} gold pieces.", s.name, s.cost)
}

/// Apply service `index`'s effect to `ch` — the paid-for half, run only after
/// [`pay`] returned something other than [`Payment::NotEnough`].
///
/// Returns the status line the temple prints. `buy_cure`'s own success line is
/// `"is cured."` (`:57`); the services that say something else say it here.
pub fn apply(index: usize, ch: &mut Character, rng: &mut EngineRng, rules: &RuleSet) -> String {
    match index {
        // `cure_blindness` (`:62-77`).
        0 => {
            affects::remove_affect(ch, aff::BLINDED);
            "is cured.".into()
        }
        // `cure_disease` (`:80-102`) — all six, `cureSpell` set so the
        // remove-side handlers know it is a cure rather than an expiry.
        1 => {
            for a in DISEASE_TYPES {
                affects::remove_affect(ch, a);
            }
            "is cured.".into()
        }
        // `cure_wounds(1..3)` (`:105-135`).
        2 => {
            let amount = i32::from(roll_dice(rng, 8, 1));
            heal_player(0, amount, ch);
            "is cured.".into()
        }
        3 => {
            let amount = i32::from(roll_dice(rng, 8, 2)) + 1;
            heal_player(0, amount, ch);
            "is cured.".into()
        }
        4 => {
            let amount = i32::from(roll_dice(rng, 8, 3)) + 3;
            heal_player(0, amount, ch);
            "is cured.".into()
        }
        // ★ `cure_wounds(4)` — **Heal** (`:136-156`). Note the sign: the amount
        // is `hit_point_max − hit_point_current − 1d4`, so the 5000gp premium
        // service deliberately leaves 1..4 points short of full, and on a
        // barely-scratched character the subtraction goes NEGATIVE — which
        // `heal_player` ignores, because its own guard only ever adds.
        5 => {
            let mut amount = ch.hit_point_max as i32 - ch.hit_point_current as i32;
            amount -= i32::from(roll_dice(rng, 4, 1));
            heal_player(0, amount, ch);
            affects::remove_affect(ch, aff::BLINDED);
            for a in DISEASE_TYPES {
                affects::remove_affect(ch, a);
            }
            affects::remove_affect(ch, aff::FEEBLEMIND);
            // `CalcStatBonuses(INT)`/`(WIS)` (`:151-152`) undo `AffectFeebleMind`'s
            // `Int.full = Wis.full = 7` (`ovr013.cs:895-900`) by recomputing from
            // the `.cur` cell the affect never touched.
            ch.stats.int.original = ch.stats.int.current;
            ch.stats.wis.original = ch.stats.wis.current;
            "is cured.".into()
        }
        // `cure_poison2` (`:240-258`) — the same three removals
        // `SpellNeutralizePoison` makes, but WITHOUT its `health_status = okey`
        // / `hit_point_current = 1` revival: the temple only unpoisons.
        6 => {
            affects::remove_affect(ch, aff::POISONED);
            affects::remove_affect(ch, aff::SLOW_POISON);
            affects::remove_affect(ch, aff::POISON_DAMAGE);
            "is cured.".into()
        }
        // `raise_dead` (`:160-236`).
        7 => {
            raise_dead(ch, rules);
            "is cured.".into()
        }
        // `remove_curse` (`:261-277`) → `SpellRemoveCurse`.
        8 => {
            crate::camp_cast::remove_curse_effect(ch);
            "is cured.".into()
        }
        // ★ `stone_to_flesh` (`:280-294`) — the medusa answer.
        9 => {
            ch.status.health_status = status::OKEY;
            ch.status.in_combat = true;
            ch.hit_point_current = 1;
            "is cured.".into()
        }
        _ => String::new(),
    }
}

/// Whether service `index` only takes effect on a member who actually needs it
/// — the second half of the original's `buy_cure(...) && <still qualifies>`
/// conjunctions.
///
/// Three services charge for nothing: `raise_dead` (`:174` —
/// `buy_cure(...) && player_dead == true`) and `stone_to_flesh` (`:290-291`)
/// both re-test after taking the money, and Cure Blindness/Disease/Poison
/// simply remove an affect that is not there. The temple keeps the gold either
/// way; that is the original's, and it is why the "cast cure anyway" prompt
/// exists at all.
pub fn requires_the_condition(index: usize) -> bool {
    matches!(index, 7 | 9)
}

/// ★ **`raise_dead` at the temple** (`ovr005.cs:160-236`), and what the
/// decompilation forced.
///
/// The unambiguous half, transcribed:
///
/// - the member must be `dead` **or** `animated` (`:164-168`) — and note the
///   temple, unlike the *spell* (`SpellRaiseDead`, `ovr023.cs:2343-2345`),
///   imposes **no elf clause and no `Con > 0` clause**. In CotAB an elf who
///   dies can be raised at a temple and cannot be raised by a cleric;
/// - `animate_dead` and `poisoned` come off, under `cureSpell` (`:178-183`);
/// - `hit_point_current = 1`, `health_status = okey`, `in_combat = true`
///   (`:185-187`);
/// - Constitution drops by one, and the maximum-hit-point total is recomputed
///   for the new score.
///
/// ★ **Two corrections, with evidence.** The decompiled temple copy reads
/// `if (player.stats2.Con.full <= 0) { player.stats2.Con.full--; }` — a guard
/// that can never fire for any character with a Constitution at all, and whose
/// body would then drive the score further negative. The *spell*, which the
/// temple is a hand-inlined copy of, is unambiguous at the same point:
/// `Con.cur > 0` is a precondition and `Con.cur--` is unconditional
/// (`ovr023.cs:2344`, `:2356`). Read as a flipped comparison — `jle` for `jg`,
/// one bit — the two agree, and that is how this transcribes it: **the raise
/// costs a point of Constitution, guarded on it being above zero.**
///
/// The second correction is the recompute that follows. The decompilation's
/// arithmetic (`var_107 = hp_max − hp_rolled; … var_107 /= var_108;
/// hit_point_max = var_107`) cannot be literal: for a Constitution-16 fighter 5
/// it sets maximum hit points to **1**. Its shape is a hand-inlined
/// `CalcStatBonuses(Stat.CON)` — `var_108`'s per-class loop is
/// `ConHitPointBonus` (`ovr024.cs:782-831`) inlined term for term — and that
/// function *is* readable, is what the spell calls at exactly this point
/// (`ovr023.cs:2358`), and does the same job correctly. So the recompute here
/// is [`recalc_hp_for_con`], the transcription of `CalcStatBonuses(Stat.CON)`
/// (`ovr024.cs:1053-1105`). The literal is preserved above so a future reader
/// with the overlay's disassembly can check the call.
/// The member ends at **exactly one hit point** either way. The temple's
/// inlined recompute writes `hit_point_max` only — it has no counterpart to
/// `CalcStatBonuses`' `hit_point_current` delta (`ovr024.cs:1090-1104`) — and
/// the spell reaches the same place by ordering, calling `CalcStatBonuses`
/// first and assigning `hit_point_current = 1` after it (`ovr023.cs:2358-2359`).
pub fn raise_dead(ch: &mut Character, rules: &RuleSet) {
    affects::remove_affect(ch, aff::ANIMATE_DEAD);
    affects::remove_affect(ch, aff::POISONED);
    ch.status.health_status = status::OKEY;
    ch.status.in_combat = true;
    if ch.stats.con.current > 0 {
        ch.stats.con.current -= 1;
    }
    recalc_hp_for_con(ch, rules);
    ch.hit_point_current = 1;
}

/// ★ `CalcStatBonuses(Stat.CON, player)` (`ovr024.cs:1053-1105`) — the maximum
/// hit-point recompute a Constitution change triggers.
///
/// Its own order, which is where the arithmetic lives:
///
/// 1. `hit_point_max = hit_point_rolled` — strip every accumulated CON bonus;
/// 2. sum `ConHitPointBonus` over the **old** class levels
///    (`ClassLevelsOld`) and, separately, over each current class level above
///    `multiclassLevel` (clamped to `max_class_hit_dice`);
/// 3. divide the sum by the number of classes with a current level — an
///    integer divide, so a multiclass character loses the remainder;
/// 4. add it back, then move `hit_point_current` by the same delta (down only
///    as far as 0);
/// 5. `Con.full = stat_a` — the recomputed score, item modifiers included;
/// 6. Constitution 20+ plants `highConRegen` with `(26 − con) × 10` rounds,
///    below 20 removes it.
///
/// The item modifiers (`:900-910`) are the two readied-item arms: an
/// `affect_3 & 0x7F == 6` item is +1 Constitution outright, and an
/// `affect_3 & 0x7F == 8` item with `affect_2 == 4` is +1 while the score is
/// under 18.
pub fn recalc_hp_for_con(ch: &mut Character, rules: &RuleSet) {
    use gbx_rules::flavor::Flavor;
    let flavor = gbx_rules::adnd1::flavor_impl::Adnd1::new(rules);

    // `stat_a` (`:840`) is the CURRENT cell, plus readied-item modifiers.
    let mut con = ch.stats.con.current as i32;
    for (i, item) in ch.items.iter().enumerate() {
        if !ch.readied_items.contains(&i) {
            continue;
        }
        let affect_3 = gbx_formats::save_orig::item_affect(item, 3);
        if affect_3 <= 0x80 {
            continue;
        }
        match affect_3 & 0x7F {
            6 => con += 1,
            8 if (ch.stats.con.current as i32) < 18
                && gbx_formats::save_orig::item_affect(item, 2) == 4 =>
            {
                con += 1
            }
            _ => {}
        }
    }
    let con = con.clamp(0, 25) as u8;

    let orig_max_hp = ch.hit_point_max as i32;
    let mut hp_max = ch.hit_point_rolled as i32;
    let mut bonus = 0i32;
    let mut class_count = 0i32;
    for class_id in 0..8usize {
        let old = ch.class_levels_old[class_id] as u32;
        if old > 0 {
            bonus += flavor.con_hp_total_bonus(
                class_id,
                old,
                con,
                ch.multiclass_level as u32,
                ch.class_levels_old[4] as u32,
            );
        }
        let mut lvl = ch.class_level[class_id] as u32;
        if lvl > 0 {
            class_count += 1;
        }
        let max_hd = gbx_rules::adnd1::hp_hd::max_class_hit_dice(rules, class_id) as u32;
        if max_hd < lvl {
            lvl = max_hd;
        }
        if lvl > ch.multiclass_level as u32 {
            lvl -= ch.multiclass_level as u32;
            bonus += flavor.con_hp_total_bonus(
                class_id,
                lvl,
                con,
                ch.multiclass_level as u32,
                ch.class_levels_old[4] as u32,
            );
        }
    }
    if class_count > 0 {
        bonus /= class_count;
    }
    hp_max += bonus;
    ch.hit_point_max = hp_max.clamp(0, u8::MAX as i32) as u8;

    // `:1090-1104` — the current total moves by the same delta.
    let delta = ch.hit_point_max as i32 - orig_max_hp;
    let cur = (ch.hit_point_current as i32 + delta).max(0);
    ch.hit_point_current = cur.clamp(0, u8::MAX as i32) as u8;

    ch.stats.con.original = con;

    // `:1107-1121` — the high-Constitution regeneration affect.
    if con >= 20 {
        if !ch.has_affect(aff::HIGH_CON_REGEN) {
            let rounds = (26 - con as u16) * 10;
            affects::add_affect(ch, aff::HIGH_CON_REGEN, rounds, 0xFF, true);
        }
    } else {
        affects::remove_affect(ch, aff::HIGH_CON_REGEN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::{Money, Party};

    fn ch(name: &str) -> Character {
        let rec = vec![0u8; gbx_formats::save_orig::CHAR_RECORD_SIZE];
        let record = gbx_formats::save_orig::decode_char_record(&rec).unwrap();
        let mut c = crate::party::character_from_record(&record, Vec::new(), Vec::new());
        c.name = name.to_string();
        c.hit_point_max = 30;
        c.hit_point_current = 30;
        c.hit_point_rolled = 30;
        c.stats.con.current = 12;
        c.stats.con.original = 12;
        c.status.in_combat = true;
        c
    }

    fn rules() -> RuleSet {
        RuleSet::load()
    }

    /// The whole `temple_sl` list, in the original's order, with the prices
    /// each handler charges (`ovr005.cs:13`, `:73/:91/:107/:120/:130/:138/:250/:174/:275/:290`).
    #[test]
    fn the_service_table_is_the_originals() {
        let names: Vec<&str> = SERVICES.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "Cure Blindness",
                "Cure Disease",
                "Cure Light Wounds",
                "Cure Serious Wounds",
                "Cure Critical Wounds",
                "Heal",
                "Neutralize Poison",
                "Raise Dead",
                "Remove Curse",
                "Stone to Flesh",
            ]
        );
        let costs: Vec<i64> = SERVICES.iter().map(|s| s.cost).collect();
        assert_eq!(
            costs,
            vec![1000, 1000, 100, 350, 600, 5000, 1000, 5500, 3500, 2000]
        );
        assert_eq!(price_line(7), "Raise Dead will only cost 5500 gold pieces.");
    }

    /// `buy_cure`'s payment order (`ovr005.cs:39-50`): own purse, then the
    /// pool, then a refusal. Another member's purse is never touched.
    #[test]
    fn the_purse_pays_before_the_pool() {
        let rules = rules();
        let mut c = ch("SHARA");
        c.money = Money {
            gold: 200,
            ..Default::default()
        };
        let mut pool = MoneySet::default();
        pool.set(3, 5000);

        assert_eq!(pay(&mut c, &mut pool, 100, &rules), Payment::OwnPurse);
        assert_eq!(c.money.gold, 100);
        assert_eq!(pool.get(3), 5000, "the pool was not touched");

        // Now the purse is short, so the pool covers it.
        assert_eq!(pay(&mut c, &mut pool, 600, &rules), Payment::Pool);
        assert_eq!(
            c.money.gold, 100,
            "the purse is left alone once it is short"
        );
        assert_eq!(pool.get(3), 4400);

        // And when neither can, nothing moves.
        assert_eq!(pay(&mut c, &mut pool, 99_999, &rules), Payment::NotEnough);
        assert_eq!(c.money.gold, 100);
        assert_eq!(pool.get(3), 4400);
    }

    /// ★ Stone to Flesh — the medusa answer, and the only one in the game.
    #[test]
    fn stone_to_flesh_stands_a_statue_up_at_one_hit_point() {
        let rules = rules();
        let mut rng = EngineRng::new(1);
        let mut c = ch("TRAVIS");
        c.status.health_status = status::STONED;
        c.status.in_combat = false;
        c.hit_point_current = 0;

        assert_eq!(not_needed_line(9, &c), None, "a statue needs it");
        apply(9, &mut c, &mut rng, &rules);
        assert_eq!(c.status.health_status, status::OKEY);
        assert!(c.status.in_combat);
        assert_eq!(c.hit_point_current, 1);

        // And a member who is not stoned is asked first.
        assert_eq!(not_needed_line(9, &c), Some("is not stoned."));
    }

    /// ★ Raise Dead's real mechanics: the gate is `dead || animated` with
    /// **no** elf clause (unlike the spell), the poison comes off with the
    /// animation, the member stands at one hit point, and Constitution drops
    /// by one with the maximum-hit-point total recomputed for it.
    #[test]
    fn raise_dead_costs_a_point_of_constitution_and_stands_the_member_up() {
        let rules = rules();
        let mut c = ch("MARK");
        c.race = RACE_ELF; // the temple does not care
        c.class_level[2] = 5; // fighter 5
        c.stats.con.current = 16;
        c.stats.con.original = 16;
        c.hit_point_rolled = 30;
        c.hit_point_max = 40; // 30 rolled + 5 levels x +2
        c.hit_point_current = 0;
        c.status.health_status = status::DEAD;
        c.status.in_combat = false;
        affects::add_affect(&mut c, aff::POISONED, 0, 0xFF, false);
        affects::add_affect(&mut c, aff::ANIMATE_DEAD, 0, 0xFF, false);

        assert_eq!(not_needed_line(7, &c), None, "a corpse needs it");
        raise_dead(&mut c, &rules);

        assert_eq!(c.status.health_status, status::OKEY);
        assert!(c.status.in_combat);
        assert_eq!(c.hit_point_current, 1);
        assert!(!c.has_affect(aff::POISONED), "the poison came off");
        assert!(!c.has_affect(aff::ANIMATE_DEAD));
        assert_eq!(c.stats.con.current, 15, "one point of Constitution");
        // Con 15 pays a fighter +1/level, so the recompute is 30 + 5 = 35.
        assert_eq!(c.hit_point_max, 35);
    }

    /// An `animated` member counts as dead for the raise (`ovr005.cs:164-168`).
    #[test]
    fn an_animated_member_is_raisable() {
        let mut c = ch("LEDERA");
        c.status.health_status = status::ANIMATED;
        assert_eq!(not_needed_line(7, &c), None);
        c.status.health_status = status::UNCONSCIOUS;
        assert_eq!(not_needed_line(7, &c), Some("is not dead."));
    }

    /// ★ `Heal` costs 5000 gold and deliberately stops 1d4 points short of
    /// full (`ovr005.cs:139-142`).
    #[test]
    fn heal_stops_one_to_four_points_short_of_full() {
        let rules = rules();
        let mut rng = EngineRng::new(7);
        let mut c = ch("MATHEW");
        c.hit_point_max = 40;
        c.hit_point_current = 3;
        affects::add_affect(&mut c, aff::BLINDED, 0, 0xFF, false);
        affects::add_affect(&mut c, aff::FEEBLEMIND, 0, 0xFF, false);
        c.stats.int.current = 16;
        c.stats.int.original = 7; // as `AffectFeebleMind` left it
        c.stats.wis.current = 15;
        c.stats.wis.original = 7;

        apply(5, &mut c, &mut rng, &rules);

        let short = c.hit_point_max as i32 - c.hit_point_current as i32;
        assert!(
            (1..=4).contains(&short),
            "Heal leaves 1..4 short, left {short}"
        );
        assert!(!c.has_affect(aff::BLINDED));
        assert!(!c.has_affect(aff::FEEBLEMIND));
        assert_eq!(c.stats.int.original, 16, "the recompute undid feeblemind");
        assert_eq!(c.stats.wis.original, 15);
    }

    /// Neutralize Poison at the temple strips all three poison affects — but
    /// unlike the spell it does NOT stand a downed member up (`ovr005.cs:246-258`
    /// vs `ovr023.cs:2252-2268`).
    #[test]
    fn the_temple_unpoisons_without_reviving() {
        let rules = rules();
        let mut rng = EngineRng::new(3);
        let mut c = ch("PHILIPPE");
        c.status.health_status = status::UNCONSCIOUS;
        c.status.in_combat = false;
        c.hit_point_current = 0;
        affects::add_affect(&mut c, aff::POISONED, 0, 0xFF, false);
        affects::add_affect(&mut c, aff::SLOW_POISON, 300, 0xFF, true);
        affects::add_affect(&mut c, aff::POISON_DAMAGE, 10, 0xFF, true);

        assert_eq!(not_needed_line(6, &c), None);
        apply(6, &mut c, &mut rng, &rules);

        assert!(!c.has_affect(aff::POISONED));
        assert!(!c.has_affect(aff::SLOW_POISON));
        assert!(!c.has_affect(aff::POISON_DAMAGE));
        assert_eq!(
            c.status.health_status,
            status::UNCONSCIOUS,
            "the temple's own cure_poison2 has no revival arm"
        );
    }

    /// Cure Disease strips the whole six-row `disease_types` array.
    #[test]
    fn cure_disease_strips_all_six_rows() {
        let rules = rules();
        let mut rng = EngineRng::new(5);
        let mut c = ch("SHARA");
        for a in DISEASE_TYPES {
            affects::add_affect(&mut c, a, 0, 0xFF, false);
        }
        assert_eq!(not_needed_line(1, &c), None);
        apply(1, &mut c, &mut rng, &rules);
        for a in DISEASE_TYPES {
            assert!(!c.has_affect(a), "affect {a:#04x} survived");
        }
        assert_eq!(not_needed_line(1, &c), Some("is not diseased."));
    }

    /// The three cure-wounds rows roll the original's dice.
    #[test]
    fn the_cure_rows_roll_their_own_dice() {
        let rules = rules();
        let mut rng = EngineRng::new(11);
        for (index, lo, hi) in [(2usize, 1, 8), (3, 3, 17), (4, 6, 27)] {
            let mut c = ch("SHARA");
            c.hit_point_max = 60;
            c.hit_point_current = 1;
            assert_eq!(not_needed_line(index, &c), None, "no precondition prompt");
            apply(index, &mut c, &mut rng, &rules);
            let healed = c.hit_point_current as i32 - 1;
            assert!(
                (lo..=hi).contains(&healed),
                "service {index} healed {healed}, expected {lo}..={hi}"
            );
        }
    }

    /// The party-level helper the screen uses.
    #[test]
    fn a_party_can_be_walked_through_a_service() {
        let rules = rules();
        let mut rng = EngineRng::new(2);
        let mut party = Party {
            members: vec![ch("A"), ch("B")],
        };
        party.members[0].money = Money {
            gold: 5000,
            ..Default::default()
        };
        party.members[0].status.health_status = status::STONED;
        let mut pool = MoneySet::default();
        assert_eq!(
            pay(&mut party.members[0], &mut pool, SERVICES[9].cost, &rules),
            Payment::OwnPurse
        );
        apply(9, &mut party.members[0], &mut rng, &rules);
        assert_eq!(party.members[0].status.health_status, status::OKEY);
        assert_eq!(party.members[0].money.gold, 3000);
    }
}
