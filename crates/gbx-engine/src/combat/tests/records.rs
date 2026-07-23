use super::*;

// --- the combat entry-state replay harness (H4, D-OR5(b)) --------------

/// A synthetic `0x1A6` record with the combat fields the replay harness reads
/// poked to the given values — the same offsets [`combatant_from_record`]
/// reads (D10-clean self-authored bytes). `dice` is `(count, size, bonus)`.
#[allow(clippy::too_many_arguments)]
fn synthetic_record(
    name: &[u8],
    hp_cur: u8,
    hp_max: u8,
    raw_ac: u8,
    hit_bonus: u8,
    dex_full: u8,
    hit_dice: u8,
    movement: u8,
    npc: bool,
    attacks_count: u8,
    dice: (u8, u8, u8),
) -> Vec<u8> {
    let mut r = vec![0u8; 0x1A6];
    r[0] = name.len() as u8;
    r[1..1 + name.len()].copy_from_slice(name);
    r[0x17] = dex_full; // stats2.Dex.full (== read_stat's `original` byte)
    r[0xe5] = hit_dice; // hit_dice
    r[0xf7] = if npc { 0x80 } else { 0x00 }; // control_morale
    r[0x11c] = attacks_count; // attacksCount (attack_profile_base[0])
    r[0x78] = hp_max; // hit_point_max
    r[0x199] = hit_bonus; // hitBonus
    r[0x19a] = raw_ac; // ac (raw)
    r[0x19c] = 1; // a1 attacks-left (overwritten by initiative)
    r[0x19e] = dice.0; // a1 dice_count
    r[0x1a0] = dice.1; // a1 dice_size
    r[0x1a2] = dice.2; // a1 dmg_bonus
    r[0x1a4] = hp_cur; // hit_point_current
    r[0x1a5] = movement; // movement
    r
}

/// D2: `combat_state_from_records` decodes each record, maps the right field
/// onto each combat input, preserves the snapshot's order + positions (no
/// `PlaceCombatants`), and produces a full melee fight whose draw stream opens
/// with exactly one initiative d6 per combatant — the §2 fingerprint, no setup
/// draw ahead of it. Synthetic records only (D10); the live differential is
/// the gated milestone test in `gbx-oracle`.
#[test]
fn replay_harness_maps_records_and_opens_with_one_d6_per_combatant() {
    use gbx_rules::adnd1::flavor_impl::Adnd1;
    use gbx_rules::pack::RuleSet;
    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);

    // 2 party + 3 monsters, distinct positions, DEX 16 (party) / 10 (monsters).
    let p0 = synthetic_record(b"HERO", 20, 22, 54, 50, 16, 1, 12, false, 2, (1, 8, 0));
    let p1 = synthetic_record(b"MAGE", 12, 12, 48, 46, 15, 1, 12, false, 2, (1, 4, 0));
    let m0 = synthetic_record(b"THUG", 8, 8, 40, 12, 10, 1, 9, true, 2, (1, 6, 0));
    let entries = vec![
        RecordCombatant {
            team: Team::Party,
            pos: GridPos::new(25, 12),
            record: &p0,
        },
        RecordCombatant {
            team: Team::Party,
            pos: GridPos::new(24, 12),
            record: &p1,
        },
        RecordCombatant {
            team: Team::Monster,
            pos: GridPos::new(34, 13),
            record: &m0,
        },
        RecordCombatant {
            team: Team::Monster,
            pos: GridPos::new(35, 13),
            record: &m0,
        },
        RecordCombatant {
            team: Team::Monster,
            pos: GridPos::new(33, 13),
            record: &m0,
        },
    ];

    let state = combat_state_from_records(&entries, CombatMap::uniform(0x17), &flavor).unwrap();
    let roster = state.roster();
    assert_eq!(roster.len(), 5);
    // Order + positions preserved verbatim (no PlaceCombatants).
    assert_eq!(roster[0].pos, GridPos::new(25, 12));
    assert_eq!(roster[2].pos, GridPos::new(34, 13));
    // Field mapping (party member 0).
    assert_eq!(roster[0].team, Team::Party);
    assert!(!roster[0].npc);
    assert_eq!(roster[0].hp_current, 20);
    assert_eq!(roster[0].hp_max, 22);
    assert_eq!(roster[0].ac, 54);
    assert_eq!(roster[0].hit_bonus, 50);
    assert_eq!(roster[0].hit_dice, 1);
    assert_eq!(roster[0].movement, 12);
    assert_eq!(roster[0].attacks_count, 2);
    assert_eq!(roster[0].dice_size, 8);
    assert_eq!(
        roster[0].reaction_adj,
        flavor.dex_reaction_bonus(16) as i8,
        "reaction_adj derived from DEX 16 via the flavor"
    );
    // Monsters are NPCs (per control_morale).
    assert!(roster[2].npc);
    assert_eq!(roster[2].team, Team::Monster);

    // Drive the fight; the first five draws are the initiative d6s (one per
    // combatant), then the d100 selection pass begins — no setup draw leaks in.
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    let mut state = state;
    let _ = state.run_combat(&mut rng, DEFAULT_NO_ACTION_LIMIT);
    let ns = log.ns();
    assert!(ns.len() >= 6, "the fight drew from the PRNG");
    for (i, n) in ns.iter().take(5).enumerate() {
        assert_eq!(*n, 6, "draw #{i} must be an initiative d6");
    }
    assert_eq!(ns[5], 100, "the d100 selection pass follows the 5 d6s");
}

/// The memorized-spell candidate list + casting-level decode (doc §41.1/§41.2).
/// A Magic-User 5 record with one memorized Magic Missile in the back slot
/// (`spell_list[83]` @ `0x1E + 83 = 0x71` — the pack-from-back layout, doc §33)
/// decodes to `memorized_list == [0x0F]`, `skill_level_magic_user == 5`, and NOT
/// the no-caster fallback — the caster-bar PHILIPPE shape.
#[test]
fn caster_record_decodes_memorized_list_and_casting_level() {
    use gbx_rules::adnd1::flavor_impl::Adnd1;
    use gbx_rules::pack::RuleSet;
    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);

    let mut r = synthetic_record(b"MAGE", 12, 12, 46, 15, 15, 1, 12, false, 2, (1, 4, 0));
    r[0x71] = 0x0F; // spell_list[83] = Magic Missile (packs from the back)
    r[0x1E] = 0x0F; // slot 0 (never read by the 1-based loop) — must be ignored
    r[0x109 + 5] = 5; // ClassLevel[MagicUser] = 5

    let entries = vec![RecordCombatant {
        team: Team::Party,
        pos: GridPos::new(23, 11),
        record: &r,
    }];
    let state = combat_state_from_records(&entries, CombatMap::uniform(0x17), &flavor).unwrap();
    let c = &state.fighters[0];
    assert_eq!(
        c.memorized_list,
        vec![0x0F],
        "one MM collected; slot 0 @0x1E ignored (the loop is 1-based)"
    );
    assert_eq!(c.skill_level_magic_user, 5);
    assert_eq!(c.skill_level_ranger, 0);
    assert!(
        !c.caster_no_class,
        "a real MU 5 is not the no-caster fallback"
    );

    // A non-caster (all class levels 0) is the fallback → casting level 6.
    let r0 = synthetic_record(b"THUG", 8, 8, 40, 12, 10, 1, 9, true, 2, (1, 6, 0));
    let entries0 = vec![RecordCombatant {
        team: Team::Monster,
        pos: GridPos::new(30, 11),
        record: &r0,
    }];
    let state0 = combat_state_from_records(&entries0, CombatMap::uniform(0x17), &flavor).unwrap();
    assert!(state0.fighters[0].caster_no_class);
    assert!(state0.fighters[0].memorized_list.is_empty());
}
