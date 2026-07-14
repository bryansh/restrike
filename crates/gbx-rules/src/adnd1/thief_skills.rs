//! Typed accessors over `packs/adnd1/thief_skills.toml` (D-RP9 cluster 4).
//! Table plumbing only — `reclac_thief_skills`'s full computation (item
//! bonuses, the `var_A`/`var_B` scroll-learning level overrides) is a
//! Group-B trait method for session 3, not this module's job.

use crate::pack::{RuleSet, TableData};

fn rows_table<'a>(rules: &'a RuleSet, id: &str) -> &'a [Vec<i64>] {
    let table = rules
        .table(id)
        .unwrap_or_else(|| panic!("{id} must be embedded (packs/adnd1/thief_skills.toml)"));
    let TableData::Rows { rows, .. } = &table.data else {
        panic!("{id} must be a rows-shape table");
    };
    rows
}

/// `base_chance[thief_level, skill]` (`ovr026.cs:530-536`) — `thief_level`
/// 1..=12, `skill` 1..=8 (1-based, matching the consuming loop). Rows for
/// thief levels 6..=11 carry an unresolved image anomaly relative to a
/// naive reading of coab's declaration (see the pack's `notes`) — the
/// values returned here are the verified image bytes regardless.
pub fn base_chance(rules: &RuleSet, thief_level: usize, skill: usize) -> u8 {
    assert!(
        (1..=12).contains(&thief_level),
        "thief_level must be 1..=12, got {thief_level}"
    );
    assert!((1..=8).contains(&skill), "skill must be 1..=8, got {skill}");
    rows_table(rules, "thief_skill_base_chance")[thief_level - 1][skill - 1] as u8
}

/// `unk_1A243[dex, skill]` (`ovr026.cs:535-536`) — only ever consulted for
/// `skill` 1..=5 (`if (skill < 6)`, `ovr026.cs:535`). `dex` is the raw DEX
/// score, `0..=21` (the image's full confirmed extent — see the pack's
/// `notes` for the unresolved column-mapping caveat).
pub fn dex_adj(rules: &RuleSet, dex: usize, skill: usize) -> i8 {
    assert!((1..=5).contains(&skill), "skill must be 1..=5, got {skill}");
    rows_table(rules, "thief_skill_dex_adj")[dex][skill - 1] as i8
}

/// `unk_1A230[race, skill]` (`ovr026.cs:426-439`/`532-539`) — `coab-only`
/// tier (see the pack's `notes`: the table's declared 13x9 shape is
/// actively disproven against the image, and only rows 0..=7 (`Race` enum,
/// `Classes/Enums.cs:45`) have any established meaning). `skill` is
/// 1-based, 1..=8, matching `reclac_thief_skills`'s indexing (column 0 is
/// an unread dead slot, stored as declared per D-RP2's `coab-only`
/// carve-out).
pub fn race_adj(rules: &RuleSet, race: usize, skill: usize) -> i8 {
    assert!((1..=8).contains(&skill), "skill must be 1..=8, got {skill}");
    assert!(race < 8, "race must be 0..=7, got {race}");
    rows_table(rules, "thief_skill_race_adj")[race][skill] as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_chance_matches_coab_for_the_clean_rows() {
        let rules = RuleSet::load();
        // thief level 1, skill 1: coab's base_chance row1 (dropping the
        // dead row0/col0) is {30,25,20,15,10,10,85,0}.
        assert_eq!(base_chance(&rules, 1, 1), 30);
        assert_eq!(base_chance(&rules, 1, 7), 85);
        // level 4 (still a "clean" row per the pack's notes): {45,37,35,33,25,15,88,20}.
        assert_eq!(base_chance(&rules, 4, 4), 33);
    }

    #[test]
    #[should_panic(expected = "1..=12")]
    fn base_chance_level_zero_panics() {
        let rules = RuleSet::load();
        base_chance(&rules, 0, 1);
    }

    #[test]
    fn dex_adj_matches_the_confirmed_image_rows() {
        let rules = RuleSet::load();
        // dex index 20, skill 5 (row {12,8,8,18,17}, skill5 = last col = 17).
        assert_eq!(dex_adj(&rules, 20, 5), 17);
        // dex index 21 (the distinctive 99-led tail row {99,0,3,18,3}).
        assert_eq!(dex_adj(&rules, 21, 1), 99);
    }

    #[test]
    #[should_panic(expected = "1..=5")]
    fn dex_adj_skill_six_panics_never_reads_the_dropped_column() {
        let rules = RuleSet::load();
        dex_adj(&rules, 0, 6);
    }

    #[test]
    fn race_adj_matches_the_declared_dwarf_row() {
        let rules = RuleSet::load();
        // dwarf (race 1): {0,0,10,15,0,0,0,-10,-5} -- skill index is the
        // raw column, so skill 2 = 10, skill 3 = 15, skill 8 = -5.
        assert_eq!(race_adj(&rules, 1, 2), 10);
        assert_eq!(race_adj(&rules, 1, 3), 15);
        assert_eq!(race_adj(&rules, 1, 8), -5);
    }

    #[test]
    #[should_panic(expected = "1..=8")]
    fn race_adj_skill_zero_panics() {
        let rules = RuleSet::load();
        race_adj(&rules, 0, 0);
    }
}
