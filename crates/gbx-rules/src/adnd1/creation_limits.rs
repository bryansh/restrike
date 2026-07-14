//! Typed accessors over `packs/adnd1/creation.toml`'s D-RP9 authoring-order
//! item 3 tables (starting age, race classes, race/sex stat limits, class
//! stat minimums, aging) — the `class_alignments` accessor itself lives in
//! [`super::creation`].

use crate::pack::{RuleSet, TableData};

fn rows_table<'a>(rules: &'a RuleSet, id: &str) -> &'a [Vec<i64>] {
    let table = rules
        .table(id)
        .unwrap_or_else(|| panic!("{id} must be embedded (packs/adnd1/creation.toml)"));
    let TableData::Rows { rows, .. } = &table.data else {
        panic!("{id} must be a rows-shape table");
    };
    rows
}

fn records_table<'a>(rules: &'a RuleSet, id: &str) -> &'a [Vec<i64>] {
    let table = rules
        .table(id)
        .unwrap_or_else(|| panic!("{id} must be embedded (packs/adnd1/creation.toml)"));
    let TableData::Records { rows, .. } = &table.data else {
        panic!("{id} must be a records-shape table");
    };
    rows
}

/// One `starting_age` entry — `ovr018.cs:624-650`'s `base + NdS` roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartingAge {
    pub base_age: u16,
    pub dice_count: u8,
    pub dice_size: u8,
}

/// `race_ages[race][class]` (`race: 0..=7`, `Race` enum; `class: 0..=6`,
/// base classes only) — `Classes/Gbl.cs:722-796`.
pub fn starting_age(rules: &RuleSet, race: usize, class: usize) -> StartingAge {
    let rows = records_table(rules, "starting_age");
    let row = &rows[race * 7 + class];
    StartingAge {
        base_age: row[0] as u16,
        dice_count: row[1] as u8,
        dice_size: row[2] as u8,
    }
}

/// `Limits.RaceAgeBrackets[race, bracket]` (`Player.cs:66-74`'s
/// `AgeEffects`) — `None` for `race == 0` (monster): the image has no data
/// for it, and character creation never ages a monster.
pub fn race_age_bracket(rules: &RuleSet, race: usize, bracket: usize) -> Option<u16> {
    if race == 0 {
        return None;
    }
    let rows = rows_table(rules, "race_age_brackets");
    Some(rows[race - 1][bracket] as u16)
}

/// The per-stat delta `AgeEffects` adds once `age` crosses a
/// `race_age_bracket` threshold — reproduces `Player.cs:66-74`'s loop
/// exactly (bracket thresholds are exclusive: `bracket_age < age`).
pub fn total_age_effect(rules: &RuleSet, stat: usize, race: usize, age: u16) -> i32 {
    let mut total = 0i32;
    for bracket in 0..5 {
        let Some(threshold) = race_age_bracket(rules, race, bracket) else {
            return 0;
        };
        if threshold < age {
            total += age_effect_delta(rules, stat, bracket) as i32;
        }
    }
    total
}

/// `Limits.<Stat>AgeEffect[bracket]` (`coab-only`, `Classes/Limits.cs:19-25`).
pub fn age_effect_delta(rules: &RuleSet, stat: usize, bracket: usize) -> i8 {
    rows_table(rules, "age_effect_deltas")[stat][bracket] as i8
}

/// One `class_stats_min` row — `Player.cs:59-64`'s `EnforceClassLimits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassStatsMin {
    pub str: u8,
    pub int: u8,
    pub wis: u8,
    pub dex: u8,
    pub con: u8,
    pub cha: u8,
}

pub fn class_stats_min(rules: &RuleSet, class: usize) -> ClassStatsMin {
    let row = &records_table(rules, "class_stats_min")[class];
    ClassStatsMin {
        str: row[0] as u8,
        int: row[1] as u8,
        wis: row[2] as u8,
        dex: row[3] as u8,
        con: row[4] as u8,
        cha: row[5] as u8,
    }
}

/// The six ability stats `EnforceRaceSexLimits`/`EnforceClassLimits` apply
/// to — `str_percentile` is the 18/xx exceptional-strength field (coab's
/// `Str00`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stat {
    Str,
    StrPercentile,
    Int,
    Wis,
    Dex,
    Con,
    Cha,
}

impl Stat {
    fn table_id(self) -> &'static str {
        match self {
            Stat::Str => "str_race_sex_min_max",
            Stat::StrPercentile => "str_percentile_race_sex_min_max",
            Stat::Int => "int_race_sex_min_max",
            Stat::Wis => "wis_race_sex_min_max",
            Stat::Dex => "dex_race_sex_min_max",
            Stat::Con => "con_race_sex_min_max",
            Stat::Cha => "cha_race_sex_min_max",
        }
    }
}

/// `Limits.<Stat>RaceSexMinMax[race, 0..=1, sex]` (`Player.cs:47-51`'s
/// `EnforceRaceSexLimits`) — `race: 0..=7` (`Race` enum, monster included),
/// `sex: 0=male, 1=female`. `coab-only` tier (see the pack's `notes`).
pub fn race_sex_min_max(rules: &RuleSet, stat: Stat, race: usize, sex: usize) -> (u8, u8) {
    let rows = rows_table(rules, stat.table_id());
    let row = &rows[race];
    (row[sex] as u8, row[2 + sex] as u8)
}

/// `RaceClasses[race]` (`ClassId` values) — `ovr018.cs:452-459`'s class
/// picker. `coab-only` tier; excludes coab's 9th "Cheaters" debug row
/// (`race` is always `0..=7`).
pub fn race_classes(rules: &RuleSet, race: usize) -> Vec<u8> {
    let table = rules
        .table("race_classes")
        .expect("race_classes must be embedded (packs/adnd1/creation.toml)");
    let TableData::Jagged { rows, .. } = &table.data else {
        panic!("race_classes must be a jagged-shape table");
    };
    rows[race].iter().map(|&v| v as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_age_reproduces_human_cleric() {
        let rules = RuleSet::load();
        // human (race 7), cleric (class 0): base 18, 1d4.
        let sa = starting_age(&rules, 7, 0);
        assert_eq!(
            sa,
            StartingAge {
                base_age: 18,
                dice_count: 1,
                dice_size: 4
            }
        );
    }

    #[test]
    fn starting_age_reproduces_elf_cleric_wide_base() {
        let rules = RuleSet::load();
        // elf (race 2), cleric (class 0): base 650 (0x28a), 10d10 -- the
        // design doc's own cited mixed-width proof value.
        let sa = starting_age(&rules, 2, 0);
        assert_eq!(
            sa,
            StartingAge {
                base_age: 650,
                dice_count: 10,
                dice_size: 10
            }
        );
    }

    /// Conformance test reproducing `StatValue.AgeEffects`'s exact loop
    /// (`Player.cs:66-74`) for a human crossing the first two brackets.
    #[test]
    fn total_age_effect_reproduces_age_effects_loop() {
        let rules = RuleSet::load();
        // human (race 7) STR (stat 0) at age 45: brackets are
        // [20,40,60,90,120]; 45 crosses brackets 0 (20<45) and 1 (40<45)
        // but not 2..4. StrAgeEffect = [0,1,-1,-2,-1] -> 0 + 1 = 1.
        assert_eq!(total_age_effect(&rules, 0, 7, 45), 1);
    }

    #[test]
    fn race_age_bracket_is_none_for_monster() {
        let rules = RuleSet::load();
        assert_eq!(race_age_bracket(&rules, 0, 0), None);
    }

    #[test]
    fn class_stats_min_matches_gbl_declaration() {
        let rules = RuleSet::load();
        let paladin = class_stats_min(&rules, 3);
        assert_eq!(
            paladin,
            ClassStatsMin {
                str: 12,
                int: 9,
                wis: 13,
                dex: 0,
                con: 9,
                cha: 17
            }
        );
    }

    #[test]
    fn race_sex_min_max_matches_limits_cs() {
        let rules = RuleSet::load();
        // dwarf (race 1) STR, male (sex 0): {8,8} min, {18,17} max -> (8, 18).
        assert_eq!(race_sex_min_max(&rules, Stat::Str, 1, 0), (8, 18));
        // dwarf STR, female (sex 1): min 8, max 17.
        assert_eq!(race_sex_min_max(&rules, Stat::Str, 1, 1), (8, 17));
    }

    #[test]
    fn race_classes_excludes_the_cheaters_row_and_matches_human() {
        let rules = RuleSet::load();
        assert_eq!(race_classes(&rules, 7), vec![0, 2, 5, 6, 3, 4]); // human
        assert_eq!(race_classes(&rules, 0), Vec::<u8>::new()); // monster
        let table = rules.table("race_classes").unwrap();
        let TableData::Jagged { rows, .. } = &table.data else {
            unreachable!()
        };
        assert_eq!(rows.len(), 8, "the 9th Cheaters row must not be present");
    }
}
