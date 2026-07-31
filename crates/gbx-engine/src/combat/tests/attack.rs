use super::*;

// === the armed/ranged slice (doc §34) test support + units ==============

/// A one-combatant state with `primary_type` readied over the synthetic
/// table; `attacks_count` seeds the melee half-action count. `ammo` sets the
/// launcher ammo.
fn ranged_state(primary_type: u8, attacks_count: u8, ammo: i32) -> CombatState {
    let mut c = Combatant::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(0, 0),
        10,
        40,
        0,
        12,
        (1, 6, 0),
        5,
        2,
    );
    c.attacks_count = attacks_count;
    let mut state = CombatState::new(CombatMap::uniform(0x17), vec![c]);
    state.item_data = Some(synth_item_table());
    state.set_loadout(
        0,
        Loadout {
            ranged: Some((primary_type, 0)),
            ammo_count: ammo,
            ammo_readied: true,
            melee: None,
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: true,
        },
    );
    state
}

#[test]
fn ranged_predicate_and_current_attack_item() {
    let mut state = ranged_state(43, 2, 40); // LongBow
    assert!(state.is_weapon_ranged(0));
    assert!(!state.is_weapon_ranged_melee(0)); // bow has no melee/flag_10
    let it = state.get_current_attack_item(0);
    assert!(it.found);
    assert_eq!(it.item, AttackItemRef::Ammo);
    assert_eq!(state.attack_item_count(0, &it), Some(40));
    // Unreadying the bow → not ranged, no attack item found.
    state.fighters[0].readied_weapon = None;
    assert!(!state.is_weapon_ranged(0));
    assert!(!state.get_current_attack_item(0).found);
    // No loadout at all → nothing ever readied → melee.
    state.fighters[0].loadout = None;
    state.fighters[0].readied_weapon = None;
    assert!(!state.is_weapon_ranged(0));
}

#[test]
fn ranged_predicate_sling_finds_null_item() {
    // Sling (flags 0x0A) "finds" a null item and still shoots (doc §34.2).
    let state = ranged_state(47, 2, 40);
    assert!(state.is_weapon_ranged(0)); // range 21 > 1
    let it = state.get_current_attack_item(0);
    assert!(it.found); // the flag_08|flag_02 == 0x0A special case
    assert_eq!(it.item, AttackItemRef::None); // no ammo item
    assert_eq!(state.attack_item_count(0, &it), None); // no ammo cap
}

#[test]
fn weapon_range_sanitizes() {
    let mut state = ranged_state(43, 2, 40); // LongBow 22 → 21
    assert_eq!(state.weapon_range(0), 21);
    // A range-1 weapon → r = 0 → sanitized to 1.
    state.set_loadout(
        0,
        Loadout {
            ranged: Some((30, 0)),
            ammo_count: 0,
            ammo_readied: true,
            melee: None,
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: true,
        },
    );
    assert_eq!(state.weapon_range(0), 1);
    // No readied weapon → 1.
    state.fighters[0].readied_weapon = None;
    assert_eq!(state.weapon_range(0), 1);
}

#[test]
fn reclac_melee_matches_this_round_action_count() {
    // No loadout: attack1_left = ThisRoundActionCount(attacksCount) — the
    // pre-slice behaviour, both parities.
    let mut c = Combatant::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(0, 0),
        10,
        40,
        0,
        12,
        (1, 6, 0),
        5,
        2,
    );
    c.attacks_count = 3;
    let mut state = CombatState::new(CombatMap::uniform(0x17), vec![c]);
    state.combat_round = 0;
    state.fighters[0].field_8 = false;
    state.reclac_attacks(0);
    assert_eq!(state.fighters[0].attack1_left, 1); // (3+0)/2
    state.combat_round = 1;
    state.fighters[0].field_8 = false;
    state.reclac_attacks(0);
    assert_eq!(state.fighters[0].attack1_left, 2); // (3+1)/2
}

#[test]
fn reclac_ranged_natk_floor_and_parity() {
    // LongBow natk 4 → 2 shots both parities ((4+0)/2, (4+1)/2 == 2).
    let mut state = ranged_state(43, 2, 40);
    state.combat_round = 0;
    state.fighters[0].field_8 = false;
    state.reclac_attacks(0);
    assert_eq!(state.fighters[0].attack1_left, 2);
    state.combat_round = 1;
    state.fighters[0].field_8 = false;
    state.reclac_attacks(0);
    assert_eq!(state.fighters[0].attack1_left, 2);
    // A natk-1 launcher floors to 2 half-actions → 1 shot even, 1 odd.
    let mut s2 = ranged_state(45, 2, 40);
    s2.combat_round = 0;
    s2.fighters[0].field_8 = false;
    s2.reclac_attacks(0);
    assert_eq!(s2.fighters[0].attack1_left, 1); // max(2,1)=2 → (2+0)/2
}

#[test]
fn reclac_ranged_ammo_cap() {
    // Ammo 1 caps the 2-shot round to 1.
    let mut state = ranged_state(43, 2, 1);
    state.combat_round = 0;
    state.fighters[0].field_8 = false;
    state.reclac_attacks(0);
    assert_eq!(state.fighters[0].attack1_left, 1);
}

#[test]
fn reclac_field_8_writeback_gate() {
    // With field_8 set (mid-turn recompute) and a ranged weapon, the gate
    // `attacks < orig` blocks a re-inflation: orig 1 < attacks 2, ranged, so
    // the count is NOT overwritten and stays at attacksCount.
    let mut state = ranged_state(43, 2, 40);
    state.combat_round = 0;
    state.fighters[0].attack1_left = 1; // orig
    state.fighters[0].field_8 = true;
    state.reclac_attacks(0);
    // gate: !field_8(F) || 2<1(F) || (T && 2<2 && !ranged=F) → F ⇒ keep the
    // attacksCount write (2) from the head of reclac.
    assert_eq!(state.fighters[0].attack1_left, 2);
}

#[test]
fn ammo_subtracts_by_swing_count_not_assigned() {
    // coab≠binary #16: the binary SUBTRACTS the attack-1 swing count from
    // `item.count`; coab assigns. Two swings from ammo 40 → 38 (not 2).
    let bowman = Combatant::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(0, 0),
        30,
        40,
        40, // hit_bonus high — swings land, but the count is what matters
        12,
        (1, 6, 0),
        5,
        2, // attack1_left = 2
    );
    let target = Combatant::new_melee(
        1,
        Team::Monster,
        true,
        GridPos::new(3, 0),
        200, // survives both swings so the loop runs fully
        40,
        0,
        12,
        (1, 2, 0),
        5,
        1,
    );
    let mut state = CombatState::new(CombatMap::uniform(0x17), vec![bowman, target]);
    state.item_data = Some(synth_item_table());
    state.set_loadout(
        0,
        Loadout {
            ranged: Some((43, 0)),
            ammo_count: 40,
            ammo_readied: true,
            melee: None,
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: true,
        },
    );
    assert_eq!(state.fighters[0].attack1_left, 2);
    let mut rng = EngineRng::new(SEED);
    state.attack_target(&mut rng, 0, 1, false, AttackItemRef::Ammo);
    assert_eq!(state.fighters[0].ammo, 38); // 40 − 2, SUBTRACT not assign
    assert!(!state.fighters[0].ammo_item_lost);
}

#[test]
fn ranged_defense_bonus_bands() {
    // LongBow (range 22) → oneThird = 7: range ≤ 7 → 0, 8..14 → +2,
    // > 14 → +5. Validate the wiring reproduces the piecewise formula over
    // `get_target_range`, and that a far target actually reaches +5.
    let mk = |tx: i32| -> CombatState {
        let bowman = Combatant::new_melee(
            0,
            Team::Party,
            false,
            GridPos::new(0, 0),
            30,
            40,
            0,
            12,
            (1, 6, 0),
            5,
            2,
        );
        let target = Combatant::new_melee(
            1,
            Team::Monster,
            true,
            GridPos::new(tx, 0),
            30,
            40,
            0,
            12,
            (1, 2, 0),
            5,
            1,
        );
        let mut state = CombatState::new(CombatMap::uniform(0x17), vec![bowman, target]);
        state.item_data = Some(synth_item_table());
        state.set_loadout(
            0,
            Loadout {
                ranged: Some((43, 0)),
                ammo_count: 40,
                ammo_readied: true,
                melee: None,
                unarmed_profile: (1, 2, 6),
                entry_ranged_readied: true,
            },
        );
        state
    };
    let band = |r: i32| -> i32 {
        let one_third = 7;
        let mut adj = 0;
        let mut rr = r;
        if rr > one_third {
            rr -= one_third;
            adj += 2;
            if rr > one_third {
                adj += 3;
            }
        }
        adj
    };
    let mut saw_plus5 = false;
    for tx in [1, 8, 20, 40] {
        let state = mk(tx);
        let r = get_target_range(&state.map, state.fighters[1].pos, state.fighters[0].pos) as i32;
        assert_eq!(state.ranged_defense_bonus(0, 1), band(r), "tx={tx} r={r}");
        if state.ranged_defense_bonus(0, 1) == 5 {
            saw_plus5 = true;
        }
    }
    assert!(saw_plus5, "a far target must reach the +5 band");
    // A non-ranged attacker (bow unreadied) → 0.
    let mut melee = mk(40);
    melee.fighters[0].readied_weapon = None;
    assert_eq!(melee.ranged_defense_bonus(0, 1), 0);
}

#[test]
fn cornered_swap_unready_then_reready() {
    // A bowman with an adjacent enemy unreadies to the unarmed profile;
    // clearing the enemy re-readies the bow and restores the entry profile.
    let bowman = Combatant::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(0, 0),
        30,
        40,
        40,
        12,
        (1, 6, 0), // entry bow profile
        5,
        2,
    );
    let patron = Combatant::new_melee(
        1,
        Team::Monster,
        true,
        GridPos::new(1, 0), // adjacent
        16,
        40,
        0,
        12,
        (1, 2, 0),
        5,
        1,
    );
    let mut state = CombatState::new(CombatMap::uniform(0x17), vec![bowman, patron]);
    state.item_data = Some(synth_item_table());
    state.set_loadout(
        0,
        Loadout {
            ranged: Some((43, 0)),
            ammo_count: 40,
            ammo_readied: true,
            melee: None,
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: true,
        },
    );
    // (`set_loadout` snapshots `entry_dice` from the live profile — no
    // hand-set needed; the re-ready below proves it.)
    assert!(state.is_weapon_ranged(0));

    // Adjacent enemy → unready to fists.
    state.ai_items_selection(0);
    assert!(state.fighters[0].readied_weapon.is_none());
    assert_eq!(
        (
            state.fighters[0].dice_count,
            state.fighters[0].dice_size,
            state.fighters[0].damage_bonus
        ),
        (1, 2, 6)
    );
    assert!(!state.is_weapon_ranged(0));

    // Clear the enemy → re-ready the bow, restore the entry profile.
    state.fighters[1].in_combat = false;
    state.rebuild_occupancy();
    state.ai_items_selection(0);
    assert!(state.fighters[0].readied_weapon.is_some());
    assert_eq!(
        (
            state.fighters[0].dice_count,
            state.fighters[0].dice_size,
            state.fighters[0].damage_bonus
        ),
        (1, 6, 0)
    );
    assert!(state.is_weapon_ranged(0));
}

#[test]
fn try_guarding_ranged_clears_never_guards() {
    // A ranged attacker never parks a guard (§34.4): clear, no guard flag.
    let mut state = ranged_state(43, 2, 40);
    state.fighters[0].delay = 5;
    state.try_guarding(0);
    assert!(!state.fighters[0].guarding);
    assert_eq!(state.fighters[0].delay, 0);
    // Unreadied (melee) with delay > 0 → guards as before.
    state.fighters[0].readied_weapon = None;
    state.fighters[0].delay = 5;
    state.try_guarding(0);
    assert!(state.fighters[0].guarding);
}

// === the §48 melee candidate + item-plus recompute ======================

/// A two-combatant state: [0] a party PC at (0,0) with str bonuses and a
/// kit loadout, [1] a monster at `enemy_pos`. `thac0` 12 so the recompute's
/// terms are visible.
fn kit_state(loadout: Loadout, race: u8, enemy_pos: GridPos) -> CombatState {
    let mut pc = Combatant::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(0, 0),
        10,
        40,
        15,
        12,
        (1, 2, 6),
        5,
        2,
    );
    pc.thac0 = 12;
    pc.str_hit_bonus = 3;
    pc.str_dmg_bonus = 6;
    pc.race = race;
    pc.base_dice = (1, 2, 0); // bare-hands base (bonus cell 0 — str rides on top)
    let foe = Combatant::new_melee(
        1,
        Team::Monster,
        true,
        enemy_pos,
        10,
        40,
        0,
        12,
        (1, 8, 0),
        5,
        2,
    );
    let mut state = CombatState::new(CombatMap::uniform(0x17), vec![pc, foe]);
    state.item_data = Some(synth_item_table());
    state.set_loadout(0, loadout);
    state
}

#[test]
fn melee_ready_recompute_adds_table_str_and_plus_terms() {
    // A melee-only kit (sword +2, no ranged): AI_items_selection readies the
    // sword whatever the adjacency, and the sub_66023 recompute installs
    // table dice 1d8, damage = table 0 + str 6 + plus 2, hit = thac0 12 +
    // str 3 + plus 2 (human: no elf rider).
    let mut s = kit_state(
        Loadout {
            ranged: None,
            ammo_count: 0,
            ammo_readied: true,
            melee: Some((36, 2)),
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: false,
        },
        7,
        GridPos::new(1, 0),
    );
    // Entry state: nothing readied, the record profile stands.
    assert_eq!(s.fighters[0].readied_weapon, None);
    s.ai_items_selection(0);
    assert_eq!(s.fighters[0].readied_weapon, Some((36, 2)));
    let f = &s.fighters[0];
    assert_eq!((f.dice_count, f.dice_size, f.damage_bonus), (1, 8, 8));
    assert_eq!(f.hit_bonus, 17);
}

#[test]
fn elf_rider_bumps_hit_only() {
    // Same kit on an ELF (race 2): the sub_66023 rider (@ovr025:019E-01CD)
    // adds +1 to the HIT bonus only — damage is written before the bump.
    let mut s = kit_state(
        Loadout {
            ranged: None,
            ammo_count: 0,
            ammo_readied: true,
            melee: Some((36, 2)),
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: false,
        },
        2,
        GridPos::new(1, 0),
    );
    s.ai_items_selection(0);
    let f = &s.fighters[0];
    assert_eq!(f.damage_bonus, 8, "damage takes plus but NOT the elf rider");
    assert_eq!(f.hit_bonus, 18, "hit takes plus AND the elf rider");
}

#[test]
fn two_candidate_selection_prefers_sword_when_adjacent_bow_when_clear() {
    // Bow + sword +1 kit (the slot-H MATHEW shape): adjacent enemy → the
    // MELEE candidate wins (not bare hands); enemy far → the bow wins.
    let kit = Loadout {
        ranged: Some((43, 0)),
        ammo_count: 40,
        ammo_readied: true,
        melee: Some((36, 1)),
        unarmed_profile: (1, 2, 6),
        entry_ranged_readied: false,
    };
    let mut adjacent = kit_state(kit, 7, GridPos::new(1, 0));
    adjacent.ai_items_selection(0);
    assert_eq!(adjacent.fighters[0].readied_weapon, Some((36, 1)));
    assert!(!adjacent.is_weapon_ranged(0));

    let mut clear = kit_state(kit, 7, GridPos::new(10, 10));
    clear.ai_items_selection(0);
    assert_eq!(clear.fighters[0].readied_weapon, Some((43, 0)));
    assert!(clear.is_weapon_ranged(0));
}

#[test]
fn weak_melee_candidate_loses_to_the_base_profile() {
    // var_8 must BEAT var_16 (the bare-hands base rating): a plain type-30
    // d8 (rating 8+3=11) loses to a base whose serialized profile rates
    // higher, so the PC stays bare-handed on the unarmed profile.
    let mut s = kit_state(
        Loadout {
            ranged: None,
            ammo_count: 0,
            ammo_readied: true,
            melee: Some((30, 0)),
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: false,
        },
        7,
        GridPos::new(1, 0),
    );
    // Base rating: ds*dc + 2*bonus(if>0) over the BASE cells — make the base
    // profile beat rating 11 (e.g. serialized 1d6+3 → 6 + 6 = 12).
    s.fighters[0].base_dice = (1, 6, 3);
    s.ai_items_selection(0);
    assert_eq!(s.fighters[0].readied_weapon, None);
    let f = &s.fighters[0];
    assert_eq!((f.dice_count, f.dice_size, f.damage_bonus), (1, 2, 6));
    assert_eq!(f.hit_bonus, 15, "bare hands: thac0 + strengthHitBonus");
}

// === the held-target slice (doc §49) =====================================

/// A melee pair: attacker [0] adjacent to target [1]; the target carries
/// `paralyze` 0x34 when `held`.
fn held_pair(held: bool) -> CombatState {
    let attacker = Combatant::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(0, 0),
        30,
        40,
        10,
        12,
        (1, 8, 1),
        5,
        1,
    );
    let mut target = Combatant::new_melee(
        1,
        Team::Monster,
        true,
        GridPos::new(1, 0),
        12,
        40,
        0,
        12,
        (1, 2, 0),
        5,
        1,
    );
    if held {
        target.add_affect(0x34, 5, 1, false); // paralyze
    }
    CombatState::new(CombatMap::uniform(0x17), vec![attacker, target])
}

#[test]
fn held_target_is_slain_draw_free() {
    // `sub_3F4EB` head (@`ovr014:152C-15E0`, doc §49): IsHeld(target) short-
    // circuits the swing loop — damage = hp_current + 5 (a guaranteed kill),
    // both attacks-left cells zero, turn complete, and NOT ONE die drawn.
    // Capture-proven: cleric-guildwar's MARK kills the held [21] at draw 519
    // between a d1 near-pick and the next pick-scan d100.
    let mut s = held_pair(true);
    let mut rng = EngineRng::new(SEED);
    let before = rng.state();
    let complete = s.attack_target(&mut rng, 0, 1, false, AttackItemRef::None);
    assert_eq!(rng.state(), before, "the slay is draw-free");
    assert!(complete);
    assert_eq!(s.fighters[1].hp_current, 0);
    assert!(!s.fighters[1].in_combat, "hp 12 − (12+5) → dead");
    assert_eq!(s.fighters[0].attack1_left, 0);
    assert_eq!(s.fighters[0].attack2_left, 0);
    // The same pair un-held rolls a d20 (control: the branch is held-gated).
    let mut s2 = held_pair(false);
    let mut rng2 = EngineRng::new(SEED);
    let before2 = rng2.state();
    s2.attack_target(&mut rng2, 0, 1, false, AttackItemRef::None);
    assert_ne!(
        rng2.state(),
        before2,
        "a live target still draws the swing d20"
    );
}

#[test]
fn restrained_turn_head_clears_actions() {
    // `sub_3A071` = `clear_actions` for the restrained family (coab
    // affect_table, ovr013.cs:1840-1842), fired by the turn-head
    // `CheckAffectsEffect(PlayerRestrained)` (`sub_33281`, ovr009.cs:108):
    // the held combatant's delay zeroes, so the `delay > 0` turn body is
    // skipped entirely — the draw-free held turn (doc §49).
    let mut s = held_pair(true);
    s.fighters[1].delay = 7;
    s.check_affects_effect(1, CheckType::PlayerRestrained);
    assert_eq!(s.fighters[1].delay, 0, "paralyze → clear_actions → no turn");
    // A combatant without a restrained affect keeps its delay.
    let mut s2 = held_pair(false);
    s2.fighters[1].delay = 7;
    s2.check_affects_effect(1, CheckType::PlayerRestrained);
    assert_eq!(s2.fighters[1].delay, 7);
}

#[test]
fn held_combatant_makes_no_departure_swing() {
    // `sub_3E954`'s per-candidate IsHeld filter (@`ovr014:0B14-0B1B`, doc
    // §49): a held enemy adjacent to the mover takes no opportunity swing
    // when the mover departs its reach.
    // Mid-board pair so a single westward step actually leaves reach:
    // mover [0]@(5,5), enemy [1]@(6,5); step W → (4,5), distance 2.
    let place = |s: &mut CombatState| {
        s.fighters[0].pos = GridPos::new(5, 5);
        s.fighters[1].pos = GridPos::new(6, 5);
        s.rebuild_occupancy();
        s.fighters[1].delay = 0;
        s.fighters[1].attacks_received = 0; // qualifies without the cone scan
    };
    let mut s = held_pair(true);
    place(&mut s);
    let mut rng = EngineRng::new(SEED);
    let before = rng.state();
    s.move_step_away_attack(&mut rng, 0, 6);
    assert_eq!(rng.state(), before, "held enemy: no departure d20");
    // Control: un-held, the departure swing draws.
    let mut s2 = held_pair(false);
    place(&mut s2);
    let mut rng2 = EngineRng::new(SEED);
    let b2 = rng2.state();
    s2.move_step_away_attack(&mut rng2, 0, 6);
    assert_ne!(rng2.state(), b2, "live enemy: departure swing fires");
}

#[test]
fn vs_large_target_with_readied_weapon_trips() {
    // `sub_3F4EB` @`ovr014:15E3-1662` (§49 residue): a readied weapon vs a
    // size>1 / field_DE>0x80 target swaps in the ITEMS LARGE damage cells —
    // unmodeled, tripwired. Bare hands (no readied weapon) never trip.
    let log = ActionLog::default();
    let mut s = held_pair(false);
    s.fighters[1].size = 2;
    s.fighters[1].field_de = 0x82;
    s.fighters[0].readied_weapon = Some((36, 1));
    s.attach_action_sink(log.sink());
    let mut rng = EngineRng::new(SEED);
    s.attack_target(&mut rng, 0, 1, false, AttackItemRef::None);
    let tripped = log.events().iter().any(|e| {
        matches!(
            e,
            ActionEvent::StubTripped {
                stub: "vs-large-dice",
                ..
            }
        )
    });
    assert!(
        tripped,
        "armed swing at a size-2 target flags the LARGE-dice territory"
    );
}
