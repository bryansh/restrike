//! ★ **The post-fight award** (`AfterCombatExpAndTreasure`, `ovr006.cs:763`)
//! — the M5 ledger's last deferred combat mechanism, and TREASURE's other
//! half.
//!
//! The original's flow, in its own order:
//!
//! ```text
//! CleanupPlayersStateAfterCombat  (:169) → battleWon? → calc_battle_exp → addExp
//! DeallocateNonTeamMemebers       (:676)
//! distributeNpcTreasure           (:713)   an NPC in the party takes a cut
//! displayCombatResults            (:381)   "The party has won." + "Each character receives N"
//! distributeCombatTreasure        (:564)   the View/Take/Pool/Share screen
//! ```
//!
//! ## Draw-neutrality — proven, not hoped (the brief's hard constraint)
//!
//! **Every function on this path is draw-free.** `calc_battle_exp`, `addExp`,
//! `CleanupPlayersStateAfterCombat`, `DeallocateNonTeamMemebers`,
//! `distributeNpcTreasure` (whose `ScaleAll` is integer arithmetic),
//! `displayCombatResults`, `poolMoney`, `share_pooled`, `TakePoolMoney`,
//! `PickupCoins` and `treasureOnGround` contain **no** call to
//! `seg051.Random` or `ovr024.roll_dice` — a grep over `ovr006.cs`, `ovr007.cs`
//! and `ovr022.cs` puts every draw in that neighbourhood inside
//! `randomBonus`/`create_item`/`appraiseGemsJewels`, none of which the award
//! path reaches (`create_item` is TREASURE's random arm, and appraisal is a
//! shop service).
//!
//! So the award cannot perturb any capture's draw stream *at all*, let alone
//! after its final draw: there is nothing to perturb it with. That is a
//! stronger statement than "the award happens after the capture ends", and it
//! is the one the guard then confirms (16/16, unchanged).
//!
//! The one draw-bearing neighbour is the **script** opcode TREASURE (0x27),
//! whose `>= 0x80` arm rolls its items — and every shipped use of that arm
//! precedes its `COMBAT` (`ECL5#49 @0x94B3` is `TREASURE …,0x82` two
//! instructions before the fight), so those draws land before a capture's
//! first recorded one.

use crate::combat::{CombatState, Team};
use crate::money::MoneySet;
use crate::party::{Character, Party};

/// `Control.NPC_Base` (`Classes/Player.cs:318`).
const NPC_BASE: u8 = 0x80;
/// `Status.okey` / `Status.animated` / `Status.running` (`Classes/Enums.cs`).
const STATUS_OKAY: u8 = 0;
const STATUS_ANIMATED: u8 = 1;
const STATUS_RUNNING: u8 = 3;

/// `ClassId` (`Classes/Enums.cs`) — the ids `addExp`'s prime-requisite bonus
/// switch names, plus the multiclass range it divides by.
mod class {
    pub const CLERIC: u8 = 0;
    pub const PALADIN: u8 = 3;
    pub const RANGER: u8 = 4;
    pub const MAGIC_USER: u8 = 5;
    pub const THIEF: u8 = 6;
    pub const FIGHTER: u8 = 2;
    /// `mc_c_f` .. `mc_f_t` — the two-class combos (`:125-127`).
    pub const MC_FIRST_DUAL: u8 = 8;
    pub const MC_LAST_DUAL: u8 = 14;
    /// `mc_c_f_m` / `mc_f_mu_t` — the three-class combos (`:132-133`).
    pub const MC_TRIPLE_A: u8 = 15;
    pub const MC_TRIPLE_B: u8 = 16;
}

/// What one fight paid out — the numbers [`crate::combat_host`]'s results
/// screen prints and the pool it then hands to the treasure screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct AwardOutcome {
    /// `gbl.exp_to_add` — experience **per surviving character**, already
    /// divided (`calc_battle_exp`'s trailing `total / (party_size -
    /// partyAnimatedCount)`).
    pub exp_each: i32,
    /// `gbl.byte_1AB14` — whether any enemy died. `displayCombatResults`
    /// forks on it: false means the "fight" was a treasure find, not a
    /// battle, and the screen says so.
    pub any_enemy_died: bool,
    /// How many item records the corpses added to the pool.
    pub items_added: usize,
}

/// ★ `calc_battle_exp` (`ovr006.cs:8-63`) — the whole formula, in the
/// original's order, because the order is where the money goes:
///
/// 1. For every **enemy** combatant whose health is neither `okey` nor
///    `running` (`:23-25` — the dead and the downed, but never one that fled):
///    its purse is added to the pool, `field_13E * hit_point_rolled +
///    field_13C` to the running total, and its items are cloned into the pool
///    with `readied` cleared (`:40-42`).
/// 2. **Then** `pooled_money.GetExpWorth()` (`:48`) — so the coins the
///    corpses just paid in are themselves worth experience, at 1 per gp plus
///    250 per gem and 2200 per piece of jewelry.
/// 3. **Then** 400 experience per `plus` of every magical item in the pool
///    (`:56-59`) — including items a *script* TREASURE put there before the
///    fight.
/// 4. Finally the total is divided by `party_size - partyAnimatedCount`
///    (`:62`) — an integer division, so the remainder is simply lost.
///
/// `area2_ptr.field_5C6 == 1` suppresses the item half (`:34`) — the
/// "monsters carry nothing" flag some scripted fights set.
///
/// Draw-free.
pub fn calc_battle_exp(
    state: &CombatState,
    pool: &mut MoneySet,
    items: &mut Vec<Vec<u8>>,
    party_size: u8,
    animated_count: u8,
    suppress_items: bool,
) -> AwardOutcome {
    let mut total: i64 = 0;
    let mut any_enemy_died = false;
    let items_before = items.len();

    for c in state.roster().iter() {
        if c.team != Team::Monster {
            continue;
        }
        // `:24-25` — alive, or it got away with its purse. Matched on the
        // VARIANTS, not on `as u8`: [`HealthStatus`]'s discriminants are
        // declaration order, not coab's `Status` codes.
        if matches!(
            c.health_status,
            crate::combat::HealthStatus::Okey | crate::combat::HealthStatus::Running
        ) {
            continue;
        }
        any_enemy_died = true; // `gbl.byte_1AB14`, `:27`
        pool.add_purse(&c.award.money); // `:29`
        total += c.award.exp_per_hp as i64 * c.award.hit_point_rolled as i64; // `:31`
        total += c.award.base_exp as i64; // `:32`
        if !suppress_items {
            for item in &c.award.items {
                let mut cloned = item.clone();
                // `newItem.readied = false` (`:41`) — a looted weapon is not
                // readied on whoever picks it up.
                if let Some(byte) = cloned.get_mut(0x34) {
                    *byte = 0;
                }
                items.push(cloned);
            }
        }
    }

    total += pool.exp_worth(); // `:48`

    for item in items.iter() {
        let plus = gbx_formats::save_orig::item_plus(item);
        if plus > 0 {
            total += plus as i64 * 400; // `:56-59`
        }
    }

    // `:62` — integer division by the surviving share count. The original
    // would divide by zero here with a wiped party; that arm is unreachable
    // (`battleWon` gates the call), and 1 is the closest safe floor.
    let shares = (party_size as i64 - animated_count as i64).max(1);
    AwardOutcome {
        exp_each: (total / shares) as i32,
        any_enemy_died,
        items_added: items.len() - items_before,
    }
}

/// ★ `addExp` (`ovr006.cs:67-145`) — the prime-requisite bonus, transcribed
/// class by class. A single-class character with the right ability **above
/// 15** gets `exp + exp/10`; a two-class character gets `exp / 2` and a
/// three-class one `exp / 3`, with no bonus at all. Only members who are
/// `in_combat` and not `animated` are paid (`:71-72`).
///
/// Note the multiclass arms are *divisions of the base*, applied instead of
/// the bonus rather than after it — a multiclassed fighter with 18 strength
/// still gets half, not half-plus-ten-percent.
///
/// Draw-free.
pub fn add_exp(party: &mut Party, exp_to_add: i32) {
    for member in party.members.iter_mut() {
        if !member.status.in_combat || member.status.health_status == STATUS_ANIMATED {
            continue;
        }
        let bonus_if = |cond: bool| {
            if cond {
                exp_to_add + exp_to_add / 10
            } else {
                exp_to_add
            }
        };
        let stats = &member.stats;
        let new_exp = match member.class_id {
            class::CLERIC => bonus_if(stats.wis.current > 15),
            class::FIGHTER => bonus_if(stats.str_score.current > 15),
            class::PALADIN => bonus_if(stats.str_score.current > 15 && stats.wis.current > 15),
            class::RANGER => bonus_if(
                stats.str_score.current > 15 && stats.int.current > 15 && stats.wis.current > 15,
            ),
            class::MAGIC_USER => bonus_if(stats.int.current > 15),
            class::THIEF => bonus_if(stats.dex.current > 15),
            id if (class::MC_FIRST_DUAL..=class::MC_LAST_DUAL).contains(&id) => exp_to_add / 2,
            class::MC_TRIPLE_A | class::MC_TRIPLE_B => exp_to_add / 3,
            _ => exp_to_add,
        };
        member.exp += new_exp;
    }
}

/// ★ `distributeNpcTreasure` (`ovr006.cs:713-760`): a joined NPC takes a cut
/// of the pooled coins before the party ever sees the screen.
///
/// The share arithmetic is the original's, integer division included:
/// `npcParts / totalParts` is computed in **`int`** and only then used as the
/// scale (`:736`), so unless the NPCs' parts outnumber everyone else's the
/// ratio is 0 and `ScaleAll` **wipes the coins entirely**. That is not a
/// transcription slip on our side — it is what the shipped code does, and it
/// is why the "takes and hides his share" message is so memorable.
///
/// Returns the names that took a share, in roster order, for the screen.
/// Draw-free.
pub fn distribute_npc_treasure(party: &Party, pool: &mut MoneySet) -> Vec<String> {
    let mut npc_parts: i64 = 0;
    let mut total_parts: i64 = 0;
    for member in party.members.iter() {
        if member.control_morale >= NPC_BASE && member.status.health_status == STATUS_OKAY {
            let parts = (member.status.npc_treasure_share_count & 7) as i64;
            npc_parts += parts;
            total_parts += parts;
        } else {
            total_parts += 1;
        }
    }
    if npc_parts == 0 {
        return Vec::new();
    }
    if !pool.scale_coins(npc_parts, total_parts) {
        return Vec::new();
    }
    party
        .members
        .iter()
        .filter(|m| {
            m.control_morale >= NPC_BASE
                && m.status.health_status == STATUS_OKAY
                && m.status.npc_treasure_share_count > 0
        })
        .map(|m| {
            let pronoun = if m.sex == 0 { "his" } else { "her" };
            format!("{} takes and hides {pronoun} share.", m.name)
        })
        .collect()
}

/// `poolMoney` (`ovr022.cs:138-154`): every **PC** (`control_morale` at the
/// two PC values, so joined NPCs keep their purses) empties into the pool.
/// Draw-free.
pub fn pool_money(party: &mut Party, pool: &mut MoneySet) {
    for member in party.members.iter_mut() {
        if member.control_morale >= NPC_BASE {
            continue;
        }
        pool.add_purse(&member.money);
        member.money = Default::default();
    }
}

/// ★ `share_pooled` (`ovr022.cs:173-259`): split the pool evenly over the
/// PCs, hand out the remainders one coin at a time, and leave whatever still
/// will not fit in the pool.
///
/// Two things the original says out loud: the split counts **PCs only**
/// (`GetPartyCount`, `:156-171`) but the payout loop's own test is
/// `control_morale < NPC_Base` — the same set — and the walk runs **jewelry
/// down to copper** (`for coin = 6; coin >= 0`), so the heaviest denominations
/// are placed first. Encumbrance is not modeled here (our `Character` carries
/// weight but no capacity rule yet), so the overload arms are omitted and the
/// remainder loop reduces to "give the leftovers to the first member" —
/// noted rather than silently smoothed.
///
/// Draw-free.
pub fn share_pooled(party: &mut Party, pool: &mut MoneySet) {
    let pcs: Vec<usize> = party
        .members
        .iter()
        .enumerate()
        .filter(|(_, m)| m.control_morale < NPC_BASE)
        .map(|(i, _)| i)
        .collect();
    if pcs.is_empty() {
        return;
    }
    let count = pcs.len() as i32;
    for coin in (0..7usize).rev() {
        let held = pool.get(coin);
        if held <= 0 {
            continue;
        }
        let each = held / count;
        let mut remainder = held % count;
        for &index in &pcs {
            let member = &mut party.members[index];
            let mut give = each;
            if remainder > 0 {
                give += 1;
                remainder -= 1;
            }
            let purse = member.money.get_coin(coin) as i32 + give;
            member.money.set_coin(coin, purse as i16);
        }
        pool.set(coin, remainder);
    }
}

/// `treasureOnGround` (`ovr022.cs:409-414`).
pub fn treasure_on_ground(pool: &MoneySet, items: &[Vec<u8>]) -> (bool, bool) {
    (!items.is_empty(), pool.any())
}

/// `take_items_treasure`'s one step (`ovr006.cs:469-501`): move one pooled
/// item onto a character. Returns false when the index is out of range.
pub fn take_item(party: &mut Party, member: usize, items: &mut Vec<Vec<u8>>, index: usize) -> bool {
    if index >= items.len() {
        return false;
    }
    let Some(target) = party.members.get_mut(member) else {
        return false;
    };
    target.items.push(items.remove(index));
    true
}

/// `CleanupPlayersStateAfterCombat`'s `partyAnimatedCount` (`ovr006.cs:240-244`):
/// members who ended the fight not `in_combat`, or `animated`. They are the
/// share divisor's subtrahend — a party that finished with three members down
/// splits the experience three ways, not six.
pub fn animated_count(party: &Party) -> u8 {
    party
        .members
        .iter()
        .filter(|m| !m.status.in_combat || m.status.health_status == STATUS_ANIMATED)
        .count() as u8
}

/// `CleanupPlayersStateAfterCombat`'s post-fight status ladder
/// (`ovr006.cs:267-302`), for the non-fled, non-wiped case: a member who was
/// `running` comes back to `okey`, a `dying` one settles at `unconscious`, and
/// an `unconscious` one with hit points left wakes up. Duels (which revive at
/// 1 hp) are not modeled — no shipped script sets one up.
pub fn settle_survivors(party: &mut Party) {
    for member in party.members.iter_mut() {
        match member.status.health_status {
            STATUS_RUNNING => {
                member.status.health_status = STATUS_OKAY;
                member.status.in_combat = true;
            }
            5 => member.status.health_status = 4, // dying → unconscious
            4 if member.hit_point_current > 0 => {
                member.status.health_status = STATUS_OKAY;
                member.status.in_combat = true;
            }
            _ => {}
        }
    }
}

/// `displayCombatResults`' headline (`ovr006.cs:383-424`), as text.
pub fn results_headline(
    outcome: &AwardOutcome,
    party_fled: bool,
    battle_won: bool,
) -> &'static str {
    if !outcome.any_enemy_died {
        // `:423` — nothing died, so this was a treasure find.
        return "The party has found Treasure!";
    }
    if party_fled {
        "The party has fled." // `:390`
    } else if !battle_won {
        "You have lost the fight." // `:404`
    } else {
        "The party has won." // `:416`
    }
}

/// `displayCombatResults`' two experience lines (`ovr006.cs:426-437`).
pub fn results_exp_lines(exp_each: i32) -> [String; 2] {
    [
        format!("Each character receives {exp_each}"),
        "experience points.".to_string(),
    ]
}

/// A member's `Character` view for the results screen's roster line.
pub fn is_pc(member: &Character) -> bool {
    member.control_morale < NPC_BASE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::{CombatMap, Combatant, GridPos, HealthStatus, MonsterAward, Team};
    use crate::party::{Character, Money};

    fn pc(name: &str, class_id: u8) -> Character {
        let rec = vec![0u8; gbx_formats::save_orig::CHAR_RECORD_SIZE];
        let record = gbx_formats::save_orig::decode_char_record(&rec).unwrap();
        let mut ch = crate::party::character_from_record(&record, Vec::new(), Vec::new());
        ch.name = name.to_string();
        ch.class_id = class_id;
        ch.status.in_combat = true;
        ch
    }

    fn corpse(award: MonsterAward) -> Combatant {
        let mut c = Combatant::new_melee(
            1,
            Team::Monster,
            true,
            GridPos::new(0, 0),
            0,
            0,
            0,
            0,
            (1, 4, 0),
            0,
            1,
        );
        c.health_status = HealthStatus::Dead;
        c.award = award;
        c
    }

    fn state_with(fighters: Vec<Combatant>) -> CombatState {
        CombatState::new(CombatMap::uniform(1), fighters)
    }

    fn item(plus: i8, readied: bool) -> Vec<u8> {
        let mut rec = vec![0u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
        rec[0x32] = plus as u8;
        rec[0x34] = u8::from(readied);
        rec
    }

    /// ★ The formula, term by term (`ovr006.cs:29-62`): per-corpse
    /// `field_13E * hit_point_rolled + field_13C`, then the pool's own
    /// experience worth, then 400 per item plus, then the share division.
    #[test]
    fn calc_battle_exp_sums_every_term_in_order() {
        let mut pool = MoneySet::default();
        let mut items = Vec::new();
        let state = state_with(vec![corpse(MonsterAward {
            money: Money {
                gold: 200,
                gems: 2,
                ..Default::default()
            },
            base_exp: 35,
            exp_per_hp: 3,
            hit_point_rolled: 8,
            items: vec![item(2, true)],
        })]);

        let out = calc_battle_exp(&state, &mut pool, &mut items, 6, 0, false);

        // 3*8 + 35 = 59 from the corpse; the pool is 200gp + 2 gems =
        // 200 + 500 = 700 experience worth; the +2 item is 800. Total 1559,
        // over six shares = 259 (the remainder is lost to integer division).
        assert_eq!(out.exp_each, (59 + 700 + 800) / 6);
        assert!(out.any_enemy_died);
        assert_eq!(pool.get(3), 200, "the corpse's purse reached the pool");
        assert_eq!(items.len(), 1);
        assert!(
            !gbx_formats::save_orig::item_readied(&items[0]),
            "`:41` clears `readied` on the clone"
        );
    }

    /// A monster that is still standing, or that RAN, pays nothing
    /// (`ovr006.cs:24-25`) — fleeing really does take its purse with it.
    #[test]
    fn a_living_or_fled_monster_pays_nothing() {
        for status in [HealthStatus::Okey, HealthStatus::Running] {
            let mut pool = MoneySet::default();
            let mut items = Vec::new();
            let mut c = corpse(MonsterAward {
                money: Money {
                    gold: 500,
                    ..Default::default()
                },
                base_exp: 100,
                ..Default::default()
            });
            c.health_status = status;
            let state = state_with(vec![c]);

            let out = calc_battle_exp(&state, &mut pool, &mut items, 6, 0, false);
            assert_eq!(out.exp_each, 0);
            assert!(!out.any_enemy_died);
            assert_eq!(pool.get(3), 0);
        }
    }

    /// The `field_5C6 == 1` flag suppresses the item half only — the purse and
    /// the experience still land (`ovr006.cs:34`).
    #[test]
    fn suppressed_items_still_pay_money_and_experience() {
        let mut pool = MoneySet::default();
        let mut items = Vec::new();
        let state = state_with(vec![corpse(MonsterAward {
            money: Money {
                gold: 200,
                ..Default::default()
            },
            base_exp: 10,
            items: vec![item(1, false)],
            ..Default::default()
        })]);

        let out = calc_battle_exp(&state, &mut pool, &mut items, 1, 0, true);
        assert!(items.is_empty());
        assert_eq!(out.exp_each, 10 + 200);
    }

    /// The share divisor is `party_size - partyAnimatedCount`: three members
    /// down means the experience splits three ways, not six.
    #[test]
    fn the_share_divisor_excludes_the_downed() {
        let mut pool = MoneySet::default();
        let mut items = Vec::new();
        let state = state_with(vec![corpse(MonsterAward {
            base_exp: 600,
            ..Default::default()
        })]);
        let out = calc_battle_exp(&state, &mut pool, &mut items, 6, 3, false);
        assert_eq!(out.exp_each, 200);
    }

    /// ★ `addExp`'s prime-requisite bonus and its multiclass divisions
    /// (`ovr006.cs:76-138`).
    #[test]
    fn add_exp_applies_the_prime_requisite_bonus_and_class_divisions() {
        let mut party = Party::default();
        let mut fighter = pc("STRONG", 2);
        fighter.stats.str_score.current = 18;
        let mut weak = pc("WEAK", 2);
        weak.stats.str_score.current = 15; // exactly 15 is NOT above 15
        let dual = pc("DUAL", 8);
        let triple = pc("TRIPLE", 15);
        party.members = vec![fighter, weak, dual, triple];

        add_exp(&mut party, 1000);

        assert_eq!(party.members[0].exp, 1100, "18 STR fighter: +10%");
        assert_eq!(party.members[1].exp, 1000, "15 STR is not > 15");
        assert_eq!(party.members[2].exp, 500, "two classes halve it");
        assert_eq!(party.members[3].exp, 333, "three classes third it");
    }

    /// An out-of-combat or animated member is paid nothing (`ovr006.cs:71-72`).
    #[test]
    fn add_exp_skips_the_downed_and_the_animated() {
        let mut party = Party::default();
        let mut down = pc("DOWN", 2);
        down.status.in_combat = false;
        let mut animated = pc("ANIM", 2);
        animated.status.health_status = STATUS_ANIMATED;
        party.members = vec![down, animated];

        add_exp(&mut party, 1000);
        assert_eq!(party.members[0].exp, 0);
        assert_eq!(party.members[1].exp, 0);
    }

    /// ★ `distributeNpcTreasure`'s integer ratio (`ovr006.cs:736`): with one
    /// NPC among five PCs the scale is `1 / 6 == 0` in `int` arithmetic, so
    /// the NPC's "share" is the whole pool's coins — the original's own
    /// behaviour, replicated rather than corrected.
    #[test]
    fn an_npc_share_wipes_the_coins_through_the_originals_integer_ratio() {
        let mut party = Party::default();
        let mut npc = pc("ALIAS", 2);
        npc.control_morale = 0xB2;
        npc.status.npc_treasure_share_count = 1;
        party.members = vec![pc("A", 2), pc("B", 2), npc];

        let mut pool = MoneySet::default();
        pool.set(3, 900);
        let lines = distribute_npc_treasure(&party, &mut pool);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("ALIAS takes and hides"));
        assert_eq!(pool.get(3), 0, "1/3 in int arithmetic is 0");
    }

    #[test]
    fn no_npc_in_the_party_leaves_the_pool_untouched() {
        let party = Party {
            members: vec![pc("A", 2), pc("B", 2)],
        };
        let mut pool = MoneySet::default();
        pool.set(3, 900);
        assert!(distribute_npc_treasure(&party, &mut pool).is_empty());
        assert_eq!(pool.get(3), 900);
    }

    /// `share_pooled` splits evenly and hands the remainder out one coin at a
    /// time, leaving nothing behind when it divides out.
    #[test]
    fn share_pooled_splits_evenly_and_spends_the_remainder() {
        let mut party = Party {
            members: vec![pc("A", 2), pc("B", 2), pc("C", 2)],
        };
        let mut pool = MoneySet::default();
        pool.set(3, 10); // 10 gold over three PCs

        share_pooled(&mut party, &mut pool);

        let gold: Vec<i16> = party.members.iter().map(|m| m.money.gold).collect();
        assert_eq!(gold, vec![4, 3, 3]);
        assert_eq!(pool.get(3), 0);
    }

    /// A joined NPC is not a shareholder (`control_morale < NPC_Base`).
    #[test]
    fn share_pooled_pays_pcs_only() {
        let mut npc = pc("ALIAS", 2);
        npc.control_morale = 0xB2;
        let mut party = Party {
            members: vec![pc("A", 2), npc],
        };
        let mut pool = MoneySet::default();
        pool.set(3, 8);

        share_pooled(&mut party, &mut pool);
        assert_eq!(party.members[0].money.gold, 8);
        assert_eq!(party.members[1].money.gold, 0);
    }

    /// `poolMoney` empties the PCs' purses and leaves the NPCs' alone.
    #[test]
    fn pool_money_takes_from_pcs_only() {
        let mut a = pc("A", 2);
        a.money.gold = 40;
        let mut npc = pc("ALIAS", 2);
        npc.control_morale = 0xB2;
        npc.money.gold = 70;
        let mut party = Party {
            members: vec![a, npc],
        };
        let mut pool = MoneySet::default();

        pool_money(&mut party, &mut pool);
        assert_eq!(pool.get(3), 40);
        assert_eq!(party.members[0].money.gold, 0);
        assert_eq!(party.members[1].money.gold, 70);
    }

    #[test]
    fn taking_an_item_moves_it_from_the_pool_to_the_character() {
        let mut party = Party {
            members: vec![pc("A", 2)],
        };
        let mut items = vec![item(0, false), item(1, false)];
        assert!(take_item(&mut party, 0, &mut items, 1));
        assert_eq!(items.len(), 1);
        assert_eq!(party.members[0].items.len(), 1);
        assert!(!take_item(&mut party, 0, &mut items, 9));
    }

    /// `displayCombatResults`' fork on `byte_1AB14` (`ovr006.cs:385`, `:423`):
    /// nothing died means this was a treasure find, whatever else happened.
    #[test]
    fn the_headline_says_treasure_when_nothing_died() {
        let found = AwardOutcome {
            any_enemy_died: false,
            ..Default::default()
        };
        assert_eq!(
            results_headline(&found, false, true),
            "The party has found Treasure!"
        );
        let won = AwardOutcome {
            any_enemy_died: true,
            ..Default::default()
        };
        assert_eq!(results_headline(&won, false, true), "The party has won.");
        assert_eq!(results_headline(&won, true, true), "The party has fled.");
        assert_eq!(
            results_headline(&won, false, false),
            "You have lost the fight."
        );
    }

    #[test]
    fn the_survivor_ladder_wakes_the_running_and_settles_the_dying() {
        let mut party = Party::default();
        let mut runner = pc("R", 2);
        runner.status.health_status = STATUS_RUNNING;
        runner.status.in_combat = false;
        let mut dying = pc("D", 2);
        dying.status.health_status = 5;
        let mut out_cold = pc("U", 2);
        out_cold.status.health_status = 4;
        out_cold.hit_point_current = 3;
        party.members = vec![runner, dying, out_cold];

        settle_survivors(&mut party);
        assert_eq!(party.members[0].status.health_status, STATUS_OKAY);
        assert!(party.members[0].status.in_combat);
        assert_eq!(party.members[1].status.health_status, 4);
        assert_eq!(party.members[2].status.health_status, STATUS_OKAY);
    }
}
