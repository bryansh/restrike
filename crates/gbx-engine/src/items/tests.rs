//! Tests for the inventory model (roll-credits §12, G6).
//!
//! Synthetic records throughout (D10) — the `ITEMS` table is built here from
//! hand-written 16-byte rows, so nothing needs the user's game data. The
//! real-data checks live in `crate::demo`'s slice-8 acceptance drive.

use super::*;
use crate::party::Character;
use crate::test_support::blank_character;
use gbx_formats::save_orig::ITEM_RECORD_SIZE;
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::pack::RuleSet;

/// One 16-byte `ITEMS` row.
#[derive(Clone, Copy, Default)]
struct Row {
    slot: u8,
    hands: u8,
    dice_count: u8,
    dice_size: u8,
    bonus_normal: i8,
    field_6: u8,
    class_flags: u8,
    flags: u8,
}

/// Build an `ITEMS` image from `(type, row)` pairs.
fn items_table(rows: &[(u8, Row)]) -> ItemDataTable {
    let mut bytes = vec![0u8; 2 + 0x81 * 0x10];
    for &(t, r) in rows {
        let off = 2 + t as usize * 0x10;
        bytes[off] = r.slot;
        bytes[off + 1] = r.hands;
        bytes[off + 6] = r.field_6;
        bytes[off + 9] = r.dice_count;
        bytes[off + 0xA] = r.dice_size;
        bytes[off + 0xB] = r.bonus_normal as u8;
        bytes[off + 0xD] = r.class_flags;
        bytes[off + 0xE] = r.flags;
    }
    ItemDataTable::parse(&bytes).expect("synthetic ITEMS parses")
}

/// A record with the fields the model reads.
#[derive(Clone, Copy, Default)]
struct Item {
    item_type: u8,
    nn: [u8; 3],
    plus: i8,
    plus_save: u8,
    readied: bool,
    hidden: u8,
    cursed: bool,
    weight: i16,
    count: u8,
    affects: [u8; 3],
}

fn record(it: Item) -> Vec<u8> {
    let mut r = vec![0u8; ITEM_RECORD_SIZE];
    r[0x2E] = it.item_type;
    r[0x2F] = it.nn[0];
    r[0x30] = it.nn[1];
    r[0x31] = it.nn[2];
    r[0x32] = it.plus as u8;
    r[0x33] = it.plus_save;
    r[0x34] = u8::from(it.readied);
    r[0x35] = it.hidden;
    r[0x36] = u8::from(it.cursed);
    r[0x37..0x39].copy_from_slice(&it.weight.to_le_bytes());
    r[0x39] = it.count;
    r[0x3C] = it.affects[0];
    r[0x3D] = it.affects[1];
    r[0x3E] = it.affects[2];
    r
}

fn rules() -> RuleSet {
    RuleSet::load()
}

// --- names -----------------------------------------------------------------

/// ★ The word table's own anchors — the entries the shipped `ITEM{area}.DAX`
/// records index, which is what makes a treasure item readable at all.
#[test]
fn the_word_table_has_the_indices_the_shipped_records_use() {
    assert_eq!(ITEM_NAMES.len(), 256);
    assert_eq!(ITEM_NAMES[0], "");
    assert_eq!(ITEM_NAMES[1], "Battle Axe");
    assert_eq!(ITEM_NAMES[36], "Long Sword");
    assert_eq!(ITEM_NAMES[48], "Mail");
    assert_eq!(ITEM_NAMES[61], "Arrow");
    // The three empty slots the binary's own heap carries (62, 63, 144).
    assert_eq!(
        (ITEM_NAMES[62], ITEM_NAMES[63], ITEM_NAMES[144]),
        ("", "", "")
    );
    assert_eq!(ITEM_NAMES[64], "Potion");
    assert_eq!(ITEM_NAMES[159], "Staff");
    assert_eq!(ITEM_NAMES[162], "+1");
    assert_eq!(ITEM_NAMES[185], "Healing");
    assert_eq!(ITEM_NAMES[187], "Extra");
    assert_eq!(ITEM_NAMES[209], "MU Scroll");
    assert_eq!(ITEM_NAMES[212], "With 3 Spells");
}

/// `GenerateName` emits words 3 → 2 → 1, and the hide bits run the other way
/// (`namenum1` hides on `0x4`).
#[test]
fn generate_name_joins_three_words_back_to_front() {
    // ITEM2.DAX#3's `+3 Long Sword`-shaped record: nn = (0, 164, 36).
    let r = record(Item {
        item_type: 36,
        nn: [0, 164, 36],
        ..Default::default()
    });
    assert_eq!(generate_name(&r, 0), "Long Sword +3");
    // `hidden_names_flag = 6` hides namenum1 and namenum2 → the bare type word.
    assert_eq!(generate_name(&r, 6), "Long Sword");
    // Studded Leather Armor: nn = (49, 50, 52) — all three words.
    let armor = record(Item {
        item_type: 52,
        nn: [49, 50, 52],
        ..Default::default()
    });
    assert_eq!(generate_name(&armor, 0), "Studded Leather Armor");
}

/// The count pluralizer: the shipped `10 Arrow` stack reads "Arrows".
#[test]
fn generate_name_pluralizes_a_stack_like_the_original() {
    let arrows = record(Item {
        item_type: TYPE_ARROW,
        nn: [0, 0, 61],
        count: 10,
        ..Default::default()
    });
    assert_eq!(generate_name(&arrows, 0), "Arrows");
    // A single arrow is not pluralized (`count < 2`).
    let one = record(Item {
        item_type: TYPE_ARROW,
        nn: [0, 0, 61],
        count: 1,
        ..Default::default()
    });
    assert_eq!(generate_name(&one, 0), "Arrow");
    // `2 Javelin` — a plain one-word stack takes the `1 << (v-1) == flags` arm.
    let javelins = record(Item {
        item_type: 21,
        nn: [0, 0, 21],
        count: 2,
        ..Default::default()
    });
    assert_eq!(generate_name(&javelins, 0), "Javelins");
}

/// `ItemDisplayNameBuild`'s three prefixes (`ovr025.cs:170-215`).
#[test]
fn display_name_carries_the_readied_column_star_and_count() {
    let mut arrows = record(Item {
        item_type: TYPE_ARROW,
        nn: [0, 0, 61],
        count: 10,
        plus: 1,
        ..Default::default()
    });
    assert_eq!(display_name(&arrows, false, true), " No   10 Arrows");
    gbx_formats::save_orig::set_item_readied(&mut arrows, true);
    assert_eq!(display_name(&arrows, false, true), " Yes  10 Arrows");
    // Detect Magic marks the `plus`.
    assert_eq!(display_name(&arrows, true, true), " Yes  * 10 Arrows");
    // Without the readied column (the drop/trade prompts).
    assert_eq!(display_name(&arrows, false, false), "10 Arrows");
}

// --- reclac ----------------------------------------------------------------

/// A fighter with a base AC of 10 (`0x3C - 10`), THAC0 base 41, DEX 12 (no AC
/// bonus), no strength bonuses.
fn plain_fighter() -> Character {
    let mut ch = blank_character();
    ch.race = 7; // human
    ch.class_level[SKILL_FIGHTER] = 3;
    ch.stats.dex.original = 12;
    ch.stats.dex.current = 12;
    ch.stats.str_score.original = 12;
    ch.stats.str_score.current = 12;
    ch.combat.base_ac = 0x3C - 10;
    ch.combat.ac = 0x3C - 10;
    ch.combat.thac0_base = 41;
    ch.combat.thac0_current = 41;
    ch.combat.base_movement = 12;
    ch.combat.movement = 12;
    ch
}

/// ★ The acceptance shape: readying armour and a weapon moves the *displayed*
/// numbers, because `reclac_player_values` runs after every items-menu action.
#[test]
fn readying_armour_and_a_sword_moves_ac_thac0_and_damage() {
    let table = items_table(&[
        // Long Sword: melee, 1d8, one hand.
        (
            36,
            Row {
                slot: 0,
                hands: 1,
                dice_count: 1,
                dice_size: 8,
                flags: gbx_formats::items::flags::MELEE,
                class_flags: 0xFF,
                ..Default::default()
            },
        ),
        // Chain Mail: armour slot, `field_6 = 0x80 | 6` (an AC-6 base).
        (
            55,
            Row {
                slot: SLOT_ARMOR,
                field_6: 0x80 | 6,
                class_flags: 0xFF,
                ..Default::default()
            },
        ),
    ]);
    let r = rules();
    let flavor = Adnd1::new(&r);
    let mut ch = plain_fighter();
    ch.items.push(record(Item {
        item_type: 36,
        nn: [0, 0, 36],
        weight: 60,
        ..Default::default()
    }));
    ch.items.push(record(Item {
        item_type: 55,
        nn: [0, 48, 55],
        weight: 300,
        plus: 1,
        ..Default::default()
    }));

    reclac_player_values(&mut ch, &table, &flavor);
    let bare_ac = ch.combat.ac;
    assert_eq!(ch.combat.weight, 360, "weight counts unreadied items too");
    assert_eq!(ch.combat.weapons_hands_used, 0);

    // Ready the mail: `sub_662A6`'s fourth arm gives `bonus[4] = plus + 6 = 7`,
    // which beats the base AC's own 0 contribution... except `stat_bonus[4]`
    // is floored at `base_ac` (50), so the mail alone cannot lower it.
    ready_item(&mut ch, 1, &table, &flavor).expect("armour readies");
    assert!(ch.combat.ac >= bare_ac);
    assert_eq!(ch.combat.movement, 12, "300 → band 9, then +3");

    // Ready the sword: hands used and the weapon's own dice/THAC0 land.
    ready_item(&mut ch, 0, &table, &flavor).expect("sword readies");
    assert_eq!(ch.combat.weapons_hands_used, 1);
    assert_eq!(ch.combat.attacks.current[2], 1, "1d8");
    assert_eq!(ch.combat.attacks.current[4], 8);
    assert_eq!(ch.combat.thac0_current, 41, "no plus, no strength bonus");

    // A +2 sword adds its plus to the hit bonus and the damage bonus.
    gbx_formats::save_orig::set_item_readied(&mut ch.items[0], false);
    ch.items[0][0x32] = 2;
    ready_item(&mut ch, 0, &table, &flavor).expect("+2 sword readies");
    assert_eq!(ch.combat.thac0_current, 43);
    assert_eq!(ch.combat.attacks.current[6], 2);

    // Unready it and the numbers go back.
    ready_item(&mut ch, 0, &table, &flavor).expect("unreadies");
    assert_eq!(ch.combat.thac0_current, 41);
    assert_eq!(ch.combat.weapons_hands_used, 0);
}

/// `CalcArmorWeightEffect`'s three weight bands (`ovr025.cs:67-89`).
#[test]
fn armour_weight_sets_the_movement_band() {
    let table = items_table(&[(
        55,
        Row {
            slot: SLOT_ARMOR,
            class_flags: 0xFF,
            ..Default::default()
        },
    )]);
    let r = rules();
    let flavor = Adnd1::new(&r);
    for (weight, expect) in [(100i16, 12u8), (300, 12), (450, 9)] {
        let mut ch = plain_fighter();
        ch.items.push(record(Item {
            item_type: 55,
            nn: [0, 48, 55],
            weight,
            readied: true,
            ..Default::default()
        }));
        reclac_player_values(&mut ch, &table, &flavor);
        assert_eq!(ch.combat.movement, expect, "armour weight {weight}");
    }
}

/// `calc_movement` only ever *lowers* the rate, and the ladder is keyed on
/// weight past `max_encumberance`.
#[test]
fn overload_takes_movement_away() {
    let table = items_table(&[]);
    let r = rules();
    let flavor = Adnd1::new(&r);
    let mut ch = plain_fighter(); // STR 12 → max encumbrance 100
    ch.money.gold = 1200; // coins weigh 1 each; 1200 - 100 = 1100 > 0x400
    reclac_player_values(&mut ch, &table, &flavor);
    assert_eq!(ch.combat.movement, 3, "badly overloaded");
}

/// ★ `ready_Item`'s `result = Weld.Ok;` override (`ovr020.cs:860`): the
/// verdict is computed and thrown away, so the wrong-class refusal is dead
/// code in the shipped build. Pinned both ways.
#[test]
fn the_weld_verdict_is_computed_and_then_ignored() {
    let table = items_table(&[(
        36,
        Row {
            slot: 0,
            hands: 1,
            class_flags: 0x01, // only class bit 0 may wield it
            ..Default::default()
        },
    )]);
    let r = rules();
    let flavor = Adnd1::new(&r);
    let mut ch = plain_fighter();
    ch.skills.class_flags = 0x02; // not bit 0
    ch.items.push(record(Item {
        item_type: 36,
        nn: [0, 0, 36],
        ..Default::default()
    }));
    assert_eq!(
        weld_verdict(&ch, 0, &table),
        Some(ReadyRefusal::WrongClass),
        "the original computes the refusal"
    );
    assert_eq!(
        ready_item(&mut ch, 0, &table, &flavor),
        Ok(ReadyOutcome::Readied),
        "…and then readies it anyway"
    );
    assert!(gbx_formats::save_orig::item_readied(&ch.items[0]));
}

/// A cursed readied item cannot come off (`ovr020.cs:796-799`).
#[test]
fn a_cursed_item_refuses_to_be_unreadied() {
    let table = items_table(&[(
        77,
        Row {
            slot: 3,
            class_flags: 0xFF,
            ..Default::default()
        },
    )]);
    let r = rules();
    let flavor = Adnd1::new(&r);
    let mut ch = plain_fighter();
    ch.items.push(record(Item {
        item_type: 77,
        nn: [0, 84, 79],
        cursed: true,
        readied: true,
        ..Default::default()
    }));
    assert_eq!(
        ready_item(&mut ch, 0, &table, &flavor),
        Err(ReadyRefusal::Cursed)
    );
    assert!(gbx_formats::save_orig::item_readied(&ch.items[0]));
}

/// The magic-item rider is *named*, not silently skipped.
#[test]
fn a_magic_item_reports_its_deferred_rider() {
    let table = items_table(&[(
        70,
        Row {
            slot: 10,
            class_flags: 0xFF,
            ..Default::default()
        },
    )]);
    let r = rules();
    let flavor = Adnd1::new(&r);
    let mut ch = plain_fighter();
    // An Ioun Stone: `affect_3 = 0x88` → masked arm 8.
    ch.items.push(record(Item {
        item_type: 70,
        nn: [0, 116, 108],
        affects: [0, 3, 0x88],
        ..Default::default()
    }));
    assert_eq!(
        ready_item(&mut ch, 0, &table, &flavor),
        Ok(ReadyOutcome::RiderDeferred {
            readied: true,
            masked_affect: 8
        })
    );
}

// --- halve / join / drop ---------------------------------------------------

#[test]
fn halve_splits_the_larger_half_onto_the_original_row() {
    let mut ch = plain_fighter();
    ch.items.push(record(Item {
        item_type: TYPE_ARROW,
        nn: [0, 0, 61],
        count: 11,
        readied: true,
        ..Default::default()
    }));
    assert!(halve_items(&mut ch, 0));
    assert_eq!(gbx_formats::save_orig::item_count(&ch.items[0]), 6);
    assert_eq!(gbx_formats::save_orig::item_count(&ch.items[1]), 5);
    assert!(
        !gbx_formats::save_orig::item_readied(&ch.items[1]),
        "the new stack is never readied"
    );
    // A stack of one cannot be halved.
    let mut single = plain_fighter();
    single.items.push(record(Item {
        count: 1,
        ..Default::default()
    }));
    assert!(!halve_items(&mut single, 0));
}

#[test]
fn join_merges_identical_stacks_and_refuses_charged_items() {
    let mut ch = plain_fighter();
    let arrows = Item {
        item_type: TYPE_ARROW,
        nn: [0, 0, 61],
        count: 10,
        weight: 4,
        ..Default::default()
    };
    ch.items.push(record(arrows));
    ch.items.push(record(arrows));
    ch.items.push(record(Item { count: 7, ..arrows }));
    assert_eq!(join_items(&mut ch, 0), 2);
    assert_eq!(ch.items.len(), 1);
    assert_eq!(gbx_formats::save_orig::item_count(&ch.items[0]), 27);

    // `affect_1 >= 2` (a charged item) never joins — the original's own clause.
    let mut charged = plain_fighter();
    let potion = Item {
        item_type: 71,
        nn: [0, 0, 64],
        count: 1,
        affects: [3, 99, 0],
        ..Default::default()
    };
    charged.items.push(record(potion));
    charged.items.push(record(potion));
    assert_eq!(join_items(&mut charged, 0), 0);
    assert_eq!(charged.items.len(), 2);
}

#[test]
fn dispose_check_blocks_a_readied_item_and_flags_a_staged_scroll() {
    let table = items_table(&[
        (
            61,
            Row {
                slot: SLOT_SCROLL_FIRST,
                class_flags: 0xFF,
                ..Default::default()
            },
        ),
        (
            36,
            Row {
                slot: 0,
                class_flags: 0xFF,
                ..Default::default()
            },
        ),
    ]);
    let readied = record(Item {
        item_type: 36,
        readied: true,
        ..Default::default()
    });
    assert_eq!(
        dispose_check(&table, &readied),
        DisposeCheck::MustBeUnreadied
    );
    let plain = record(Item {
        item_type: 36,
        ..Default::default()
    });
    assert_eq!(dispose_check(&table, &plain), DisposeCheck::Allowed);
    let staged = record(Item {
        item_type: 61,
        affects: [0x80 | 15, 0, 0],
        ..Default::default()
    });
    assert_eq!(
        dispose_check(&table, &staged),
        DisposeCheck::ConfirmScribedScroll
    );
}

// --- the bar ---------------------------------------------------------------

/// The conditional-word table, transcribed from `ovr020.cs:449-494`.
#[test]
fn the_verb_bar_carries_the_original_conditions() {
    let mut ch = plain_fighter();
    ch.status.in_combat = true;
    ch.items.push(record(Item::default()));
    let w = items_menu_words(&ch, false, false);
    assert_eq!(w.text(), "Ready Use Trade Drop Halve Join");

    // The area ban takes Use away.
    assert_eq!(
        items_menu_words(&ch, true, false).text(),
        "Ready Trade Drop Halve Join"
    );

    // A full inventory takes Halve away.
    let mut full = ch.clone();
    while full.items.len() < MAX_ITEMS {
        full.items.push(record(Item::default()));
    }
    assert_eq!(
        items_menu_words(&full, false, false).text(),
        "Ready Use Trade Drop Join"
    );

    // An NPC still in the party may not trade their gear away.
    let mut npc = ch.clone();
    npc.control_morale = NPC_BASE;
    assert_eq!(
        items_menu_words(&npc, false, false).text(),
        "Ready Use Drop Halve Join"
    );

    // A shop adds Sell and Id.
    assert_eq!(
        items_menu_words(&ch, false, true).text(),
        "Ready Use Trade Drop Halve Join Sell Id"
    );
}
