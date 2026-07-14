//! Cross-SOURCE conformance: spell-slot progressions checked against the
//! AD&D 1e PHB tables (as reprinted in SSI's shipped manual) — a leg
//! INDEPENDENT of the coab reading the accessors were implemented from.
//! Motivated by M3 step 3's find: step-2's accessors shipped two bugs that
//! their own coab-derived conformance tests could not catch (tests inherit
//! their reading's errors). The engine truncates to 5 spell levels, so
//! book columns 6+ are dropped (cleric 11's sixth-level slot).

use gbx_rules::adnd1::spell_slots::{cleric_spell_slots, mu_spell_slots};
use gbx_rules::pack::RuleSet;

#[test]
fn cleric_progression_matches_the_published_1e_table() {
    let rules = RuleSet::load();
    for (lvl, expect) in [
        (1, [1, 0, 0, 0, 0]),
        (3, [2, 1, 0, 0, 0]),
        (5, [3, 3, 1, 0, 0]),
        (7, [3, 3, 2, 1, 0]),
        (9, [4, 4, 3, 2, 1]),
        (11, [5, 4, 4, 3, 2]), // book adds a 6th-level slot; engine caps at 5 columns
    ] {
        assert_eq!(
            cleric_spell_slots(&rules, lvl),
            expect,
            "cleric level {lvl}"
        );
    }
}

#[test]
fn magic_user_progression_matches_the_published_1e_table() {
    let rules = RuleSet::load();
    for (lvl, expect) in [
        (1, [1, 0, 0, 0, 0]),
        (3, [2, 1, 0, 0, 0]),
        (5, [4, 2, 1, 0, 0]),
        (7, [4, 3, 2, 1, 0]),
        (9, [4, 3, 3, 2, 1]),
        (11, [4, 4, 4, 3, 3]),
    ] {
        assert_eq!(
            mu_spell_slots(&rules, lvl),
            expect,
            "magic-user level {lvl}"
        );
    }
}
