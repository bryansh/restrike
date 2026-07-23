use super::*;

// --- pure selection logic (the two-if tie-break) -----------------------

#[test]
fn selection_picks_highest_delay() {
    // delay 8 beats delay 5 regardless of rolls.
    assert_eq!(select_combatant(&[5, 8, 3], &[99, 1, 50]), Some((1, 1)));
}

#[test]
fn selection_breaks_ties_by_highest_roll() {
    // All delay 5: highest roll (index 2) wins.
    assert_eq!(select_combatant(&[5, 5, 5], &[30, 20, 50]), Some((2, 50)));
    // Equal rolls at the max: the later member wins (`>=` overwrite).
    assert_eq!(select_combatant(&[5, 5], &[40, 40]), Some((1, 40)));
}

#[test]
fn selection_exercises_the_gt_only_branch_reset() {
    // The `>`-only branch (first if) resets max_roll so a strictly-higher
    // delay wins even with a LOWER roll than the running max. Without the
    // reset, index 1 (roll 10 < 90) would fail the second if and index 0
    // would wrongly win.
    assert_eq!(select_combatant(&[5, 8], &[90, 10]), Some((1, 10)));
    // Three-way: A(5,90) then B(8,10) then C(8,50) → C (delay 8, higher roll).
    assert_eq!(select_combatant(&[5, 8, 8], &[90, 10, 50]), Some((2, 50)));
}

#[test]
fn selection_ends_when_all_delays_zero() {
    assert_eq!(select_combatant(&[0, 0, 0], &[99, 50, 1]), None);
    // A transient delay-0 pick is nulled out by the max_delay==0 guard.
    assert_eq!(select_combatant(&[0], &[100]), None);
}

// --- initiative draw sequence ------------------------------------------

#[test]
fn initiative_draws_one_d6_per_in_combat_combatant_in_roster_order() {
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());

    // Mixed in_combat: members 0,1,3 in combat; member 2 out.
    let roster = vec![
        party(0, 3),                              // dex reaction +3
        party(1, -2),                             // dex reaction -2
        Combatant::new(2, Team::Party, 0, false), // not in combat → no d6
        monster(3),                               // reaction 0
    ];
    let mut state = CombatState::initiative_only(roster);

    let step = state.step(&mut rng);
    assert_eq!(step, CombatStep::RoundStarted { round: 0 });

    // Exactly three d6 draws, in order, for the three in-combat members.
    assert_eq!(log.ns(), vec![6, 6, 6]);

    // Delays match a by-hand replay of the same seed.
    let mut oracle = Replay::new(SEED);
    let d0 = oracle.roll(6);
    let d1 = oracle.roll(6);
    let d3 = oracle.roll(6);
    assert_eq!(state.roster()[0].delay, clamp_init(d0, 3));
    assert_eq!(state.roster()[1].delay, clamp_init(d1, -2));
    assert_eq!(state.roster()[2].delay, 0, "not in combat");
    assert_eq!(state.roster()[3].delay, clamp_init(d3, 0));
}

#[test]
fn surprise_subtracts_six_after_the_min_one_clamp() {
    // reaction -3, d6 min 1 → pre-surprise delay clamps up to 1, then -6 →
    // -5 → out of range → 0. Prove the clamp-then-subtract ordering.
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    let actions = ActionLog::default();

    // Party is bit (0+1)=1 → surprise_mask bit 0 set. A monster is needed
    // for the fight to actually start (the emptiness guard).
    let mut state =
        CombatState::initiative_only(vec![party(0, -3), monster(9)]).with_surprise_mask(0b01);
    state.attach_action_sink(actions.sink());
    state.step(&mut rng);

    // Whatever the d6 (1..6), with reaction -3 the pre-surprise value is in
    // 1..3 (after the min-1 clamp), minus 6 is negative → 0.
    assert_eq!(state.roster()[0].delay, 0);
    match actions.events()[0] {
        ActionEvent::Init {
            combatant_id,
            delay,
            dex_adj,
            surprise,
        } => {
            assert_eq!((combatant_id, delay, dex_adj, surprise), (0, 0, -3, true));
        }
        other => panic!("expected Init, got {other:?}"),
    }
}

#[test]
fn dex_reaction_bonus_comes_from_the_flavor_not_a_hardcode() {
    use gbx_rules::adnd1::flavor_impl::Adnd1;
    use gbx_rules::pack::RuleSet;
    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    // 18 DEX → +3 reaction (ovr025.cs:551-553), sourced through gbx-rules.
    let c = Combatant::from_dex(0, Team::Party, 18, true, &flavor);
    assert_eq!(c.reaction_adj, 3);
    let c = Combatant::from_dex(1, Team::Party, 3, true, &flavor);
    assert_eq!(c.reaction_adj, -3); // dex 3 → 3-6 = -3
}

// --- per-pass d100 burst = roster size ---------------------------------

#[test]
fn every_selection_pass_draws_exactly_one_d100_per_roster_member() {
    // A 16-combatant roster — the §15 live signature: bursts of exactly 16.
    // (In the real game turns interleave their own draws, splitting the raw
    // stream into separate 16-runs; here the stub draws nothing between
    // passes, so we assert the count PER pass directly.)
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());

    let mut roster = Vec::new();
    for id in 0..6 {
        roster.push(party(id, 0));
    }
    for id in 6..16 {
        roster.push(monster(id));
    }
    assert_eq!(roster.len(), 16);
    let mut state = CombatState::initiative_only(roster);

    // RoundStarted step: 16 d6 (all in combat).
    let mut before = log.len();
    assert_eq!(state.step(&mut rng), CombatStep::RoundStarted { round: 0 });
    assert_eq!(
        log.ns()[before..],
        [6u16; 16],
        "one d6 per in-combat member"
    );

    // Every subsequent step (each Turn, and the terminating RoundEnded)
    // consumes exactly 16 d100 draws.
    loop {
        before = log.len();
        let step = state.step(&mut rng);
        let burst = &log.ns()[before..];
        assert_eq!(
            burst.len(),
            16,
            "each selection pass rolls one d100 per member"
        );
        assert!(burst.iter().all(|&n| n == 100), "the burst is all d100s");
        match step {
            CombatStep::Turn { .. } => continue,
            CombatStep::RoundEnded { .. } => break,
            other => panic!("unexpected step {other:?}"),
        }
    }
}

// --- whole-round draw total --------------------------------------------

#[test]
fn a_round_draws_kc_d6_then_a_plus_one_times_k_d100() {
    // K = 4, all in combat, reaction 0 → every d6 gives delay 1..6 > 0, so
    // all A = 4 act: 4 d6 + (4+1)*4 = 4 + 20 = 24 draws.
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());

    let roster = vec![party(0, 0), party(1, 0), monster(2), monster(3)];
    let k = roster.len();
    let mut state = CombatState::initiative_only(roster);

    let mut turns = 0;
    loop {
        match state.step(&mut rng) {
            CombatStep::RoundStarted { .. } => {}
            CombatStep::Turn { .. } => turns += 1,
            CombatStep::RoundEnded { round, .. } => {
                assert_eq!(round, 1);
                break;
            }
            CombatStep::Ended => panic!("ended mid-round"),
        }
    }
    assert_eq!(turns, k, "every in-combat member with delay>0 acts once");
    // K_c d6 + (A+1)*K d100.
    assert_eq!(log.len(), k + (turns + 1) * k);
    assert_eq!(log.len(), 4 + 5 * 4);
}

// --- pick events + tie-break through the real state machine ------------

#[test]
fn pick_events_track_selection_order_and_zero_the_picked_delay() {
    let mut rng = EngineRng::new(SEED);
    let actions = ActionLog::default();
    let roster = vec![party(0, 0), party(1, 0), monster(2)];
    let mut state = CombatState::initiative_only(roster);
    state.attach_action_sink(actions.sink());

    let mut picks = Vec::new();
    loop {
        match state.step(&mut rng) {
            CombatStep::RoundStarted { .. } => {}
            CombatStep::Turn { combatant_id } => picks.push(combatant_id),
            CombatStep::RoundEnded { .. } => break,
            CombatStep::Ended => panic!("ended mid-round"),
        }
    }

    // Every in-combat member is picked exactly once (each acts, then zeroed).
    let mut sorted = picks.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![0, 1, 2]);

    // The action log holds 3 Init then one Pick per selection, in order, and
    // the pass indices ascend from 0.
    let events = actions.events();
    let inits = events
        .iter()
        .filter(|e| matches!(e, ActionEvent::Init { .. }))
        .count();
    assert_eq!(inits, 3);
    let pick_events: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ActionEvent::Pick {
                pass, combatant_id, ..
            } => Some((*pass, *combatant_id)),
            _ => None,
        })
        .collect();
    assert_eq!(pick_events.len(), 3);
    for (i, (pass, id)) in pick_events.iter().enumerate() {
        assert_eq!(*pass as usize, i, "pass index ascends from 0");
        assert_eq!(*id, picks[i], "pick event matches yielded combatant");
    }
}

// --- termination --------------------------------------------------------

#[test]
fn combat_terminates_at_the_stalemate_cap() {
    // Nobody dies in the stub, so the only terminator is combat_round >= 15.
    let mut rng = EngineRng::new(SEED);
    let mut state = CombatState::initiative_only(vec![party(0, 0), monster(1)]);

    let mut rounds_ended = 0;
    let final_step = loop {
        match state.step(&mut rng) {
            CombatStep::RoundEnded {
                battle_over: true, ..
            } => {
                rounds_ended += 1;
                break CombatStep::Ended;
            }
            CombatStep::RoundEnded { .. } => rounds_ended += 1,
            CombatStep::Ended => break CombatStep::Ended,
            _ => {}
        }
    };
    assert_eq!(final_step, CombatStep::Ended);
    assert_eq!(rounds_ended, DEFAULT_NO_ACTION_LIMIT);
    assert_eq!(state.combat_round(), DEFAULT_NO_ACTION_LIMIT);
    assert_eq!(state.step(&mut rng), CombatStep::Ended, "stays ended");
}

#[test]
fn empty_side_ends_before_any_draw() {
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    // No monsters → no fight.
    let mut state = CombatState::initiative_only(vec![party(0, 0), party(1, 0)]);
    assert_eq!(state.step(&mut rng), CombatStep::Ended);
    assert_eq!(log.len(), 0, "the emptiness guard draws nothing");
}

// --- roll_dice byte truncation (FD-29) ---------------------------------

#[test]
fn roll_dice_truncates_the_total_to_a_byte() {
    // 100 dice of d100: the untruncated total blows past 255, so the
    // (byte)roll_total truncation (ovr024.cs:595) is observable — the
    // data-driven FD-29 clause. Our roll_dice must wrap mod 256.
    let mut rng = EngineRng::new(SEED);
    let got = roll_dice(&mut rng, 100, 100);

    let mut o = Replay::new(SEED);
    let mut full = 0u32;
    for _ in 0..100 {
        full += o.roll(100) as u32;
    }
    assert!(full > 255, "the untruncated total must exceed a byte");
    assert_eq!(got, (full as u8) as u16, "roll_dice truncates to a byte");
    // A total under 256 is unaffected (the initiative d6/d100 case).
    let mut rng = EngineRng::new(SEED);
    let small = roll_dice(&mut rng, 6, 3); // max 18
    let mut o = Replay::new(SEED);
    assert_eq!(small, o.roll(6) + o.roll(6) + o.roll(6));
}

// --- to-hit: both paths, the auto-rules, and the >/>= boundary ---------

#[test]
fn to_hit_natural_1_misses_and_natural_20_hits_via_the_100_promotion() {
    // AC 50 with 0 bonus: a plain roll (effective ≤ 19) can never reach it,
    // but a nat-20 promotes to 100 and clears it. A nat-1 misses (the gate).
    let mut rng = EngineRng::new(SEED);
    let (mut saw1, mut saw20, mut saw_plain) = (false, false, false);
    for _ in 0..2000 {
        let r = pc_can_hit_target(&mut rng, 50, 0, 0);
        match r.d20 {
            1 => {
                assert!(!r.hit, "nat-1 auto-miss");
                saw1 = true;
            }
            20 => {
                assert!(r.hit, "nat-20 → 100 beats AC 50");
                saw20 = true;
            }
            d => {
                assert!((2..=19).contains(&d));
                assert!(!r.hit, "a plain d20 can't reach AC 50 with 0 bonus");
                saw_plain = true;
            }
        }
        if saw1 && saw20 && saw_plain {
            break;
        }
    }
    assert!(
        saw1 && saw20 && saw_plain,
        "expected a nat-1, a nat-20, and a plain roll within budget"
    );
}

#[test]
fn natural_1_misses_even_when_it_would_otherwise_certainly_hit() {
    // AC 0, 0 bonus: every non-1 roll hits (>= path, effective ≥ 2 ≥ 0);
    // only the nat-1 gate produces a miss.
    let mut rng = EngineRng::new(SEED);
    let mut saw1 = false;
    for _ in 0..2000 {
        let r = pc_can_hit_target(&mut rng, 0, 0, 0);
        if r.d20 == 1 {
            assert!(!r.hit, "nat-1 overrides an otherwise-certain hit");
            saw1 = true;
            break;
        }
        assert!(r.hit, "any non-1 vs raw AC 0 hits under >=");
    }
    assert!(saw1, "expected a nat-1 within budget");
}

#[test]
fn gt_path_and_ge_path_disagree_at_the_equality_point() {
    // The single load-bearing asymmetry (study §14.4): at the exact equality
    // point, the weapon path (PC_CanHitTarget, >=) HITS while the scripted
    // path (CanHitTarget, >) MISSES — for the *same* d20.
    let d20 = Replay::new(SEED).roll(20);
    assert!(
        (2..=19).contains(&d20),
        "this boundary test needs the seed's first d20 to be a plain roll (got {d20})"
    );
    // effective(=d20) + bonus(0) == target_ac exactly.
    let target_ac = d20 as u8;

    let mut rng = EngineRng::new(SEED);
    let ge = pc_can_hit_target(&mut rng, target_ac, 0, 0);
    assert_eq!(ge.d20 as u16, d20);
    assert!(ge.hit, "PC_CanHitTarget uses >=, so equality hits");

    let mut rng = EngineRng::new(SEED);
    let gt = can_hit_target(&mut rng, 0, target_ac);
    assert_eq!(gt.d20 as u16, d20);
    assert!(!gt.hit, "CanHitTarget uses strict >, so equality misses");
}

#[test]
fn to_hit_draws_exactly_one_d20() {
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    pc_can_hit_target(&mut rng, 40, 5, 1);
    can_hit_target(&mut rng, 3, 40);
    assert_eq!(log.ns(), vec![20, 20], "one d20 per to-hit, no more");
}

// --- damage: dice + bonus, clamp, backstab, exact draw count -----------

#[test]
fn damage_is_dice_plus_bonus_with_exact_draw_count() {
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());

    let dmg = roll_damage(&mut rng, 8, 3, 2, None); // 3d8+2
    assert_eq!(log.ns(), vec![8, 8, 8], "exactly dice_count draws");

    let mut o = Replay::new(SEED);
    let base = o.roll(8) + o.roll(8) + o.roll(8) + 2;
    assert_eq!(dmg.amount, base as i32);
    assert!(!dmg.backstab);
}

#[test]
fn damage_applies_the_backstab_multiplier() {
    let mut rng = EngineRng::new(SEED);
    let dmg = roll_damage(&mut rng, 4, 2, 1, Some(3)); // (2d4+1) × 3
    let mut o = Replay::new(SEED);
    let base = o.roll(4) + o.roll(4) + 1;
    assert_eq!(dmg.amount, base as i32 * 3);
    assert!(dmg.backstab);
}

#[test]
fn backstab_multiplier_matches_the_thief_level_bands() {
    // ((level - 1) / 4) + 2, truncating.
    assert_eq!(backstab_multiplier(1), 2);
    assert_eq!(backstab_multiplier(4), 2);
    assert_eq!(backstab_multiplier(5), 3);
    assert_eq!(backstab_multiplier(8), 3);
    assert_eq!(backstab_multiplier(9), 4);
    assert_eq!(backstab_multiplier(13), 5);
}

#[test]
fn damage_clamp_and_byte_bonus_quirk() {
    // The sbyte→byte reinterpret of attack1's bonus (Player.cs:690): a
    // "negative" bonus passed as the byte the accessor yields (e.g. -1 → 255)
    // is added as 255, never clamped — the faithful quirk. Damage stays >= 0.
    let mut rng = EngineRng::new(SEED);
    let dmg = roll_damage(&mut rng, 1, 1, 255, None); // d1 (=1) + 255
    assert_eq!(dmg.amount, 1 + 255);
}

// --- saving throws ------------------------------------------------------

#[test]
fn saving_throw_nat1_fails_nat20_succeeds_else_compares() {
    let mut rng = EngineRng::new(SEED);
    let (mut saw1, mut saw20, mut saw_plain) = (false, false, false);
    for _ in 0..2000 {
        let s = roll_saving_throw(&mut rng, 0, 0, 11); // target 11, no bonus
        match s.d20 {
            1 => {
                assert!(!s.made, "nat-1 always fails");
                saw1 = true;
            }
            20 => {
                assert!(s.made, "nat-20 always succeeds");
                saw20 = true;
            }
            d => {
                assert_eq!(s.made, d as i32 >= 11, "plain roll compares vs target");
                saw_plain = true;
            }
        }
        if saw1 && saw20 && saw_plain {
            break;
        }
    }
    assert!(saw1 && saw20 && saw_plain);
}

#[test]
fn saving_throw_applies_bonus_and_field_186() {
    let mut rng = EngineRng::new(SEED);
    for _ in 0..200 {
        let s = roll_saving_throw(&mut rng, 3, -1, 15);
        if (2..=19).contains(&s.d20) {
            assert_eq!(s.made, (s.d20 as i32 + 3 - 1) >= 15);
        }
    }
}

// --- resolve_attack: the full to-hit → damage tie, draw-faithful -------

#[test]
fn resolve_attack_hit_draws_d20_then_damage_and_emits_both_events() {
    assert!(
        Replay::new(SEED).roll(20) > 1,
        "the hit case needs the seed's first d20 to not be a nat-1"
    );

    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    let actions = ActionLog::default();
    let mut sink = actions.sink();

    // AC 0 + hitBonus 40: the first roll (>1) certainly hits.
    let p = AttackProfile {
        attacker_id: 2,
        target_id: 7,
        target_ac: 0,
        hit_bonus: 40,
        team_bonus: 0,
        dice_size: 6,
        dice_count: 2,
        damage_bonus: 1,
        backstab: None,
    };
    let out = resolve_attack(&mut rng, p, Some(&mut *sink));
    assert!(out.to_hit.hit);

    // Exactly: one d20, then two d6 (damage) — the hit-branch draw shape.
    assert_eq!(log.ns(), vec![20, 6, 6]);

    let mut o = Replay::new(SEED);
    let d20 = o.roll(20);
    let dmg = o.roll(6) + o.roll(6) + 1;
    assert_eq!(out.to_hit.d20 as u16, d20);
    assert_eq!(out.damage.unwrap().amount, dmg as i32);

    let ev = actions.events();
    assert_eq!(ev.len(), 2, "Attack then Dmg");
    assert!(matches!(
        ev[0],
        ActionEvent::Attack {
            attacker_id: 2,
            target_id: 7,
            hit: true,
            ..
        }
    ));
    assert!(matches!(
        ev[1],
        ActionEvent::Dmg {
            attacker_id: 2,
            target_id: 7,
            backstab: false,
            ..
        }
    ));
}

#[test]
fn resolve_attack_miss_draws_only_the_d20_and_emits_no_dmg() {
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    let actions = ActionLog::default();
    let mut sink = actions.sink();

    // AC 200 is unreachable even by a nat-20 (→100), so every roll misses.
    let p = AttackProfile {
        attacker_id: 0,
        target_id: 1,
        target_ac: 200,
        hit_bonus: 0,
        team_bonus: 0,
        dice_size: 8,
        dice_count: 3,
        damage_bonus: 5,
        backstab: None,
    };
    let out = resolve_attack(&mut rng, p, Some(&mut *sink));
    assert!(!out.to_hit.hit);
    assert!(out.damage.is_none());
    assert_eq!(log.ns(), vec![20], "a miss draws no damage dice");

    let ev = actions.events();
    assert_eq!(ev.len(), 1);
    assert!(matches!(ev[0], ActionEvent::Attack { hit: false, .. }));
}

#[test]
fn resolve_attack_works_without_a_sink() {
    let mut rng = EngineRng::new(SEED);
    let p = AttackProfile {
        attacker_id: 0,
        target_id: 1,
        target_ac: 0,
        hit_bonus: 40,
        team_bonus: 0,
        dice_size: 4,
        dice_count: 1,
        damage_bonus: 0,
        backstab: Some(backstab_multiplier(5)), // ×3
    };
    let out = resolve_attack(&mut rng, p, None);
    assert!(out.to_hit.hit);
    let mut o = Replay::new(SEED);
    let _d20 = o.roll(20);
    let dice = o.roll(4);
    assert_eq!(out.damage.unwrap().amount, dice as i32 * 3);
    assert!(out.damage.unwrap().backstab);
}

// === tactical battlefield (M4 combat #3) ==============================

const WALL_TILE: u8 = 1; // BACKGROUND_MOVE_COST[1] == 0xFF

// --- map & passability -------------------------------------------------

#[test]
fn map_dimensions_are_50_by_25() {
    assert_eq!((MAP_W, MAP_H), (50, 25));
    assert_eq!(BACKGROUND_MOVE_COST.len(), 74);
}

#[test]
fn tile_passability_decodes_move_cost_and_the_void_sentinel() {
    // Tile 0 is the void sentinel regardless of BACKGROUND_MOVE_COST[0].
    assert_eq!(tile_passability(0), TilePassability::Void);
    // Index 1 is move_cost 0xFF → wall.
    assert_eq!(BACKGROUND_MOVE_COST[1], 0xFF);
    assert_eq!(tile_passability(1), TilePassability::Wall);
    // A normal floor (0x17), heavy terrain (0x1A=26 → mc 2, 0x3C=60 → mc 4).
    assert_eq!(
        tile_passability(0x17),
        TilePassability::Passable { move_cost: 1 }
    );
    assert_eq!(
        tile_passability(26),
        TilePassability::Passable { move_cost: 2 }
    );
    assert_eq!(
        tile_passability(60),
        TilePassability::Passable { move_cost: 4 }
    );
    // Out-of-table index → wall (defensive).
    assert_eq!(tile_passability(200), TilePassability::Wall);
}

#[test]
fn map_reads_are_bounds_safe() {
    let mut map = CombatMap::uniform(FLOOR);
    assert_eq!(
        map.passability(GridPos::new(10, 10)),
        TilePassability::Passable { move_cost: 1 }
    );
    // Out-of-bounds → void ground, 0xFF move cost, no occupant.
    assert_eq!(map.ground_tile(GridPos::new(-1, 0)), 0);
    assert_eq!(
        map.passability(GridPos::new(MAP_W, 0)),
        TilePassability::Void
    );
    assert_eq!(map.move_cost(GridPos::new(0, MAP_H)), 0xFF);
    assert_eq!(map.occupant(GridPos::new(-5, -5)), 0);
    // A stamped wall reads back as a wall.
    map.set_tile(GridPos::new(3, 3), WALL_TILE);
    assert_eq!(map.passability(GridPos::new(3, 3)), TilePassability::Wall);
}

#[test]
fn size_footprint_matches_the_steps_table() {
    let p = GridPos::new(4, 7);
    assert!(size_footprint(0, p).is_empty(), "size 0 occupies no cell");
    assert_eq!(size_footprint(1, p), vec![GridPos::new(4, 7)]);
    assert_eq!(
        size_footprint(4, p),
        vec![
            GridPos::new(4, 7),
            GridPos::new(5, 7),
            GridPos::new(4, 8),
            GridPos::new(5, 8),
        ]
    );
}

// --- placement: exact positions ---------------------------------------

/// The canonical layout: 3 party + 3 monsters, party facing north (dir 0),
/// enemies 1 tile ahead, on all-floor ground. The exact cells below are the
/// transliteration's output; member 0 is re-derived by hand in the doc comment
/// as the worked example.
///
/// **Worked example — party member 0** (`place_combatant`, team 0,
/// `team_direction=0`, `team_start=(0,0)`):
/// - iteration 1, tri-state `start`: `half_dir = DIRECTION_165FC[0][0]/2 = 0`;
///   `iso_dir = HALF_DIR_TO_ISO[2] = 3`, `delta=(1,1)`;
///   `base = (UNK_16610[0], UNK_16618[0]) = (5,3)`, `row_scale=0` → `cur=(5,3)`.
/// - `cur=(5,3)` is in range; `valid[0][0][3][5]` is set (row 3 of `UNK_16620[0]`
///   is `[2,9]`, so col 5 is valid); ground is floor, unoccupied → placed.
/// - iso transform: `pos.x = 5 + 0·6 + 0·5 + 22 = 27`,
///   `pos.y = 3 + 0·5 + 10 = 13` → **(27, 13)**.
#[test]
fn placement_exact_positions_party_north() {
    let mut map = CombatMap::uniform(FLOOR);
    let roster: Vec<PlacementInput> = (0..3)
        .map(|_| place_input(Team::Party))
        .chain((0..3).map(|_| place_input(Team::Monster)))
        .collect();
    let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);

    let cells: Vec<(i32, i32)> = p.iter().map(|c| (c.pos.x, c.pos.y)).collect();
    assert_eq!(
        cells,
        vec![
            (27, 13), // party 0 — hand-derived above
            (28, 13), // party 1
            (28, 14), // party 2
            (22, 7),  // monster 0
            (21, 7),  // monster 1
            (21, 6),  // monster 2
        ]
    );
    assert!(p.iter().all(|c| c.placed), "all six find a cell");
}

// --- provisional area terrain (D2) ------------------------------------

/// A `0x402`-byte GEO payload with the named squares fully enclosed (all
/// four wall nibbles nonzero); every other square is fully open. Mirrors
/// the plane layout `gbx_formats::geo` documents (NE plane packs N high /
/// E low at offset 2; SW plane packs S high / W low at offset 2+256).
fn synthetic_geo_with_walled_squares(cells: &[(usize, usize)]) -> GeoBlock {
    const PLANE_NE: usize = 2;
    const PLANE_SW: usize = 2 + 256;
    let mut data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
    for &(gx, gy) in cells {
        let i = gx + 16 * gy;
        data[PLANE_NE + i] = (3 << 4) | 3; // N=3, E=3
        data[PLANE_SW + i] = (3 << 4) | 3; // S=3, W=3
    }
    GeoBlock::parse(&data).unwrap()
}

#[test]
fn provisional_map_stamps_fully_walled_squares_as_rock() {
    // (0,0) fully walled → rock at (17,3); (1,0) only partially walled →
    // stays floor.
    let mut data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
    data[2] = (3 << 4) | 3; // sq (0,0): N=3,E=3
    data[2 + 256] = (3 << 4) | 3; // sq (0,0): S=3,W=3
    data[2 + 1] = 3 << 4; // sq (1,0): N=3 only (not enclosed)
    let geo = GeoBlock::parse(&data).unwrap();

    let map = provisional_combat_map(&geo);
    assert!(
        matches!(map.passability(GridPos::new(17, 3)), TilePassability::Wall),
        "a fully-walled GEO square becomes a rock obstacle"
    );
    assert!(
        matches!(
            map.passability(GridPos::new(18, 3)),
            TilePassability::Passable { .. }
        ),
        "a partially-walled square stays open floor"
    );
    // A cell nowhere near any wall is open floor.
    assert!(matches!(
        map.passability(GridPos::new(45, 20)),
        TilePassability::Passable { .. }
    ));
}

#[test]
fn provisional_map_keeps_the_deployment_core_clear() {
    // Square (5,5) maps to (22,8), which lands INSIDE the deployment core
    // (x 20..=30, y 6..=16) — so even though it is fully walled, the core
    // re-clear stamps it back to floor and the roster can deploy there.
    let geo = synthetic_geo_with_walled_squares(&[(5, 5)]);
    let map = provisional_combat_map(&geo);
    assert!(
        matches!(
            map.passability(GridPos::new(22, 8)),
            TilePassability::Passable { .. }
        ),
        "the deployment core is re-cleared over any wall"
    );
    // And the whole party origin (27,13) region places.
    let roster: Vec<PlacementInput> = (0..3)
        .map(|_| place_input(Team::Party))
        .chain((0..3).map(|_| place_input(Team::Monster)))
        .collect();
    let mut map = map;
    let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);
    assert!(p.iter().all(|c| c.placed), "everyone finds a cell");
}

// --- encounter runner (D3) --------------------------------------------

fn weak_goblin() -> LoadedMonster {
    use crate::monster::MonsterAttack;
    LoadedMonster {
        name: "GOB".to_string(),
        hit_dice: 1,
        hit_point_max: 3,
        ac: 10,
        thac0: 20,
        turn_undead_type: 0,
        monster_type: 3,
        control_morale: 0x80,
        movement: 6,
        attacks: [
            MonsterAttack {
                attacks: 1,
                dice_count: 1,
                dice_size: 2,
                damage_bonus: 0,
            },
            MonsterAttack {
                attacks: 0,
                dice_count: 0,
                dice_size: 0,
                damage_bonus: 0,
            },
        ],
    }
}

fn strong_party_member() -> PartyCombatStats {
    PartyCombatStats {
        hp: 40,
        raw_ac: 54, // displayed AC -18, near-untouchable
        hit_bonus: 50,
        movement: 12,
        dice: (2, 8, 5),
        npc: false,
    }
}

#[test]
fn encounter_distance_wilderness_is_2() {
    let geo = synthetic_geo_with_walled_squares(&[]);
    assert_eq!(encounter_distance(&geo, 0, 5, 5, false), 2);
}

#[test]
fn encounter_distance_dungeon_ray_walks_open_cells_and_stops_at_a_wall() {
    // Open everywhere: the ray walks its full 2 cells.
    let open = synthetic_geo_with_walled_squares(&[]);
    assert_eq!(encounter_distance(&open, 2, 5, 5, true), 2);
    // A wall on the east edge of the party's own cell blocks immediately.
    let mut data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
    data[2 + (5 + 16 * 5)] = 0x03; // sq (5,5): E nibble = 3 (wall)
    let walled = GeoBlock::parse(&data).unwrap();
    assert_eq!(encounter_distance(&walled, 2, 5, 5, true), 0);
}

#[test]
fn run_encounter_party_beats_a_weak_monster() {
    let geo = synthetic_geo_with_walled_squares(&[]);
    let map = provisional_combat_map(&geo);
    let party = vec![strong_party_member(), strong_party_member()];
    let monsters = vec![weak_goblin()];
    let mut rng = EngineRng::new(0x0C0F_FEE0);
    let result = run_encounter(&party, &monsters, map, 0, 1, &mut rng);
    assert_eq!(result.outcome, CombatOutcome::PartyWins);
    assert!(result.rounds >= 1, "at least one round resolved");
}

/// Local-tier: the real Tilverton City block (`GEO2.DAX` block 1) derives
/// a provisional field with the invariants the wiring relies on — the
/// deployment core is fully passable, and it is real GEO data (at least
/// one rock cell is stamped from the block's enclosed squares).
#[test]
fn provisional_map_from_real_geo2_block1_invariants() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: provisional_map_from_real_geo2_block1_invariants needs GBX_DATA_DIR");
        return;
    };
    let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir))
        .expect("GBX_DATA_DIR must be readable");
    let geo = GeoBlock::parse(&data.block("GEO2.DAX", 1).expect("GEO2.DAX block 1 loads"))
        .expect("GEO2 block 1 parses");
    let map = provisional_combat_map(&geo);

    for y in 6..=16 {
        for x in 20..=30 {
            assert!(
                matches!(
                    map.passability(GridPos::new(x, y)),
                    TilePassability::Passable { .. }
                ),
                "deployment core cell ({x},{y}) must be passable"
            );
        }
    }
    let rocks = (0..MAP_H)
        .flat_map(|y| (0..MAP_W).map(move |x| GridPos::new(x, y)))
        .filter(|&p| matches!(map.passability(p), TilePassability::Wall))
        .count();
    assert!(rocks > 0, "real GEO2 block 1 stamps at least one rock cell");
    eprintln!("GEO2 block 1 → {rocks} rock cell(s) on the provisional field");
}

#[test]
fn placement_offsets_monsters_along_the_facing_direction() {
    // East (dir 2): monsters end up at larger x than the party; south (dir 4):
    // larger y. The team origin shift is encounter_distance · facing.
    let roster: Vec<PlacementInput> = (0..3)
        .map(|_| place_input(Team::Party))
        .chain((0..3).map(|_| place_input(Team::Monster)))
        .collect();

    for (dir, enc, axis) in [(2u8, 2i32, 'x'), (4, 1, 'y')] {
        let mut map = CombatMap::uniform(FLOOR);
        let p = place_combatants(&mut map, &roster, dir, enc, GridPos::new(0, 0), None);
        assert!(p.iter().all(|c| c.placed), "dir {dir}: all placed");
        let party_mean: i32 = (0..3)
            .map(|i| if axis == 'x' { p[i].pos.x } else { p[i].pos.y })
            .sum::<i32>()
            / 3;
        let mon_mean: i32 = (3..6)
            .map(|i| if axis == 'x' { p[i].pos.x } else { p[i].pos.y })
            .sum::<i32>()
            / 3;
        assert!(
            mon_mean > party_mean,
            "dir {dir}: monsters should be ahead along {axis} (party {party_mean}, mon {mon_mean})"
        );
    }
}

#[test]
fn placement_cells_are_distinct_and_on_passable_ground() {
    let mut map = CombatMap::uniform(FLOOR);
    let roster: Vec<PlacementInput> = (0..6)
        .map(|_| place_input(Team::Party))
        .chain((0..6).map(|_| place_input(Team::Monster)))
        .collect();
    let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);

    assert!(p.iter().all(|c| c.placed), "a 6v6 all fits");
    let mut seen = std::collections::HashSet::new();
    for c in &p {
        assert!(
            seen.insert((c.pos.x, c.pos.y)),
            "no two combatants share a cell"
        );
        assert!(
            matches!(map.passability(c.pos), TilePassability::Passable { .. }),
            "every combatant stands on passable ground: {:?}",
            c.pos
        );
    }
}

#[test]
fn placement_skips_a_walled_cell() {
    // Wall off party member 0's natural cell (27,13); it must land elsewhere,
    // still on passable ground, and the fan-out still places everyone.
    let mut map = CombatMap::uniform(FLOOR);
    map.set_tile(GridPos::new(27, 13), WALL_TILE);
    let roster: Vec<PlacementInput> = (0..3)
        .map(|_| place_input(Team::Party))
        .chain((0..1).map(|_| place_input(Team::Monster)))
        .collect();
    let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);

    assert!(p.iter().all(|c| c.placed));
    assert_ne!(
        (p[0].pos.x, p[0].pos.y),
        (27, 13),
        "the walled cell is skipped"
    );
    assert!(matches!(
        map.passability(p[0].pos),
        TilePassability::Passable { .. }
    ));
}

#[test]
fn placement_paints_occupancy_by_one_based_index() {
    let mut map = CombatMap::uniform(FLOOR);
    let roster: Vec<PlacementInput> = (0..3)
        .map(|_| place_input(Team::Party))
        .chain((0..3).map(|_| place_input(Team::Monster)))
        .collect();
    let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);
    for (i, c) in p.iter().enumerate() {
        assert_eq!(
            map.occupant(c.pos),
            (i + 1) as u16,
            "cell {:?} is owned by combatant {} (1-based)",
            c.pos,
            i + 1
        );
    }
}

// --- movement / facing / distance -------------------------------------

#[test]
fn calc_moves_clamps_then_doubles() {
    assert_eq!(calc_moves(12), 24); // in range → ×2
    assert_eq!(calc_moves(1), 2);
    assert_eq!(calc_moves(96), 192);
    assert_eq!(calc_moves(0), 2, "< 1 collapses to 1 → 2 half-moves");
    assert_eq!(
        calc_moves(97),
        2,
        "the faithful quirk: > 96 also collapses to 1"
    );
}

#[test]
fn step_cost_diagonal_is_x3_orthogonal_x2_and_offmap_is_none() {
    let map = CombatMap::uniform(FLOOR); // move_cost 1 everywhere
    let from = GridPos::new(25, 12);
    // East (dir 2, even → orthogonal): dest (26,12), cost 1·2.
    assert_eq!(step_cost(&map, from, 2), Some((GridPos::new(26, 12), 2)));
    // NE (dir 1, odd → diagonal): dest (26,11), cost 1·3.
    assert_eq!(step_cost(&map, from, 1), Some((GridPos::new(26, 11), 3)));
    // Off the top edge → None (the MapInBounds guard).
    assert_eq!(step_cost(&map, GridPos::new(0, 0), 0), None);
}

#[test]
fn step_cost_into_a_wall_is_huge() {
    let mut map = CombatMap::uniform(FLOOR);
    map.set_tile(GridPos::new(26, 12), WALL_TILE); // move_cost 0xFF
                                                   // Orthogonal into the wall: 0xFF · 2.
    assert_eq!(
        step_cost(&map, GridPos::new(25, 12), 2),
        Some((GridPos::new(26, 12), 0xFF * 2))
    );
}

#[test]
fn deduct_move_zeroes_on_overspend() {
    assert_eq!(deduct_move(10, 3), 7);
    assert_eq!(deduct_move(2, 3), 0, "can't half-finish a step");
    assert_eq!(deduct_move(3, 3), 0);
}

#[test]
fn target_direction_classifies_the_eight_octants() {
    let o = GridPos::new(10, 10);
    // y grows downward, so "north" is a smaller y.
    assert_eq!(target_direction(o, GridPos::new(10, 5)), 0, "N");
    assert_eq!(target_direction(o, GridPos::new(15, 5)), 1, "NE");
    assert_eq!(target_direction(o, GridPos::new(15, 10)), 2, "E");
    assert_eq!(target_direction(o, GridPos::new(15, 15)), 3, "SE");
    assert_eq!(target_direction(o, GridPos::new(10, 15)), 4, "S");
    assert_eq!(target_direction(o, GridPos::new(5, 15)), 5, "SW");
    assert_eq!(target_direction(o, GridPos::new(5, 10)), 6, "W");
    assert_eq!(target_direction(o, GridPos::new(5, 5)), 7, "NW");
}

// === wall-respecting range — the Bresenham reach ray (M4 combat #4) =====

fn rc(team: Team, x: i32, y: i32) -> RangeCombatant {
    RangeCombatant {
        pos: GridPos::new(x, y),
        size: 1,
        team,
    }
}

#[test]
fn reach_ray_open_ground_step_counts() {
    let map = CombatMap::uniform(FLOOR);
    let o = GridPos::new(20, 12);
    // Orthogonal neighbour: 1 step ×2 = 2.
    assert_eq!(reach_ray(&map, o, GridPos::new(21, 12), false).steps, 2);
    // Diagonal neighbour: 2 + 1 = 3.
    assert_eq!(reach_ray(&map, o, GridPos::new(21, 13), false).steps, 3);
    // Distance-2 orthogonal: 4.
    assert_eq!(reach_ray(&map, o, GridPos::new(22, 12), false).steps, 4);
    // 2·max + min: (dx=3,dy=1) → 6+1 = 7.
    assert_eq!(reach_ray(&map, o, GridPos::new(23, 13), false).steps, 7);
    // Symmetric in endpoint order (abs deltas).
    assert_eq!(
        reach_ray(&map, GridPos::new(23, 13), o, false).steps,
        reach_ray(&map, o, GridPos::new(23, 13), false).steps
    );
    // Self: zero steps, reachable.
    let r = reach_ray(&map, o, o, false);
    assert!(r.reach && r.steps == 0);
}

#[test]
fn get_target_range_halves_steps_for_adjacency() {
    let map = CombatMap::uniform(FLOOR);
    let o = GridPos::new(20, 12);
    assert_eq!(
        get_target_range(&map, GridPos::new(21, 12), o),
        1,
        "ortho adj"
    );
    assert_eq!(
        get_target_range(&map, GridPos::new(21, 13), o),
        1,
        "diag adj"
    );
    assert_eq!(get_target_range(&map, GridPos::new(22, 12), o), 2, "dist 2");
    assert_eq!(get_target_range(&map, GridPos::new(24, 12), o), 4, "dist 4");
}

#[test]
fn reach_ray_blocks_on_a_taller_wall_but_ignore_walls_passes() {
    let mut map = CombatMap::uniform(FLOOR); // floor height 1
                                             // A wall tile (field_2 == 2 > floor height 1) mid-line blocks.
    map.set_tile(GridPos::new(12, 10), WALL_TILE);
    let a = GridPos::new(10, 10);
    let t = GridPos::new(14, 10);
    let blocked = reach_ray(&map, a, t, false);
    assert!(!blocked.reach, "the wall blocks the ray");
    assert_eq!(
        blocked.steps, 4,
        "blocked after reaching the wall cell (2 ortho steps)"
    );
    // Ignoring walls, the full line is traversed: 4 ortho steps ×2 = 8.
    let ignored = reach_ray(&map, a, t, true);
    assert!(ignored.reach);
    assert_eq!(ignored.steps, 8);
    // getTargetRange ignores walls, so it still measures the geometric range.
    assert_eq!(get_target_range(&map, t, a), 4);
    // can_reach reflects the block within budget.
    assert_eq!(can_reach(&map, a, t, 0xff, false), None, "blocked");
    assert_eq!(can_reach(&map, a, t, 0xff, true), Some(8), "wall ignored");
}

#[test]
fn tile_height_tables_are_74_and_match_move_cost_walls() {
    assert_eq!(TILE_HEIGHT.len(), 74);
    assert_eq!(TILE_WALL_HEIGHT.len(), 74);
    // Every impassable wall tile presents a wall taller than the floor height 1.
    for t in 0..74u8 {
        if BACKGROUND_MOVE_COST[t as usize] == 0xFF && TILE_HEIGHT[t as usize] == 1 {
            assert!(
                TILE_WALL_HEIGHT[t as usize] > 1,
                "wall tile {t} should block a height-1 attacker"
            );
        }
    }
    // A floor tile (0x17) never blocks a height-1 attacker.
    assert!(TILE_WALL_HEIGHT[0x17] <= TILE_HEIGHT[0x17]);
}

#[test]
fn build_near_targets_filters_team_and_sorts_nearest_first() {
    let map = CombatMap::uniform(FLOOR);
    let combatants = [
        rc(Team::Party, 25, 12),   // 0 = attacker (same team → excluded)
        rc(Team::Monster, 26, 12), // 1 = adjacent (steps 2)
        rc(Team::Monster, 28, 12), // 2 = dist 3 (steps 6)
        rc(Team::Monster, 25, 16), // 3 = dist 4 (steps 8)
        rc(Team::Party, 24, 12),   // 4 = ally (excluded by team filter)
    ];
    let near = build_near_targets(&map, &combatants, 0, 0xff, false);
    let idxs: Vec<usize> = near.iter().map(|n| n.idx).collect();
    assert_eq!(idxs, vec![1, 2, 3], "opposite team only, nearest-first");
    assert_eq!(near[0].steps, 2, "true min steps at large max_range");
    assert_eq!(near[1].steps, 6);
    assert_eq!(near[2].steps, 8);
}

#[test]
fn build_near_targets_range_1_is_melee_adjacency() {
    let map = CombatMap::uniform(FLOOR);
    let combatants = [
        rc(Team::Party, 25, 12),   // attacker
        rc(Team::Monster, 26, 13), // diagonal-adjacent (steps 3 ≤ 1·2+1)
        rc(Team::Monster, 28, 12), // dist 3 (steps 6 > 3) — excluded at range 1
    ];
    let near = build_near_targets(&map, &combatants, 0, 1, false);
    assert_eq!(near.len(), 1, "only the adjacent enemy is near at range 1");
    assert_eq!(near[0].idx, 1);
    // §20 bug #8 (`ovr032:097B`): the binary's best-pair init is 0xFF, not
    // max_range, so the entry stores the REAL steps (3 for a diagonal step)
    // even at range 1 — this is what direction-sorts the range-1 re-pick.
    assert_eq!(near[0].steps, 3);
}

#[test]
fn range_layer_is_draw_free() {
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    let map = CombatMap::uniform(FLOOR);
    let combatants = [
        rc(Team::Party, 25, 12),
        rc(Team::Monster, 26, 12),
        rc(Team::Monster, 30, 15),
    ];
    let _ = reach_ray(&map, combatants[0].pos, combatants[1].pos, false);
    let _ = get_target_range(&map, combatants[1].pos, combatants[0].pos);
    let _ = build_near_targets(&map, &combatants, 0, 0xff, false);
    let _ = find_combatant_direction(combatants[1].pos, combatants[0].pos);
    assert_eq!(log.len(), 0, "the range layer draws nothing (D9)");
    let _ = &mut rng;
}
