//! The party/character model (`docs/design/save-formats.md` D-SAVE11, task
//! deliverable 2): field-complete enough that original-save import (D-SAVE5)
//! is lossless — every datum `CHRDAT`'s 0x1A6 record stores, this model
//! holds, grouped sensibly. Fields with an established engine/rules meaning
//! use `gbx-rules`' flavor-trait value types (via conversion methods, since
//! the *stored* shape needs full current+max fidelity for display/save
//! round-trip, which those single-value types don't carry); every remaining
//! `field_XX` cell is carried opaquely, so completeness is by construction
//! (proven by the D-SAVE10 round-trip test) rather than by enumeration.
//!
//! serde-derived throughout — this *is* the `.rsav` storage mechanism
//! (D-SAVE1). Deterministic collections only (D-SAVE1): `BTreeSet`, never
//! `HashSet`/`HashMap`.

use gbx_rules::flavor::{AbilityStat, ClassLevel, StatBlock};
use std::collections::BTreeSet;

/// One ability score's current and original (unmodified max) values —
/// `gbx-rules::flavor::StatBlock` only carries a single "current" value per
/// stat (what creation/leveling math needs); the party model needs both, so
/// a drained stat can still display "current (max)" (§1.3's `stats2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct AbilityScorePair {
    pub current: u8,
    pub original: u8,
}

/// The seven stored ability-score pairs, in `CHRDAT`'s own order (§1.3
/// offset 0x10): STR, INT, WIS, DEX, CON, CHA, then Str00 (exceptional-
/// strength percentile, §1.7 item 5 — never clamped to 25 here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct AbilityScores {
    pub str_score: AbilityScorePair,
    pub int: AbilityScorePair,
    pub wis: AbilityScorePair,
    pub dex: AbilityScorePair,
    pub con: AbilityScorePair,
    pub cha: AbilityScorePair,
    /// The 1..=100 percentile fine-grained strength value, meaningful when
    /// `str_score.current == 18` (`adnd1` reads it there) — see
    /// `gbx_rules::flavor::StatBlock`'s own doc comment.
    pub str_exceptional: AbilityScorePair,
}

impl AbilityScores {
    /// Projects the *current* values into `gbx-rules`' flavor-trait input
    /// type — the conversion D-SAVE11 asks for ("using gbx-rules' value
    /// types where they exist"), used whenever a level-up/training flow
    /// needs to call into `Flavor` with this character's live stats.
    pub fn to_stat_block(self) -> StatBlock {
        StatBlock {
            str: self.str_score.current,
            str_exceptional: self.str_exceptional.current,
            int: self.int.current,
            wis: self.wis.current,
            dex: self.dex.current,
            con: self.con.current,
            cha: self.cha.current,
        }
    }

    /// Every stat by [`AbilityStat`] tag, current+original — a convenience
    /// iterator for validation (D-SAVE10 tier 2's per-stat bounds check).
    pub fn pairs(&self) -> [(AbilityStat, AbilityScorePair); 6] {
        [
            (AbilityStat::Str, self.str_score),
            (AbilityStat::Int, self.int),
            (AbilityStat::Wis, self.wis),
            (AbilityStat::Dex, self.dex),
            (AbilityStat::Con, self.con),
            (AbilityStat::Cha, self.cha),
        ]
    }
}

/// Icon presentation data (§1.3 0x141-0x14b): party-order display
/// (`icon_id`), portrait/sprite selection, and the nibble-packed color
/// pairs (unpacked by a future consumer, not here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct IconInfo {
    pub head_icon: u8,
    pub weapon_icon: u8,
    /// Party display order (§1.3: "order number").
    pub icon_id: u8,
    /// `1` = small, `2` = normal.
    pub icon_size: u8,
    pub colours: [u8; 6],
}

/// The two stored attack profiles (§1.3 0x11c/0x19c) — base (creation-time)
/// and current (post-modifiers). No established per-byte sub-layout beyond
/// "dice count/size/bonus, base + current" (§1.3's own notes), so each is
/// carried as an 8-byte opaque block rather than guessed apart; a future
/// combat-math session decomposes these once the exact original layout is
/// confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct AttackProfiles {
    pub base: [u8; 8],
    pub current: [u8; 8],
}

/// Combat statistics (§1.3's "combat" grouping): base/current THAC0 and AC,
/// attack counts, movement/initiative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct CombatStats {
    pub thac0_base: i8,
    /// Current THAC0 (`hitBonus`, §1.3 0x199).
    pub thac0_current: u8,
    /// Display AC = `0x3C - ac` (§1.3 0x19a's note).
    pub base_ac: i8,
    pub ac: i8,
    pub ac_behind: i8,
    pub attacks: AttackProfiles,
    pub attack_level: u8,
    pub base_movement: u8,
    /// Current movement/initiative (§1.3 0x1a5).
    pub movement: u8,
    pub weapons_hands_used: u8,
    pub weight: i16,
}

/// Magic state: known spells, memorized-spell list, and memorized-cast
/// counts (§1.7 item 2's pinned stride — already decoded by
/// `gbx_formats::save_orig::decode_char_record` into the clean `[[u8;5];3]`
/// shape by the time it reaches here). Slot→spell interpretation is a
/// rules concern (§5 item 7 — the doc's own docket), so these stay raw.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct MagicState {
    /// `spellBook[100]` (§1.3 0x79): per-spell known flags.
    pub spell_book: Vec<u8>,
    /// `spellList` (§1.3 0x1e, 84 bytes): the per-slot memorized-spell list.
    pub spell_list: Vec<u8>,
    /// `spellCastCount[3,5]` (§1.3 0x12d): cleric/druid/mage memorized-cast
    /// counts per spell level 1-5.
    pub cast_count: [[u8; 5]; 3],
    pub spell_to_learn_count: u8,
}

/// Skills, saves, and turn-undead (§1.3's "skills/saves" grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct SkillsAndSaves {
    /// pick/locks/traps/silent/hide/hear/climb/read.
    pub thief_skills: [u8; 8],
    /// paralyze/petrify/rod/breath/spell.
    pub save_verse: [u8; 5],
    /// Item limits (`classFlags`, §1.3 0x12b).
    pub class_flags: u8,
    /// Turn-undead type index (`field_E9`, §1.3 0xe9).
    pub turn_undead_type: u8,
}

/// The 7-coin `MoneySet` (§1.3 0xfb): copper/silver/electrum/gold/plat/gems/
/// jewelry, in that order (`Classes/MoneySet.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Money {
    pub copper: i16,
    pub silver: i16,
    pub electrum: i16,
    pub gold: i16,
    pub platinum: i16,
    pub gems: i16,
    pub jewelry: i16,
}

impl Money {
    /// One denomination by coab coin-type index (`MoneySet.cs`: Copper=0,
    /// Silver=1, Electrum=2, Gold=3, Platinum=4, Gems=5, Jewelry=6). Any other
    /// index reads 0.
    pub fn get_coin(&self, index: usize) -> i16 {
        match index {
            0 => self.copper,
            1 => self.silver,
            2 => self.electrum,
            3 => self.gold,
            4 => self.platinum,
            5 => self.gems,
            6 => self.jewelry,
            _ => 0,
        }
    }

    /// Sets one denomination by coin-type index (see [`Self::get_coin`]). An
    /// out-of-range index is ignored.
    pub fn set_coin(&mut self, index: usize, value: i16) {
        match index {
            0 => self.copper = value,
            1 => self.silver = value,
            2 => self.electrum = value,
            3 => self.gold = value,
            4 => self.platinum = value,
            5 => self.gems = value,
            6 => self.jewelry = value,
            _ => {}
        }
    }
}

impl From<[i16; 7]> for Money {
    fn from(m: [i16; 7]) -> Self {
        Money {
            copper: m[0],
            silver: m[1],
            electrum: m[2],
            gold: m[3],
            platinum: m[4],
            gems: m[5],
            jewelry: m[6],
        }
    }
}

/// Status/combat-flow fields (§1.3's "status" grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct StatusFlags {
    /// `0` = okay .. `8` = gone (GBC-doc).
    pub health_status: u8,
    pub in_combat: bool,
    /// `0` = ours, `1` = enemy.
    pub combat_team: u8,
    pub quick_fight: u8,
    pub paladin_cures_left: u8,
    pub npc_treasure_share_count: u8,
    /// Save bonus (`field_186`, signed, §1.3 0x186).
    pub save_bonus: i8,
}

/// Every remaining `CHRDAT` byte with no established name beyond a `field_XX`
/// label — carried opaquely (D-VM5's raw-store discipline applied to the
/// character record, per D-SAVE11) so field-completeness — and the
/// round-trip test that proves it — doesn't depend on enumerating every
/// cell's meaning up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct OpaqueFields {
    pub field_de: u8,
    pub field_f6: u8,
    pub field_f9_fa: [u8; 2],
    pub field_125: u8,
    pub field_13c: i16,
    pub field_13e_140: [u8; 3],
    pub field_14b: u8,
    pub field_192_194: [u8; 3],
}

/// One party member — every datum the 0x1A6 `CHRDAT` record stores
/// (D-SAVE11), grouped for readability. Items/affects are opaque-record
/// lists (§5.5 defers interior decoding); [`Character::readied_items`] is
/// the one reconstructed derivative (§1.7 item 3) — indices into `items`,
/// rebuilt from each item's `readied` flag at import time (the on-disk
/// `activeItems` pointer array is never trusted, per D-SAVE6).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Character {
    pub name: String,
    pub race: u8,
    /// The raw on-disk `ClassId` (§1.3 0x75) — `0..=7` single-class, `8..=16`
    /// coab's multiclass combo ids. Kept verbatim for round-trip fidelity;
    /// use [`Character::class_levels`] for the flavor-trait-friendly
    /// per-base-class breakdown (multiclass combo ids never need to leak
    /// above `adnd1`'s own internals, matching the M3 step-3 flavor-impl
    /// design choice).
    pub class_id: u8,
    pub sex: u8,
    pub alignment: u8,
    /// Drives `AgeEffects` (D-RP5).
    pub age: i16,
    pub monster_type: u8,
    /// Monster table index (`mod_id`, §1.3 0x126).
    pub monster_index: u8,
    pub icon: IconInfo,
    /// `>= 0x80` on the raw `control_morale` byte means NPC (`Control.cs:322`).
    pub control_morale: u8,

    pub stats: AbilityScores,

    pub exp: i32,
    /// Current per-class levels, index = base `ClassId` 0..=7 (§1.3 0x109).
    pub class_level: [u8; 8],
    /// Former (dual-class) levels, same indexing (§1.3 0x111).
    pub class_levels_old: [u8; 8],
    pub hit_dice: u8,
    pub multiclass_level: u8,
    pub lost_levels: u8,
    pub lost_hp: u8,

    pub hit_point_max: u8,
    pub hit_point_current: u8,
    /// The rolled component before the CON adjustment (the level-up flow's
    /// roll-vs-+CON split, D-RP5).
    pub hit_point_rolled: u8,

    pub combat: CombatStats,
    pub magic: MagicState,
    pub skills: SkillsAndSaves,
    pub money: Money,
    pub status: StatusFlags,
    pub opaque: OpaqueFields,

    /// From `.swg` — opaque fixed-size records (§5.5).
    pub items: Vec<Vec<u8>>,
    /// Indices into [`Character::items`] currently readied/equipped —
    /// reconstructed from each item's `readied` flag (§1.7 item 3), not
    /// from the record's dead pointer bytes.
    pub readied_items: BTreeSet<usize>,
    /// From `.fx` — opaque fixed-size records.
    pub affects: Vec<Vec<u8>>,
}

impl Character {
    /// The flavor-trait-friendly per-base-class breakdown: one
    /// [`ClassLevel`] per base class (`0..=7`) with a nonzero current level
    /// — [`Character::class_id`]'s raw multiclass combo id never needs to
    /// leak above this (matches the step-3 `adnd1` flavor impl's own
    /// `&[ClassLevel]` modeling choice).
    pub fn class_levels(&self) -> Vec<ClassLevel> {
        self.class_level
            .iter()
            .enumerate()
            .filter(|&(_, &lvl)| lvl > 0)
            .map(|(class, &lvl)| ClassLevel {
                class,
                level: lvl as u32,
            })
            .collect()
    }

    /// `control_morale >= 0x80` (`Control.cs:322`).
    pub fn is_npc(&self) -> bool {
        self.control_morale >= 0x80
    }
}

/// The full party roster — a `.rsav`'s `PartySnapshot` (D-SAVE3), also the
/// direct result of importing `party_count` `CHRDAT` records (D-SAVE5).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Party {
    pub members: Vec<Character>,
}

/// Builds a [`Character`] from a decoded [`gbx_formats::save_orig::CharRecord`]
/// plus its `.swg` item records — the format-to-model mapping D-SAVE5/D-SAVE11
/// call for. Reconstructs [`Character::readied_items`] from each item's
/// `readied` flag (§1.7 item 3); never reads the record's own pointer bytes
/// (already absent from `CharRecord` by construction, §1.7 item 3 / D-SAVE6).
pub fn character_from_record(
    record: &gbx_formats::save_orig::CharRecord,
    items: Vec<Vec<u8>>,
    affects: Vec<Vec<u8>>,
) -> Character {
    let readied_items = items
        .iter()
        .enumerate()
        .filter(|(_, item)| gbx_formats::save_orig::item_readied(item))
        .map(|(i, _)| i)
        .collect();

    Character {
        name: record.name.clone(),
        race: record.race,
        class_id: record.class,
        sex: record.sex,
        alignment: record.alignment,
        age: record.age,
        monster_type: record.monster_type,
        monster_index: record.mod_id,
        icon: IconInfo {
            head_icon: record.head_icon,
            weapon_icon: record.weapon_icon,
            icon_id: record.icon_id,
            icon_size: record.icon_size,
            colours: record.icon_colours,
        },
        control_morale: record.control_morale,
        stats: AbilityScores {
            str_score: AbilityScorePair {
                current: record.stats.str.current,
                original: record.stats.str.original,
            },
            int: AbilityScorePair {
                current: record.stats.int.current,
                original: record.stats.int.original,
            },
            wis: AbilityScorePair {
                current: record.stats.wis.current,
                original: record.stats.wis.original,
            },
            dex: AbilityScorePair {
                current: record.stats.dex.current,
                original: record.stats.dex.original,
            },
            con: AbilityScorePair {
                current: record.stats.con.current,
                original: record.stats.con.original,
            },
            cha: AbilityScorePair {
                current: record.stats.cha.current,
                original: record.stats.cha.original,
            },
            str_exceptional: AbilityScorePair {
                current: record.stats.str_exceptional.current,
                original: record.stats.str_exceptional.original,
            },
        },
        exp: record.exp,
        class_level: record.class_level,
        class_levels_old: record.class_levels_old,
        hit_dice: record.hit_dice,
        multiclass_level: record.multiclass_level,
        lost_levels: record.lost_lvls,
        lost_hp: record.lost_hp,
        hit_point_max: record.hit_point_max,
        hit_point_current: record.hit_point_current,
        hit_point_rolled: record.hit_point_rolled,
        combat: CombatStats {
            thac0_base: record.thac0_base,
            thac0_current: record.hit_bonus,
            base_ac: record.base_ac,
            ac: record.ac,
            ac_behind: record.ac_behind,
            attacks: AttackProfiles {
                base: record.attack_profile_base,
                current: record.attack_profile_current,
            },
            attack_level: record.attack_level,
            base_movement: record.base_movement,
            movement: record.movement,
            weapons_hands_used: record.weapons_hands_used,
            weight: record.weight,
        },
        magic: MagicState {
            spell_book: record.spell_book.clone(),
            spell_list: record.spell_list.clone(),
            cast_count: record.spell_cast_count,
            spell_to_learn_count: record.spell_to_learn_count,
        },
        skills: SkillsAndSaves {
            thief_skills: record.thief_skills,
            save_verse: record.save_verse,
            class_flags: record.class_flags,
            turn_undead_type: record.field_e9,
        },
        money: Money::from(record.money),
        status: StatusFlags {
            health_status: record.health_status,
            in_combat: record.in_combat,
            combat_team: record.combat_team,
            quick_fight: record.quick_fight,
            paladin_cures_left: record.paladin_cures_left,
            npc_treasure_share_count: record.npc_treasure_share_count,
            save_bonus: record.field_186,
        },
        opaque: OpaqueFields {
            field_de: record.field_de,
            field_f6: record.field_f6,
            field_f9_fa: record.field_f9_fa,
            field_125: record.field_125,
            field_13c: record.field_13c,
            field_13e_140: record.field_13e_140,
            field_14b: record.field_14b,
            field_192_194: record.field_192_194,
        },
        items,
        readied_items,
        affects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::save_orig::{decode_char_record, CHAR_RECORD_SIZE, ITEM_RECORD_SIZE};

    fn synthetic_record() -> gbx_formats::save_orig::CharRecord {
        let mut bytes = vec![0u8; CHAR_RECORD_SIZE];
        bytes[0] = 4;
        bytes[1..5].copy_from_slice(b"Aran");
        bytes[0x10] = 17; // str current
        bytes[0x11] = 18; // str original
        bytes[0x74] = 7; // race
        bytes[0x75] = 10; // multiclass combo id
        bytes[0x109] = 3; // class_level[0] (cleric) = 3
        bytes[0x109 + 3] = 5; // class_level[3] (fighter-ish slot) = 5
        bytes[0x127..0x12b].copy_from_slice(&999i32.to_le_bytes());
        decode_char_record(&bytes).unwrap()
    }

    #[test]
    fn character_from_record_carries_every_named_field() {
        let rec = synthetic_record();
        let ch = character_from_record(&rec, vec![], vec![]);
        assert_eq!(ch.name, "Aran");
        assert_eq!(
            ch.stats.str_score,
            AbilityScorePair {
                current: 17,
                original: 18
            }
        );
        assert_eq!(ch.race, 7);
        assert_eq!(ch.class_id, 10);
        assert_eq!(ch.exp, 999);
    }

    #[test]
    fn class_levels_derives_per_base_class_breakdown() {
        let rec = synthetic_record();
        let ch = character_from_record(&rec, vec![], vec![]);
        let levels = ch.class_levels();
        assert_eq!(
            levels,
            vec![
                ClassLevel { class: 0, level: 3 },
                ClassLevel { class: 3, level: 5 },
            ]
        );
    }

    #[test]
    fn to_stat_block_projects_current_values_only() {
        let rec = synthetic_record();
        let ch = character_from_record(&rec, vec![], vec![]);
        let sb = ch.stats.to_stat_block();
        assert_eq!(sb.str, 17);
    }

    #[test]
    fn readied_items_reconstructed_from_item_flags_not_pointers() {
        let rec = synthetic_record();
        let mut item_a = vec![0u8; ITEM_RECORD_SIZE];
        item_a[0x34] = 1; // readied
        let item_b = vec![0u8; ITEM_RECORD_SIZE]; // not readied
        let mut item_c = vec![0u8; ITEM_RECORD_SIZE];
        item_c[0x34] = 1;
        let ch = character_from_record(&rec, vec![item_a, item_b, item_c], vec![]);
        assert_eq!(ch.readied_items, BTreeSet::from([0, 2]));
    }

    #[test]
    fn is_npc_uses_the_0x80_threshold() {
        let mut rec = synthetic_record();
        rec.control_morale = 0x7F;
        assert!(!character_from_record(&rec, vec![], vec![]).is_npc());
        rec.control_morale = 0x80;
        assert!(character_from_record(&rec, vec![], vec![]).is_npc());
    }
}
