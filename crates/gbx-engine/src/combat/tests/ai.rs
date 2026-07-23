use super::*;

// === the field_15 mode-gate (M4 combat #4, deliverable 3 start) =========

#[test]
fn field_15_gate_short_circuits_on_0_and_over_4() {
    // §15 bug #1: the entry short-circuit is `field_15 == 0 || field_15 > 4`
    // (binary `cmp 4; ja`), NOT `== 4`. So field_15 ∈ {0} ∪ {5,6,…} skips the
    // d4 gate → exactly TWO draws (d8 then the swapped tail), never three.
    for start in [0u8, 5u8, 6u8, 7u8] {
        let mut oracle = Replay::new(SEED);
        let d8 = oracle.roll(8);
        let tail = if d8 != 8 { 4 } else { 2 }; // swapped branch: d8!=8→d4, d8==8→d2+4

        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let out = field_15_mode_gate(&mut rng, start);
        let ns = log.ns();
        assert_eq!(ns.len(), 2, "field_15={start}: no d4 gate, just d8 + tail");
        assert_eq!(ns[0], 8, "first body draw is the d8");
        assert_eq!(
            ns[1], tail,
            "field_15={start}: d8={d8} → tail d{tail} (d8!=8→d4, d8==8→d2+4)"
        );
        assert!((1..=6).contains(&out), "result in 1..=6, got {out}");
    }
}

#[test]
fn field_15_gate_enters_the_body_when_over_4_gate_is_skipped() {
    // A concrete `field_15 > 4` start (5): the || short-circuits the d4 gate
    // and the body's swapped branch runs. Compare the exact stream + result to
    // an independent replay.
    let mut oracle = Replay::new(SEED);
    let d8 = oracle.roll(8);
    let expected = if d8 != 8 {
        oracle.roll(4)
    } else {
        oracle.roll(2) + 4
    };

    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    let out = field_15_mode_gate(&mut rng, 5);
    assert_eq!(log.ns(), vec![8, if d8 != 8 { 4 } else { 2 }]);
    assert_eq!(out as u16, expected, "matches an independent replay");
}

#[test]
fn field_15_gate_draws_the_d4_gate_for_1_through_4() {
    // §15 bug #1: field_15 ∈ 1..=4 evaluates the d4 gate (not short-circuited,
    // since it is neither 0 nor > 4). One d4 gate draw always; if it rolls 1 →
    // the 2-draw body follows (3 total); else just the gate (1 draw, value kept).
    let mut oracle = Replay::new(SEED);
    let gate = oracle.roll(4); // the first draw the gate will make

    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    let out = field_15_mode_gate(&mut rng, 3);
    let ns = log.ns();
    assert_eq!(ns[0], 4, "the gate's d4 is the first draw");
    if gate == 1 {
        assert_eq!(ns.len(), 3, "gate==1 → body follows");
        assert_eq!(ns[1], 8);
        assert!((1..=6).contains(&out));
    } else {
        assert_eq!(ns.len(), 1, "gate!=1 → only the gate draws");
        assert_eq!(out, 3, "field_15 unchanged when the gate doesn't fire");
    }
}

#[test]
fn field_15_gate_distribution_stays_in_range_and_respects_the_branch() {
    // Over many entries via the persistent field_15, every produced value is
    // 1..=6 and honors the §15-corrected branch: entry short-circuits on
    // `0 || >4`, and the body draws d4(1..4) when d8!=8 / d2+4(5..6) when d8==8.
    // Re-derive each gate with an independent replay to check the branch.
    let mut rng = EngineRng::new(SEED);
    let mut oracle = Replay::new(SEED);
    let mut field_15 = 0u8;
    for _ in 0..500 {
        let entered = field_15 == 0 || field_15 > 4 || {
            let g = oracle.roll(4);
            g == 1
        };
        let expected = if entered {
            let d8 = oracle.roll(8);
            if d8 != 8 {
                oracle.roll(4)
            } else {
                oracle.roll(2) + 4
            }
        } else {
            field_15 as u16
        };
        field_15 = field_15_mode_gate(&mut rng, field_15);
        assert_eq!(field_15 as u16, expected, "matches an independent replay");
        assert!((1..=6).contains(&field_15) || !entered);
    }
}

// === the melee AI turn — the parity artifact (M4 combat #4, D3/D6) =======

#[test]
fn melee_turn_adjacent_draws_the_exact_sequence() {
    // A monster (NPC) adjacent to a PC: mode-gate → the two behavior-guard d7s
    // → find_target pick (d1) → attack (d20 + damage on a hit). The exact
    // operand sequence AND values are hand-derived from an INDEPENDENT replay
    // (not the engine), so this is a real parity assertion (study §4.1.7).
    let dice = (2u8, 6u8, 1u8); // 2d6+1
    let mut world = CombatWorld::new(
        CombatMap::uniform(FLOOR),
        vec![
            Fighter::new_melee(
                0,
                Team::Monster,
                true,
                GridPos::new(25, 12),
                20,
                5,
                20,
                12,
                dice,
                5,
                1,
            ),
            Fighter::new_melee(
                1,
                Team::Party,
                false,
                GridPos::new(26, 12),
                20,
                5,
                0,
                12,
                (1, 4, 0),
                5,
                1,
            ),
        ],
    );

    // Independent replay → the expected (operand) stream, branch-following.
    let mut o = Replay::new(SEED);
    let mut expect: Vec<u16> = Vec::new();
    // field_15 gate: field_15 starts 0 → the || short-circuits the d4 gate.
    // §15 bug #1 swapped branch: d8!=8 → d4 (1..4); d8==8 → d2+4 (5..6).
    let d8 = o.roll(8);
    expect.push(8);
    if d8 != 8 {
        o.roll(4);
        expect.push(4);
    } else {
        o.roll(2);
        expect.push(2);
    }
    // wand-scan d7 (normal area), memorized-spell d7 (unconditional).
    o.roll(7);
    expect.push(7);
    o.roll(7);
    expect.push(7);
    // find_target: one target, d1 pick.
    o.roll(1);
    expect.push(1);
    // §18 bug #6: a monster attacker's held target is on the party team, so
    // the target-validity check drops it (ovr010:0F36 `cmp combat_team, 0`)
    // and it re-picks among adjacent PCs — one adjacent enemy → a d1 re-pick.
    o.roll(1);
    expect.push(1);
    // attack: one d20 to-hit; damage dice on a hit.
    let d20 = o.roll(20);
    expect.push(20);
    let effective = if d20 == 20 { 100 } else { d20 as i32 };
    let hit = d20 > 1 && effective + 20 >= 5; // hit_bonus 20 vs raw AC 5
    if hit {
        for _ in 0..dice.0 {
            o.roll(dice.1 as u16);
            expect.push(dice.1 as u16);
        }
    }

    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());
    world.melee_ai_turn(&mut rng, 0);

    assert_eq!(
        log.ns(),
        expect,
        "the melee turn's exact draw operand sequence"
    );
    assert_eq!(world.fighters[0].target, Some(1), "target was picked");
    assert_eq!(world.fighters[0].delay, 0, "turn spent (delay zeroed)");
    assert!(
        (1..=6).contains(&world.fighters[0].field_15),
        "field_15 updated"
    );
    if hit {
        assert!(
            world.fighters[1].hp_current < 20,
            "the PC took damage on a hit"
        );
    }
}

#[test]
fn monster_approach_draws_a_d100_per_step_but_a_pc_does_not() {
    // The control asymmetry (§4.1.4): an NPC approaching a distant target draws
    // the morale-advance d100 on each step; a PC in the identical geometry
    // short-circuits it and draws none. Both still close and attack.
    for npc in [true, false] {
        let (a_team, t_team) = if npc {
            (Team::Monster, Team::Party)
        } else {
            (Team::Party, Team::Monster)
        };
        let mut world = CombatWorld::new(
            CombatMap::uniform(FLOOR),
            vec![
                Fighter::new_melee(
                    0,
                    a_team,
                    npc,
                    GridPos::new(25, 8),
                    30,
                    5,
                    20,
                    12,
                    (1, 4, 2),
                    5,
                    1,
                ),
                Fighter::new_melee(
                    1,
                    t_team,
                    !npc,
                    GridPos::new(25, 12),
                    30,
                    5,
                    20,
                    12,
                    (1, 4, 2),
                    5,
                    1,
                ),
            ],
        );
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let start = world.fighters[0].pos;
        world.melee_ai_turn(&mut rng, 0);

        let d100s = log.ns().iter().filter(|&&n| n == 100).count();
        if npc {
            assert!(d100s >= 1, "an NPC draws a morale d100 per approach step");
        } else {
            assert_eq!(d100s, 0, "a PC never draws the morale-advance d100");
        }
        assert_ne!(
            world.fighters[0].pos, start,
            "the actor moved toward the target"
        );
        assert!(
            log.ns().contains(&20),
            "and eventually swung (a d20 to-hit)"
        );
    }
}

#[test]
fn all_ai_1v1_fight_is_deterministic_terminates_and_is_prng_consistent() {
    // The D6 artifact (turn level): two adjacent all-AI combatants trade blows
    // over rounds until one falls. Same seed → byte-identical draw stream
    // (determinism); a victor emerges (termination); and every captured draw
    // reproduces through an independent `Prng` (before→result→after chain).
    fn run_fight(seed: u32) -> (Vec<RngDraw>, usize) {
        let log = DrawLog::default();
        let mut rng = EngineRng::new(seed);
        rng.attach_sink(log.sink());
        let mut world = CombatWorld::new(
            CombatMap::uniform(FLOOR),
            vec![
                Fighter::new_melee(
                    0,
                    Team::Monster,
                    true,
                    GridPos::new(25, 12),
                    12,
                    5,
                    20,
                    12,
                    (1, 6, 1),
                    5,
                    1,
                ),
                Fighter::new_melee(
                    1,
                    Team::Party,
                    false,
                    GridPos::new(26, 12),
                    12,
                    5,
                    20,
                    12,
                    (1, 6, 1),
                    5,
                    1,
                ),
            ],
        );
        let mut winner = usize::MAX;
        for _round in 0..100 {
            for actor in 0..2 {
                if world.fighters[actor].in_combat && world.fighters[actor].delay > 0 {
                    world.melee_ai_turn(&mut rng, actor);
                }
            }
            let alive: Vec<usize> = (0..2).filter(|&i| world.fighters[i].in_combat).collect();
            if alive.len() <= 1 {
                winner = *alive.first().unwrap_or(&usize::MAX);
                break;
            }
            // Initiative stub for the next round: re-arm each survivor's delay +
            // per-round attack (so multi-round trades occur).
            for i in 0..2 {
                if world.fighters[i].in_combat {
                    world.fighters[i].delay = 5;
                    world.fighters[i].attack1_left = 1;
                    world.fighters[i].attack_idx = 2;
                }
            }
        }
        let draws = log.draws.borrow().clone();
        (draws, winner)
    }

    let (draws1, w1) = run_fight(SEED);
    let (draws2, w2) = run_fight(SEED);
    assert_eq!(draws1, draws2, "same seed → identical draw stream");
    assert_eq!(w1, w2, "deterministic victor");
    assert_ne!(w1, usize::MAX, "the fight produced a victor");
    assert!(!draws1.is_empty(), "the fight drew from the PRNG");

    // Every draw reproduces through an independent Prng replay of the seed.
    let mut p = Prng::new(SEED);
    for (i, d) in draws1.iter().enumerate() {
        assert_eq!(
            d.before,
            p.state(),
            "draw {i}: before-state matches the replay"
        );
        let r = p.random(d.n.expect("operand recorded"));
        assert_eq!(Some(r), d.result, "draw {i}: result matches the replay");
        assert_eq!(
            d.after,
            p.state(),
            "draw {i}: after-state matches the replay"
        );
    }
}

#[test]
fn run_combat_full_round_loop_is_a_parity_artifact() {
    // The real all-AI round loop (initiative → FindNextCombatant → melee turns):
    // a 2v2 fight run to a decision. Deterministic, terminating, Prng-consistent,
    // and it opens with the round-loop fingerprint — one initiative d6 per
    // combatant before any d100 selection (study §2).
    fn run(seed: u32) -> (Vec<RngDraw>, CombatOutcome, [bool; 4]) {
        let log = DrawLog::default();
        let mut rng = EngineRng::new(seed);
        rng.attach_sink(log.sink());
        let mut world = CombatWorld::new(
            CombatMap::uniform(FLOOR),
            vec![
                Fighter::new_melee(
                    0,
                    Team::Party,
                    false,
                    GridPos::new(25, 14),
                    8,
                    5,
                    20,
                    12,
                    (1, 6, 2),
                    0,
                    1,
                ),
                Fighter::new_melee(
                    1,
                    Team::Party,
                    false,
                    GridPos::new(26, 14),
                    8,
                    5,
                    20,
                    12,
                    (1, 6, 2),
                    0,
                    1,
                ),
                Fighter::new_melee(
                    2,
                    Team::Monster,
                    true,
                    GridPos::new(25, 12),
                    8,
                    5,
                    20,
                    12,
                    (1, 6, 2),
                    0,
                    1,
                ),
                Fighter::new_melee(
                    3,
                    Team::Monster,
                    true,
                    GridPos::new(26, 12),
                    8,
                    5,
                    20,
                    12,
                    (1, 6, 2),
                    0,
                    1,
                ),
            ],
        );
        let outcome = world.run_combat(&mut rng, DEFAULT_NO_ACTION_LIMIT);
        let alive = [
            world.fighters[0].in_combat,
            world.fighters[1].in_combat,
            world.fighters[2].in_combat,
            world.fighters[3].in_combat,
        ];
        let draws = log.draws.borrow().clone();
        (draws, outcome, alive)
    }

    let (draws1, o1, a1) = run(SEED);
    let (draws2, o2, a2) = run(SEED);
    assert_eq!(draws1, draws2, "same seed → identical draw stream");
    assert_eq!((o1, a1), (o2, a2), "deterministic outcome");
    assert!(!draws1.is_empty());

    // The round opens with one d6 per combatant (initiative), before selection.
    let ns: Vec<u16> = draws1.iter().map(|d| d.n.unwrap()).collect();
    assert_eq!(&ns[0..4], &[6, 6, 6, 6], "four initiative d6s open round 0");
    assert_eq!(ns[4], 100, "then the first FindNextCombatant d100");

    // A decisive fight ends with one side wiped; a stalemate leaves both alive.
    let party_alive = a1[0] || a1[1];
    let monsters_alive = a1[2] || a1[3];
    match o1 {
        CombatOutcome::PartyWins => assert!(party_alive && !monsters_alive),
        CombatOutcome::MonstersWin => assert!(!party_alive && monsters_alive),
        CombatOutcome::Stalemate => {}
    }

    // Prng-consistent across the whole fight.
    let mut p = Prng::new(SEED);
    for (i, d) in draws1.iter().enumerate() {
        assert_eq!(d.before, p.state(), "draw {i} before");
        assert_eq!(Some(p.random(d.n.unwrap())), d.result, "draw {i} result");
        assert_eq!(d.after, p.state(), "draw {i} after");
    }
}

#[test]
fn run_combat_driver_matches_raw_step_pumping_draw_for_draw() {
    // Deliverable 3b — the model-unification proof: `run_combat` is now a THIN
    // DRIVER over `step()`, so the tick machine alone must produce the ENTIRE
    // fight. Drive one fight via `run_combat` and an identical one by pumping
    // `step()` straight to `Ended` (a bare `while step() != Ended {}`), and
    // assert the two whole-fight draw streams are byte-identical and the final
    // combatant state matches — the merge added nothing and hid nothing. (This
    // is the "whole-fight draw stream identical whether driven by the driver or
    // the raw tick loop" assertion the brief asks for.)
    fn build() -> CombatState {
        CombatState::new(
            CombatMap::uniform(FLOOR),
            vec![
                Fighter::new_melee(
                    0,
                    Team::Party,
                    false,
                    GridPos::new(25, 14),
                    8,
                    5,
                    20,
                    12,
                    (1, 6, 2),
                    0,
                    1,
                ),
                Fighter::new_melee(
                    1,
                    Team::Party,
                    false,
                    GridPos::new(26, 14),
                    8,
                    5,
                    20,
                    12,
                    (1, 6, 2),
                    0,
                    1,
                ),
                Fighter::new_melee(
                    2,
                    Team::Monster,
                    true,
                    GridPos::new(25, 12),
                    8,
                    5,
                    20,
                    12,
                    (1, 6, 2),
                    0,
                    1,
                ),
                Fighter::new_melee(
                    3,
                    Team::Monster,
                    true,
                    GridPos::new(26, 12),
                    8,
                    5,
                    20,
                    12,
                    (1, 6, 2),
                    0,
                    1,
                ),
            ],
        )
    }

    // Path A: the run_combat driver.
    let log_a = DrawLog::default();
    let mut rng_a = EngineRng::new(SEED);
    rng_a.attach_sink(log_a.sink());
    let mut a = build();
    a.run_combat(&mut rng_a, DEFAULT_NO_ACTION_LIMIT);

    // Path B: pump step() directly to Ended (a headless `while step() != Ended`).
    // `new` already defaulted no_action_limit to DEFAULT_NO_ACTION_LIMIT — the
    // same cap run_combat applied — so the two fights share every parameter.
    let log_b = DrawLog::default();
    let mut rng_b = EngineRng::new(SEED);
    rng_b.attach_sink(log_b.sink());
    let mut b = build();
    while b.step(&mut rng_b) != CombatStep::Ended {}

    let draws_a = log_a.draws.borrow().clone();
    let draws_b = log_b.draws.borrow().clone();
    assert!(!draws_a.is_empty(), "the fight drew from the PRNG");
    assert_eq!(
        draws_a, draws_b,
        "run_combat and raw step() pumping draw the exact same whole-fight stream"
    );

    // …and reach the exact same fight (final HP + alive flags across the roster).
    let final_a: Vec<(i32, bool)> = a
        .fighters
        .iter()
        .map(|f| (f.hp_current, f.in_combat))
        .collect();
    let final_b: Vec<(i32, bool)> = b
        .fighters
        .iter()
        .map(|f| (f.hp_current, f.in_combat))
        .collect();
    assert_eq!(final_a, final_b, "identical final combatant state");
}

#[test]
fn ai_action_events_emit_and_are_inert_on_the_draw_stream() {
    // D-OR3: attaching an ActionSink must NOT change the draw stream. Run the
    // same monster-approach turn with and without a sink — identical draws —
    // and confirm the sink saw the pinned ai/morale/move events.
    fn run(with_sink: bool) -> (Vec<u16>, Vec<ActionEvent>) {
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let actions = ActionLog::default();
        let mut world = CombatWorld::new(
            CombatMap::uniform(FLOOR),
            vec![
                Fighter::new_melee(
                    0,
                    Team::Monster,
                    true,
                    GridPos::new(25, 8),
                    30,
                    5,
                    20,
                    12,
                    (1, 4, 2),
                    5,
                    1,
                ),
                Fighter::new_melee(
                    1,
                    Team::Party,
                    false,
                    GridPos::new(25, 12),
                    30,
                    5,
                    20,
                    12,
                    (1, 4, 2),
                    5,
                    1,
                ),
            ],
        );
        if with_sink {
            world.attach_action_sink(actions.sink());
        }
        world.melee_ai_turn(&mut rng, 0);
        (log.ns(), actions.events())
    }

    let (ns_plain, _) = run(false);
    let (ns_sunk, events) = run(true);
    assert_eq!(
        ns_plain, ns_sunk,
        "the action sink is inert on the draw stream"
    );

    // The monster resolved a target (ai), checked morale on each step, and moved.
    assert!(
        events.iter().any(|e| matches!(
            e,
            ActionEvent::Ai {
                combatant_id: 0,
                target_id: 1,
                ..
            }
        )),
        "an ai event names the picked target"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            ActionEvent::Morale {
                combatant_id: 0,
                ..
            }
        )),
        "a morale event per approach step"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            ActionEvent::Move {
                combatant_id: 0,
                ..
            }
        )),
        "a move event per step"
    );
}

// --- stub tripwires (doc §24: the M5 ledger names itself) ---------------

/// Every deliberately-stubbed original mechanic must EMIT when reached, so
/// a replay that wanders into unmodeled territory produces a named finding
/// instead of a silent divergence. Three wires: `0-hd-sweep`
/// (try_sweep_attack vs hit_dice 0), `surrender-int5` (flee_check's omitted
/// Int branch), `spell-entry` (sub_3560B/ShouldCastSpellX reaching an
/// untranscribed spell, doc §41.2). The `downed-pc` wire was retired once the
/// downed-PC path was built (§26/§27); this test also pins that downing a party
/// member no longer trips. (The old `memorized-spells` wire was replaced by the
/// faithful selection loop, doc §41.)
#[test]
fn stub_tripwires_fire_when_unmodeled_mechanics_are_reached() {
    #[derive(Clone, Default)]
    struct Trips(Rc<RefCell<Vec<(usize, &'static str)>>>);
    impl ActionSink for Trips {
        fn on_action(&mut self, e: ActionEvent) {
            if let ActionEvent::StubTripped { combatant_id, stub } = e {
                self.0.borrow_mut().push((combatant_id, stub));
            }
        }
    }

    let mk = |team, npc, pos, movement| {
        Fighter::new_melee(0, team, npc, pos, 30, 5, 20, movement, (1, 4, 2), 5, 1)
    };
    let mut world = CombatWorld::new(
        CombatMap::uniform(FLOOR),
        vec![
            {
                let mut f = mk(Team::Party, false, GridPos::new(25, 12), 12);
                f.id = 0;
                f
            },
            {
                let mut f = mk(Team::Monster, true, GridPos::new(26, 12), 12);
                f.id = 1;
                f
            },
            // A fast opposing monster so flee_check's `max_opp > own/2` else
            // branch (the surrender wire) is reachable for fighter 1.
            {
                let mut f = mk(Team::Monster, true, GridPos::new(30, 12), 12);
                f.id = 2;
                f
            },
        ],
    );
    let trips = Trips::default();
    world.attach_action_sink(Box::new(trips.clone()));

    // 1. downing a party member: no longer trips (the downed-pc wire was
    // retired, §26/§27). Overkill 99 ≫ 9 → dead, out of combat, tile stamped.
    world.apply_damage(0, 99);
    assert!(!world.fighters[0].in_combat);
    assert_eq!(world.fighters[0].health_status, HealthStatus::Dead);
    assert_eq!(
        world.map.ground_tile(GridPos::new(25, 12)),
        TILE_DOWN_PLAYER
    );

    // 2. 0-hd-sweep: a 0-HD target reaches the stubbed sweep guard.
    world.fighters[2].hit_dice = 0;
    assert!(!world.try_sweep_attack(2, 1));

    // 3. surrender-int5: an NPC whose fastest opponent outruns half its own
    // moves lands in the binary's Int>5 surrender branch. Party fighter 0 is
    // down, so make the survivor fast via a fresh party opponent. fighter 1 is
    // an NPC (control_morale 0x80 → the faithful gate-2 seed is 0, so gate 1
    // passes via `== 0`); enemy_health_pct 5 < 100 − field_58C(0) → gate 2
    // passes; max_opp = calc_moves(48)/2 = 48 > calc_moves(12)/2 = 12 → the
    // surrender fork.
    world.fighters[0].in_combat = true; // revive the opponent for the ladder
    world.fighters[0].movement = 48;
    world.enemy_health_pct = 5;
    world.area_field_58c = 0;
    assert!(!world.flee_check(1));

    // 4. spell-entry: an NPC caster whose memorized list holds an untranscribed
    // spell (Shield 0x10) reaches ShouldCastSpellX with a non-MM id — the lazy-
    // transcription reject (doc §41.2), via the `control_morale >= 0x80` arm of
    // the sub_3560B gate.
    world.fighters[1].memorized_list = vec![0x10];
    let mut rng = EngineRng::new(SEED);
    world.melee_ai_turn(&mut rng, 1);

    // 4b. the sub_3560B PC gates (`ovr010:0682-0692`): a PARTY caster with
    // memorized slots reaches no selection draws (hence no spell-entry) while
    // `AutoPCsCastMagic` is off (capture-proven: bar-fists-2 closes with two
    // memorized slots and zero spell draws, doc §33) — the toggle arms it.
    let pc_spell_entry = |trips: &Trips| {
        trips
            .0
            .borrow()
            .iter()
            .filter(|(id, s)| *id == 0 && *s == "spell-entry")
            .count()
    };
    // Fighter 1's turn above re-killed the negative-hp fighter 0 — restore
    // him to a real live PC before running HIS turns.
    world.fighters[0].in_combat = true;
    world.fighters[0].hp_current = 30;
    world.fighters[0].health_status = HealthStatus::Okey;
    world.fighters[0].memorized_list = vec![0x10];
    world.melee_ai_turn(&mut rng, 0);
    assert_eq!(pc_spell_entry(&trips), 0, "PC + magic OFF must not select");
    world.auto_pcs_cast_magic = true;
    world.melee_ai_turn(&mut rng, 0);
    assert!(pc_spell_entry(&trips) >= 1, "PC + magic ON must select");

    let got: Vec<&'static str> = trips.0.borrow().iter().map(|(_, s)| *s).collect();
    assert!(
        !got.contains(&"downed-pc"),
        "the downed-pc wire was retired (§26/§27): {got:?}"
    );
    assert!(got.contains(&"0-hd-sweep"), "trips: {got:?}");
    assert!(got.contains(&"surrender-int5"), "trips: {got:?}");
    assert!(got.contains(&"spell-entry"), "trips: {got:?}");
}

/// §38 — the mid-combat "Magic On" toggle schedule: each listed global
/// turn ordinal is one buffered '2' press, consumed at that turn's head
/// (the `sub_36269` keyboard poll, run from `sub_3504B+D`), flipping
/// `AutoPCsCastMagic` — so a second entry flips it back off.
#[test]
fn auto_cast_toggle_schedule_flips_at_the_listed_turn_heads() {
    let mk = |id, team, npc, pos| {
        let mut f = Fighter::new_melee(0, team, npc, pos, 30, 5, 20, 12, (1, 4, 2), 5, 1);
        f.id = id;
        f
    };
    let mut w = CombatWorld::initiative_only(vec![
        mk(0, Team::Party, false, GridPos::new(25, 12)),
        mk(1, Team::Monster, true, GridPos::new(26, 12)),
    ]);
    w.auto_cast_toggles = vec![1, 3];
    let mut rng = EngineRng::new(SEED);
    // Turn ordinals 0..=3: press #1 lands at ordinal 1's head, #2 at 3's.
    for (turn, want) in [false, true, true, false].into_iter().enumerate() {
        w.take_turn(&mut rng, turn % 2);
        assert_eq!(
            w.auto_pcs_cast_magic, want,
            "flag after turn ordinal {turn}"
        );
    }
}

/// §38 — the head-of-turn flip is visible to the SAME turn's `sub_3560B`
/// gate (the poll at `sub_3504B+D` precedes the gate read @`ovr010:068D`):
/// a press scheduled at a PC caster's own turn ordinal arms that very
/// turn's selection gate. This is the boundary that separates §38's
/// in-window ordinals from the round-1 overdraw (caster-bar's ordinal-2
/// contrast trips @83).
#[test]
fn auto_cast_toggle_arms_the_flipped_turns_own_spell_gate() {
    #[derive(Clone, Default)]
    struct Trips(Rc<RefCell<Vec<(usize, &'static str)>>>);
    impl ActionSink for Trips {
        fn on_action(&mut self, e: ActionEvent) {
            if let ActionEvent::StubTripped { combatant_id, stub } = e {
                self.0.borrow_mut().push((combatant_id, stub));
            }
        }
    }

    let mk = |id, team, npc, pos| {
        let mut f = Fighter::new_melee(0, team, npc, pos, 30, 5, 20, 12, (1, 4, 2), 5, 1);
        f.id = id;
        f
    };
    let mut world = CombatWorld::new(
        CombatMap::uniform(FLOOR),
        vec![
            mk(0, Team::Party, false, GridPos::new(25, 12)),
            mk(1, Team::Monster, true, GridPos::new(26, 12)),
        ],
    );
    // An untranscribed memorized spell (Shield 0x10) makes the gate observable
    // through `spell-entry`: a passing gate runs the selection loop, which
    // reaches ShouldCastSpellX with a non-MM id (doc §41.2).
    world.fighters[0].memorized_list = vec![0x10];
    world.auto_cast_toggles = vec![0];
    let trips = Trips::default();
    world.attach_action_sink(Box::new(trips.clone()));
    let mut rng = EngineRng::new(SEED);
    world.take_turn(&mut rng, 0);
    assert!(
        world.auto_pcs_cast_magic,
        "the ordinal-0 press flips the flag at the first turn's head"
    );
    let selected = trips
        .0
        .borrow()
        .iter()
        .filter(|(id, s)| *id == 0 && *s == "spell-entry")
        .count();
    assert!(
        selected >= 1,
        "the flipped turn's own gate must see the flag ON (selection ran)"
    );
}

/// **Bug #12 pinned** — `FleeCheck_001`'s gate 2 is an UNSIGNED 16-bit `jb`
/// over `100 − area2.field_58C` computed as a 16-bit `sub` (`sub_3637F`
/// @`ovr010:1473`/`:1481`), so a `field_58C > 100` underflows the threshold to
/// ~0xFFxx and the gate is **always true** — where coab's signed int makes it
/// always false. This pins the always-true behavior: with a monster at 100%
/// enemy-health (a morale that a *signed* threshold `100 − 150 = −50` would
/// reject), a `field_58C = 150` still lets the ladder proceed to the speed fork
/// and set `moral_failure`. The `field_58C = 50` contrast (signed==unsigned in
/// range) rejects the same morale, proving it is the wrap, not the value.
#[test]
fn flee_check_gate2_field_58c_over_100_is_always_true_bug12() {
    // fighter 0: a slow party opponent (so the speed fork takes the flee
    // branch, not surrender). fighter 1: the acting NPC monster (control_morale
    // 0x80 → morale seed 0 → gate 1 passes via `== 0`; full HP).
    let slow = Fighter::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(25, 12),
        30,
        5,
        20,
        1,
        (1, 4, 2),
        5,
        1,
    );
    let fast_npc = Fighter::new_melee(
        1,
        Team::Monster,
        true,
        GridPos::new(26, 12),
        30,
        5,
        20,
        96,
        (1, 4, 2),
        5,
        1,
    );
    let mut world = CombatWorld::new(CombatMap::uniform(FLOOR), vec![slow, fast_npc]);
    // 100% enemy health → after gate 1, monster_morale = 100. A *signed*
    // `100 − field_58C` at field_58C > 100 is negative, so `100 < negative`
    // would be false; the unsigned wrap makes it true.
    world.enemy_health_pct = 100;

    // field_58C = 150 (> 100): gate 2 is always-true (the underflow) → the
    // speed fork sets moral_failure (max_opp = calc_moves(1)/2 = 1 ≤
    // calc_moves(96)/2 = 96 → the flee branch).
    world.area_field_58c = 150;
    assert!(
        !world.flee_check(1),
        "the flee fork returns false (not surrender)"
    );
    assert!(
        world.fighters[1].moral_failure,
        "field_58C > 100 underflows gate 2 to always-true (bug #12), so the ladder \
         proceeds and sets moral_failure even at 100% enemy health"
    );

    // Contrast: field_58C = 50 (≤ 100, signed == unsigned) rejects the same
    // 100% morale at gate 2 (`100 < 100 − 50 = 50` is false) → no flee.
    world.area_field_58c = 50;
    assert!(!world.flee_check(1));
    assert!(
        !world.fighters[1].moral_failure,
        "field_58C ≤ 100 gates normally: 100% enemy health does not rout"
    );
}
