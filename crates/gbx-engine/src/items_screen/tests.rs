//! The Items screen, driven through the real `Engine::tick` loop
//! (roll-credits §12, G6). Everything here is D10 synthetic — a hand-built
//! `ITEMS` table plus hand-built item records — so it all runs in CI.

use crate::combat_wiring::{
    combat_game_data, open_geo, party_member, synthetic_font, synthetic_set4,
};
use crate::engine::Engine;
use crate::input::InputEvent;
use crate::items;
use crate::party::Character;
use crate::screens::{ReturnTo, Screen};
use crate::shell::Shell;
use gbx_formats::game_data::GameData;
use gbx_formats::save_orig as rec;
use gbx_formats::save_orig::ITEM_RECORD_SIZE;

/// The `ITEMS` rows the fixtures need: a long sword (36), chain mail (55),
/// a potion (71, slot 10) and an MU scroll (61, slot 11).
fn items_file() -> Vec<u8> {
    let mut b = vec![0u8; 2 + 0x81 * 0x10];
    let mut row = |t: usize, slot: u8, hands: u8, dc: u8, ds: u8, flags: u8| {
        let off = 2 + t * 0x10;
        b[off] = slot;
        b[off + 1] = hands;
        b[off + 9] = dc;
        b[off + 0xA] = ds;
        b[off + 0xD] = 0xFF;
        b[off + 0xE] = flags;
    };
    row(36, 0, 1, 1, 8, gbx_formats::items::flags::MELEE);
    row(55, items::SLOT_ARMOR, 0, 0, 0, 0);
    row(71, 10, 0, 0, 0, 0); // Potion
    row(61, items::SLOT_SCROLL_FIRST, 0, 0, 0, 0); // MU Scroll
    row(62, items::SLOT_CLERIC_SCROLL, 0, 0, 0, 0); // Cleric scroll
    b
}

fn game_data() -> GameData {
    let program = crate::test_support::exit_only_block();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let base = combat_game_data(program);
    for name in [
        format!("ECL{}.DAX", crate::engine::GAME_AREA),
        format!("MON{}CHA.DAX", crate::engine::GAME_AREA),
    ] {
        if let Some(bytes) = base.raw_file(&name) {
            files.push((name, bytes.to_vec()));
        }
    }
    files.push(("ITEMS".to_string(), items_file()));
    GameData::from_files(files)
}

/// An item record with the fields the screen reads.
#[derive(Clone, Copy, Default)]
struct Item {
    item_type: u8,
    nn: [u8; 3],
    plus: i8,
    readied: bool,
    hidden: u8,
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
    r[0x34] = u8::from(it.readied);
    r[0x35] = it.hidden;
    r[0x37..0x39].copy_from_slice(&it.weight.to_le_bytes());
    r[0x39] = it.count;
    r[0x3C] = it.affects[0];
    r[0x3D] = it.affects[1];
    r[0x3E] = it.affects[2];
    r
}

fn engine_with(party: Vec<Character>) -> Engine {
    let mut sets = crate::symbols::SymbolSets::new();
    sets.load(4, synthetic_set4());
    let mut e = Engine::new_fixture(synthetic_font(), sets, open_geo(), game_data(), 1);
    e.party = crate::party::Party { members: party };
    e.state.pos = (8, 8);
    e
}

/// A member carrying `items`, with `in_combat` set (the roster's "still with
/// the party" flag, which is what the Use word tests).
fn carrier(name: &str, items: Vec<Vec<u8>>) -> Character {
    let mut ch = party_member(name, 30, 50, 41);
    ch.status.in_combat = true;
    ch.items = items;
    ch
}

/// Open the sheet, then the Items leaf, settling the tick loop each time.
fn open_items(e: &mut Engine) {
    e.open_party_view();
    e.tick(&[]);
    e.tick(&[InputEvent::Char(b'I')]);
    e.tick(&[]);
}

fn screen(e: &Engine) -> &super::ItemsScreen {
    match e.shell() {
        Shell::Screen(Screen::Items(s)) => s,
        other => panic!("not on the items screen: {other:?}"),
    }
}

fn rows(e: &Engine) -> Vec<String> {
    screen(e)
        .list
        .items
        .iter()
        .map(|i| match i {
            crate::widgets::ListItem::Entry(t) | crate::widgets::ListItem::Heading(t) => t.clone(),
        })
        .collect()
}

// --- the list and the bar --------------------------------------------------

/// ★ The Items screen opens from the sheet, lists the real inventory with
/// generated names, and shows the conditional verb bar.
#[test]
fn the_sheet_opens_the_items_list_with_generated_names() {
    let sword = record(Item {
        item_type: 36,
        nn: [0, 0, 36],
        weight: 60,
        ..Default::default()
    });
    let mail = record(Item {
        item_type: 55,
        nn: [0, 48, 55],
        weight: 300,
        readied: true,
        ..Default::default()
    });
    let mut e = engine_with(vec![carrier("RAVD", vec![sword, mail])]);
    open_items(&mut e);
    assert_eq!(rows(&e), vec![" No   Long Sword", " Yes  Chain Mail"]);
    // `Use` is present (in_combat, no area ban) and `Halve` is (2 < 16).
    assert_eq!(
        screen(&e).bar.text,
        "Ready Use Trade Drop Halve Join",
        "the bar carries the original's conditional words"
    );
    // `sl_select_item` seeds `gbl.menuSelectedWord = 1` — the SECOND word.
    let (start, _) = screen(&e).bar.selected_span().expect("a highlighted word");
    assert_eq!(&screen(&e).bar.text[start..start + 3], "Use");
}

/// A member with nothing carried never opens the leaf — the same condition
/// that keeps `Items` off the sheet's own bar.
#[test]
fn an_empty_inventory_does_not_open() {
    let mut e = engine_with(vec![carrier("RAVD", vec![])]);
    e.open_party_view();
    e.tick(&[]);
    e.tick(&[InputEvent::Char(b'I')]);
    e.tick(&[]);
    assert!(matches!(e.shell(), Shell::Screen(Screen::PartyView(_))));
}

/// ★ Ready is reflected in the numbers the sheet draws, because
/// `reclac_player_values` runs after every menu action (`ovr020.cs:615`).
#[test]
fn readying_from_the_screen_moves_the_sheets_numbers() {
    let sword = record(Item {
        item_type: 36,
        nn: [0, 162, 36],
        plus: 1,
        weight: 60,
        ..Default::default()
    });
    let mut e = engine_with(vec![carrier("RAVD", vec![sword])]);
    open_items(&mut e);
    let before = e.party().members[0].combat.thac0_current;
    assert_eq!(rows(&e), vec![" No   Long Sword +1"]);

    e.tick(&[InputEvent::Char(b'R')]);
    e.tick(&[]);
    assert_eq!(rows(&e), vec![" Yes  Long Sword +1"]);
    let after = e.party().members[0].combat.thac0_current;
    assert_eq!(after, before + 1, "the +1 landed on the hit bonus");
    assert_eq!(e.party().members[0].combat.attacks.current[4], 8, "1d8");
    assert_eq!(e.party().members[0].combat.weapons_hands_used, 1);

    // And back off again.
    e.tick(&[InputEvent::Char(b'R')]);
    e.tick(&[]);
    assert_eq!(e.party().members[0].combat.thac0_current, before);
}

/// Drop is a two-beat confirm (`press_any_key` then `yes_no`), and a readied
/// item is refused before either.
#[test]
fn drop_confirms_twice_and_refuses_a_readied_item() {
    let sword = record(Item {
        item_type: 36,
        nn: [0, 0, 36],
        readied: true,
        ..Default::default()
    });
    let potion = record(Item {
        item_type: 71,
        nn: [0, 0, 64],
        count: 1,
        affects: [1, 0x63, 0],
        ..Default::default()
    });
    let mut e = engine_with(vec![carrier("RAVD", vec![sword, potion])]);
    open_items(&mut e);

    // Row 0 (the readied sword) refuses.
    e.tick(&[InputEvent::Char(b'D')]);
    e.tick(&[]);
    assert_eq!(screen(&e).status.as_deref(), Some("Must be unreadied"));
    assert_eq!(e.party().members[0].items.len(), 2);

    // Move to row 1 (End/Kp1 is the original's own step-down) and drop it.
    e.tick(&[InputEvent::Ext(crate::input::ExtKey::End)]);
    e.tick(&[]);
    e.tick(&[InputEvent::Char(b'D')]);
    e.tick(&[]);
    assert!(matches!(screen(&e).stage, super::Stage::DropWarn { .. }));
    e.tick(&[InputEvent::Enter]);
    e.tick(&[]);
    assert!(matches!(screen(&e).stage, super::Stage::DropConfirm { .. }));
    e.tick(&[InputEvent::Char(b'Y')]);
    e.tick(&[]);
    assert_eq!(e.party().members[0].items.len(), 1, "the potion is gone");
}

/// The loop ends the moment the last item leaves (`ovr020.cs:443`).
#[test]
fn dropping_the_last_item_leaves_the_screen() {
    let potion = record(Item {
        item_type: 71,
        nn: [0, 0, 64],
        count: 1,
        ..Default::default()
    });
    let mut e = engine_with(vec![carrier("RAVD", vec![potion])]);
    open_items(&mut e);
    e.tick(&[InputEvent::Char(b'D')]);
    e.tick(&[]);
    e.tick(&[InputEvent::Enter]);
    e.tick(&[]);
    e.tick(&[InputEvent::Char(b'Y')]);
    e.tick(&[]);
    assert!(
        matches!(e.shell(), Shell::Screen(Screen::PartyView(_))),
        "an empty inventory ends PlayerItemsMenu's while loop"
    );
}

/// Trade moves the record to the member the selector lands on.
#[test]
fn trade_hands_the_item_to_another_member() {
    let sword = record(Item {
        item_type: 36,
        nn: [0, 0, 36],
        weight: 60,
        ..Default::default()
    });
    let mut e = engine_with(vec![
        carrier("RAVD", vec![sword]),
        carrier(
            "ILMA",
            vec![record(Item {
                item_type: 71,
                nn: [0, 0, 64],
                ..Default::default()
            })],
        ),
    ]);
    open_items(&mut e);
    e.tick(&[InputEvent::Char(b'T')]);
    e.tick(&[]);
    assert!(matches!(screen(&e).stage, super::Stage::TradeWhom { .. }));
    // Step the cursor onto ILMA, then commit.
    e.tick(&[InputEvent::Ext(crate::input::ExtKey::End)]);
    e.tick(&[InputEvent::Enter]);
    e.tick(&[]);
    assert_eq!(e.party().members[1].items.len(), 2, "ILMA has the sword");
    assert!(
        matches!(e.shell(), Shell::Screen(Screen::PartyView(_))),
        "RAVD's last item left, so the loop ended"
    );
}

// --- Use -------------------------------------------------------------------

/// ★ A healing potion drunk out of combat: it must be **readied** first, then
/// Use heals through the real record and spends the charge.
#[test]
fn a_readied_potion_heals_and_is_consumed() {
    // ITEM2.DAX#2's Potion Extra Healing shape: affect_1 = 3 charges,
    // affect_2 = 0x63 (`cast_heal2`, 2d4+2), affect_3 = 0.
    let potion = record(Item {
        item_type: 71,
        nn: [187, 185, 64],
        hidden: 6,
        count: 0,
        affects: [3, 0x63, 0],
        ..Default::default()
    });
    let mut ch = carrier("RAVD", vec![potion]);
    ch.hit_point_max = 30;
    ch.hit_point_current = 10;
    let mut e = engine_with(vec![ch]);
    open_items(&mut e);

    // Unreadied → "Must be Readied", and nothing is spent.
    e.tick(&[InputEvent::Char(b'U')]);
    e.tick(&[]);
    assert_eq!(screen(&e).status.as_deref(), Some("Must be Readied"));
    assert_eq!(rec::item_affect(&e.party().members[0].items[0], 1), 3);

    e.tick(&[InputEvent::Char(b'R')]);
    e.tick(&[]);
    e.tick(&[InputEvent::Char(b'U')]);
    e.tick(&[]);

    let hp = e.party().members[0].hit_point_current;
    assert!(hp > 10, "the potion healed (2d4+2), hp={hp}");
    assert!(hp <= 30);
    assert_eq!(
        rec::item_affect(&e.party().members[0].items[0], 1),
        2,
        "one charge spent"
    );
}

/// ★ **A wand in camp burns a charge for nothing.** Every wand CotAB ships is
/// a `SpellTargets.Combat` row, so out of combat the item arm of
/// `sub_5D2E1`'s first branch fires: `"is a combat-only item..."` +
/// `"Use it? "`, and Yes spends without casting.
#[test]
fn a_combat_only_item_offers_to_burn_a_charge_and_does() {
    // A Wand of Fireballs shape: 12 charges, spell 0x2F (Fireball, Combat).
    let wand = record(Item {
        item_type: 79,
        nn: [0, 0, 69],
        readied: true,
        affects: [12, 0x2F, 0],
        ..Default::default()
    });
    let mut e = engine_with(vec![carrier("RAVD", vec![wand])]);
    open_items(&mut e);
    e.tick(&[InputEvent::Char(b'U')]);
    e.tick(&[]);
    assert!(matches!(screen(&e).stage, super::Stage::CombatOnly { .. }));

    // No: nothing is spent.
    e.tick(&[InputEvent::Char(b'N')]);
    e.tick(&[]);
    assert_eq!(rec::item_affect(&e.party().members[0].items[0], 1), 12);

    // Yes: a charge goes, and nothing is cast.
    e.tick(&[InputEvent::Char(b'U')]);
    e.tick(&[]);
    e.tick(&[InputEvent::Char(b'Y')]);
    e.tick(&[]);
    assert_eq!(
        rec::item_affect(&e.party().members[0].items[0], 1),
        11,
        "the charge is spent even though nothing was cast"
    );
}

/// ★ A scroll: the read-magic sweep opens it, the picker lists its spells, the
/// chosen one casts, and `remove_spell_from_scroll` blanks that affect byte and
/// counts `namenum2` down.
#[test]
fn a_readied_scroll_lists_its_spells_casts_one_and_loses_it() {
    // An `MU Scroll With 1 Spell` (`namenum2 = 0xD2`) carrying Bless — a §9.1
    // row that is castable out of combat, so the drive stays inside the
    // implemented set. One spell keeps the picker single-level: a *multi*-level
    // scroll list opens on the last row, not the first, because
    // `sl_select_item`'s entry step wraps backwards off the leading heading
    // (the slice-4 finding, `roll-credits.md` §8.1).
    let scroll = record(Item {
        item_type: 61,
        nn: [0, 0xD2, 209],
        readied: true,
        hidden: 0,
        affects: [0x01, 0, 0],
        ..Default::default()
    });
    let mut caster = carrier("SHARA", vec![scroll]);
    caster.class_level[crate::party::SKILL_MAGIC_USER] = 5;
    let mut e = engine_with(vec![caster]);
    open_items(&mut e);

    e.tick(&[InputEvent::Char(b'U')]);
    e.tick(&[]);
    assert!(matches!(screen(&e).stage, super::Stage::ScrollPick { .. }));

    // The first row is Bless — a `WholeParty` row, so it lands with no
    // selector at all.
    e.tick(&[InputEvent::Enter]);
    e.tick(&[]);
    assert!(
        e.party().members[0].has_affect(crate::spells::AFF_BLESS),
        "the scroll's Bless landed"
    );
    // `namenum2` counts down past `"With 1 Spell"` (0xD2), so the whole scroll
    // goes — and with the last item gone, `PlayerItemsMenu`'s loop ends.
    assert!(
        e.party().members[0].items.is_empty(),
        "a one-spell scroll is consumed entirely"
    );
    assert!(matches!(e.shell(), Shell::Screen(Screen::PartyView(_))));
}

/// The countdown itself, on a three-spell scroll: the affect byte is blanked
/// and `namenum2` drops one — the scroll renames itself as it is read.
#[test]
fn reading_one_spell_off_a_three_spell_scroll_renames_it() {
    let mut ch = carrier(
        "SHARA",
        vec![record(Item {
            item_type: 61,
            nn: [0, 0xD4, 209],
            affects: [0x01, 0x06, 0x16],
            ..Default::default()
        })],
    );
    assert!(!items::remove_spell_from_scroll(&mut ch, 0, 0x06));
    assert_eq!(rec::item_affect(&ch.items[0], 2), 0);
    assert_eq!(rec::item_namenum(&ch.items[0], 2), 0xD3);
    assert!(!items::remove_spell_from_scroll(&mut ch, 0, 0x01));
    assert_eq!(rec::item_namenum(&ch.items[0], 2), 0xD2);
    assert!(items::remove_spell_from_scroll(&mut ch, 0, 0x16));
    assert!(ch.items.is_empty(), "the third reading uses it up");
}

/// A scroll nobody can read: no cleric, no magic-user, no thief above 9 →
/// "oops!", and the scroll is untouched.
#[test]
fn a_scroll_a_fighter_cannot_read_says_oops_and_is_not_consumed() {
    let scroll = record(Item {
        item_type: 61,
        nn: [0, 0xD4, 209],
        readied: true,
        affects: [0x01, 0, 0],
        ..Default::default()
    });
    let mut fighter = carrier("RAVD", vec![scroll]);
    fighter.class_level = [0; 8];
    fighter.class_level[2] = 5; // fighter only
    let mut e = engine_with(vec![fighter]);
    open_items(&mut e);
    e.tick(&[InputEvent::Char(b'U')]);
    e.tick(&[]);
    e.tick(&[InputEvent::Enter]);
    e.tick(&[]);
    assert_eq!(screen(&e).status.as_deref(), Some("RAVD oops!"));
    assert_eq!(rec::item_affect(&e.party().members[0].items[0], 1), 0x01);
}

/// ★ `scroll_5C912`'s unhide half (`ovr023.cs:349-356`), which slice 4 deferred:
/// a hidden scroll lists nothing until a `read_magic` affect — or a cleric
/// holding a *clerical* scroll — opens it, and the flag is cleared for good.
#[test]
fn read_magic_opens_a_hidden_scroll_permanently() {
    let table = gbx_formats::items::ItemDataTable::parse(&items_file()).unwrap();
    let hidden = record(Item {
        item_type: 61,
        nn: [0, 0xD4, 209],
        hidden: 6,
        affects: [0x01, 0, 0],
        ..Default::default()
    });
    let mut ch = carrier("RAVD", vec![hidden.clone()]);
    assert_eq!(items::apply_read_magic(&mut ch, &table), 0, "still hidden");

    crate::affects::add_affect(&mut ch, crate::spells::AFF_READ_MAGIC, 20, 0xFF, false);
    assert_eq!(items::apply_read_magic(&mut ch, &table), 1);
    assert_eq!(rec::item_hidden_names_flag(&ch.items[0]), 0);

    // A cleric opens a *clerical* scroll (slot 12) without Read Magic — and
    // only a clerical one.
    let mut cleric = carrier(
        "SHARA",
        vec![
            record(Item {
                item_type: 61, // MU scroll, slot 11
                nn: [0, 0xD4, 209],
                hidden: 6,
                affects: [0x01, 0, 0],
                ..Default::default()
            }),
            record(Item {
                item_type: 62, // cleric scroll, slot 12
                nn: [0, 0xD4, 208],
                hidden: 6,
                affects: [0x01, 0, 0],
                ..Default::default()
            }),
        ],
    );
    cleric.class_level = [0; 8];
    cleric.class_level[crate::party::SKILL_CLERIC] = 5;
    assert_eq!(items::apply_read_magic(&mut cleric, &table), 1);
    assert_eq!(
        rec::item_hidden_names_flag(&cleric.items[0]),
        6,
        "the MU scroll stays shut"
    );
    assert_eq!(rec::item_hidden_names_flag(&cleric.items[1]), 0);
}

/// A parked Items screen round-trips through serde and keeps working (D-CV7's
/// obligation, which every screen in this shell carries).
#[test]
fn a_parked_items_screen_round_trips() {
    let sword = record(Item {
        item_type: 36,
        nn: [0, 0, 36],
        ..Default::default()
    });
    let mut e = engine_with(vec![carrier("RAVD", vec![sword])]);
    open_items(&mut e);
    let bytes = postcard::to_allocvec(e.shell()).expect("serializes");
    let back: Shell = postcard::from_bytes(&bytes).expect("deserializes");
    assert!(matches!(back, Shell::Screen(Screen::Items(_))));
}

/// The screen returns to whichever sheet opened it.
#[test]
fn exit_returns_to_the_character_sheet() {
    let sword = record(Item {
        item_type: 36,
        nn: [0, 0, 36],
        ..Default::default()
    });
    let mut e = engine_with(vec![carrier("RAVD", vec![sword])]);
    open_items(&mut e);
    assert_eq!(screen(&e).return_to, ReturnTo::World);
    e.tick(&[InputEvent::Escape]);
    e.tick(&[]);
    assert!(matches!(e.shell(), Shell::Screen(Screen::PartyView(_))));
}
