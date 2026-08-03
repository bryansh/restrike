//! **Live party kits** (`docs/design/combat-visualizer.md` D-CV6 item 2 / §8.4)
//! — the party half of a live fight's roster, derived from real M3 party state
//! through the **same decode path capture rosters use**.
//!
//! What this retires: `shell.rs`'s `party_combat_stats`, which mapped a
//! [`Character`] onto six numbers and gave every member a documented 1d8
//! (`DEFAULT_PARTY_WEAPON_DIE`). A live bar brawl fought with placeholder dice
//! *resembled* the capture-proven fight; this makes it **be** that fight.
//!
//! ## One decode path, two sources
//!
//! [`combatant_from_record`](super::combatant_from_record) is the fifty-line
//! record→[`Combatant`] mapping every closed capture rides. It takes a decoded
//! [`CharRecord`], so the live path's job is to *produce* one:
//! [`record_from_character`] is the exact inverse of
//! [`crate::party::character_from_record`], field for field. Nothing about how a
//! record becomes a fighter is duplicated here — if the two ever disagreed, the
//! kit-equivalence fixture test would catch it.
//!
//! ## The three things a `CharRecord` cannot carry
//!
//! `decode_char_record` drops the record's runtime pointer arrays (§1.7 item 3 /
//! D-SAVE6), which is exactly where equipment lives. All three are recovered
//! from [`Character::items`] + [`Character::readied_items`] — the party model's
//! own reconstruction of `activeItems` from each item's `readied` flag:
//!
//! 1. **`activeItems.armor` @0x159** — the §15 mage-hold gate
//!    ([`Combatant::field_159_null`]).
//! 2. **the two weapon candidates** — `AI_items_selection`'s `var_4` (ranged)
//!    and `var_8` (melee) scans (`ovr010.cs:894-936`), which is precisely what a
//!    [`Loadout`] row is. The capture path hand-pins these because a
//!    `combat_entry` snapshot never chased the pointers; a live party has the
//!    real inventory, so here they are *derived*.
//! 3. **`activeItems.arrows` @0x17D / `quarrels` @0x181** — §49's readied-ammo
//!    gate. Arrows count **only when readied**: an unreadied quiver is invisible
//!    to `var_1F`, so the launcher loses every selection and the PC fights with
//!    the melee candidate. Capture-proven by cleric-guildwar (doc §49).
//!
//! ## Honesty (D-CV6)
//!
//! The derivation is cited, and its *outputs* are the same shape the closed
//! captures pinned by hand — but no capture has been staged on a live-derived
//! kit. The first live-fight capture converts it.

use super::records::combatant_from_record;
use super::{Combatant, GridPos, Loadout, Team};
use crate::party::Character;
use gbx_formats::items::{flags, ItemDataTable};
use gbx_formats::save_orig::{CharRecord, RawStat, RawStatBlock};
use gbx_rules::flavor::Flavor;

/// `Item` on-disk offsets inside a `.swg` record (coab `Classes/Item.cs:122-141`).
mod item {
    /// `type` @0x2E — the `ITEMS` table index.
    pub const TYPE: usize = 0x2E;
    /// `plus` @0x32 (sbyte).
    pub const PLUS: usize = 0x32;
    /// `count` @0x39 — a quiver's arrows, a stack's size.
    pub const COUNT: usize = 0x39;
}

/// `ItemData.item_slot` values, which double as `activeItems` array indices
/// (coab `Classes/Player.cs:216-280`: slot 0 primary weapon, 1 secondary,
/// 2 armor, 11 arrows, 12 quarrels).
mod slot {
    pub const PRIMARY_WEAPON: u8 = 0;
    pub const ARMOR: u8 = 2;
    pub const ARROWS: u8 = 11;
    pub const QUARRELS: u8 = 12;
}

/// One party member's readied/carried gear, as the selection scan sees it.
struct KitItem {
    item_type: u8,
    plus: i8,
    count: i32,
    readied: bool,
}

fn kit_items(c: &Character) -> Vec<KitItem> {
    c.items
        .iter()
        .enumerate()
        .map(|(i, raw)| KitItem {
            item_type: raw.get(item::TYPE).copied().unwrap_or(0),
            plus: raw.get(item::PLUS).map(|&b| b as i8).unwrap_or(0),
            count: raw.get(item::COUNT).copied().unwrap_or(0) as i32,
            readied: c.readied_items.contains(&i),
        })
        .collect()
}

/// `CalcItemPowerRating` (`ovr010:1572-158E`, doc §48) for a candidate that is
/// not yet readied — the same arithmetic
/// [`CombatState::calc_item_power_rating`](super::CombatState) applies once the
/// fight owns the table.
fn power_rating(items: &ItemDataTable, item_type: u8, plus: i8) -> i32 {
    let it = items.get(item_type);
    let mut rating = it.dice_size_normal as i32 * it.dice_count_normal as i32;
    if plus > 0 {
        rating += plus as i32 * 8;
    }
    if it.bonus_normal > 0 {
        rating += it.bonus_normal as i32 * 2;
    }
    if it.flags & flags::FLAG_08 != 0 {
        rating += (it.number_attacks as i32 - 1) * 2;
    }
    if it.hands_count <= 1 {
        rating += 3;
    }
    rating
}

/// Whether **any readied item** occupies `activeItems[slot]`.
fn readied_in_slot(items: &ItemDataTable, kit: &[KitItem], slot: u8) -> bool {
    kit.iter()
        .any(|k| k.readied && items.get(k.item_type).item_slot == slot)
}

/// The bare-hands attack-1 profile a [`Loadout`] falls back to: the record's
/// BASE dice (`attack1_*Base` @0x11E/0x120/0x122) plus the strength damage
/// adjustment — the same sum §48 cross-checked against every slot-H PC's
/// serialized nothing-readied profile (`@0x1A2 == @0x122 + strengthDamBonus`).
fn unarmed_profile(c: &Character, flavor: &dyn Flavor) -> (u8, u8, u8) {
    let base = c.combat.attacks.base;
    let str_dmg = if c.opaque.field_125 != 0 {
        flavor.strength_damage_bonus(c.stats.str_score.original, c.stats.str_exceptional.current)
    } else {
        0
    };
    (
        base[2],                                        // attack1_DiceCountBase @0x11E
        base[4],                                        // attack1_DiceSizeBase  @0x120
        (base[6] as i32 + str_dmg).clamp(0, 255) as u8, // + strengthDamBonus
    )
}

/// ★ **`AI_items_selection`'s two candidate scans, run once at fight entry**
/// (`ovr010.cs:894-936`) — the live-party equivalent of the capture path's
/// hand-pinned kit rows.
///
/// Doing it at entry rather than per turn is the modelling choice a [`Loadout`]
/// already encodes: a fight never changes a combatant's inventory (only which
/// item is readied, which the per-turn selection still decides from these two
/// candidates), so scanning once yields the same two answers every turn would.
///
/// Faithful details kept from the scan:
/// - only `item_slot == 0` items are weapon candidates, and only those whose
///   `classFlags` intersect the character's (`ovr010.cs:899` — SHARA's plain
///   sling is 1e-cleric-forbidden and never becomes a candidate);
/// - the ranged candidate needs `flag_08 | flag_10` and a rating strictly above
///   `1`; the melee candidate needs `!flag_08` and a rating strictly above the
///   running best (starting at the bare-hands rating) — so ties keep the FIRST
///   item, in inventory order;
/// - `ammo_readied` reads the READIED ammo slot for the ranged candidate's own
///   flavour of ammo (§49), never the inventory.
///
/// Returns `None` when the character has no weapon candidate at all — which is
/// [`Loadout`]-free bare hands, i.e. exactly today's behaviour for that member.
pub fn loadout_for(c: &Character, items: &ItemDataTable, flavor: &dyn Flavor) -> Option<Loadout> {
    let kit = kit_items(c);
    let class_flags = c.skills.class_flags;

    let mut ranged: Option<(u8, i8)> = None;
    let mut var_15 = 1i32;
    let mut melee: Option<(u8, i8)> = None;
    let unarmed = unarmed_profile(c, flavor);
    // `var_16` starts at the BASE bare-hands rating (`ovr010.cs:886-891`).
    let mut var_16 = unarmed.1 as i32 * unarmed.0 as i32;
    if unarmed.2 as i32 > 0 {
        var_16 += unarmed.2 as i32 * 2;
    }

    for k in &kit {
        let data = items.get(k.item_type);
        if data.item_slot != slot::PRIMARY_WEAPON || (data.class_flags & class_flags) == 0 {
            continue;
        }
        let rating = power_rating(items, k.item_type, k.plus);
        if data.flags & (flags::FLAG_08 | flags::FLAG_10) != 0 && rating > var_15 {
            ranged = Some((k.item_type, k.plus));
            var_15 = rating;
        }
        if data.flags & flags::FLAG_08 == 0 && rating > var_16 {
            melee = Some((k.item_type, k.plus));
            var_16 = rating;
        }
    }

    if ranged.is_none() && melee.is_none() {
        return None;
    }

    // §49: the ammo term reads the READIED slot the launcher draws from.
    let (ammo_readied, ammo_count) = match ranged {
        Some((wtype, _)) => {
            let launcher = items.get(wtype).flags;
            let want = if launcher & flags::QUARRELS != 0 {
                Some(slot::QUARRELS)
            } else if launcher & flags::ARROWS != 0 {
                Some(slot::ARROWS)
            } else {
                None // a self-launcher (flag_10) is its own ammo
            };
            match want {
                Some(s) => {
                    let readied = readied_in_slot(items, &kit, s);
                    // The count comes from the readied quiver when there is
                    // one; an unreadied quiver's count is inert (the launcher
                    // can never win the gate) but is carried so a later ready
                    // has something honest to spend.
                    let count = kit
                        .iter()
                        .filter(|k| items.get(k.item_type).item_slot == s)
                        .max_by_key(|k| (k.readied, k.count))
                        .map(|k| k.count)
                        .unwrap_or(0);
                    (readied, count)
                }
                // `flag_10` self-launchers spend their own `count`.
                None => (
                    true,
                    kit.iter()
                        .find(|k| (k.item_type, k.plus) == (wtype, ranged.unwrap().1))
                        .map(|k| k.count)
                        .unwrap_or(0),
                ),
            }
        }
        None => (false, 0),
    };

    // Whether the record enters with the RANGED candidate readied — the flag
    // `CombatState::set_loadout` reads to decide whether to install its table
    // profile at setup. A member entering with a MELEE weapon readied takes the
    // other arm: the record's own serialized profile stands until round 0's
    // selection readies it again (the slot-H shape, doc §48).
    let entry_ranged_readied = match ranged {
        Some((wtype, plus)) => kit
            .iter()
            .any(|k| k.readied && (k.item_type, k.plus) == (wtype, plus)),
        None => false,
    };

    Some(Loadout {
        ranged,
        ammo_count,
        ammo_readied,
        melee,
        unarmed_profile: unarmed,
        entry_ranged_readied,
    })
}

/// Whether this member has armour readied — [`Combatant::field_159_null`]'s
/// live-path source (`activeItems.armor` @0x159, `Classes/Player.cs:225`).
pub fn armor_readied(c: &Character, items: Option<&ItemDataTable>) -> bool {
    let Some(items) = items else {
        // Without the `ITEMS` table an item's slot is unknowable. `false` keeps
        // the field at its `Combatant::new_melee` default (`field_159_null =
        // true`), which is every closed capture's PC value — a documented,
        // conservative fallback rather than a guess in the other direction.
        return false;
    };
    readied_in_slot(items, &kit_items(c), slot::ARMOR)
}

/// The exact inverse of [`crate::party::character_from_record`]: every field the
/// party model carries, put back where the `0x1A6` decode found it.
///
/// D-SAVE11 makes this total by construction — the party model holds *every*
/// datum the record stores, opaque `field_XX` cells included — so nothing here
/// invents a value. The one thing it cannot rebuild is the record's pointer
/// arrays, which `decode_char_record` never produced either (§1.7 item 3).
pub fn record_from_character(c: &Character) -> CharRecord {
    let pair = |p: crate::party::AbilityScorePair| RawStat {
        current: p.current,
        original: p.original,
    };
    CharRecord {
        name: c.name.clone(),
        stats: RawStatBlock {
            str: pair(c.stats.str_score),
            int: pair(c.stats.int),
            wis: pair(c.stats.wis),
            dex: pair(c.stats.dex),
            con: pair(c.stats.con),
            cha: pair(c.stats.cha),
            str_exceptional: pair(c.stats.str_exceptional),
        },
        spell_list: c.magic.spell_list.clone(),
        spell_to_learn_count: c.magic.spell_to_learn_count,
        thac0_base: c.combat.thac0_base,
        race: c.race,
        class: c.class_id,
        age: c.age,
        hit_point_max: c.hit_point_max,
        spell_book: c.magic.spell_book.clone(),
        attack_level: c.combat.attack_level,
        field_de: c.opaque.field_de,
        save_verse: c.skills.save_verse,
        base_movement: c.combat.base_movement,
        hit_dice: c.hit_dice,
        multiclass_level: c.multiclass_level,
        lost_lvls: c.lost_levels,
        lost_hp: c.lost_hp,
        field_e9: c.skills.turn_undead_type,
        thief_skills: c.skills.thief_skills,
        field_f6: c.opaque.field_f6,
        control_morale: c.control_morale,
        npc_treasure_share_count: c.status.npc_treasure_share_count,
        field_f9_fa: c.opaque.field_f9_fa,
        money: [
            c.money.copper,
            c.money.silver,
            c.money.electrum,
            c.money.gold,
            c.money.platinum,
            c.money.gems,
            c.money.jewelry,
        ],
        class_level: c.class_level,
        class_levels_old: c.class_levels_old,
        sex: c.sex,
        monster_type: c.monster_type,
        alignment: c.alignment,
        attack_profile_base: c.combat.attacks.base,
        base_ac: c.combat.base_ac,
        field_125: c.opaque.field_125,
        mod_id: c.monster_index,
        exp: c.exp,
        class_flags: c.skills.class_flags,
        hit_point_rolled: c.hit_point_rolled,
        spell_cast_count: c.magic.cast_count,
        field_13c: c.opaque.field_13c,
        field_13e_140: c.opaque.field_13e_140,
        head_icon: c.icon.head_icon,
        weapon_icon: c.icon.weapon_icon,
        icon_id: c.icon.icon_id,
        icon_size: c.icon.icon_size,
        icon_colours: c.icon.colours,
        field_14b: c.opaque.field_14b,
        weapons_hands_used: c.combat.weapons_hands_used,
        field_186: c.status.save_bonus,
        weight: c.combat.weight,
        paladin_cures_left: c.status.paladin_cures_left,
        field_192_194: c.opaque.field_192_194,
        health_status: c.status.health_status,
        in_combat: c.status.in_combat,
        combat_team: c.status.combat_team,
        quick_fight: c.status.quick_fight,
        hit_bonus: c.combat.thac0_current,
        ac: c.combat.ac,
        ac_behind: c.combat.ac_behind,
        attack_profile_current: c.combat.attacks.current,
        hit_point_current: c.hit_point_current,
        movement: c.combat.movement,
    }
}

/// One party member's whole combat entry: the fighter and the kit it fights
/// with.
pub struct PartyKit {
    pub combatant: Combatant,
    pub loadout: Option<Loadout>,
    /// The member's index in [`crate::party::Party::members`] — the scene needs
    /// it to pair the fighter with its `CHEAD`/`CBODY` icon data.
    pub member_index: usize,
}

/// Build the party half of a live fight's roster (`TeamList`'s leading run).
///
/// `positions[i]` is where `PlaceCombatants` put member `i`. Only living members
/// enter the fight (`hit_point_current > 0`) — the same filter
/// `party_combat_stats` applied — and the caller must have placed exactly that
/// many.
pub fn party_kits(
    members: &[Character],
    positions: &[GridPos],
    items: Option<&ItemDataTable>,
    flavor: &dyn Flavor,
) -> Vec<PartyKit> {
    members
        .iter()
        .enumerate()
        .filter(|(_, c)| c.hit_point_current > 0)
        .zip(positions)
        .enumerate()
        .map(|(id, ((member_index, c), &pos))| {
            let record = record_from_character(c);
            let mut combatant = combatant_from_record(
                id,
                Team::Party,
                pos,
                &record,
                armor_readied(c, items),
                flavor,
            );
            // §9.1's Use word: `player.items.Count > 0` (`ovr009.cs:322`). The
            // record image cannot carry the inventory (items are a heap list),
            // so the live party is the only place this predicate exists.
            combatant.has_items = !c.items.is_empty();
            PartyKit {
                combatant,
                loadout: items.and_then(|t| loadout_for(c, t, flavor)),
                member_index,
            }
        })
        .collect()
}

/// How many members of `party` would enter a fight — the count the caller must
/// place before calling [`party_kits`].
pub fn living_count(members: &[Character]) -> usize {
    members.iter().filter(|c| c.hit_point_current > 0).count()
}

#[cfg(test)]
mod tests;
