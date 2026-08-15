//! ★ **M6c's core** (doc §9/§9.6): the D-CV5 suspensions, the legality tables
//! both ways round, the movement loop's edges, loud refusals — and the
//! **dark-landing parity proof** that all of it is invisible to a fight nobody
//! is playing.

use super::*;
use crate::combat::manual::{movement_key_direction, TurnCmd, TurnOutcome, TurnRefusal};

/// A two-combatant fight on open floor: a party PC at (4,4) and a monster
/// adjacent to its east at (5,4), both alive, with the PC's turn ready to open.
///
/// `quick_fight` starts **false** on the PC (a record-decoded player), so an
/// interactive state suspends for it and a headless one does not.
fn duel() -> CombatState {
    let mut pc = Combatant::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(4, 4),
        20,
        40,
        10,
        12,
        (1, 8, 0),
        5,
        1,
    );
    pc.quick_fight = false;
    let foe = Combatant::new_melee(
        1,
        Team::Monster,
        true,
        GridPos::new(5, 4),
        20,
        40,
        10,
        12,
        (1, 6, 0),
        5,
        1,
    );
    CombatState::new(CombatMap::uniform(FLOOR), vec![pc, foe])
}

/// Pumps `state` until it suspends on a manual turn, returning the actor.
fn open_manual_turn(state: &mut CombatState, rng: &mut EngineRng) -> usize {
    state.set_interactive(true);
    for _ in 0..64 {
        if let CombatStep::AwaitPlayerTurn { combatant_id } = state.step(rng) {
            return combatant_id;
        }
    }
    panic!("no manual turn opened in 64 steps");
}

// === the suspension itself (§9.5) =======================================

#[test]
fn a_party_pc_with_quick_fight_off_suspends_and_the_turn_head_has_run() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    // A parked guard from a previous round: the turn head clears it
    // (`ovr009.cs:107`), and the menu must see the cleared value.
    state.fighters[0].guarding = true;
    state.fighters[0].attacks_received = 3;

    let actor = open_manual_turn(&mut state, &mut rng);
    assert_eq!(actor, 0, "the monster fights itself; only the PC suspends");
    assert!(
        !state.fighters[0].guarding,
        "the turn head ran before the menu"
    );
    assert_eq!(state.fighters[0].attacks_received, 0);
    assert_eq!(state.manual_turn().map(|m| m.actor()), Some(0));
    // Initiative gave it moves, so the menu's Move word is on — a legality
    // read taken *after* the head, which is the whole point of §9.5's order.
    assert!(state.fighters[0].move_left > 0);
    assert!(state.menu_words().unwrap().move_);
}

#[test]
fn stepping_a_suspended_fight_re_reports_the_same_suspension() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    let actor = open_manual_turn(&mut state, &mut rng);
    for _ in 0..5 {
        assert_eq!(
            state.step(&mut rng),
            CombatStep::AwaitPlayerTurn {
                combatant_id: actor
            },
            "a host that ticks while waiting must not fall through"
        );
    }
}

#[test]
fn a_quick_fighting_pc_never_suspends() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.fighters[0].quick_fight = true;
    state.set_interactive(true);
    for _ in 0..40 {
        let step = state.step(&mut rng);
        assert!(
            !matches!(step, CombatStep::AwaitPlayerTurn { .. }),
            "quick-fight on ⇒ the AI takes the turn"
        );
        if step == CombatStep::Ended {
            return;
        }
    }
}

#[test]
fn a_headless_state_never_suspends_whatever_the_roster_says() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel(); // pc.quick_fight == false
    assert!(!state.is_interactive());
    for _ in 0..40 {
        let step = state.step(&mut rng);
        assert!(!matches!(
            step,
            CombatStep::AwaitPlayerTurn { .. } | CombatStep::AwaitContinueBattle
        ));
        if step == CombatStep::Ended {
            return;
        }
    }
}

#[test]
fn the_continue_battle_prompt_suspends_and_its_answer_returns_the_round() {
    let mut rng = EngineRng::new(SEED);
    // Two live party members and a monster that is already gone: the round
    // ends with `friends > 1 && foes == 0`, which is the prompt's condition.
    let mut a = Combatant::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(4, 4),
        20,
        40,
        10,
        12,
        (1, 8, 0),
        0,
        1,
    );
    a.quick_fight = true;
    let mut b = a.clone();
    b.id = 1;
    b.pos = GridPos::new(4, 5);
    let foe = Combatant::new_melee(
        2,
        Team::Monster,
        true,
        GridPos::new(9, 9),
        20,
        40,
        10,
        12,
        (1, 6, 0),
        0,
        1,
    );
    let mut state = CombatState::new(CombatMap::uniform(FLOOR), vec![a, b, foe]);
    state.set_interactive(true);

    // Round 1 opens with a live foe (the pre-loop emptiness guard, doc §48,
    // would otherwise end the fight before it starts) and it leaves the board
    // mid-round — which is the prompt's own condition, `friends > 1 && foes == 0`.
    assert_eq!(state.step(&mut rng), CombatStep::RoundStarted { round: 0 });
    state.fighters[2].in_combat = false;

    let mut saw_prompt = false;
    for _ in 0..64 {
        match state.step(&mut rng) {
            CombatStep::AwaitContinueBattle => {
                saw_prompt = true;
                // 'Y' overrides the verdict: the fight plays another round.
                let step = state.answer_continue_battle(true);
                assert_eq!(
                    step,
                    CombatStep::RoundEnded {
                        round: 1,
                        battle_over: false
                    }
                );
                break;
            }
            CombatStep::Ended => panic!("the prompt should have fired first"),
            _ => {}
        }
    }
    assert!(saw_prompt, "the prompt never fired");

    // And 'N' ends it.
    let mut rng = EngineRng::new(SEED);
    let mut state = {
        let mut s = duel();
        s.fighters[0].quick_fight = true;
        let extra = s.fighters[0].clone();
        s.fighters.push(Combatant {
            id: 2,
            pos: GridPos::new(4, 5),
            ..extra
        });
        s
    };
    state.set_interactive(true);
    assert_eq!(state.step(&mut rng), CombatStep::RoundStarted { round: 0 });
    state.fighters[1].in_combat = false;
    for _ in 0..64 {
        if let CombatStep::AwaitContinueBattle = state.step(&mut rng) {
            let step = state.answer_continue_battle(false);
            assert!(matches!(
                step,
                CombatStep::RoundEnded {
                    battle_over: true,
                    ..
                }
            ));
            return;
        }
    }
    panic!("the prompt never fired");
}

#[test]
#[should_panic(expected = "no Continue-Battle prompt is open")]
fn answering_a_prompt_nobody_asked_is_loud() {
    let mut state = duel();
    state.answer_continue_battle(true);
}

#[test]
#[should_panic(expected = "headless driver")]
fn run_combat_refuses_an_interactive_fight() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.set_interactive(true);
    state.run_combat(&mut rng, 15);
}

// === §9.1's legality table, both polarities ==============================

#[test]
fn the_move_word_needs_a_half_move_left() {
    let mut state = duel();
    state.fighters[0].move_left = 2;
    assert!(state.menu_words_for(0).move_);
    // `> 0`, not `> 1`: a single leftover half-move still shows the word (and
    // the loop it opens then refuses to step — the §9.3 asymmetry).
    state.fighters[0].move_left = 1;
    assert!(state.menu_words_for(0).move_);
    state.fighters[0].move_left = 0;
    assert!(!state.menu_words_for(0).move_);
}

#[test]
fn the_use_word_needs_items() {
    let mut state = duel();
    assert!(!state.menu_words_for(0).use_item);
    state.fighters[0].has_items = true;
    assert!(state.menu_words_for(0).use_item);
}

#[test]
fn the_cast_word_needs_spells_a_live_can_cast_and_no_area_ban() {
    let mut state = duel();
    // No memorized spells → no word, whatever else is true.
    assert!(!state.menu_words_for(0).cast);
    state.fighters[0].memorized_list = vec![0x0F];
    assert!(state.fighters[0].can_cast);
    assert!(!state.area_can_cast_spells);
    assert!(state.menu_words_for(0).cast);
    // §45's disruption: an arrow hit this round zeroes `can_cast`.
    state.fighters[0].can_cast = false;
    assert!(!state.menu_words_for(0).cast);
    state.fighters[0].can_cast = true;
    // The area flag is a BAN when set (the inverted name, §4.1.1).
    state.area_can_cast_spells = true;
    assert!(!state.menu_words_for(0).cast);
}

#[test]
fn the_turn_word_needs_a_cleric_who_has_not_turned() {
    let mut state = duel();
    assert!(!state.menu_words_for(0).turn_undead);
    state.fighters[0].skill_level_cleric = 5;
    assert!(state.menu_words_for(0).turn_undead);
    state.fighters[0].has_turned_undead = true;
    assert!(!state.menu_words_for(0).turn_undead);
}

#[test]
fn the_menu_line_is_the_original_word_order() {
    let mut state = duel();
    state.fighters[0].move_left = 6;
    assert_eq!(state.menu_words_for(0).text(), "Move View Aim Quick Done");
    state.fighters[0].has_items = true;
    state.fighters[0].memorized_list = vec![0x0F];
    state.fighters[0].skill_level_cleric = 3;
    assert_eq!(
        state.menu_words_for(0).text(),
        "Move View Aim Use Cast Turn Quick Done"
    );
    state.fighters[0].move_left = 0;
    assert_eq!(
        state.menu_words_for(0).text(),
        "View Aim Use Cast Turn Quick Done"
    );
}

// === §9.2's Done submenu ================================================

#[test]
fn the_guard_word_is_off_for_a_pure_ranged_weapon() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.item_data = Some(synth_item_table());
    open_manual_turn(&mut state, &mut rng);
    // Bare hands: guarding is fine.
    assert!(state.done_words().unwrap().guard);
    // A LongBow (range 22, no melee flag) cannot guard.
    state.fighters[0].readied_weapon = Some((43, 0));
    assert!(!state.done_words().unwrap().guard);
    // A ranged-melee weapon (the Sling's flag_02 twin, type 47 here carries
    // flag_08|flag_02) can — that is the `||` arm.
    state.fighters[0].readied_weapon = Some((30, 0)); // range 1 ⇒ not ranged
    assert!(state.done_words().unwrap().guard);
}

#[test]
fn the_bandage_word_needs_a_dying_team_member() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    let mut ally = state.fighters[0].clone();
    ally.id = 2;
    ally.pos = GridPos::new(4, 5);
    ally.quick_fight = true; // only combatant 0 suspends
    state.fighters.push(ally);
    assert_eq!(open_manual_turn(&mut state, &mut rng), 0);
    assert!(!state.done_words().unwrap().bandage);
    state.fighters[2].health_status = HealthStatus::Dying;
    assert!(state.done_words().unwrap().bandage);
    // An allied NPC that is not a team member never triggers it (doc §47).
    state.fighters[2].non_team_member = true;
    assert!(!state.done_words().unwrap().bandage);
}

#[test]
fn the_done_line_is_the_original_word_order() {
    assert_eq!(
        DoneWords {
            guard: true,
            bandage: false
        }
        .text(),
        "Guard Delay Quit Speed Exit"
    );
    assert_eq!(
        DoneWords {
            guard: false,
            bandage: true
        }
        .text(),
        "Delay Quit Bandage Speed Exit"
    );
}

// === §9.2's turn-ending commands ========================================

#[test]
fn guard_delay_and_quit_each_end_the_turn_their_own_way() {
    let mut rng = EngineRng::new(SEED);

    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    assert_eq!(
        state.issue(&mut rng, TurnCmd::Guard),
        Ok(TurnOutcome::TurnEnded)
    );
    assert!(state.fighters[0].guarding);
    assert_eq!(state.fighters[0].delay, 0);
    assert!(state.manual_turn().is_none());

    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    assert_eq!(
        state.issue(&mut rng, TurnCmd::DelayTurn),
        Ok(TurnOutcome::TurnEnded)
    );
    assert_eq!(state.fighters[0].delay, 1, "delay = 1, back in the pool");

    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    assert_eq!(
        state.issue(&mut rng, TurnCmd::Quit),
        Ok(TurnOutcome::TurnEnded)
    );
    assert_eq!(state.fighters[0].delay, 0);
    assert_eq!(
        state.fighters[0].move_left, 0,
        "clear_actions zeroed the moves"
    );
}

#[test]
fn speed_is_validated_and_handed_back_without_ending_the_turn() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    assert_eq!(
        state.issue(&mut rng, TurnCmd::SetSpeed(7)),
        Ok(TurnOutcome::Speed(7))
    );
    assert!(state.manual_turn().is_some(), "Speed does not end the turn");
    assert_eq!(
        state.issue(&mut rng, TurnCmd::SetSpeed(10)),
        Err(TurnRefusal::BadSpeed(10))
    );
}

#[test]
fn engaging_quick_fight_hands_the_turn_to_the_ai_for_good() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    assert_eq!(
        state.issue(&mut rng, TurnCmd::EngageQuickFight),
        Ok(TurnOutcome::TurnEnded)
    );
    assert!(state.fighters[0].quick_fight);
    // ...and the next round runs it headlessly: no second suspension for 0.
    for _ in 0..40 {
        match state.step(&mut rng) {
            CombatStep::AwaitPlayerTurn { combatant_id } => {
                panic!("combatant {combatant_id} suspended after Quick")
            }
            CombatStep::Ended => return,
            _ => {}
        }
    }
}

// === §9.3, the movement loop ============================================

#[test]
fn the_movement_keys_are_the_original_table() {
    for (key, dir) in [
        (b'G', 7),
        (b'H', 0),
        (b'I', 1),
        (b'K', 6),
        (b'M', 2),
        (b'O', 5),
        (b'P', 4),
        (b'Q', 3),
    ] {
        assert_eq!(movement_key_direction(key), Some(dir));
    }
    assert_eq!(movement_key_direction(b'Z'), None);
}

#[test]
fn a_single_half_move_shows_the_word_and_refuses_the_step() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    state.fighters[0].move_left = 1;
    assert!(state.menu_words_for(0).move_, "`> 0` shows the word");
    assert_eq!(
        state.issue(&mut rng, TurnCmd::BeginMove),
        Ok(TurnOutcome::Continue)
    );
    assert_eq!(
        state.issue(&mut rng, TurnCmd::MoveStep(0)),
        Err(TurnRefusal::NoMovesLeft),
        "`> 1` gates the step"
    );
    // Closing the loop spends the orphan half-move, which takes the word off.
    assert_eq!(
        state.issue(&mut rng, TurnCmd::EndMove),
        Ok(TurnOutcome::Continue)
    );
    assert_eq!(state.fighters[0].move_left, 0);
    assert!(!state.menu_words_for(0).move_);
}

#[test]
fn a_step_moves_through_the_proven_primitive_and_spends_its_cost() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    let before = state.fighters[0].move_left;
    state.issue(&mut rng, TurnCmd::BeginMove).unwrap();
    // West (dir 6) is away from the monster and orthogonal: cost 2 on floor.
    assert_eq!(
        state.issue(&mut rng, TurnCmd::MoveStep(6)),
        Ok(TurnOutcome::Continue)
    );
    assert_eq!(state.fighters[0].pos, GridPos::new(3, 4));
    assert_eq!(state.fighters[0].move_left, before - 2);
    assert_eq!(state.fighters[0].direction, 6, "the step set the facing");
}

#[test]
fn walking_into_an_enemy_attacks_it() {
    let log = ActionLog::default();
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.attach_action_sink(log.sink());
    open_manual_turn(&mut state, &mut rng);
    state.issue(&mut rng, TurnCmd::BeginMove).unwrap();
    let before = state.fighters[0].pos;
    // East (dir 2) is the monster's cell.
    let out = state.issue(&mut rng, TurnCmd::MoveStep(2)).unwrap();
    assert!(matches!(
        out,
        TurnOutcome::Continue | TurnOutcome::TurnEnded
    ));
    assert_eq!(state.fighters[0].pos, before, "an attack is not a step");
    assert!(
        log.events()
            .iter()
            .any(|e| matches!(e, ActionEvent::Attacking { attacker_id: 0, .. })),
        "the swing went through `attack_target`"
    );
    assert_eq!(state.fighters[0].target, Some(1));
}

#[test]
fn a_pure_ranged_weapon_cannot_swing_at_the_cell_it_walks_into() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.item_data = Some(synth_item_table());
    open_manual_turn(&mut state, &mut rng);
    state.fighters[0].readied_weapon = Some((43, 0)); // LongBow
    state.issue(&mut rng, TurnCmd::BeginMove).unwrap();
    assert_eq!(
        state.issue(&mut rng, TurnCmd::MoveStep(2)),
        Err(TurnRefusal::NotWithThatWeapon)
    );
}

#[test]
fn stepping_off_the_map_asks_to_flee_and_flee_ends_the_turn() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.fighters[0].pos = GridPos::new(0, 4);
    state.rebuild_occupancy();
    open_manual_turn(&mut state, &mut rng);
    state.issue(&mut rng, TurnCmd::BeginMove).unwrap();
    // West off the board.
    assert_eq!(
        state.issue(&mut rng, TurnCmd::MoveStep(6)),
        Ok(TurnOutcome::FleePrompt)
    );
    assert_eq!(
        state.fighters[0].pos,
        GridPos::new(0, 4),
        "nothing moved yet"
    );
    assert_eq!(
        state.issue(&mut rng, TurnCmd::Flee),
        Ok(TurnOutcome::TurnEnded)
    );
}

#[test]
fn an_unaffordable_step_is_blocked_not_taken() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    state.issue(&mut rng, TurnCmd::BeginMove).unwrap();
    // A cost-4 tile (`BACKGROUND_MOVE_COST[60]`) west of the actor: an
    // orthogonal step wants 8 half-moves and only 3 are left.
    state.map.set_tile(GridPos::new(3, 4), 60);
    state.fighters[0].move_left = 3;
    assert_eq!(state.map.move_cost(GridPos::new(3, 4)), 4);
    let out = state.issue(&mut rng, TurnCmd::MoveStep(6)).unwrap();
    assert_eq!(out, TurnOutcome::Blocked);
    assert_eq!(
        state.fighters[0].pos,
        GridPos::new(4, 4),
        "no step was taken"
    );
    assert_eq!(state.fighters[0].direction, 6, "but the icon turned");
}

#[test]
fn esc_restores_the_moves_and_the_cell_and_says_so_in_an_event() {
    let log = ActionLog::default();
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.attach_action_sink(log.sink());
    open_manual_turn(&mut state, &mut rng);
    let entry_pos = state.fighters[0].pos;
    let entry_moves = state.fighters[0].move_left;
    let entry_dir = state.fighters[0].direction;

    state.issue(&mut rng, TurnCmd::BeginMove).unwrap();
    state.issue(&mut rng, TurnCmd::MoveStep(6)).unwrap();
    state.issue(&mut rng, TurnCmd::MoveStep(6)).unwrap();
    assert_ne!(state.fighters[0].pos, entry_pos);
    assert!(state.fighters[0].move_left < entry_moves);

    assert_eq!(
        state.issue(&mut rng, TurnCmd::AbortMove),
        Ok(TurnOutcome::Continue)
    );
    assert_eq!(
        state.fighters[0].pos, entry_pos,
        "put back on the entry cell"
    );
    assert_eq!(state.fighters[0].move_left, entry_moves, "moves restored");
    assert_eq!(state.fighters[0].direction, entry_dir, "facing restored");
    assert_eq!(
        state.map.occupant(entry_pos),
        1,
        "occupancy follows the restore"
    );
    assert!(
        log.events().iter().any(|e| matches!(
            e,
            ActionEvent::MoveAborted {
                combatant_id: 0,
                ..
            }
        )),
        "the scene rewinds from the abort event, not from a state read"
    );
    // The loop is closed: another step needs a fresh `BeginMove`.
    assert_eq!(
        state.issue(&mut rng, TurnCmd::MoveStep(6)),
        Err(TurnRefusal::NotMoving)
    );
}

#[test]
fn movement_commands_outside_a_loop_are_refused() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    assert_eq!(
        state.issue(&mut rng, TurnCmd::MoveStep(0)),
        Err(TurnRefusal::NotMoving)
    );
    assert_eq!(
        state.issue(&mut rng, TurnCmd::EndMove),
        Err(TurnRefusal::NotMoving)
    );
    assert_eq!(
        state.issue(&mut rng, TurnCmd::AbortMove),
        Err(TurnRefusal::NotMoving)
    );
    state.issue(&mut rng, TurnCmd::BeginMove).unwrap();
    assert_eq!(
        state.issue(&mut rng, TurnCmd::BeginMove),
        Err(TurnRefusal::AlreadyMoving)
    );
    assert_eq!(
        state.issue(&mut rng, TurnCmd::MoveStep(9)),
        Err(TurnRefusal::BadDirection(9))
    );
    // With no moves at all the word is gone and the loop refuses to open.
    state.issue(&mut rng, TurnCmd::EndMove).unwrap();
    state.fighters[0].move_left = 0;
    assert_eq!(
        state.issue(&mut rng, TurnCmd::BeginMove),
        Err(TurnRefusal::WordUnavailable { word: "Move" })
    );
}

// === §9.4, Aim ==========================================================

#[test]
fn the_aim_list_is_the_unfiltered_scan_order() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    let mut ally = state.fighters[0].clone();
    ally.id = 2;
    ally.pos = GridPos::new(4, 6);
    ally.quick_fight = true; // only combatant 0 suspends
    state.fighters.push(ally);
    state.rebuild_occupancy();
    assert_eq!(open_manual_turn(&mut state, &mut rng), 0);
    let list = state.aim_list(0);
    // `copy_sorted_players`' filter is `p => true`: the attacker and its ally
    // are in the cycle, not just the enemy.
    assert!(list.contains(&0), "the attacker is in its own aim list");
    assert!(list.contains(&1));
    assert!(list.contains(&2));
    // Nearest first: the adjacent monster precedes the two-cells-away ally.
    let pos_of = |id: usize| list.iter().position(|&x| x == id).unwrap();
    assert!(pos_of(1) < pos_of(2));
}

#[test]
fn aim_commits_at_melee_reach_and_refuses_beyond_it() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    assert!(state.can_commit_aim(0, 1), "adjacent, bare hands ⇒ range 1");
    assert!(!state.can_commit_aim(0, 0), "never yourself");
    // Push the monster out of reach: bare hands cap at range 1.
    state.fighters[1].pos = GridPos::new(9, 4);
    state.rebuild_occupancy();
    assert!(!state.can_commit_aim(0, 1));
    assert_eq!(
        state.issue(&mut rng, TurnCmd::AttackTarget { target: 1 }),
        Err(TurnRefusal::IllegalTarget { target: 1 })
    );
}

#[test]
fn a_bow_commits_at_range_and_a_cornered_bow_does_not() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.item_data = Some(synth_item_table());
    open_manual_turn(&mut state, &mut rng);
    // Out of everyone's reach — set after the round ran, so the monster's own
    // approach cannot walk back into the archer's face.
    state.fighters[1].pos = GridPos::new(9, 4);
    state.rebuild_occupancy();
    state.set_loadout(
        0,
        Loadout {
            ranged: Some((43, 0)),
            ammo_count: 10,
            ammo_readied: true,
            melee: None,
            unarmed_profile: (1, 2, 0),
            entry_ranged_readied: true,
        },
    );
    assert!(state.is_weapon_ranged(0));
    assert!(state.can_commit_aim(0, 1), "LongBow reaches 21");
    // An enemy in its face: `BuildNearTargets(1).Count > 0` and the bow is not
    // ranged-melee, so the shot is off the table (`ovr014.cs:1776-1786`).
    let mut brawler = state.fighters[1].clone();
    brawler.id = 2;
    brawler.pos = GridPos::new(5, 4);
    state.fighters.push(brawler);
    state.rebuild_occupancy();
    assert!(!state.can_commit_aim(0, 1));
}

/// A duel plus an adjacent ally of combatant 0, ready for the `"Attack Ally: "`
/// path. `npc` says whether that ally is party-controlled or an NPC —
/// `control_morale >= Control.NPC_Base` is the flip's own gate.
fn duel_with_ally(npc: bool) -> crate::combat::CombatState {
    let mut state = duel();
    let mut ally = state.fighters[0].clone();
    ally.id = 2;
    ally.pos = GridPos::new(4, 5);
    ally.quick_fight = true; // only combatant 0 suspends
    ally.npc = npc;
    ally.control_morale = if npc { 0x80 } else { 0x10 };
    state.fighters.push(ally);
    state.rebuild_occupancy();
    state
}

/// `can_attack_target`'s `N` answer (`ovr014.cs:1725-1728`): the commit is
/// refused and the aim menu stays open. Unconfirmed means "answered N" — a
/// driver that never asks never swings at a friend by accident.
#[test]
fn aiming_at_an_ally_without_confirmation_is_refused() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel_with_ally(false);
    assert_eq!(open_manual_turn(&mut state, &mut rng), 0);
    assert_eq!(
        state.issue(&mut rng, TurnCmd::AttackTarget { target: 2 }),
        Err(TurnRefusal::AllyTarget { target: 2 })
    );
    assert!(
        state.manual_turn().is_some(),
        "a refused ally commit leaves the turn open"
    );
}

/// The `Y` branch (`ovr014.cs:1730-1745`): the swing happens, `field_666` goes
/// up, and every conscious combatant with `control_morale >= NPC_Base` flips to
/// the enemy team with its held target dropped. A **party-controlled** member
/// must not flip.
#[test]
fn a_confirmed_ally_attack_swings_and_does_not_flip_party_members() {
    let log = ActionLog::default();
    let mut rng = EngineRng::new(SEED);
    let mut state = duel_with_ally(false);
    state.attach_action_sink(log.sink());
    assert_eq!(open_manual_turn(&mut state, &mut rng), 0);
    state
        .issue(&mut rng, TurnCmd::ConfirmAttackAlly)
        .expect("arming the confirmation is always legal mid-turn");
    state
        .issue(&mut rng, TurnCmd::AttackTarget { target: 2 })
        .expect("a confirmed ally commit swings");
    assert_eq!(state.area_field_666(), 1, "field_666 = 1");
    assert!(
        log.events().iter().any(|e| matches!(
            e,
            ActionEvent::Attacking {
                attacker_id: 0,
                target_id: 2,
                ..
            }
        )),
        "the swing really went through attack_target"
    );
    assert_eq!(
        state.roster()[2].team,
        crate::combat::Team::Party,
        "a party-controlled ally (control_morale < 0x80) must NOT flip"
    );
    assert!(!log.events().iter().any(|e| matches!(
        e,
        ActionEvent::StubTripped {
            stub: "attack-ally",
            ..
        }
    )));
}

/// The other half of the gate: a **synthetic NPC ally** (`control_morale >=
/// NPC_Base`) does flip, loses its held target, and the refreshed
/// `CountCombatTeamMembers` counts it on the other side.
#[test]
fn a_confirmed_ally_attack_flips_conscious_npcs_to_the_enemy_team() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel_with_ally(true);
    assert_eq!(open_manual_turn(&mut state, &mut rng), 0);
    // The NPC ally's own QuickFight turn ran before ours and may have walked;
    // put it back beside the actor and give it a held target so the flip's
    // `actions.target = null` is observable.
    state.fighters[2].pos = GridPos::new(4, 5);
    state.fighters[2].target = Some(1);
    state.rebuild_occupancy();
    state.issue(&mut rng, TurnCmd::ConfirmAttackAlly).unwrap();
    state
        .issue(&mut rng, TurnCmd::AttackTarget { target: 2 })
        .expect("a confirmed ally commit swings");
    assert_eq!(
        state.roster()[2].team,
        crate::combat::Team::Monster,
        "a conscious NPC ally flips to Enemy"
    );
    assert_eq!(
        state.roster()[2].target,
        None,
        "and its held target is dropped (actions.target = null)"
    );
}

/// The consent is **one-shot**: `can_attack_target` asks once per commit, so a
/// second ally swing needs a second `Y`.
#[test]
fn ally_confirmation_is_consumed_by_one_commit() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel_with_ally(false);
    assert_eq!(open_manual_turn(&mut state, &mut rng), 0);
    state.issue(&mut rng, TurnCmd::ConfirmAttackAlly).unwrap();
    state
        .issue(&mut rng, TurnCmd::AttackTarget { target: 2 })
        .expect("the first commit consumes the confirmation");
    if state.manual_turn().is_some() {
        assert_eq!(
            state.issue(&mut rng, TurnCmd::AttackTarget { target: 2 }),
            Err(TurnRefusal::AllyTarget { target: 2 }),
            "the second swing must ask again"
        );
    }
}

#[test]
fn an_aim_commit_swings_through_attack_target() {
    let log = ActionLog::default();
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.attach_action_sink(log.sink());
    open_manual_turn(&mut state, &mut rng);
    let out = state
        .issue(&mut rng, TurnCmd::AttackTarget { target: 1 })
        .unwrap();
    assert!(matches!(
        out,
        TurnOutcome::Continue | TurnOutcome::TurnEnded
    ));
    let events = log.events();
    assert!(events.iter().any(|e| matches!(
        e,
        ActionEvent::Attacking {
            attacker_id: 0,
            target_id: 1,
            ..
        }
    )));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ActionEvent::Attack { .. })),
        "the to-hit roll went through the proven path"
    );
}

// === loud refusals ======================================================

#[test]
fn commands_without_an_open_turn_are_refused() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    assert_eq!(
        state.issue(&mut rng, TurnCmd::Quit),
        Err(TurnRefusal::NoManualTurn)
    );
}

#[test]
fn turn_undead_trips_its_stub_loudly() {
    let log = ActionLog::default();
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.attach_action_sink(log.sink());
    state.fighters[0].skill_level_cleric = 5;
    state.fighters[0].has_items = true;
    open_manual_turn(&mut state, &mut rng);

    assert_eq!(
        state.issue(&mut rng, TurnCmd::TurnUndead),
        Err(TurnRefusal::Unmodeled {
            stub: "turn-undead"
        })
    );
    // ★ Roll-credits slice 9a: an item whose spell has no transcribed row
    // still refuses — the tripwire moved from the command to the spell.
    assert_eq!(
        state.issue(
            &mut rng,
            TurnCmd::UseItem {
                spell_id: 0x41, // the Wand of Magic Missiles' id — no row
                targets: vec![1],
            }
        ),
        Err(TurnRefusal::Unmodeled {
            stub: "spell-entry"
        })
    );
    let stubs: Vec<&str> = log
        .events()
        .iter()
        .filter_map(|e| match e {
            ActionEvent::StubTripped { stub, .. } => Some(*stub),
            _ => None,
        })
        .collect();
    assert_eq!(stubs, vec!["turn-undead", "spell-entry"]);
    assert!(
        state.manual_turn().is_some(),
        "a refusal never ends the turn"
    );
    // And a non-cleric is refused before the stub even trips.
    state.fighters[0].skill_level_cleric = 0;
    assert_eq!(
        state.issue(&mut rng, TurnCmd::TurnUndead),
        Err(TurnRefusal::WordUnavailable { word: "Turn" })
    );
}

#[test]
fn a_word_whose_condition_is_false_is_refused_even_if_the_driver_offers_it() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.item_data = Some(synth_item_table());
    open_manual_turn(&mut state, &mut rng);
    // Guard, with a bow readied.
    state.fighters[0].readied_weapon = Some((43, 0));
    assert_eq!(
        state.issue(&mut rng, TurnCmd::Guard),
        Err(TurnRefusal::WordUnavailable { word: "Guard" })
    );
    assert!(!state.fighters[0].guarding);
    // Bandage, with nobody dying.
    assert_eq!(
        state.issue(&mut rng, TurnCmd::Bandage),
        Err(TurnRefusal::WordUnavailable { word: "Bandage" })
    );
    // Cast, with nothing memorized.
    assert_eq!(
        state.issue(
            &mut rng,
            TurnCmd::CastSpell {
                spell_id: 0x0F,
                targets: vec![1]
            }
        ),
        Err(TurnRefusal::WordUnavailable { word: "Cast" })
    );
}

// === §9.1's Cast ========================================================

#[test]
fn a_manual_cast_draws_no_targeting_dice() {
    let draws = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    state.fighters[0].memorized_list = vec![0x0F]; // Magic Missile
    state.fighters[0].skill_level_magic_user = 5; // PHILIPPE's level
    rng.attach_sink(draws.sink());

    let out = state
        .issue(
            &mut rng,
            TurnCmd::CastSpell {
                spell_id: 0x0F,
                targets: vec![1],
            },
        )
        .unwrap();
    assert_eq!(
        out,
        TurnOutcome::TurnEnded,
        "an immediate cast ends the turn"
    );
    // Magic Missile's own damage dice are the only draws: the AI arm's
    // `find_target` d(count) pick is what the aim menu replaced.
    let ns = draws.ns();
    assert!(!ns.is_empty(), "the missile still rolls its damage");
    assert!(
        ns.iter().all(|&n| n == 4),
        "manual targeting is draw-free; got {ns:?}"
    );
    assert!(state.fighters[1].hp_current < 20, "the target took damage");
}

#[test]
fn a_manual_cast_validates_its_aim_picks() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    state.fighters[0].memorized_list = vec![0x0F, 0x17];

    assert_eq!(
        state.issue(
            &mut rng,
            TurnCmd::CastSpell {
                spell_id: 0x03,
                targets: vec![0]
            }
        ),
        Err(TurnRefusal::SpellNotMemorized { spell_id: 0x03 })
    );
    // Magic Missile takes one target; two is a driver bug.
    assert_eq!(
        state.issue(
            &mut rng,
            TurnCmd::CastSpell {
                spell_id: 0x0F,
                targets: vec![1, 1]
            }
        ),
        Err(TurnRefusal::TooManyTargets { max: 1 })
    );
    // Hold Person takes three, but not the same one twice.
    assert_eq!(
        state.issue(
            &mut rng,
            TurnCmd::CastSpell {
                spell_id: 0x17,
                targets: vec![1, 1]
            }
        ),
        Err(TurnRefusal::DuplicateTarget { target: 1 })
    );
    // An aborted aim is not a cast, and does not spend the turn.
    assert_eq!(
        state.issue(
            &mut rng,
            TurnCmd::CastSpell {
                spell_id: 0x0F,
                targets: vec![]
            }
        ),
        Ok(TurnOutcome::Continue)
    );
    assert!(state.manual_turn().is_some());
}

#[test]
fn a_queued_cast_resolves_at_the_next_manual_turn_without_a_menu() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    state.fighters[0].memorized_list = vec![0x17]; // hold person: delay 5/3 = 1
    let out = state
        .issue(
            &mut rng,
            TurnCmd::CastSpell {
                spell_id: 0x17,
                targets: vec![1],
            },
        )
        .unwrap();
    assert_eq!(out, TurnOutcome::TurnEnded);
    assert_eq!(
        state.pending_spell(0),
        Some(0x17),
        "a delayed cast queues instead of firing"
    );

    // Next suspension: the host must resolve the queue, not open a menu.
    let actor = loop {
        match state.step(&mut rng) {
            CombatStep::AwaitPlayerTurn { combatant_id } => break combatant_id,
            CombatStep::Ended => panic!("the fight ended before the cast resolved"),
            _ => {}
        }
    };
    assert_eq!(actor, 0);
    assert_eq!(state.pending_spell(0), Some(0x17));
    assert_eq!(
        state.issue(&mut rng, TurnCmd::ResolvePendingCast { targets: vec![1] }),
        Ok(TurnOutcome::TurnEnded)
    );
    assert_eq!(state.pending_spell(0), None);
}

// === SPACE, the AI-turn interrupt (§9.5) ================================

#[test]
fn space_revokes_quick_fight_and_hands_the_turn_back() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.fighters[0].quick_fight = true;
    // Initiative order is the point of this test: the PC must act first, so
    // that ITS OWN AI turn is the one the poll interrupts.
    state.fighters[0].reaction_adj = 5;
    state.fighters[1].reaction_adj = -4;
    state.set_interactive(true);
    // Queue the press at a step head, as a host does.
    state.queue_quick_fight_revoke();

    let mut saw_suspension = false;
    for _ in 0..40 {
        match state.step(&mut rng) {
            CombatStep::AwaitPlayerTurn { combatant_id } => {
                assert_eq!(combatant_id, 0);
                saw_suspension = true;
                break;
            }
            CombatStep::Ended => break,
            _ => {}
        }
    }
    assert!(saw_suspension, "SPACE never gave the turn back");
    assert!(!state.fighters[0].quick_fight, "the flag was revoked");
    assert!(
        state.fighters[1].quick_fight,
        "a monster keeps fighting itself"
    );
    // The 20 the poll parks was clamped to 19 by the turn head.
    assert_eq!(state.fighters[0].delay, 19);
}

#[test]
fn revoking_from_inside_the_menu_touches_every_player_controlled_combatant() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    let mut ally = state.fighters[0].clone();
    ally.id = 2;
    ally.pos = GridPos::new(4, 5);
    ally.quick_fight = true;
    state.fighters.push(ally);
    state.rebuild_occupancy();
    open_manual_turn(&mut state, &mut rng);
    assert_eq!(
        state.issue(&mut rng, TurnCmd::RevokeQuickFight),
        Ok(TurnOutcome::Continue)
    );
    assert!(!state.fighters[2].quick_fight);
    assert!(state.fighters[1].quick_fight, "monsters are not touched");
}

#[test]
fn the_auto_magic_toggle_flips_the_flag() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);
    assert!(!state.auto_pcs_cast_magic);
    state.issue(&mut rng, TurnCmd::ToggleAutoMagic).unwrap();
    assert!(state.auto_pcs_cast_magic);
    state.issue(&mut rng, TurnCmd::ToggleAutoMagic).unwrap();
    assert!(!state.auto_pcs_cast_magic);
}

// === ★ the dark-landing parity proof (§9.5's binding order) =============

/// Runs the duel to completion and returns `(steps, draws, events)`.
fn run_to_end(interactive: bool) -> (Vec<CombatStep>, Vec<u16>, Vec<ActionEvent>) {
    let draws = DrawLog::default();
    let actions = ActionLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(draws.sink());
    let mut state = duel();
    // Every PC quick-fighting — the D-CV5 invariant's precondition.
    state.fighters[0].quick_fight = true;
    state.attach_action_sink(actions.sink());
    state.set_interactive(interactive);

    let mut steps = Vec::new();
    for _ in 0..1000 {
        let step = state.step(&mut rng);
        steps.push(step);
        if step == CombatStep::Ended {
            break;
        }
    }
    (steps, draws.ns(), actions.events())
}

#[test]
fn with_every_quick_fight_flag_on_an_interactive_fight_is_bit_identical() {
    let (dark_steps, dark_draws, dark_events) = run_to_end(false);
    let (live_steps, live_draws, live_events) = run_to_end(true);
    assert_eq!(dark_steps, live_steps, "the step sequence must not move");
    assert_eq!(dark_draws, live_draws, "the draw stream must not move");
    assert_eq!(dark_events, live_events, "nor the event stream");
    assert!(
        dark_steps.len() > 3 && !dark_draws.is_empty(),
        "the fixture fight must actually fight"
    );
}

// === ★ the replay-side script driver (§9.6's closing capture) ============

use crate::combat::reel::{run_scripted, ScriptedTurn};

/// The duel with the PC's `quick_fight` still off AND the interactive flag
/// raised — the exact state `build_state` hands `run_scripted` for a capture
/// that carries a manual-turn schedule.
fn scripted_duel() -> CombatState {
    let mut state = duel();
    state.set_interactive(true);
    state
}

#[test]
fn run_scripted_with_an_empty_script_is_run_combat_observed() {
    // The 15 all-QuickFight captures ride this equivalence (their states are
    // never interactive), and the guard referees it at full scale; this is
    // the same claim in CI miniature.
    let run = |scripted: bool| {
        let draws = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(draws.sink());
        let mut state = duel(); // NOT interactive — an empty script never is
        let outcome = if scripted {
            run_scripted(&mut state, &mut rng, 50, &[], |_, _| {})
        } else {
            state.run_combat_observed(&mut rng, 50, |_, _| {})
        };
        (outcome, draws.ns())
    };
    let (a_outcome, a_draws) = run(false);
    let (b_outcome, b_draws) = run(true);
    assert_eq!(a_outcome, b_outcome);
    assert_eq!(a_draws, b_draws, "line for line");
    assert!(!a_draws.is_empty());
}

#[test]
fn an_unscripted_suspension_replays_as_quick() {
    // Default-Q is the staged captures' own testimony ("Quick to the end"):
    // the suspension resolves, the flag stays up, and the PC never suspends
    // again — the fight completes with no scripted rows at all.
    let mut rng = EngineRng::new(SEED);
    let mut state = scripted_duel();
    run_scripted(&mut state, &mut rng, 50, &[], |_, _| {});
    assert!(
        state.fighters[0].quick_fight,
        "the default answered Quick, which raises the flag"
    );
}

#[test]
fn a_scripted_turn_executes_at_its_occurrence_and_later_ones_default() {
    // Occurrence 0 is scripted (Guard); the PC's round-1 suspension is not,
    // so it defaults to Quick and the fight runs to an outcome.
    let mut rng = EngineRng::new(SEED);
    let mut state = scripted_duel();
    let script = [ScriptedTurn {
        occurrence: 0,
        actor: 0,
        cmds: vec![TurnCmd::Guard],
    }];
    run_scripted(&mut state, &mut rng, 50, &script, |_, _| {});
    assert!(state.fighters[0].quick_fight, "round 1 defaulted to Quick");
}

#[test]
#[should_panic(expected = "pins actor")]
fn a_scripted_turn_with_the_wrong_actor_panics() {
    let mut rng = EngineRng::new(SEED);
    let mut state = scripted_duel();
    let script = [ScriptedTurn {
        occurrence: 0,
        actor: 1, // the monster — the suspension parks combatant 0
        cmds: vec![TurnCmd::Guard],
    }];
    run_scripted(&mut state, &mut rng, 50, &script, |_, _| {});
}

#[test]
#[should_panic(expected = "never fired")]
fn a_scripted_row_that_never_fires_panics() {
    let mut rng = EngineRng::new(SEED);
    let mut state = scripted_duel();
    let script = [ScriptedTurn {
        occurrence: 99,
        actor: 0,
        cmds: vec![TurnCmd::Guard],
    }];
    run_scripted(&mut state, &mut rng, 50, &script, |_, _| {});
}

#[test]
#[should_panic(expected = "the turn ended at cmd")]
fn a_command_after_the_turn_ends_panics() {
    let mut rng = EngineRng::new(SEED);
    let mut state = scripted_duel();
    let script = [ScriptedTurn {
        occurrence: 0,
        actor: 0,
        cmds: vec![TurnCmd::Guard, TurnCmd::Quit], // Guard already ended it
    }];
    run_scripted(&mut state, &mut rng, 50, &script, |_, _| {});
}

#[test]
#[should_panic(expected = "left the turn open")]
fn a_script_that_does_not_close_the_turn_panics() {
    let mut rng = EngineRng::new(SEED);
    let mut state = scripted_duel();
    let script = [ScriptedTurn {
        occurrence: 0,
        actor: 0,
        cmds: vec![TurnCmd::BeginMove], // opens the loop, ends nothing
    }];
    run_scripted(&mut state, &mut rng, 50, &script, |_, _| {});
}

// === roll-credits slice 9a: in-combat item use ==========================

/// ★ `UseMagicItem`'s combat arm (`ovr020.cs:980-1086`): the item's spell is
/// cast through the ordinary combat machinery and **the turn ends** —
/// `whenCast != Camp` sets `arg_0` and runs `clear_actions` (`:1055-1060`).
///
/// The Wand of Fireballs' id (`0x2F`) is the one shipped combat item with a
/// transcribed row.
#[test]
fn using_an_item_casts_its_spell_and_ends_the_turn() {
    let log = ActionLog::default();
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.attach_action_sink(log.sink());
    state.fighters[0].has_items = true;
    let actor = open_manual_turn(&mut state, &mut rng);
    assert_eq!(actor, 0);
    log.events().clear();

    let outcome = state
        .issue(
            &mut rng,
            TurnCmd::UseItem {
                spell_id: 0x2F,
                targets: vec![1],
            },
        )
        .expect("a transcribed spell id");
    assert_eq!(outcome, TurnOutcome::TurnEnded, "the turn is spent");
    assert!(state.manual_turn().is_none());
    // The ordinary combat casting machinery ran: `sub_5D2E1`'s `Cast` beat
    // with the item's own id, and the area shape gathered its blast list.
    assert!(
        log.events().iter().any(|e| matches!(
            e,
            ActionEvent::Cast {
                caster_id: 0,
                spell_id: 0x2F
            }
        )),
        "the fireball was cast: {:?}",
        log.events()
    );
    // `clear_actions` (`ovr025.cs`): moves gone, no queued cast.
    assert_eq!(state.fighters[0].move_left, 0);
    assert!(state.fighters[0].pending_spell.is_none());
}

/// ★ **No memorized-list test.** A Cast of the same id is refused when it is
/// not memorized; the item carries the spell, so the Use is not.
#[test]
fn using_an_item_needs_no_memorized_spell() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.fighters[0].has_items = true;
    open_manual_turn(&mut state, &mut rng);
    assert!(state.fighters[0].memorized_list.is_empty());

    assert_eq!(
        state.issue(
            &mut rng,
            TurnCmd::CastSpell {
                spell_id: 0x2F,
                targets: vec![1],
            }
        ),
        Err(TurnRefusal::WordUnavailable { word: "Cast" }),
        "no memorized list, no Cast word"
    );
    assert!(state
        .issue(
            &mut rng,
            TurnCmd::UseItem {
                spell_id: 0x2F,
                targets: vec![1],
            }
        )
        .is_ok());
}

/// The two gates: no items at all, and `actions.can_use` spent
/// (`ovr009.cs:322`, `ovr020.cs:456-463`).
#[test]
fn use_is_refused_without_items_or_without_can_use() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    open_manual_turn(&mut state, &mut rng);

    let cmd = || TurnCmd::UseItem {
        spell_id: 0x2F,
        targets: vec![1],
    };
    assert_eq!(
        state.issue(&mut rng, cmd()),
        Err(TurnRefusal::WordUnavailable { word: "Use" }),
        "no items"
    );
    state.fighters[0].has_items = true;
    state.fighters[0].can_use = false;
    assert_eq!(
        state.issue(&mut rng, cmd()),
        Err(TurnRefusal::WordUnavailable { word: "Use" }),
        "can_use spent"
    );
}

/// `if (spellId == 0) arg_0 = false;` (`ovr020.cs:999-1002`) — an item with no
/// spell, or a scroll picker backed out of, does not spend the turn.
#[test]
fn using_an_item_with_no_spell_leaves_the_turn_open() {
    let mut rng = EngineRng::new(SEED);
    let mut state = duel();
    state.fighters[0].has_items = true;
    open_manual_turn(&mut state, &mut rng);
    assert_eq!(
        state.issue(
            &mut rng,
            TurnCmd::UseItem {
                spell_id: 0,
                targets: vec![],
            }
        ),
        Ok(TurnOutcome::Continue)
    );
    assert!(state.manual_turn().is_some());
}
