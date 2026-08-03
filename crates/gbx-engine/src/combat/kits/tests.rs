//! The kit-equivalence fixture tests (§8.5): a party built from **M3 state**
//! must decode to the same combat record fields the **capture path** produces
//! for equivalent equipment — readied-ammo gate included.
//!
//! Every record and item here is hand-authored (D10); no game bytes.

use super::*;
use crate::combat::{combat_state_from_records, CombatMap, RecordCombatant};
use crate::party::{character_from_record, Character};
use gbx_formats::items::ItemDataTable;
use gbx_formats::save_orig::{decode_char_record, CHAR_RECORD_SIZE, ITEM_RECORD_SIZE};
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::pack::RuleSet;

// --- fixtures -------------------------------------------------------------

/// A hand-authored `0x1A6` record with the fields the combat decode reads.
fn record_bytes(name: &str) -> Vec<u8> {
    let mut r = vec![0u8; CHAR_RECORD_SIZE];
    r[0] = name.len() as u8;
    r[1..1 + name.len()].copy_from_slice(name.as_bytes());
    r[0x10] = 18; // str current
    r[0x11] = 18; // str original
    r[0x16] = 16; // dex current
    r[0x17] = 16; // dex original
    r[0x73] = 15; // thac0_base
    r[0x74] = 2; // race: elf
    r[0x75] = 3; // class
    r[0x78] = 30; // hit_point_max
    r[0xde] = 0x81; // field_de → size 1
    r[0xe5] = 5; // hit_dice
    r[0xf7] = 0x10; // control_morale (a PC)
    r[0x11a] = 0; // monster_type
    r[0x11b] = 4; // alignment
    r[0x11c] = 2; // attacksCount
    r[0x11e] = 1; // attack1_DiceCountBase
    r[0x120] = 2; // attack1_DiceSizeBase
    r[0x122] = 0; // attack1_DamageBonusBase
    r[0x125] = 1; // field_125 (the strength-bonus gate: ON)
    r[0x14b] = 0x0E;
    r[0x186] = 2; // save bonus
    r[0x199] = 40; // hitBonus
    r[0x19a] = 0x30; // ac
    r[0x19b] = 0x31; // ac_behind
    r[0x19e] = 1; // attack1 dice count (current)
    r[0x1a0] = 8; // attack1 dice size  (current)
    r[0x1a2] = 6; // attack1 dmg bonus  (current)
    r[0x1a4] = 27; // hit_point_current
    r[0x1a5] = 12; // movement
    r
}

/// One `.swg` item record: type, plus, count, readied.
fn item_bytes(name: &str, item_type: u8, plus: i8, count: u8, readied: bool) -> Vec<u8> {
    let mut it = vec![0u8; ITEM_RECORD_SIZE];
    let n = name.as_bytes();
    it[..n.len()].copy_from_slice(n);
    it[0x2E] = item_type;
    it[0x32] = plus as u8;
    it[0x34] = u8::from(readied);
    it[0x39] = count;
    it
}

/// Item-table types this module uses, with the `ITEMS` fields the scan reads.
const T_LONG_SWORD: u8 = 0x24;
const T_LONG_BOW: u8 = 0x29;
const T_ARROWS: u8 = 0x2F;
const T_ARMOR: u8 = 0x40;
const T_CLERIC_ONLY: u8 = 0x50;

/// A hand-authored `ITEMS` image (D10) — the 2-byte header plus 16-byte entries
/// laid out exactly as `ItemDataTable::parse` reads them.
fn item_table() -> ItemDataTable {
    let mut bytes = vec![0u8; 2 + 0x81 * 0x10];
    let mut put = |t: u8, entry: [u8; 16]| {
        let off = 2 + t as usize * 0x10;
        bytes[off..off + 16].copy_from_slice(&entry);
    };
    // [slot, hands, dcL, dsL, bonusL, nAtk, _, _, _, dcN, dsN, bonusN, range, classFlags, flags, _]
    put(
        T_LONG_SWORD,
        [
            0,
            1,
            1,
            12,
            0,
            2,
            0,
            0,
            0,
            1,
            8,
            0,
            1,
            0xFF,
            flags::MELEE,
            0,
        ],
    );
    put(
        T_LONG_BOW,
        [
            0,
            2,
            1,
            6,
            0,
            4,
            0,
            0,
            0,
            1,
            6,
            0,
            8,
            0xFF,
            flags::FLAG_08 | flags::ARROWS | flags::FLAG_02,
            0,
        ],
    );
    put(
        T_ARROWS,
        [slot::ARROWS, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0, 0],
    );
    put(
        T_ARMOR,
        [slot::ARMOR, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0, 0],
    );
    // A weapon only class-flag 0x02 may wield (SHARA's forbidden sling shape).
    put(
        T_CLERIC_ONLY,
        [0, 1, 1, 8, 0, 2, 0, 0, 0, 1, 6, 0, 1, 0x02, flags::MELEE, 0],
    );
    ItemDataTable::parse(&bytes).expect("the fixture table parses")
}

fn flavor_pack() -> RuleSet {
    RuleSet::load()
}

fn make_character(items: Vec<Vec<u8>>) -> Character {
    let rec = decode_char_record(&record_bytes("LEDERA")).expect("fixture record decodes");
    let mut c = character_from_record(&rec, items, vec![]);
    c.skills.class_flags = 0xFF; // wields everything the fixture table allows
    c
}

/// Every field [`combatant_from_record`] writes, as `name = value` lines — the
/// equivalence surface the two paths must agree on. A `Vec<String>` rather than
/// a tuple so a mismatch names the field that drifted.
fn fingerprint(c: &Combatant) -> Vec<String> {
    vec![
        format!("id = {}", c.id),
        format!("team = {:?}", c.team),
        format!("npc = {}", c.npc),
        format!("non_team_member = {}", c.non_team_member),
        format!("control_morale = {}", c.control_morale),
        format!("int_score = {}", c.int_score),
        format!("monster_type = {}", c.monster_type),
        format!("field_14b = {}", c.field_14b),
        format!("size = {}", c.size),
        format!("pos = {:?}", c.pos),
        format!("hp_current = {}", c.hp_current),
        format!("hp_max = {}", c.hp_max),
        format!("ac = {}", c.ac),
        format!("ac_behind = {}", c.ac_behind),
        format!("hit_bonus = {}", c.hit_bonus),
        format!("hit_dice = {}", c.hit_dice),
        format!("movement = {}", c.movement),
        format!("reaction_adj = {}", c.reaction_adj),
        format!("class = {}", c.class),
        format!("attacks_count = {}", c.attacks_count),
        format!("dice = {:?}", (c.dice_count, c.dice_size, c.damage_bonus)),
        format!("field_159_null = {}", c.field_159_null),
        format!("memorized_list = {:?}", c.memorized_list),
        format!("skill_level_magic_user = {}", c.skill_level_magic_user),
        format!("skill_level_ranger = {}", c.skill_level_ranger),
        format!("skill_level_cleric = {}", c.skill_level_cleric),
        format!("skill_level_paladin = {}", c.skill_level_paladin),
        format!("thief_skill_level = {}", c.thief_skill_level),
        format!("caster_no_class = {}", c.caster_no_class),
        format!("health_status = {:?}", c.health_status),
        format!("entry_dice = {:?}", c.entry_dice),
        format!("attack2_dice = {:?}", c.attack2_dice),
        format!("base_half_moves = {}", c.base_half_moves),
        format!("base_dice = {:?}", c.base_dice),
        format!("field_de = {}", c.field_de),
        format!("thac0 = {}", c.thac0),
        format!("str_hit_bonus = {}", c.str_hit_bonus),
        format!("str_dmg_bonus = {}", c.str_dmg_bonus),
        format!("race = {}", c.race),
        format!("alignment = {}", c.alignment),
        format!("saves = {:?}", c.saves),
        format!("field_186 = {}", c.field_186),
    ]
}

/// The capture path: raw record bytes → `combat_state_from_records`.
fn via_capture_path(record: &[u8], armor_pointer: bool) -> Combatant {
    let rules = flavor_pack();
    let flavor = Adnd1::new(&rules);
    let mut bytes = record.to_vec();
    if armor_pointer {
        // A live `activeItems.armor` far pointer, exactly as a capture's record
        // image carries it (nonzero == armour readied).
        bytes[0x159..0x15D].copy_from_slice(&[0x34, 0x12, 0x78, 0x56]);
    }
    let entries = vec![RecordCombatant {
        team: Team::Party,
        pos: GridPos::new(20, 12),
        record: &bytes,
        affects: Vec::new(),
    }];
    let state = combat_state_from_records(&entries, CombatMap::uniform(0x17), &flavor)
        .expect("the fixture record decodes");
    state.roster()[0].clone()
}

/// The live path: M3 `Character` → `party_kits`.
fn via_live_path(c: &Character, items: Option<&ItemDataTable>) -> Combatant {
    let rules = flavor_pack();
    let flavor = Adnd1::new(&rules);
    let kits = party_kits(
        std::slice::from_ref(c),
        &[GridPos::new(20, 12)],
        items,
        &flavor,
    );
    assert_eq!(kits.len(), 1);
    kits.into_iter().next().unwrap().combatant
}

// --- the equivalence ------------------------------------------------------

#[test]
fn a_party_built_from_m3_state_decodes_to_the_capture_paths_combatant() {
    // ★ The §8.5 kit-equivalence fixture: the SAME record reaching a fighter
    // two ways — as capture bytes, and as party state round-tripped through
    // `character_from_record` / `record_from_character` — must produce the same
    // combatant, field for field.
    let raw = record_bytes("LEDERA");
    let table = item_table();
    let c = make_character(vec![item_bytes("Long Sword", T_LONG_SWORD, 2, 1, true)]);

    assert_eq!(
        fingerprint(&via_live_path(&c, Some(&table))),
        fingerprint(&via_capture_path(&raw, false)),
        "no armour readied ⇔ a null activeItems.armor pointer"
    );
}

#[test]
fn readied_armour_is_the_field_159_the_capture_path_reads_from_the_pointer() {
    // The §15 mage-hold gate, both ways: a live member with armour readied must
    // land on the same `field_159_null` as a capture whose record carries a
    // non-null `activeItems.armor` pointer at 0x159.
    let raw = record_bytes("LEDERA");
    let table = item_table();
    let armoured = make_character(vec![
        item_bytes("Long Sword", T_LONG_SWORD, 2, 1, true),
        item_bytes("Chain Mail", T_ARMOR, 0, 1, true),
    ]);
    let live = via_live_path(&armoured, Some(&table));
    assert!(!live.field_159_null, "readied armour ⇒ field_159 non-null");
    assert_eq!(
        fingerprint(&live),
        fingerprint(&via_capture_path(&raw, true))
    );

    // Carried but NOT readied is the same as not carried.
    let carried = make_character(vec![
        item_bytes("Long Sword", T_LONG_SWORD, 2, 1, true),
        item_bytes("Chain Mail", T_ARMOR, 0, 1, false),
    ]);
    assert!(via_live_path(&carried, Some(&table)).field_159_null);
}

#[test]
fn record_from_character_is_the_exact_inverse_of_character_from_record() {
    // D-SAVE11's field-completeness, used in anger: decode → model → re-encode
    // must be a fixed point, or the live path would silently fight with
    // different numbers than the record says.
    let raw = record_bytes("MATHEW");
    let decoded = decode_char_record(&raw).expect("decodes");
    let model = character_from_record(&decoded, vec![], vec![]);
    let round_tripped = record_from_character(&model);
    assert_eq!(
        format!("{decoded:?}"),
        format!("{round_tripped:?}"),
        "every field survives the model round trip"
    );
}

// --- the loadout scan -----------------------------------------------------

#[test]
fn the_scan_picks_the_best_rated_candidate_in_each_class() {
    let table = item_table();
    let rules = flavor_pack();
    let flavor = Adnd1::new(&rules);
    let c = make_character(vec![
        item_bytes("Long Sword", T_LONG_SWORD, 0, 1, true),
        item_bytes("Long Sword +2", T_LONG_SWORD, 2, 1, false),
        item_bytes("Long Bow", T_LONG_BOW, 0, 1, false),
        item_bytes("Arrows", T_ARROWS, 0, 20, false),
    ]);
    let l = loadout_for(&c, &table, &flavor).expect("a weapon candidate exists");
    assert_eq!(
        l.melee,
        Some((T_LONG_SWORD, 2)),
        "the +2 outrates the plain"
    );
    assert_eq!(l.ranged, Some((T_LONG_BOW, 0)));
    assert_eq!(
        l.unarmed_profile,
        (1, 2, 2),
        "1d2 base dice + the STR-18 damage adjustment (+2)"
    );
}

#[test]
fn an_unreadied_quiver_loses_the_ammo_gate() {
    // ★ §49, the whole point: arrows count only when READIED. An archer whose
    // quiver is merely carried fights as a swordsman.
    let table = item_table();
    let rules = flavor_pack();
    let flavor = Adnd1::new(&rules);

    let carried = make_character(vec![
        item_bytes("Long Sword +1", T_LONG_SWORD, 1, 1, true),
        item_bytes("Long Bow", T_LONG_BOW, 0, 1, false),
        item_bytes("Arrows", T_ARROWS, 0, 20, false),
    ]);
    let l = loadout_for(&carried, &table, &flavor).unwrap();
    assert!(
        !l.ammo_readied,
        "an unreadied quiver is invisible to var_1F"
    );
    assert_eq!(l.ammo_count, 20, "the count is still carried, just inert");

    let readied = make_character(vec![
        item_bytes("Long Sword +1", T_LONG_SWORD, 1, 1, false),
        item_bytes("Long Bow", T_LONG_BOW, 0, 1, true),
        item_bytes("Arrows", T_ARROWS, 0, 20, true),
    ]);
    let l = loadout_for(&readied, &table, &flavor).unwrap();
    assert!(l.ammo_readied);
    assert!(l.entry_ranged_readied, "the bow is the readied primary");
}

#[test]
fn the_ammo_gate_reads_the_launchers_own_ammo_slot() {
    // A launcher flagged `arrows` looks at slot 11 only — a readied quiver of
    // quarrels does not arm a bow.
    let mut bytes = vec![0u8; 2 + 0x81 * 0x10];
    let quarrels_only = {
        let off = 2 + T_ARROWS as usize * 0x10;
        bytes[off] = slot::QUARRELS;
        bytes[off + 0x0D] = 0xFF;
        let off = 2 + T_LONG_BOW as usize * 0x10;
        bytes[off + 0x0C] = 8; // range
        bytes[off + 0x0D] = 0xFF; // classFlags
        bytes[off + 0x09] = 1; // dice count
        bytes[off + 0x0A] = 6; // dice size
        bytes[off + 0x0E] = flags::FLAG_08 | flags::ARROWS;
        ItemDataTable::parse(&bytes).unwrap()
    };
    let rules = flavor_pack();
    let flavor = Adnd1::new(&rules);
    let c = make_character(vec![
        item_bytes("Long Bow", T_LONG_BOW, 0, 1, true),
        item_bytes("Quarrels", T_ARROWS, 0, 20, true),
    ]);
    let l = loadout_for(&c, &quarrels_only, &flavor).unwrap();
    assert!(
        !l.ammo_readied,
        "readied quarrels do not feed an arrows launcher"
    );
}

#[test]
fn a_class_ineligible_weapon_never_becomes_a_candidate() {
    // The `classFlags` gate (`ovr010.cs:899`), folded into row construction the
    // same way the capture path's hand-pinned kits fold it (SHARA's sling).
    let table = item_table();
    let rules = flavor_pack();
    let flavor = Adnd1::new(&rules);
    let mut c = make_character(vec![item_bytes("Cleric Mace", T_CLERIC_ONLY, 1, 1, true)]);
    c.skills.class_flags = 0x02; // eligible
    assert_eq!(
        loadout_for(&c, &table, &flavor).and_then(|l| l.melee),
        Some((T_CLERIC_ONLY, 1))
    );
    c.skills.class_flags = 0x04; // not eligible
    assert!(
        loadout_for(&c, &table, &flavor).is_none(),
        "no eligible weapon ⇒ no loadout at all (bare hands, today's behaviour)"
    );
}

#[test]
fn a_melee_candidate_must_beat_bare_hands_to_be_picked_up() {
    // `var_16` starts at the bare-hands rating (`ovr010.cs:886`): a weapon that
    // rates no better than fists is not a candidate.
    let mut bytes = vec![0u8; 2 + 0x81 * 0x10];
    let off = 2 + T_LONG_SWORD as usize * 0x10;
    bytes[off + 1] = 1; // hands
    bytes[off + 0x09] = 1; // 1d1
    bytes[off + 0x0A] = 1;
    bytes[off + 0x0C] = 1;
    bytes[off + 0x0D] = 0xFF;
    bytes[off + 0x0E] = flags::MELEE;
    let weak = ItemDataTable::parse(&bytes).unwrap();
    let rules = flavor_pack();
    let flavor = Adnd1::new(&rules);
    // Bare hands here are 1d2+2 → rating 2 + 4 = 6; the twig rates 1 + 3 = 4.
    let c = make_character(vec![item_bytes("Twig", T_LONG_SWORD, 0, 1, true)]);
    assert!(loadout_for(&c, &weak, &flavor).is_none());
}

#[test]
fn without_the_items_table_a_party_fights_exactly_as_it_does_today() {
    // The D10/CI shape: no `ITEMS` file → no loadout rows, no armour gate, and
    // the record's own serialized profile stands. This is what keeps every
    // fixture-data test in the workspace unaffected by the kit work.
    let table_less = make_character(vec![item_bytes("Long Sword", T_LONG_SWORD, 2, 1, true)]);
    let c = via_live_path(&table_less, None);
    assert!(c.loadout.is_none());
    assert!(c.field_159_null);
    assert_eq!((c.dice_count, c.dice_size), (1, 8), "the record's own dice");
}

#[test]
fn only_living_members_enter_the_fight() {
    let mut alive = make_character(vec![]);
    alive.name = "ALIVE".into();
    let mut down = make_character(vec![]);
    down.name = "DOWN".into();
    down.hit_point_current = 0;
    let members = vec![alive, down.clone(), {
        let mut third = down.clone();
        third.name = "THIRD".into();
        third.hit_point_current = 4;
        third
    }];
    assert_eq!(living_count(&members), 2);

    let rules = flavor_pack();
    let flavor = Adnd1::new(&rules);
    let kits = party_kits(
        &members,
        &[GridPos::new(20, 12), GridPos::new(20, 13)],
        None,
        &flavor,
    );
    assert_eq!(kits.len(), 2);
    // Roster ids are contiguous from 0 (TeamList order), but `member_index`
    // still points back at the real roster slot — the scene needs that to find
    // each fighter's CHEAD/CBODY icon.
    assert_eq!(kits[0].combatant.id, 0);
    assert_eq!(kits[1].combatant.id, 1);
    assert_eq!(kits[0].member_index, 0);
    assert_eq!(kits[1].member_index, 2);
}
