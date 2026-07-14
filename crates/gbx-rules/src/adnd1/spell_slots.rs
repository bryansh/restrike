//! Typed accessors over `packs/adnd1/spell_slots.toml` (D-RP9 cluster 5).
//! Each accessor reproduces `sub_6A00F`'s accumulation loop for its class —
//! table plumbing, not the surrounding trait method (session 3).

use crate::pack::{RuleSet, TableData};

fn rows_table<'a>(rules: &'a RuleSet, id: &str) -> &'a [Vec<i64>] {
    let table = rules
        .table(id)
        .unwrap_or_else(|| panic!("{id} must be embedded (packs/adnd1/spell_slots.toml)"));
    let TableData::Rows { rows, .. } = &table.data else {
        panic!("{id} must be a rows-shape table");
    };
    rows
}

/// Accumulated cleric spell slots at `skill_level` (`ovr026.cs:71-79`'s
/// `for (PlayerLvl = 0; PlayerLvl <= skillLevel - 2; PlayerLvl++)` sum over
/// `ClericSpellLevels`). Returns `[0; 5]` for `skill_level < 2` (the loop
/// never executes), matching coab exactly rather than underflowing.
pub fn cleric_spell_slots(rules: &RuleSet, skill_level: i32) -> [u8; 5] {
    let rows = rows_table(rules, "cleric_spell_levels");
    let mut total = [0u8; 5];
    if skill_level < 2 {
        return total;
    }
    let max_player_lvl = (skill_level - 2) as usize;
    for row in rows.iter().take(max_player_lvl + 1) {
        for (i, v) in row.iter().enumerate() {
            total[i] += *v as u8;
        }
    }
    total
}

/// Accumulated paladin spell slots at `skill_level` (`ovr026.cs:99-108`'s
/// `if (skillLevel > 8) { for (addLvl = 8; addLvl < skillLevel; addLvl++)
/// ... }`). Returns `[0; 5]` at or below level 8 — paladins cast no spells
/// before level 9.
pub fn paladin_spell_slots(rules: &RuleSet, skill_level: i32) -> [u8; 5] {
    let mut total = [0u8; 5];
    if skill_level <= 8 {
        return total;
    }
    let rows = rows_table(rules, "paladin_spell_levels");
    // paladin_spell_levels' row 0 is player level 9 (addLvl 8); addLvl runs
    // 8..skill_level (exclusive), i.e. skill_level - 8 rows.
    let count = (skill_level - 8) as usize;
    for row in rows.iter().take(count) {
        for (i, v) in row.iter().enumerate() {
            total[i] += *v as u8;
        }
    }
    total
}

/// Accumulated ranger spell slots at `skill_level`, split into the
/// druid-side track (spell levels 1-3) and the MU-side track (spell levels
/// 4-5) — `ovr026.cs:126-142`'s `if (skillLevel > 7) { for (var_3 = 8;
/// var_3 <= skillLevel; var_3++) ... }` (note the inclusive `<=`, unlike
/// paladin's exclusive `<`). Returns `([0;3], [0;2])` at or below level 7.
pub fn ranger_spell_slots(rules: &RuleSet, skill_level: i32) -> ([u8; 3], [u8; 2]) {
    let mut druid_track = [0u8; 3];
    let mut mu_track = [0u8; 2];
    if skill_level <= 7 {
        return (druid_track, mu_track);
    }
    let rows = rows_table(rules, "ranger_spell_levels");
    // ranger_spell_levels' row 0 is player level 9 (var_3 8); var_3 runs
    // 8..=skill_level (inclusive), i.e. skill_level - 7 rows.
    let count = (skill_level - 7) as usize;
    for row in rows.iter().take(count) {
        for i in 0..3 {
            druid_track[i] += row[i] as u8;
        }
        for i in 3..5 {
            mu_track[i - 3] += row[i] as u8;
        }
    }
    (druid_track, mu_track)
}

/// Accumulated magic-user spell slots at `skill_level` (`ovr020.cs:672-680`/
/// `ovr026.cs:157-165`'s straight `for (lvl = 0; lvl < skillLevel; lvl++)`
/// sum over `MU_spell_lvl_learn`).
pub fn mu_spell_slots(rules: &RuleSet, skill_level: i32) -> [u8; 5] {
    let rows = rows_table(rules, "mu_spell_lvl_learn");
    let mut total = [0u8; 5];
    if skill_level <= 0 {
        return total;
    }
    for row in rows.iter().take(skill_level as usize) {
        for (i, v) in row.iter().enumerate() {
            total[i] += *v as u8;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleric_slots_below_level_2_are_zero() {
        let rules = RuleSet::load();
        assert_eq!(cleric_spell_slots(&rules, 1), [0; 5]);
    }

    #[test]
    fn cleric_slots_accumulate_the_first_three_player_levels() {
        let rules = RuleSet::load();
        // skill_level 4 -> PlayerLvl 0..=2 -> rows 0,1,2 summed:
        // [1,0,0,0,0]+[0,1,0,0,0]+[1,1,0,0,0] = [2,2,0,0,0].
        assert_eq!(cleric_spell_slots(&rules, 4), [2, 2, 0, 0, 0]);
    }

    #[test]
    fn paladin_slots_are_zero_at_and_below_level_8() {
        let rules = RuleSet::load();
        assert_eq!(paladin_spell_slots(&rules, 8), [0; 5]);
    }

    #[test]
    fn paladin_slots_accumulate_from_level_9() {
        let rules = RuleSet::load();
        // skill_level 9 -> addLvl 8..9 (one iteration) -> row0 = [1,0,0,0,0].
        assert_eq!(paladin_spell_slots(&rules, 9), [1, 0, 0, 0, 0]);
        // skill_level 10 -> addLvl 8..10 (two iterations) -> rows0+1 = [2,0,0,0,0].
        assert_eq!(paladin_spell_slots(&rules, 10), [2, 0, 0, 0, 0]);
    }

    #[test]
    fn ranger_slots_split_druid_and_mu_tracks() {
        let rules = RuleSet::load();
        // skill_level 9 -> var_3 8..=9 (two iterations, INCLUSIVE unlike
        // paladin's exclusive bound) -> rows0+1 = [1,0,0,0,0]+[0,0,0,1,0].
        let (druid, mu) = ranger_spell_slots(&rules, 9);
        assert_eq!(druid, [1, 0, 0]);
        assert_eq!(mu, [1, 0]);
    }

    #[test]
    fn ranger_slots_are_zero_at_and_below_level_7() {
        let rules = RuleSet::load();
        let (druid, mu) = ranger_spell_slots(&rules, 7);
        assert_eq!(druid, [0; 3]);
        assert_eq!(mu, [0; 2]);
    }

    #[test]
    fn mu_slots_accumulate_from_level_1() {
        let rules = RuleSet::load();
        // skill_level 3 -> lvl 0..3 -> rows0,1,2 summed:
        // [1,0,0,0,0]+[0,1,0,0,0]+[1,1,0,0,0] = [2,2,0,0,0].
        assert_eq!(mu_spell_slots(&rules, 3), [2, 2, 0, 0, 0]);
    }
}
