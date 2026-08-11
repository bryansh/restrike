use super::*;
use crate::party::{character_from_record, Party};
use crate::rest::status;

fn ch(name: &str) -> Character {
    let rec = vec![0u8; gbx_formats::save_orig::CHAR_RECORD_SIZE];
    let record = gbx_formats::save_orig::decode_char_record(&rec).unwrap();
    let mut c = character_from_record(&record, Vec::new(), Vec::new());
    c.name = name.to_string();
    c.magic.spell_list = vec![0u8; crate::magic::SPELL_LIST_SIZE];
    c.hit_point_max = 30;
    c.hit_point_current = 30;
    c.race = 7; // human
    c
}

/// SHARA: a cleric 5 with the whole of §9.1's clerical set memorized.
fn cleric() -> Character {
    let mut c = ch("SHARA");
    c.class_level[crate::party::SKILL_CLERIC] = 5;
    c.stats.wis.original = 16;
    c.magic.cast_count[0] = [5, 5, 2, 1, 1];
    c
}

fn party_of(members: Vec<Character>) -> Party {
    Party { members }
}

fn rng() -> crate::rng::EngineRng {
    crate::rng::EngineRng::new(0x0C0F_FEE0)
}

// --- `NonCombatSpellCast`'s switch (`ovr023.cs:622-660`) -------------------

/// Each row's `targetType` picks its arm; the `Combat` rows never get there,
/// because `sub_5D2E1`'s own gate has already refused them.
#[test]
fn the_target_switch_reads_the_rows_target_type() {
    let cases = [
        (0x16u8, NonCombatTargets::Caster),   // Find Traps — Self
        (0x12, NonCombatTargets::Caster),     // Read Magic — Self
        (0x03, NonCombatTargets::ChooseOne),  // Cure Light — PartyMember
        (0x4B, NonCombatTargets::ChooseOne),  // Raise Dead — PartyMember
        (0x01, NonCombatTargets::WholeParty), // Bless
        (0x2A, NonCombatTargets::WholeParty), // Prayer
        (0x0F, NonCombatTargets::None),       // Magic Missile — Combat
    ];
    for (id, want) in cases {
        let e = spells::spell_entry(id).expect("must-have");
        assert_eq!(NonCombatTargets::of(&e), want, "{id:#04x}");
    }
}

/// ★ The five combat-only rows in §9.1's set are exactly the ones camp refuses
/// with `"can't be cast here..."` + `"Lose it? "`. Note **Hold Person** is one
/// of them: its `targetType` is `Combat` even though its `whenCast` is too, so
/// a cleric who wakes with a Hold Person memorized and no fight to use it in
/// can only burn the slot.
#[test]
fn the_combat_rows_are_the_ones_camp_refuses() {
    let refused: Vec<u8> = spells::MUST_HAVE_IDS
        .iter()
        .copied()
        .filter(|&id| cant_be_cast_here(&spells::spell_entry(id).unwrap()))
        .collect();
    assert_eq!(
        refused,
        vec![0x02, 0x0F, 0x15, 0x17, 0x2F],
        "Curse, Magic Missile, Sleep, Hold Person and Fireball are combat-only"
    );
}

/// The `WholeParty` duplicate (`:647-650`): `AddRange` on top of the already
/// seeded caster, so the caster is in the list twice. Faithful, and harmless —
/// the second pass removes and re-adds the same affect.
#[test]
fn whole_party_carries_the_caster_twice() {
    let party = party_of(vec![ch("A"), ch("B"), ch("C")]);
    assert_eq!(whole_party_targets(1, &party), vec![1, 0, 1, 2]);
}

// --- the effects ----------------------------------------------------------

/// ★ **Bless out of combat** lands on every party member for six minutes, with
/// the caster's level in the affect's `data`. Out of combat the melee filter is
/// gated off (`:994` tests `game_state == Combat`), so nobody is skipped.
#[test]
fn bless_in_camp_covers_the_whole_party() {
    let mut party = party_of(vec![cleric(), ch("MATHEW"), ch("MARK")]);
    let targets = whole_party_targets(0, &party);
    let out = cast(&mut party, &mut rng(), 0, 0x01, &targets);
    for m in 0..3 {
        let a = affects::find_affect(&party.members[m], spells::AFF_BLESS)
            .unwrap_or_else(|| panic!("member {m} is blessed"));
        assert_eq!(a.minutes, 6);
        assert_eq!(a.data, 5, "the cleric's level");
    }
    assert_eq!(
        party.members[0]
            .affects
            .iter()
            .filter(
                |r| gbx_formats::affects::AffectRecord::decode(r).unwrap().kind
                    == spells::AFF_BLESS
            )
            .count(),
        1,
        "the duplicated caster entry replaces rather than stacks"
    );
    assert_eq!(out.len(), 4, "one line per pass, caster twice");
}

/// ★ **Protection from Evil** in camp is the spell's real home: `PartyMember`
/// targeting means it can be put on the fighter who is about to be hit, not
/// only on the caster (which is all the combat arm can do).
#[test]
fn protection_from_evil_in_camp_can_be_put_on_somebody_else() {
    let mut party = party_of(vec![cleric(), ch("MATHEW")]);
    cast(&mut party, &mut rng(), 0, 0x06, &[1]);
    let a = affects::find_affect(&party.members[1], spells::AFF_PROT_EVIL).expect("prot evil");
    assert_eq!(a.minutes, 15, "0 + 3 × 5");
    assert!(!party.members[0].has_affect(spells::AFF_PROT_EVIL));
}

/// ★ **Read Magic** (`is_affected`) plants the affect `scroll_5C912` looks for
/// — the gate that decides whether an unknown scroll lists anything at all
/// (`ovr023.cs:351-356`). Slice 4 named this as G7's; it closes here.
#[test]
fn read_magic_plants_the_affect_the_scribe_gate_reads() {
    let mut c = ch("PHILIPPE");
    c.class_level[crate::party::SKILL_MAGIC_USER] = 5;
    c.stats.int.original = 17;
    let mut party = party_of(vec![c]);
    cast(&mut party, &mut rng(), 0, 0x12, &[0]);
    let a = affects::find_affect(&party.members[0], spells::AFF_READ_MAGIC).expect("read magic");
    assert_eq!(a.minutes, 10, "0 + 2 × 5");
}

/// ★ **The cures** roll their own dice and cap at `hp_max`, and `heal_player`'s
/// status gate keeps a corpse from being topped up.
#[test]
fn the_cures_roll_and_cap() {
    for (id, count, bonus) in [(0x03u8, 1usize, 0i32), (0x3A, 2, 1), (0x47, 3, 3)] {
        let mut wounded = ch("MATHEW");
        wounded.hit_point_current = 4;
        let mut party = party_of(vec![cleric(), wounded]);
        let mut r = rng();
        let out = cast(&mut party, &mut r, 0, id, &[1]);
        let mut oracle = crate::rng::EngineRng::new(0x0C0F_FEE0);
        let rolled: i32 = (0..count)
            .map(|_| i32::from(crate::rest::roll_dice(&mut oracle, 8, 1)))
            .sum();
        assert_eq!(
            party.members[1].hit_point_current,
            (4 + rolled + bonus).min(30) as u8,
            "{id:#04x}"
        );
        assert_eq!(out.len(), 1);
    }
    // A dead target is not healed at all (`heal_player`'s status gate).
    let mut corpse = ch("MARK");
    corpse.status.health_status = status::DEAD;
    corpse.hit_point_current = 0;
    let mut party = party_of(vec![cleric(), corpse]);
    let out = cast(&mut party, &mut rng(), 0, 0x03, &[1]);
    assert_eq!(party.members[1].hit_point_current, 0);
    assert!(out.is_empty(), "no line, because nothing happened");
}

/// ★ **Neutralize Poison** — the spider/wyvern answer. It strips all three
/// poison affects, lifts a member off 0 hit points, and puts them back on their
/// feet. A member who is not poisoned gets "is unaffected" and keeps the slot's
/// cost, which is the original's own bluntness.
#[test]
fn neutralize_poison_undoes_the_whole_poison_stack() {
    let mut victim = ch("TRAVIS");
    victim.status.health_status = status::DYING;
    victim.hit_point_current = 0;
    affects::add_affect(&mut victim, spells::AFF_POISONED, 0, 0xFF, false);
    affects::add_affect(&mut victim, spells::AFF_SLOW_POISON, 60, 0xFF, true);
    affects::add_affect(&mut victim, spells::AFF_POISON_DAMAGE, 10, 0xFF, true);
    let mut party = party_of(vec![cleric(), victim]);
    let out = cast(&mut party, &mut rng(), 0, 0x43, &[1]);
    let v = &party.members[1];
    assert!(!v.has_affect(spells::AFF_POISONED));
    assert!(!v.has_affect(spells::AFF_SLOW_POISON));
    assert!(!v.has_affect(spells::AFF_POISON_DAMAGE));
    assert_eq!(v.hit_point_current, 1, "lifted off the floor");
    assert_eq!(v.status.health_status, status::OKEY);
    assert!(v.status.in_combat);
    assert_eq!(out[0].text, "is unpoisoned");

    // Not poisoned → nothing but the line.
    let mut party = party_of(vec![cleric(), ch("MARK")]);
    let out = cast(&mut party, &mut rng(), 0, 0x43, &[1]);
    assert_eq!(out[0].text, "is unaffected");
}

/// ★ **Slow Poison** is not a cure: it re-arms a ten-minute `poison_damage`
/// countdown and adds `slow_poison` for **an hour per caster level**
/// (`perLvlDuration = 60`). When that runs out in camp, `AffectSlowPoison`
/// kills anybody still poisoned — which is what makes Neutralize Poison the
/// real answer and this the field stopgap.
#[test]
fn slow_poison_buys_an_hour_and_a_heartbeat() {
    let mut victim = ch("TRAVIS");
    victim.hit_point_current = 0;
    affects::add_affect(&mut victim, spells::AFF_POISONED, 0, 0xFF, false);
    let mut party = party_of(vec![cleric(), victim]);
    cast(&mut party, &mut rng(), 0, 0x1A, &[1]);
    let v = &party.members[1];
    assert!(v.has_affect(spells::AFF_POISONED), "still poisoned");
    assert_eq!(
        affects::find_affect(v, spells::AFF_SLOW_POISON)
            .unwrap()
            .minutes,
        300,
        "0 + 60 × cleric level 5 — five hours of grace"
    );
    assert_eq!(
        affects::find_affect(v, spells::AFF_POISON_DAMAGE)
            .unwrap()
            .minutes,
        10
    );
    assert_eq!(v.hit_point_current, 1);

    // An unpoisoned target gets nothing at all.
    let mut party = party_of(vec![cleric(), ch("MARK")]);
    let out = cast(&mut party, &mut rng(), 0, 0x1A, &[1]);
    assert!(out.is_empty());
    assert!(!party.members[1].has_affect(spells::AFF_SLOW_POISON));
}

/// ★ **Cure Disease**'s three cures and their cascades (`sub_5F037`).
#[test]
fn cure_disease_runs_all_three_cures_and_their_cascades() {
    let mut sick = ch("MATHEW");
    affects::add_affect(&mut sick, spells::AFF_WEAKEN, 0, 0xFF, false);
    affects::add_affect(&mut sick, spells::AFF_CAUSE_DISEASE_2, 0, 0xFF, false);
    affects::add_affect(&mut sick, spells::AFF_HELPLESS, 0, 0xFF, false);
    affects::add_affect(&mut sick, spells::AFF_BLESS, 6, 5, false);
    let mut party = party_of(vec![cleric(), sick]);
    let out = cast(&mut party, &mut rng(), 0, 0x27, &[1]);
    let m = &party.members[1];
    assert!(!m.has_affect(spells::AFF_WEAKEN));
    assert!(!m.has_affect(spells::AFF_CAUSE_DISEASE_2), "the cascade");
    assert!(!m.has_affect(spells::AFF_HELPLESS), "…and its second half");
    assert!(m.has_affect(spells::AFF_BLESS), "unrelated affects survive");
    assert_eq!(out.len(), 1);
}

/// ★ **Remove Curse**'s two arms: the affect first, and only if it is absent
/// the cursed item. The item arm is the one combat cannot have.
#[test]
fn remove_curse_prefers_the_affect_then_un_readies_the_item() {
    // Arm 1 — the affect.
    let mut cursed = ch("MARK");
    affects::add_affect(&mut cursed, spells::AFF_BESTOW_CURSE, 0, 0xFF, false);
    let mut party = party_of(vec![cleric(), cursed]);
    let out = cast(&mut party, &mut rng(), 0, 0x2B, &[1]);
    assert_eq!(out[0].text, "is un-cursed");
    assert!(!party.members[1].has_affect(spells::AFF_BESTOW_CURSE));

    // Arm 2 — the item. Synthetic record bytes only (D10).
    let mut holder = ch("LEDERA");
    let mut item = vec![0u8; 0x40];
    item[0x34] = 1; // readied
    item[0x36] = 1; // cursed
    holder.items.push(item);
    holder.readied_items.insert(0);
    let mut party = party_of(vec![cleric(), holder]);
    let out = cast(&mut party, &mut rng(), 0, 0x2B, &[1]);
    assert_eq!(out[0].text, "has an item un-cursed");
    assert!(!gbx_formats::save_orig::item_readied(
        &party.members[1].items[0]
    ));
    assert!(party.members[1].readied_items.is_empty());
    assert!(
        gbx_formats::save_orig::item_is_cursed(&party.members[1].items[0]),
        "the curse stays on the item — only the readying is undone"
    );
}

/// ★ **Raise Dead**'s three gates and its cost (`cast_raise`,
/// `ovr023.cs:2341-2365`). The effect lands here; G8's temple is what will
/// sell it.
#[test]
fn raise_dead_costs_a_point_of_constitution_and_never_works_on_an_elf() {
    let mut corpse = ch("MATHEW");
    corpse.status.health_status = status::DEAD;
    corpse.hit_point_current = 0;
    corpse.stats.con.current = 15;
    affects::add_affect(&mut corpse, spells::AFF_POISONED, 0, 0xFF, false);
    let mut party = party_of(vec![cleric(), corpse]);
    let out = cast(&mut party, &mut rng(), 0, 0x4B, &[1]);
    let m = &party.members[1];
    assert_eq!(m.status.health_status, status::OKEY);
    assert_eq!(m.hit_point_current, 1, "back up at exactly one hit point");
    assert_eq!(m.stats.con.current, 14, "one point of Constitution");
    assert!(!m.has_affect(spells::AFF_POISONED));
    assert_eq!(out[0].text, "is raised");

    // An elf stays dead (`:2347`).
    let mut elf = ch("LEDERA");
    elf.race = 2;
    elf.status.health_status = status::DEAD;
    elf.stats.con.current = 15;
    let mut party = party_of(vec![cleric(), elf]);
    assert!(cast(&mut party, &mut rng(), 0, 0x4B, &[1]).is_empty());
    assert_eq!(party.members[1].status.health_status, status::DEAD);

    // Constitution already at zero (`:2346`) — raised once too often.
    let mut spent = ch("MARK");
    spent.status.health_status = status::DEAD;
    spent.stats.con.current = 0;
    let mut party = party_of(vec![cleric(), spent]);
    assert!(cast(&mut party, &mut rng(), 0, 0x4B, &[1]).is_empty());

    // A living character is not a candidate at all (`:2345`).
    let mut party = party_of(vec![cleric(), ch("MARK")]);
    assert!(cast(&mut party, &mut rng(), 0, 0x4B, &[1]).is_empty());
}

/// ★ **Dispel Magic** out of camp: a d100 per spell-planted affect, and the
/// `0xFF` racial marker is never rolled against.
#[test]
fn dispel_magic_in_camp_spares_the_racial_affects() {
    let mut target = ch("TRAVIS");
    affects::add_affect(&mut target, spells::AFF_BLESS, 6, 0, false);
    affects::add_affect(&mut target, 0x1A, 0, 0xFF, false); // dwarf_vs_orc
    affects::add_affect(&mut target, 0x2F, 0, 0xFF, false); // dwarf/gnome vs giants
    let mut party = party_of(vec![cleric(), target]);
    let mut r = rng();
    cast(&mut party, &mut r, 0, 0x29, &[1]);
    let m = &party.members[1];
    assert!(m.has_affect(0x1A), "racial affects are undispellable");
    assert!(m.has_affect(0x2F));
    // The d100 was spent: a cleric 5 against a level-0 affect needs 75 or less,
    // and this seed's first d100 is well inside that.
    assert!(!m.has_affect(spells::AFF_BLESS));
}

/// The ladder's sign (`:1690-1704`): a level-1 caster against affects stored at
/// level 15 needs a 22, so a run of eight of them cannot all clear.
#[test]
fn dispel_magics_ladder_punishes_a_weak_caster() {
    let mut weak = cleric();
    weak.class_level[crate::party::SKILL_CLERIC] = 1;
    let mut target = ch("TRAVIS");
    for _ in 0..8 {
        affects::add_affect(&mut target, spells::AFF_BLESS, 6, 15, false);
    }
    let mut party = party_of(vec![weak, target]);
    let mut r = rng();
    cast(&mut party, &mut r, 0, 0x29, &[1]);
    let left = affects::decoded(&party.members[1])
        .filter(|a| a.kind == spells::AFF_BLESS)
        .count();
    assert!(left > 0, "not all eight cleared (left {left})");
}

/// Casting is otherwise draw-free: the affect-only rows spend nothing.
#[test]
fn the_buff_rows_are_draw_free() {
    for id in [0x01u8, 0x06, 0x07, 0x12, 0x16, 0x2A, 0x45] {
        let mut party = party_of(vec![cleric(), ch("MATHEW")]);
        let before = crate::rng::EngineRng::new(0x0C0F_FEE0);
        let mut r = crate::rng::EngineRng::new(0x0C0F_FEE0);
        cast(&mut party, &mut r, 0, id, &[0, 1]);
        assert_eq!(
            r.state(),
            before.state(),
            "{id:#04x} moved the PRNG and should not have"
        );
    }
}
