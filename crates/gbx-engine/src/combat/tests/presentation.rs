use super::*;

// ===========================================================================
// D-CV2 presentation events — emission ORDER and payload (M6 slice 2)
// ===========================================================================
//
// These pin what the combat scene consumes: the sequence a step's `ActionEvent`
// batch arrives in, and which payload each removal/heal/cast site carries. They
// do NOT pin draw-neutrality — that is the frontier guard's job at every commit
// (15/15 exact), plus slice 4's headless-vs-scene draw-parity invariant. What
// they add is the half the guard cannot see: the guard drops these events, so
// only a test that reads the sink can catch an emit that fires at the wrong
// moment, on the wrong path, or not at all.
//
// The one compile-level guarantee lives in `gbx-oracle`: `CollectorActionSink`
// matches `ActionEvent` exhaustively with a per-variant drop arm and no `_ =>`
// catch-all, so adding a variant here without deciding whether it belongs on the
// `.gbxtrace` wire fails to build. See `combat_events_are_dropped_by_the_collector`
// in `gbx-oracle/src/sink.rs` for the runtime half.

/// A two-combatant melee over open floor, with the action sink attached and the
/// camera parked on a known window. `combat_setup` is run and marked done, so a
/// later `step()` will not re-run it (and re-seed the camera) mid-test.
fn scene_world(positions: &[(Team, GridPos)]) -> (CombatWorld, ActionLog) {
    let fighters = positions
        .iter()
        .enumerate()
        .map(|(i, (team, pos))| {
            Fighter::new_melee(
                i,
                *team,
                *team == Team::Monster,
                *pos,
                20,
                5,
                20,
                12,
                (1, 6, 0),
                5,
                1,
            )
        })
        .collect();
    let mut w = CombatWorld::new(CombatMap::uniform(FLOOR), fighters);
    w.combat_setup();
    w.combat_setup_done = true;
    let log = ActionLog::default();
    w.attach_action_sink(log.sink());
    (w, log)
}

// --- Camera ----------------------------------------------------------------

#[test]
fn camera_emits_only_when_the_window_actually_moves() {
    let (mut w, log) = scene_world(&[(Team::Party, GridPos::new(20, 12))]);
    let before = w.camera_top_left();

    // A probe inside the ±radius box does not scroll (`screen_map_check`
    // returns false) — and must emit nothing.
    let inside = GridPos::new(before.x + SCREEN_HALF, before.y + SCREEN_HALF);
    assert!(!w.screen_map_check(3, inside));
    assert!(log.events().is_empty(), "a no-op scroll emits no Camera");

    // A forced recenter far from the window scrolls, and reports the NEW
    // top-left — the value the scene's presented camera adopts.
    assert!(w.screen_map_check(0xFF, GridPos::new(40, 18)));
    let after = w.camera_top_left();
    assert_ne!(after, before, "the window moved");
    assert_eq!(
        log.events(),
        vec![ActionEvent::Camera { top_left: after }],
        "one Camera carrying the post-scroll top-left"
    );
}

#[test]
fn the_setup_camera_is_a_boundary_read_not_an_event() {
    // D-CV2: the entry snapshot reads the camera AFTER the first step(),
    // because `combat_setup` seeds it lazily there (`ovr011.cs:1209` — centre
    // on roster index 0). That write is not a scroll and emits nothing.
    let fighters = vec![Fighter::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(20, 12),
        20,
        5,
        20,
        12,
        (1, 6, 0),
        5,
        1,
    )];
    let mut w = CombatWorld::new(CombatMap::uniform(FLOOR), fighters);
    let log = ActionLog::default();
    w.attach_action_sink(log.sink());

    let mut rng = EngineRng::new(SEED);
    w.step(&mut rng);

    assert_eq!(
        w.camera_top_left(),
        GridPos::new(20 - SCREEN_HALF, 12 - SCREEN_HALF),
        "the setup camera centres on roster index 0"
    );
    assert!(
        !log.events()
            .iter()
            .any(|e| matches!(e, ActionEvent::Camera { .. })),
        "the setup write is the snapshot, not a scroll"
    );
}

// --- Move relocation -------------------------------------------------------

#[test]
fn move_emits_after_the_scroll_and_before_everything_downstream() {
    // `sub_3e748` has two focus-gated scroll sites: radius-2 to the OLD cell
    // before the position write (@294) and radius-3 to the NEW cell after it
    // (@309). D-CV2 relocated `Move` from after BOTH of them to just after the
    // position write, so playback reads scroll, step — not scroll, scroll, step.
    //
    // Finding worth recording: in the modeled state the two sites are mutually
    // exclusive, so only the FIRST can be observed. `on_screen_pos` and
    // `screen_map_check`'s box test share the same ±3 half-window, so a
    // destination that is off-screen (firing site 1) always leaves the centre
    // within 1 cell of the destination afterwards — including at the map edges,
    // where the [3,46]×[3,21] centre clamp is exactly 3 from the boundary — and
    // site 2's radius-3 test then finds nothing to do. The relocation is
    // therefore presentation-correct by construction rather than currently
    // order-visible; this test pins the sequence that IS observable, so a future
    // change that makes site 2 live cannot silently re-order the step.
    let (mut w, log) = scene_world(&[(Team::Party, GridPos::new(20, 12))]);
    w.focus = true;
    // Park the window far away so the destination is off-screen (site 1 fires).
    w.screen_map_check(0xFF, GridPos::new(40, 20));
    let events_before = log.events().len();
    w.fighters[0].move_left = 99;

    let mut rng = EngineRng::new(SEED);
    w.sub_3e748(&mut rng, 0, 2); // step east

    let ev = log.events();
    assert_eq!(
        &ev[events_before..],
        &[
            // Site 1, scrolling to the OLD cell — the pre-move window.
            ActionEvent::Camera {
                top_left: GridPos::new(20 - SCREEN_HALF, 12 - SCREEN_HALF),
            },
            ActionEvent::Move {
                combatant_id: 0,
                from_x: 20,
                from_y: 12,
                to_x: 21,
                to_y: 12,
                cost: 2,
            },
            // §1.4: the step has no timer — a redraw and a step sound.
            ActionEvent::Sound { id: sound::STEP },
        ],
        "scroll, then step: {ev:?}"
    );
}

#[test]
fn a_move_with_no_scroll_emits_move_and_its_sound_alone() {
    let (mut w, log) = scene_world(&[(Team::Party, GridPos::new(20, 12))]);
    w.focus = false; // both scroll sites are focus-gated
    w.fighters[0].move_left = 99;
    let mut rng = EngineRng::new(SEED);
    w.sub_3e748(&mut rng, 0, 2);
    assert_eq!(
        log.events(),
        vec![
            ActionEvent::Move {
                combatant_id: 0,
                from_x: 20,
                from_y: 12,
                to_x: 21,
                to_y: 12,
                cost: 2,
            },
            ActionEvent::Sound { id: sound::STEP },
        ]
    );
}

// --- Removed ---------------------------------------------------------------

/// Damage a lone target to a chosen health status and return the removal event.
fn removal_by_damage(hp: i32, damage: i32) -> (HealthStatus, Vec<ActionEvent>) {
    let (mut w, log) = scene_world(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Monster, GridPos::new(21, 12)),
    ]);
    w.focus = false;
    w.fighters[1].hp_current = hp;
    let mut rng = EngineRng::new(SEED);
    w.apply_damage(&mut rng, 1, damage);
    (w.fighters[1].health_status, log.events())
}

#[test]
fn removed_carries_the_reason_of_each_removal_path() {
    // The `ovr025.cs:1197-1216` ladder, one arm at a time.
    let (status, ev) = removal_by_damage(10, 25); // overkill 15 > 9 → dead
    assert_eq!(status, HealthStatus::Dead);
    assert!(
        ev.contains(&ActionEvent::Removed {
            combatant_id: 1,
            reason: RemovalReason::Killed,
        }),
        "overkill > 9 is \"is killed\": {ev:?}"
    );

    let (status, ev) = removal_by_damage(10, 15); // overkill 5 → dying
    assert_eq!(status, HealthStatus::Dying);
    assert!(
        ev.contains(&ActionEvent::Removed {
            combatant_id: 1,
            reason: RemovalReason::Downed { dying: true },
        }),
        "overkill 1..=9 goes down Dying: {ev:?}"
    );

    let (status, ev) = removal_by_damage(10, 10); // exact 0 → unconscious
    assert_eq!(status, HealthStatus::Unconscious);
    assert!(
        ev.contains(&ActionEvent::Removed {
            combatant_id: 1,
            reason: RemovalReason::Downed { dying: false },
        }),
        "an exact drop to 0 goes down unconscious: {ev:?}"
    );
}

#[test]
fn a_survivor_is_never_removed() {
    let (status, ev) = removal_by_damage(20, 5);
    assert_eq!(status, HealthStatus::Okey);
    assert!(ev.is_empty(), "a wound is not a removal: {ev:?}");
}

#[test]
fn the_death_beat_is_sound_then_removed_after_its_camera() {
    // `CombatantKilled` scrolls to an off-screen victim BEFORE `size = 0`
    // (`ovr033:550`), so the Camera that brings the death into view precedes
    // the beat itself.
    let (mut w, log) = scene_world(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Monster, GridPos::new(45, 20)),
    ]);
    w.fighters[1].hp_current = 4;
    assert!(!w.on_screen(1), "the victim starts off-screen");
    let mut rng = EngineRng::new(SEED);
    w.apply_damage(&mut rng, 1, 40);

    let ev = log.events();
    assert!(
        matches!(ev.first(), Some(ActionEvent::Camera { .. })),
        "the CombatantKilled scroll comes first: {ev:?}"
    );
    assert_eq!(
        &ev[ev.len() - 2..],
        &[
            ActionEvent::Sound { id: sound::DEATH },
            ActionEvent::Removed {
                combatant_id: 1,
                reason: RemovalReason::Killed,
            },
        ],
        "then the death sound and the removal: {ev:?}"
    );
}

#[test]
fn surrender_removal_reports_surrendered() {
    // The §50 capture-proven surrender fork (buffed-otyugh): a slow Int>5 NPC
    // against a faster party lands in `RemoveFromCombat("Surrenders")`.
    let mk = |team, npc, pos, movement| {
        Fighter::new_melee(0, team, npc, pos, 30, 5, 20, movement, (1, 4, 2), 5, 1)
    };
    let mut w = CombatWorld::new(
        CombatMap::uniform(FLOOR),
        vec![
            {
                let mut f = mk(Team::Party, false, GridPos::new(25, 12), 48);
                f.id = 0;
                f
            },
            {
                let mut f = mk(Team::Monster, true, GridPos::new(26, 12), 12);
                f.id = 1;
                f
            },
        ],
    );
    w.combat_setup();
    w.combat_setup_done = true;
    w.enemy_health_pct = 5;
    w.area_field_58c = 0;
    w.fighters[1].int_score = 10;
    let log = ActionLog::default();
    w.attach_action_sink(log.sink());

    assert!(w.flee_check(1), "Int>5 surrenders");
    let ev = log.events();
    assert_eq!(
        ev.last(),
        Some(&ActionEvent::Removed {
            combatant_id: 1,
            reason: RemovalReason::Surrendered,
        }),
        "the surrender removal is the turn's last beat: {ev:?}"
    );
    assert!(
        !ev.iter()
            .any(|e| matches!(e, ActionEvent::Sound { id: sound::DEATH })),
        "nobody died: {ev:?}"
    );
}

#[test]
fn got_away_removal_reports_fled() {
    let (mut w, log) = scene_world(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Monster, GridPos::new(21, 12)),
    ]);
    w.focus = false;
    // `flee_battle`'s Got-Away arm keeps the fleer's hp (status `running`).
    w.fighters[1].moral_failure = true;
    w.fighters[1].fleeing = true;
    let mut rng = EngineRng::new(SEED);
    w.flee_battle(&mut rng, 1);
    let ev = log.events();
    if w.fighters[1].health_status == HealthStatus::Running {
        assert!(
            ev.contains(&ActionEvent::Removed {
                combatant_id: 1,
                reason: RemovalReason::Fled,
            }),
            "the Got-Away removal reports Fled: {ev:?}"
        );
        assert!(w.fighters[1].hp_current > 0, "a fleer keeps its hp");
    } else {
        assert!(
            !ev.iter().any(|e| matches!(e, ActionEvent::Removed { .. })),
            "escape was blocked — no removal: {ev:?}"
        );
    }
}

// --- Bled + ContinueBattlePrompt (the round-end batch) ----------------------

#[test]
fn bled_fires_once_per_dying_member_and_flags_the_bleed_out() {
    let (mut w, log) = scene_world(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Party, GridPos::new(21, 12)),
        (Team::Monster, GridPos::new(30, 12)),
    ]);
    // One member mid-bleed, one at the `bleeding > 9` edge.
    w.fighters[0].health_status = HealthStatus::Dying;
    w.fighters[0].bleeding = 3;
    w.fighters[1].health_status = HealthStatus::Dying;
    w.fighters[1].bleeding = 9; // → 10 > 9 → dead

    let step = w.battle_round_checks();
    assert!(matches!(step, CombatStep::RoundEnded { .. }));

    assert_eq!(
        log.events(),
        vec![
            ActionEvent::Bled {
                combatant_id: 0,
                died: false,
            },
            ActionEvent::Bled {
                combatant_id: 1,
                died: true,
            },
        ],
        "one Bled per dying member, in roster order, `died` on the bleed-out"
    );
    assert_eq!(w.fighters[0].bleeding, 4);
    assert_eq!(w.fighters[1].health_status, HealthStatus::Dead);
}

#[test]
fn a_healthy_round_end_emits_no_bleed_beats() {
    let (mut w, log) = scene_world(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Monster, GridPos::new(30, 12)),
    ]);
    w.battle_round_checks();
    assert!(
        log.events().is_empty(),
        "nothing is dying: {:?}",
        log.events()
    );
}

#[test]
fn the_continue_battle_prompt_emits_on_both_answers() {
    // The prompt fires with 2+ party members up and no foes left
    // (`ovr009.cs:404-410`). It is emitted even though the schedule answers it
    // — the original displays the prompt in every case.
    let build = |yes: Vec<u16>| {
        let (mut w, log) = scene_world(&[
            (Team::Party, GridPos::new(20, 12)),
            (Team::Party, GridPos::new(21, 12)),
            (Team::Monster, GridPos::new(30, 12)),
        ]);
        w.fighters[2].in_combat = false; // no foes left
        w.continue_battle_yes = yes;
        let step = w.battle_round_checks();
        (step, log.events())
    };

    let (step, ev) = build(vec![]);
    assert_eq!(
        ev,
        vec![ActionEvent::ContinueBattlePrompt {
            answered_yes: false
        }]
    );
    assert!(matches!(
        step,
        CombatStep::RoundEnded {
            battle_over: true,
            ..
        }
    ));

    let (step, ev) = build(vec![0]);
    assert_eq!(
        ev,
        vec![ActionEvent::ContinueBattlePrompt { answered_yes: true }],
        "a scheduled 'Y' still displays the prompt"
    );
    assert!(
        matches!(
            step,
            CombatStep::RoundEnded {
                battle_over: false,
                ..
            }
        ),
        "'Y' overrides battleOver (doc §48)"
    );
}

#[test]
fn no_prompt_while_foes_remain() {
    let (mut w, log) = scene_world(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Party, GridPos::new(21, 12)),
        (Team::Monster, GridPos::new(30, 12)),
    ]);
    w.battle_round_checks();
    assert!(
        !log.events()
            .iter()
            .any(|e| matches!(e, ActionEvent::ContinueBattlePrompt { .. })),
        "the prompt is gated on `monsters == 0`"
    );
}

// --- the cast triple -------------------------------------------------------

/// The §48 cleric roster — a wounded cleric with cure/hold memorized.
fn cast_world() -> (CombatWorld, ActionLog) {
    let mut cleric = Fighter::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(10, 10),
        55,
        5,
        20,
        12,
        (1, 2, 2),
        8,
        1,
    );
    cleric.hp_current = 51;
    cleric.memorized_list = vec![3, 3, 0x17, 0x17];
    cleric.skill_level_cleric = 5;
    let mut fighters = vec![cleric];
    for i in 0..4 {
        let mut m = Fighter::new_melee(
            1 + i,
            Team::Monster,
            true,
            GridPos::new(12 + i as i32, 12),
            26,
            5,
            20,
            12,
            (1, 8, 0),
            5,
            1,
        );
        m.saves = [12, 11, 12, 15, 13];
        fighters.push(m);
    }
    let mut w = CombatWorld::new(CombatMap::uniform(FLOOR), fighters);
    w.combat_setup();
    w.combat_setup_done = true;
    w.auto_pcs_cast_magic = true;
    let log = ActionLog::default();
    w.attach_action_sink(log.sink());
    (w, log)
}

#[test]
fn a_queued_cast_emits_begins_casting_then_cast_and_its_targets() {
    // Hold person (0x17): `castingDelay / 3 == 1` ⇒ it QUEUES ("Begins
    // Casting", `ovr014.cs:1414-1427`) and resolves at the caster's next pick,
    // where the multi-target pass picks three targets (doc §48).
    let (mut w, log) = cast_world();
    w.fighters[0].delay = 8;
    let mut rng = EngineRng::new(SEED);
    assert!(w.spell_menu3(&mut rng, 0, 0x17), "the cast was accepted");
    assert_eq!(
        log.events(),
        vec![ActionEvent::BeginsCasting { caster_id: 0 }],
        "the queue emits BeginsCasting alone — the message does not name the spell"
    );
    assert_eq!(w.fighters[0].pending_spell, Some(0x17));

    // The resolution turn.
    let before = log.events().len();
    w.sub_5d2e1(&mut rng, 0, 0x17);
    let ev = log.events();
    let ev = &ev[before..];

    let cast_at = ev
        .iter()
        .position(|e| matches!(e, ActionEvent::Cast { .. }))
        .expect("the resolution emitted Cast");
    assert_eq!(
        ev[cast_at],
        ActionEvent::Cast {
            caster_id: 0,
            spell_id: 0x17,
        }
    );
    // Every SpellTarget of this cast follows its Cast immediately, in pick
    // order — one event per pick, no list payload (`ActionEvent` stays `Copy`).
    let picks: Vec<usize> = ev[cast_at + 1..]
        .iter()
        .take_while(|e| matches!(e, ActionEvent::SpellTarget { .. }))
        .map(|e| match e {
            ActionEvent::SpellTarget { target_id } => *target_id,
            _ => unreachable!(),
        })
        .collect();
    assert!(!picks.is_empty(), "hold person picked targets: {ev:?}");
    assert!(
        picks.iter().all(|&t| w.fighters[t].team == Team::Monster),
        "hold person targets enemies: {picks:?}"
    );
    assert!(
        !ev[..cast_at]
            .iter()
            .any(|e| matches!(e, ActionEvent::SpellTarget { .. })),
        "no SpellTarget precedes its Cast: {ev:?}"
    );
}

#[test]
fn an_immediate_cast_emits_no_begins_casting() {
    // Magic Missile: `castingDelay / 3 == 0` ⇒ it resolves in place
    // (`ovr014.cs:1406-1411`), so there is no "Begins Casting" beat.
    let (mut w, log) = cast_world();
    w.fighters[0].memorized_list = vec![0x0F];
    w.fighters[0].skill_level_magic_user = 5;
    w.fighters[0].delay = 8;
    let mut rng = EngineRng::new(SEED);
    w.spell_menu3(&mut rng, 0, 0x0F);
    let ev = log.events();
    assert!(
        !ev.iter()
            .any(|e| matches!(e, ActionEvent::BeginsCasting { .. })),
        "an immediate cast never queues: {ev:?}"
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, ActionEvent::Cast { spell_id: 0x0F, .. })),
        "…it emits Cast directly: {ev:?}"
    );
}

// --- Healed ----------------------------------------------------------------

#[test]
fn a_cure_reports_its_rolled_amount() {
    let (mut w, log) = cast_world();
    w.fighters[0].pending_spell = Some(0x03);
    let hp_before = w.fighters[0].hp_current;
    let mut rng = EngineRng::new(SEED);
    w.sub_5d2e1(&mut rng, 0, 0x03);

    let ev = log.events();
    let healed = ev
        .iter()
        .find_map(|e| match e {
            ActionEvent::Healed {
                healer_id,
                target_id,
                amount,
                kind,
            } => Some((*healer_id, *target_id, *amount, *kind)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the cure emitted Healed: {ev:?}"));
    assert_eq!(healed.0, 0, "the caster heals");
    assert_eq!(healed.3, HealKind::Cure);
    assert!((1..=8).contains(&healed.2), "one d8: {healed:?}");
    assert_eq!(
        w.fighters[healed.1].hp_current - hp_before,
        healed.2,
        "an uncapped heal moves hp by exactly the reported roll"
    );
    // The heal follows its own Cast, not some earlier one.
    let cast_at = ev
        .iter()
        .position(|e| matches!(e, ActionEvent::Cast { spell_id: 3, .. }))
        .expect("Cast");
    let heal_at = ev
        .iter()
        .position(|e| matches!(e, ActionEvent::Healed { .. }))
        .expect("Healed");
    assert!(cast_at < heal_at, "Cast precedes its effect: {ev:?}");
}

#[test]
fn a_bandage_reports_zero_amount_and_names_the_bandager() {
    let (mut w, log) = scene_world(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Party, GridPos::new(21, 12)),
    ]);
    w.fighters[1].health_status = HealthStatus::Dying;
    w.fighters[1].bleeding = 4;

    assert!(w.bandage(Some(0), true));
    assert_eq!(
        log.events(),
        vec![ActionEvent::Healed {
            healer_id: 0,
            target_id: 1,
            amount: 0,
            kind: HealKind::Bandage,
        }],
        "\"is bandaged\" restores no hp — it lifts dying → unconscious"
    );
    assert_eq!(w.fighters[1].health_status, HealthStatus::Unconscious);
    assert_eq!(w.fighters[1].bleeding, 0);
}

#[test]
fn a_display_only_bandage_scan_emits_nothing() {
    let (mut w, log) = scene_world(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Party, GridPos::new(21, 12)),
    ]);
    w.fighters[1].health_status = HealthStatus::Dying;
    assert!(w.bandage(None, false), "the scan still reports the bleeder");
    assert!(log.events().is_empty(), "a scan heals nobody");
    assert_eq!(w.fighters[1].health_status, HealthStatus::Dying);
}

// --- SlayHelpless ----------------------------------------------------------

#[test]
fn the_held_slay_announces_itself_before_the_kill() {
    // `sub_3F4EB`'s head (§49): a held target is slain draw-free, so the whole
    // beat is SlayHelpless → the damage cascade's Sound(DEATH) + Removed.
    let (mut w, log) = scene_world(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Monster, GridPos::new(21, 12)),
    ]);
    w.focus = false;
    w.fighters[0].delay = 5;
    w.fighters[0].attack1_left = 1;
    w.fighters[0].attack_idx = 1;
    w.fighters[1].hp_current = 12;
    w.fighters[1].add_affect(0x34, 5, 1, false); // paralyze — one of IsHeld's four
    assert!(w.is_held(1));

    let log_len = log.events().len();
    let mut rng = EngineRng::new(SEED);
    let draws = DrawLog::default();
    rng.attach_sink(draws.sink());
    assert!(
        w.attack_target(&mut rng, 0, 1, false, AttackItemRef::None),
        "the slay ends the turn"
    );
    assert_eq!(draws.len(), 0, "the held slay is draw-free (§49)");

    let ev = log.events();
    let ev = &ev[log_len..];
    assert_eq!(
        ev[0],
        ActionEvent::SlayHelpless {
            attacker_id: 0,
            target_id: 1,
        },
        "the announcement heads the beat: {ev:?}"
    );
    // The slay's damage is `hp_current + 5`, so its overkill is ALWAYS exactly
    // 5 — inside the ladder's `1..=9` dying band (`ovr025.cs:1197-1216`). A
    // "guaranteed kill" in the sense that matters (removed from combat, hp 0,
    // never picked again) lands as `Downed { dying: true }`, not `Killed`;
    // §1.5's message for it is "goes down and is Dying".
    assert_eq!(
        ev.last(),
        Some(&ActionEvent::Removed {
            combatant_id: 1,
            reason: RemovalReason::Downed { dying: true },
        }),
        "damage = hp + 5 is an overkill of 5, the dying band: {ev:?}"
    );
    assert!(!w.fighters[1].in_combat, "removed either way");
}
