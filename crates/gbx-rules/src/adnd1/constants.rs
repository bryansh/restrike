//! Typed accessors over `packs/adnd1/constants.toml` (D-RP9 cluster 6, the
//! final M3 cluster).

use crate::pack::{RuleSet, TableData};

fn rows_table<'a>(rules: &'a RuleSet, id: &str) -> &'a [Vec<i64>] {
    let table = rules
        .table(id)
        .unwrap_or_else(|| panic!("{id} must be embedded (packs/adnd1/constants.toml)"));
    let TableData::Rows { rows, .. } = &table.data else {
        panic!("{id} must be a rows-shape table");
    };
    rows
}

/// `timeScales[unit]` (`ovr021.cs:8-202`).
pub fn time_scale(rules: &RuleSet, unit: usize) -> u16 {
    rows_table(rules, "time_scales")[unit][0] as u16
}

/// `unk_1A1B2[class]` (`ovr018.cs:583`'s `classFlags` accumulation).
pub fn class_item_flag(rules: &RuleSet, class: usize) -> u8 {
    rows_table(rules, "class_item_flags")[class][0] as u8
}

/// `classMasks[class]` (`ovr018.cs:2134-2395`'s training-eligibility bitmask).
pub fn class_training_mask(rules: &RuleSet, class: usize) -> u8 {
    rows_table(rules, "class_training_masks")[class][0] as u8
}

/// The 6 `Affects` values `PaladinCureDisease` checks/removes
/// (`ovr020.cs:1497-1534`).
pub fn paladin_cureable_diseases(rules: &RuleSet) -> Vec<u8> {
    rows_table(rules, "paladin_cureable_diseases")
        .iter()
        .map(|r| r[0] as u8)
        .collect()
}

/// `Money.per_copper[denomination]` (`MoneySet.cs:19-122`) — copper-
/// equivalent value; `denomination`: Copper=0, Silver=1, Electrum=2,
/// Gold=3, Platinum=4.
pub fn coin_conversion_rate(rules: &RuleSet, denomination: usize) -> i32 {
    rows_table(rules, "coin_conversion_rates")[denomination][0] as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_scales_match_ovr021() {
        let rules = RuleSet::load();
        assert_eq!(time_scale(&rules, 0), 10);
        assert_eq!(time_scale(&rules, 6), 256);
    }

    #[test]
    fn class_item_flag_matches_declared_bitmask() {
        let rules = RuleSet::load();
        assert_eq!(class_item_flag(&rules, 0), 0x02); // cleric
        assert_eq!(class_item_flag(&rules, 3), 0x40); // paladin
    }

    #[test]
    fn class_training_mask_matches_declared_bitmask() {
        let rules = RuleSet::load();
        assert_eq!(class_training_mask(&rules, 2), 0x08); // fighter
    }

    #[test]
    fn paladin_cureable_diseases_matches_affect_cs_order() {
        let rules = RuleSet::load();
        assert_eq!(
            paladin_cureable_diseases(&rules),
            vec![0x1f, 0x22, 0x2b, 0x2c, 0x32, 0x39]
        );
    }

    #[test]
    fn coin_conversion_matches_money_set_per_copper() {
        let rules = RuleSet::load();
        assert_eq!(coin_conversion_rate(&rules, 0), 1); // copper
        assert_eq!(coin_conversion_rate(&rules, 3), 200); // gold
        assert_eq!(coin_conversion_rate(&rules, 4), 1000); // platinum
    }
}
