//! Typed accessors over `packs/adnd1/hp_hd.toml` (D-RP9 cluster 2).

use crate::pack::{RuleSet, TableData};

fn row_scalar(rules: &RuleSet, id: &str, index: usize) -> i64 {
    let table = rules
        .table(id)
        .unwrap_or_else(|| panic!("{id} must be embedded (packs/adnd1/hp_hd.toml)"));
    let TableData::Rows { rows, .. } = &table.data else {
        panic!("{id} must be a rows-shape table");
    };
    rows[index][0]
}

/// `gbl.max_class_hit_dice[class]` (`ClassId` 0..=7) — the class level at
/// which `calc_max_hp`/`get_con_hp_adj`/`sub_509E0` switch from
/// dice-rolling to a flat post-max HP formula.
pub fn max_class_hit_dice(rules: &RuleSet, class: usize) -> u8 {
    row_scalar(rules, "max_class_hit_dice", class) as u8
}

/// `unk_16B2A[class]` (`ovr018.cs:2122`) — dice rolled (best-of-2) at
/// character level 1.
pub fn hit_die_count_at_level_1(rules: &RuleSet, class: usize) -> u8 {
    row_scalar(rules, "hit_die_count_at_level_1", class) as u8
}

/// The dice count `sub_509E0` (`ovr018.cs:2136-2140`) actually rolls for a
/// level-up: [`hit_die_count_at_level_1`] at character level 1, capped to a
/// single die at every level thereafter (`ovr018.cs:2140`'s `if
/// (player.ClassLevel[_class] > 1) { var_5 = 1; }`).
pub fn level_up_dice_count(rules: &RuleSet, class: usize, class_level: u32) -> u8 {
    if class_level > 1 {
        1
    } else {
        hit_die_count_at_level_1(rules, class)
    }
}

/// `unk_16B32[class]` (`ovr018.cs:2123`) — hit die size (d-sides).
pub fn hit_die_size(rules: &RuleSet, class: usize) -> u8 {
    row_scalar(rules, "hit_die_size", class) as u8
}

/// `con_hp_adj[con_score]` (`ovr018.cs:1980-1988`'s `get_con_hp_adj`),
/// honoring the table's `[CON-3]` image indexing (`con_score` is the raw
/// CON stat, 3..=25; the pack itself only stores those 23 entries).
/// Panics outside `3..=25` — `get_con_hp_adj` never calls with a CON below
/// the game's minimum stat value, so an out-of-range score here is an
/// engine bug, not a runtime condition to handle gracefully.
pub fn con_hp_adj(rules: &RuleSet, con_score: u8) -> i8 {
    assert!(
        (3..=25).contains(&con_score),
        "CON score must be 3..=25, got {con_score}"
    );
    row_scalar(rules, "con_hp_adj", (con_score - 3) as usize) as i8
}

/// `hp_calc_table[class]`'s four fields (`ovr018.cs:2068-2085`,
/// `calc_max_hp`'s alternate/display HP formula) — `coab-only` tier, no
/// image anchor found this session (see the pack's `notes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpCalc {
    pub dice: u8,
    pub lvl_bonus: u8,
    pub max_base: u8,
    pub max_mult: u8,
}

pub fn hp_calc(rules: &RuleSet, class: usize) -> HpCalc {
    let table = rules
        .table("hp_calc_table")
        .expect("hp_calc_table must be embedded (packs/adnd1/hp_hd.toml)");
    let TableData::Records { rows, .. } = &table.data else {
        panic!("hp_calc_table must be a records-shape table");
    };
    let row = &rows[class];
    HpCalc {
        dice: row[0] as u8,
        lvl_bonus: row[1] as u8,
        max_base: row[2] as u8,
        max_mult: row[3] as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_class_hit_dice_matches_the_corrected_monk_value() {
        let rules = RuleSet::load();
        assert_eq!(max_class_hit_dice(&rules, 0), 10); // cleric
                                                       // monk (class 7): the image byte is 0x13 (19), not coab's literal
                                                       // "13" -- this session's confirmed transcription-error finding.
        assert_eq!(max_class_hit_dice(&rules, 7), 19);
    }

    /// Conformance test reproducing `sub_509E0`'s level-up dice-count
    /// capping (`ovr018.cs:2136-2140`).
    #[test]
    fn level_up_dice_count_caps_to_one_past_level_1() {
        let rules = RuleSet::load();
        // ranger (class 4): 2 dice at level 1, then 1 at every level after.
        assert_eq!(level_up_dice_count(&rules, 4, 1), 2);
        assert_eq!(level_up_dice_count(&rules, 4, 2), 1);
        assert_eq!(level_up_dice_count(&rules, 4, 9), 1);
    }

    #[test]
    fn hit_die_size_matches_standard_adnd_dice() {
        let rules = RuleSet::load();
        assert_eq!(hit_die_size(&rules, 2), 10); // fighter d10
        assert_eq!(hit_die_size(&rules, 5), 4); // magic_user d4
    }

    /// Conformance test reproducing `get_con_hp_adj`'s indexing
    /// (`ovr018.cs:1982-1992`).
    #[test]
    fn con_hp_adj_honors_the_con_minus_3_offset() {
        let rules = RuleSet::load();
        assert_eq!(con_hp_adj(&rules, 3), -2); // image index 0
        assert_eq!(con_hp_adj(&rules, 6), -1);
        assert_eq!(con_hp_adj(&rules, 15), 1);
        assert_eq!(con_hp_adj(&rules, 25), 2); // image index 22, the last entry
    }

    #[test]
    #[should_panic(expected = "3..=25")]
    fn con_hp_adj_below_minimum_stat_panics() {
        let rules = RuleSet::load();
        con_hp_adj(&rules, 2);
    }

    #[test]
    fn hp_calc_dice_column_cross_checks_against_hit_die_size() {
        let rules = RuleSet::load();
        for class in 0..8 {
            assert_eq!(
                hp_calc(&rules, class).dice,
                hit_die_size(&rules, class),
                "hp_calc_table.dice must agree with the independently image-anchored hit_die_size for class {class}"
            );
        }
    }

    #[test]
    fn hp_calc_ranger_has_the_declared_level_bonus() {
        let rules = RuleSet::load();
        let hp = hp_calc(&rules, 4); // ranger
        assert_eq!(hp.lvl_bonus, 1);
        assert_eq!(hp.max_base, 88);
        assert_eq!(hp.max_mult, 2);
    }
}
