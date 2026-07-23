use super::*;
use crate::combat::spells::{spell_entry, DamageOnSave, SpellClass, SpellTargets, SpellWhen};

// --- the SpellEntry row + the lazy-transcription rule (doc §41.2) -----------

/// Magic Missile (id 0x0F) decodes to the doc §41.2 row — every cell the
/// selection/cast path reads, pinned against an accidental edit to
/// `gbl.spellCastingTable[0xf]` (`Gbl.cs:583`).
#[test]
fn magic_missile_row_matches_the_binary_table() {
    let mm = spell_entry(0x0F).expect("Magic Missile is transcribed");
    assert_eq!(mm.id, 0x0F);
    assert_eq!(mm.spell_class, SpellClass::MagicUser);
    assert_eq!(mm.fixed_range, 6);
    assert_eq!(mm.per_lvl_range, 4);
    assert_eq!(mm.field_6, 4);
    assert_eq!(mm.target_type, SpellTargets::Combat);
    assert_eq!(mm.damage_on_save, DamageOnSave::Normal);
    assert_eq!(mm.save_verse, 4); // SaveVerseType.Spell
    assert_eq!(mm.affect_id, 0); // Affects.none
    assert_eq!(mm.when_cast, SpellWhen::Combat);
    assert_eq!(mm.casting_delay, 1);
    assert_eq!(mm.priority, 4);
    assert_eq!(mm.field_e, 1);
    assert_eq!(mm.field_f, 0);
}

/// `DamageOnSave.Normal == 0` — the value `DoSpellCastingWork` compares against
/// to decide "no save draw" (`ovr023.cs:587`). Pinning it guards the enum
/// discriminant the cast path depends on.
#[test]
fn damage_on_save_normal_is_zero() {
    assert_eq!(DamageOnSave::Normal as u8, 0);
    assert_eq!(DamageOnSave::Zero as u8, 1);
}

/// The lazy-transcription rule: only Magic Missile is transcribed. Every other
/// id — a neighbouring Magic-User row (Shield 0x13, Sleep 0x15), the Cleric
/// heal (id 3 `find_healing_target` special), and an out-of-range id — returns
/// `None`, so the selection AI trips `spell-entry` and rejects it (doc §41.2).
#[test]
fn only_magic_missile_is_transcribed() {
    assert!(spell_entry(0x0F).is_some());
    for id in [0u8, 1, 3, 0x0E, 0x10, 0x13, 0x15, 0x24, 0xFF] {
        assert!(
            spell_entry(id).is_none(),
            "id {id:#x} must be untranscribed (spell-entry trip)"
        );
    }
}

// --- the selection loop + ShouldCastSpellX (doc §41.1/§41.2) ----------------

/// Build a tiny fight: an NPC caster [0] with one memorized Magic Missile and
/// Magic-User level 5, and one live enemy [1] two tiles away (well within the
/// spell's range 26). The NPC arm of the sub_3560B gate is satisfied; the
/// enemy makes `BuildNearTargets` non-empty at priority 4.
fn caster_world() -> CombatWorld {
    let mut caster = Fighter::new_melee(
        0,
        Team::Monster,
        true,
        GridPos::new(10, 10),
        30,
        5,
        20,
        12,
        (1, 4, 2),
        5,
        1,
    );
    caster.memorized_list = vec![0x0F];
    caster.skill_level_magic_user = 5;
    let enemy = Fighter::new_melee(
        1,
        Team::Party,
        false,
        GridPos::new(12, 10),
        30,
        5,
        20,
        12,
        (1, 4, 2),
        5,
        1,
    );
    CombatWorld::new(CombatMap::uniform(FLOOR), vec![caster, enemy])
}

/// `spell_range` for Magic Missile (`ovr023.cs:515`): `fixedRange 6 + perLvlRange
/// 4 × castingLvl`. A Magic-User 5 caster → castingLvl 5 → 26; the no-caster
/// fallback → castingLvl 6 → 30.
#[test]
fn spell_range_magic_missile_scales_with_casting_level() {
    let mut world = caster_world();
    assert_eq!(world.spell_range(0, 0x0F), 26, "MU 5 → 6 + 4×5");
    assert_eq!(
        world.spell_max_target_count(0, SpellClass::MagicUser),
        5,
        "max(SkillLevel(MU)=5, SkillLevel(Ranger)−8=−8)"
    );
    world.fighters[0].caster_no_class = true;
    assert_eq!(
        world.spell_range(0, 0x0F),
        30,
        "no-caster fallback → 6 + 4×6"
    );
}

/// `ShouldCastSpellX`'s Magic Missile chain (`ovr010.cs:143`), draw-free: the
/// priority gate (MM priority 4), the enemy near-list, and the field_F == 0
/// accept. An untranscribed id trips `spell-entry` and rejects.
#[test]
fn should_cast_spell_x_magic_missile_chain() {
    let mut world = caster_world();
    assert!(
        !world.should_cast_spell_x(5, 0x0F, 0),
        "priority 4 < minPriority 5 → reject at the gate"
    );
    assert!(
        world.should_cast_spell_x(4, 0x0F, 0),
        "priority 4 ≥ 4, an enemy is near, field_F 0 → accept"
    );

    // An untranscribed id (Shield 0x10) → spell-entry trip + reject.
    let alog = ActionLog::default();
    world.attach_action_sink(alog.sink());
    assert!(!world.should_cast_spell_x(1, 0x10, 0));
    let stubs: Vec<&'static str> = alog
        .events()
        .into_iter()
        .filter_map(|e| match e {
            ActionEvent::StubTripped { stub, .. } => Some(stub),
            _ => None,
        })
        .collect();
    assert_eq!(stubs, vec!["spell-entry"]);
}

/// With no enemy in range, `BuildNearTargets` is empty, so a field_E-≠0 spell
/// (Magic Missile) rejects even at its own priority (`ovr010.cs:156-158`).
#[test]
fn should_cast_spell_x_rejects_with_no_enemy_in_range() {
    let mut world = caster_world();
    world.fighters[1].in_combat = false; // the only enemy leaves combat
    assert!(!world.should_cast_spell_x(4, 0x0F, 0));
}

/// The selection loop with the gate ON: the unconditional d7 bound, then the
/// priority-pass picks — Magic Missile (priority 4) rejects at priority 7/6/5
/// and accepts at priority 4, so a bound reaching pass 4 casts after 3+3+3+1 =
/// 10 picks. Every pick is `roll_dice(1,1)` (spells_count 1). The exact count is
/// driven by the seed's d7 (computed with the independent `Replay` oracle).
#[test]
fn selection_loop_casts_magic_missile_when_gate_and_bound_allow() {
    let mut world = caster_world();
    let mut rng = EngineRng::new(SEED);
    let log = DrawLog::default();
    rng.attach_sink(log.sink());

    let bound = Replay::new(SEED).roll(7); // the first draw IS the d7 bound
    let cast = world.sub_3560b(&mut rng, 0);

    let ns = log.ns();
    assert_eq!(ns[0], 7, "the first draw is the d7 bound");
    if bound >= 4 {
        assert!(cast, "bound {bound} ≥ 4 → MM accepted at priority 4");
        // The 10 selection picks (3+3+3+1) are d1s; the cast's own draws follow.
        for (i, n) in ns[1..=10].iter().enumerate() {
            assert_eq!(*n, 1, "selection pick #{i} is roll_dice(spells_count=1,1)");
        }
    } else {
        assert!(!cast, "bound {bound} < 4 → MM never reaches priority 4");
        // No cast: exactly d7 + 3 picks per pass, all d1s.
        assert_eq!(ns.len(), 1 + 3 * bound as usize, "d7 + 3 picks per pass");
        for n in &ns[1..] {
            assert_eq!(*n, 1, "each selection pick is roll_dice(spells_count=1,1)");
        }
    }
}

/// The gate OFF (a PC caster with `AutoPCsCastMagic` off): sub_3560B draws ONLY
/// the unconditional d7 bound and returns false — the §33 capture-proof
/// (bar-fists-2 closes with memorized MM slots and zero selection draws).
#[test]
fn selection_loop_gate_off_draws_only_the_d7() {
    let mut world = caster_world();
    world.fighters[0].team = Team::Party;
    world.fighters[0].npc = false; // a PC
    world.fighters[1].team = Team::Monster; // keep a live opponent
    let mut rng = EngineRng::new(SEED);
    let log = DrawLog::default();
    rng.attach_sink(log.sink());
    assert!(!world.sub_3560b(&mut rng, 0), "magic off → no cast");
    assert_eq!(log.len(), 1, "gate off → only the d7 bound is drawn");
    assert_eq!(log.ns()[0], 7);
}

/// The full Magic Missile cast (doc §41.3): once the selection accepts (a d7
/// bound reaching pass 4), sub_5D2E1 draws the targeting `find_target` pick and
/// the 3 damage d4s (`castingLvl 5 → 3 + 3d4`), rolls **no** save d20
/// (damageOnSave Normal), applies the damage, and consumes the memorized slot
/// (ClearSpell → `memorized_list` empty). The AI-turn returns `true`.
#[test]
fn magic_missile_cast_targets_damages_and_consumes_the_slot() {
    // A seed whose d7 bound ≥ 4 → the selection reaches priority 4 and casts.
    let seed = (0u32..)
        .find(|s| Replay::new(*s).roll(7) >= 4)
        .expect("some seed rolls a d7 ≥ 4");
    let mut world = caster_world();
    let mut rng = EngineRng::new(seed);
    let log = DrawLog::default();
    rng.attach_sink(log.sink());
    let hp_before = world.fighters[1].hp_current;

    assert!(
        world.sub_3560b(&mut rng, 0),
        "gate on + bound ≥ 4 → Magic Missile casts"
    );
    // Slot consumed (ClearSpell) — the caster's later turns draw no selection d1s.
    assert!(
        world.fighters[0].memorized_list.is_empty(),
        "the cast consumed the one memorized Magic Missile"
    );
    // No save: the stream carries no d20 (damageOnSave Normal ⇒ saved = false).
    let ns = log.ns();
    assert!(!ns.contains(&20), "Magic Missile rolls no save d20: {ns:?}");
    // Damage = n/2 + roll_dice(4, n/2) with n = castingLvl(5) + 1 = 6 → the three
    // damage d4s are the LAST three draws (targeting precedes them).
    let tail = &ns[ns.len() - 3..];
    assert!(
        tail.iter().all(|&n| n == 4),
        "the damage is 3 separate d4s at the tail: {ns:?}"
    );
    let dmg = hp_before - world.fighters[1].hp_current;
    assert!(
        (3 + 3..=3 + 12).contains(&dmg),
        "damage 3 + 3d4 ∈ 6..=15, applied to the target; got {dmg}"
    );
}
