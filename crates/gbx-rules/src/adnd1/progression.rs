//! Typed accessors over `packs/adnd1/progression.toml` (D-RP9 cluster 1).
//! Table plumbing only — roll/combat resolution semantics are M4 scope
//! (D-RP5: "Combat-facing methods get table plumbing now, roll semantics
//! oracle-verified at M4"). Every accessor here does exactly one thing:
//! translate the pack's image-true storage shape into the index/value
//! convention coab's own consumption code actually uses.

use crate::pack::{RuleSet, TableData};

fn rows_table<'a>(rules: &'a RuleSet, id: &str) -> &'a [Vec<i64>] {
    let table = rules
        .table(id)
        .unwrap_or_else(|| panic!("{id} must be embedded (packs/adnd1/progression.toml)"));
    let TableData::Rows { rows, .. } = &table.data else {
        panic!("{id} must be a rows-shape table");
    };
    rows
}

/// Stored THAC0 for `class` (`ClassId` 0..=7) at character `level`
/// (1..=12) — `ovr018.cs:576-585`/`ovr026.cs:188-193`'s
/// `thac0_table[class, level]` read directly (level 0 is a dead slot the
/// image stores but no consumer ever reads; this accessor doesn't expose
/// it).
pub fn thac0_stored(rules: &RuleSet, class: usize, level: usize) -> i8 {
    assert!(
        (1..=12).contains(&level),
        "thac0 level must be 1..=12, got {level}"
    );
    rows_table(rules, "thac0_table")[class][level] as i8
}

/// The displayed THAC0 (`engine/seg043.cs:133`'s `0x3c - p.hitBonus`, with
/// `hitBonus == player.thac0` per `ovr025.cs:18/427`) — lower is better, the
/// opposite sense of the stored value.
pub fn thac0_display(stored: i8) -> i32 {
    0x3C - stored as i32
}

/// Saving throw target number for `class` (0..=7) at character `level`
/// (1..=12) against `save_type` (`Classes/Spells.cs:7`'s `SaveVerseType`:
/// Poison=0, Petrification=1, RodStaffWand=2, BreathWeapon=3, Spell=4) —
/// `ovr026.cs:334-365`'s `SaveThrowValues[class, ClassLevel, save]` read,
/// honoring the image's dropped level-0 column (this table has no level-0
/// entry at all; coab's `{20,20,20,20,20}` sentinel row is never stored).
pub fn save_throw(rules: &RuleSet, class: usize, level: usize, save_type: usize) -> u8 {
    assert!(
        (1..=12).contains(&level),
        "save throw level must be 1..=12, got {level}"
    );
    let rows = rows_table(rules, "save_throw_values");
    let row = &rows[class];
    row[(level - 1) * 5 + save_type] as u8
}

/// `ovr014.cs:604-616`'s `cleric_lvl -> var_B` bracket mapping, reindexed
/// 0-based to match `turn_undead`'s stored column layout (var_B 1..=10 maps
/// to columns 0..=9; var_B=0 never occurs in the original and has no
/// column here).
fn cleric_bracket_column(cleric_lvl: u8) -> usize {
    let var_b = if (1..=8).contains(&cleric_lvl) {
        cleric_lvl
    } else if (9..=13).contains(&cleric_lvl) {
        9
    } else {
        10
    };
    (var_b - 1) as usize
}

/// The raw `turn_undead` table entry for `undead_type` (monster
/// `field_76`/`Player.field_E9`) against a cleric of level `cleric_lvl` —
/// `ovr014.cs:642`'s `unk_16679[type * 10 + var_B]` read. Positive values
/// are a flee threshold, non-positive a destroy threshold (both compared as
/// `roll >= abs(value)` against a 1d20 roll — the roll/effect resolution
/// itself is M4 scope), and `99` is unreachable on a d20 (cannot be
/// turned). Returns `None` for `undead_type >= 11`: the image's real data
/// only covers types 0..=10 (a design-doc-contradicting finding this
/// session made — type 11's would-be row is ASCII menu text, not table
/// data; see the pack's `notes` and the fidelity docket).
pub fn turn_undead_entry(rules: &RuleSet, undead_type: usize, cleric_lvl: u8) -> Option<i8> {
    if undead_type >= 11 {
        return None;
    }
    let rows = rows_table(rules, "turn_undead");
    let col = cleric_bracket_column(cleric_lvl);
    Some(rows[undead_type][col] as i8)
}

/// `ClassId` indices with a real `exp_thresholds_*` table (druid=1 and
/// monk=7 have none — coab declares their whole row `-1`, and no image
/// data exists to anchor, per this cluster's authoring notes).
fn exp_table_id(class: usize) -> Option<&'static str> {
    match class {
        0 => Some("exp_thresholds_cleric"),
        2 => Some("exp_thresholds_fighter"),
        3 => Some("exp_thresholds_paladin"),
        4 => Some("exp_thresholds_ranger"),
        5 => Some("exp_thresholds_magic_user"),
        6 => Some("exp_thresholds_thief"),
        _ => None,
    }
}

/// XP required to advance from character `current_level` to
/// `current_level + 1`, for `class` (`ClassId` 0..=7) — `ovr018.cs:2226-2244`
/// (`train_player`)'s `exp_table[class, class_lvl]` read (this accessor's
/// `current_level` *is* coab's `class_lvl`; the image drops coab's
/// synthesized always-0 leading column, so index 0 here already means
/// "level 1's threshold", one earlier than coab's raw array index).
/// `None` for druid/monk (no data) or a `current_level` beyond the class's
/// stored range (that class caps out below `current_level`, or has no
/// further trainable level via this table) — never a panic, matching
/// coab's own `> 0` guard on every read.
pub fn exp_threshold(rules: &RuleSet, class: usize, current_level: usize) -> Option<i32> {
    let id = exp_table_id(class)?;
    if current_level == 0 {
        return None;
    }
    let rows = rows_table(rules, id);
    let row = rows.get(current_level - 1)?;
    Some(row[0] as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thac0_display_matches_the_seg043_formula() {
        // cleric level 1, stored = 40 (0x28); display = 0x3C - 0x28 = 0x14 = 20.
        let rules = RuleSet::load();
        let stored = thac0_stored(&rules, 0, 1);
        assert_eq!(stored, 40);
        assert_eq!(thac0_display(stored), 20);
    }

    /// Conformance test reproducing `ReclacClassBonuses`'s max() loop
    /// (`ovr026.cs:188-193`) for a level-3/level-2 multiclass fighter/cleric.
    #[test]
    fn thac0_max_over_classes_reproduces_reclac_class_bonuses() {
        let rules = RuleSet::load();
        let class_levels: [(usize, usize); 2] = [(2, 3), (0, 2)]; // (class, level)
        let mut thac0 = i8::MIN;
        for (class, level) in class_levels {
            thac0 = thac0.max(thac0_stored(&rules, class, level));
        }
        // fighter level 3 = 42 (row2[3]); cleric level 2 = 40 (row0[2]); max = 42.
        assert_eq!(thac0, 42);
    }

    #[test]
    fn save_throw_reproduces_reclac_saving_throws_indexing() {
        let rules = RuleSet::load();
        // cleric (class 0), level 1, Poison (save_type 0) = 10 per coab's
        // declared level-1 row {10,13,14,16,15}.
        assert_eq!(save_throw(&rules, 0, 1, 0), 10);
        // level 12, Spell (save_type 4) = 11 (cleric's last declared row).
        assert_eq!(save_throw(&rules, 0, 12, 4), 11);
    }

    #[test]
    #[should_panic(expected = "1..=12")]
    fn save_throw_level_zero_panics_not_silently_wraps() {
        let rules = RuleSet::load();
        save_throw(&rules, 0, 0, 0);
    }

    #[test]
    fn cleric_bracket_column_matches_ovr014_brackets() {
        assert_eq!(cleric_bracket_column(1), 0);
        assert_eq!(cleric_bracket_column(8), 7);
        assert_eq!(cleric_bracket_column(9), 8);
        assert_eq!(cleric_bracket_column(13), 8);
        assert_eq!(cleric_bracket_column(14), 9);
        assert_eq!(cleric_bracket_column(255), 9);
    }

    #[test]
    fn turn_undead_covers_types_0_through_10_and_stops_at_11() {
        let rules = RuleSet::load();
        // type 0, cleric level 1 (bracket column 0) = 17 per the transcribed row.
        assert_eq!(turn_undead_entry(&rules, 0, 1), Some(17));
        // type 10 (the image's last real row), cleric level >=14 (column 9) = 4.
        assert_eq!(turn_undead_entry(&rules, 10, 14), Some(4));
        // type 11+ is text data in the image, not a table row -- must be None,
        // never a misread byte.
        assert_eq!(turn_undead_entry(&rules, 11, 1), None);
        assert_eq!(turn_undead_entry(&rules, 255, 1), None);
    }

    #[test]
    fn exp_threshold_is_none_for_druid_and_monk() {
        let rules = RuleSet::load();
        assert_eq!(exp_threshold(&rules, 1, 1), None); // druid
        assert_eq!(exp_threshold(&rules, 7, 1), None); // monk
    }

    #[test]
    fn exp_threshold_reproduces_train_players_cleric_progression() {
        let rules = RuleSet::load();
        assert_eq!(exp_threshold(&rules, 0, 1), Some(1501));
        assert_eq!(exp_threshold(&rules, 0, 9), Some(450001));
        // level 10 is beyond cleric's 9 stored thresholds -- None, not a panic.
        assert_eq!(exp_threshold(&rules, 0, 10), None);
    }

    #[test]
    fn exp_threshold_level_zero_is_none() {
        let rules = RuleSet::load();
        assert_eq!(exp_threshold(&rules, 0, 0), None);
    }
}
