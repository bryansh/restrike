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
            primary_type,
            ammo_count: ammo,
            unarmed_profile: (1, 2, 6),
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
    state.fighters[0].weapon_readied = false;
    assert!(!state.is_weapon_ranged(0));
    assert!(!state.get_current_attack_item(0).found);
    // No loadout at all → melee.
    state.fighters[0].loadout = None;
    state.fighters[0].weapon_readied = true;
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
            primary_type: 30,
            ammo_count: 0,
            unarmed_profile: (1, 2, 6),
        },
    );
    assert_eq!(state.weapon_range(0), 1);
    // No readied weapon → 1.
    state.fighters[0].weapon_readied = false;
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
            primary_type: 43,
            ammo_count: 40,
            unarmed_profile: (1, 2, 6),
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
                primary_type: 43,
                ammo_count: 40,
                unarmed_profile: (1, 2, 6),
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
    melee.fighters[0].weapon_readied = false;
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
            primary_type: 43,
            ammo_count: 40,
            unarmed_profile: (1, 2, 6),
        },
    );
    // (`set_loadout` snapshots `entry_dice` from the live profile — no
    // hand-set needed; the re-ready below proves it.)
    assert!(state.is_weapon_ranged(0));

    // Adjacent enemy → unready to fists.
    state.ai_items_selection(0);
    assert!(!state.fighters[0].weapon_readied);
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
    assert!(state.fighters[0].weapon_readied);
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
    state.fighters[0].weapon_readied = false;
    state.fighters[0].delay = 5;
    state.try_guarding(0);
    assert!(state.fighters[0].guarding);
}
