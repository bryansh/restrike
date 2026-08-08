//! Unit tests for the playback timeline (M6 slice 4): the §1.4 beat
//! durations, the order a step's batch plays back in, the §1.5 message
//! composition, and the fast-drain's equivalence to a played-out step.
//!
//! Every expected tick count here is **hand-computed from §1.4's table**, not
//! read back out of [`super::time`] — the point is to catch a beat that is
//! scheduled with the wrong quantity, which a test that re-derives its
//! expectation from the same constant cannot do.

use super::missile::{self, SpriteRef};
use super::render::{PanelOp, Row};
use super::tests::{board, presented};
use super::time::BeatClock;
use super::timeline::{burst_instructions, compose, Instruction, Op, Timeline};
use super::{PresentedBoard, SceneArt};
use crate::combat::{
    ActionEvent, AttackKind, GridPos, HealKind, HealthStatus, RemovalReason, Team,
};
use crate::combat_art::IconPose;

/// A three-combatant board: two party members and a monster, all adjacent
/// enough to swing at each other.
fn scene_board() -> PresentedBoard {
    PresentedBoard::new(
        vec![
            presented(0, Team::Party, GridPos::new(20, 12)),
            presented(1, Team::Party, GridPos::new(20, 13)),
            presented(2, Team::Monster, GridPos::new(21, 12)),
        ],
        super::tests::floor_map(),
        // Centred on the party, so every combatant is inside the 7x7 window.
        GridPos::new(17, 9),
    )
}

/// Plays a schedule to completion one tick at a time, returning every op in
/// the order it was applied and the tick each landed on.
fn play(schedule: Vec<Instruction>) -> Vec<(u32, Op)> {
    let mut timeline = Timeline::default();
    timeline.load(schedule);
    let mut applied = Vec::new();
    let mut tick = 0;
    while timeline.is_playing() {
        tick += 1;
        timeline.advance(1, |op| applied.push((tick, op.clone())));
        assert!(tick < 10_000, "a schedule must terminate");
    }
    applied
}

fn total_ticks(schedule: &[Instruction]) -> u32 {
    schedule.iter().map(|i| i.hold).sum()
}

// --- beat durations at representative speeds -------------------------------

#[test]
fn a_swing_beat_costs_the_pose_hold_plus_one_game_delay() {
    // §1.4 by hand: the attack pose holds 100 ms = 6 ticks, and
    // `DisplayAttackMessage`'s one `GameDelay` is speed × 100 ms.
    // speed 0 → max(1, 0) = 1 tick; speed 4 → 400 ms = 24; speed 9 → 900 = 54.
    for (speed, game_delay) in [(0u8, 1u32), (4, 24), (9, 54)] {
        let schedule = compose(
            &scene_board(),
            BeatClock::new(speed),
            &[
                ActionEvent::Attacking {
                    attacker_id: 0,
                    target_id: 2,
                    kind: AttackKind::Normal,
                },
                ActionEvent::Attack {
                    attacker_id: 0,
                    target_id: 2,
                    roll: 15,
                    hit: true,
                },
                ActionEvent::Sound { id: 7 },
                ActionEvent::Dmg {
                    attacker_id: 0,
                    target_id: 2,
                    amount: 4,
                    backstab: false,
                },
            ],
        );
        assert_eq!(
            total_ticks(&schedule),
            6 + game_delay,
            "speed {speed}: pose hold + one message beat"
        );
    }
}

#[test]
fn speed_changes_wall_time_and_never_frame_content() {
    // The D-CV3 determinism claim, as a test: the same batch at two speeds
    // schedules the same ops in the same order, with different holds.
    let batch = [
        ActionEvent::Attacking {
            attacker_id: 0,
            target_id: 2,
            kind: AttackKind::Normal,
        },
        ActionEvent::Sound { id: 9 },
    ];
    let slow = compose(&scene_board(), BeatClock::new(9), &batch);
    let fast = compose(&scene_board(), BeatClock::new(0), &batch);
    let ops = |s: &[Instruction]| s.iter().map(|i| i.op.clone()).collect::<Vec<_>>();
    assert_eq!(ops(&slow), ops(&fast), "identical frames");
    assert!(
        total_ticks(&slow) > total_ticks(&fast),
        "different wall time"
    );
}

#[test]
fn a_movement_step_is_untimed() {
    // §1.4: "Movement step — no timer". The step sound rides along, also free.
    let schedule = compose(
        &scene_board(),
        BeatClock::default(),
        &[
            ActionEvent::Move {
                combatant_id: 0,
                from_x: 20,
                from_y: 12,
                to_x: 20,
                to_y: 11,
                cost: 2,
            },
            ActionEvent::Sound { id: 0x0A },
        ],
    );
    assert_eq!(total_ticks(&schedule), 0);
    assert_eq!(schedule.len(), 2);
}

// --- event order within a step's playback ----------------------------------

#[test]
fn the_turn_head_parks_the_focus_box_then_the_panel() {
    let schedule = compose(
        &scene_board(),
        BeatClock::default(),
        &[ActionEvent::Pick {
            pass: 0,
            combatant_id: 2,
            delay: 3,
            roll: 40,
        }],
    );
    assert!(matches!(schedule[0].op, Op::Focus(Some(c)) if c.pos == GridPos::new(21, 12)));
    assert_eq!(schedule[1].op, Op::Panel(2));
}

#[test]
fn the_focus_box_rides_the_focused_mover() {
    // The original's step redraws draw the box at the focused player's
    // CURRENT position (`RedrawCombatIfFocusOn`) — a walking actor carries
    // its box (the 2026-08-08 ratification find: ours parked it on the
    // turn-start cell for the whole AI walk).
    let schedule = compose(
        &scene_board(),
        BeatClock::default(),
        &[
            ActionEvent::Pick {
                pass: 0,
                combatant_id: 2,
                delay: 3,
                roll: 40,
            },
            ActionEvent::Move {
                combatant_id: 2,
                from_x: 21,
                from_y: 12,
                to_x: 22,
                to_y: 12,
                cost: 2,
            },
            // Another combatant's move must NOT re-aim the box.
            ActionEvent::Move {
                combatant_id: 1,
                from_x: 20,
                from_y: 13,
                to_x: 20,
                to_y: 14,
                cost: 2,
            },
        ],
    );
    let focus_ops: Vec<&Op> = schedule
        .iter()
        .filter(|i| matches!(i.op, Op::Focus(_)))
        .map(|i| &i.op)
        .collect();
    assert_eq!(
        focus_ops.len(),
        2,
        "Pick parks it, the focused move carries it"
    );
    assert!(matches!(focus_ops[0], Op::Focus(Some(c)) if c.pos == GridPos::new(21, 12)));
    assert!(matches!(focus_ops[1], Op::Focus(Some(c)) if c.pos == GridPos::new(22, 12)));
}

#[test]
fn a_swing_plays_pose_then_message_then_the_pose_comes_down() {
    let played = play(compose(
        &scene_board(),
        BeatClock::default(),
        &[
            ActionEvent::Attacking {
                attacker_id: 0,
                target_id: 2,
                kind: AttackKind::Normal,
            },
            ActionEvent::Attack {
                attacker_id: 0,
                target_id: 2,
                roll: 15,
                hit: true,
            },
            ActionEvent::Sound { id: 7 },
            ActionEvent::Dmg {
                attacker_id: 0,
                target_id: 2,
                amount: 3,
                backstab: false,
            },
            ActionEvent::Pick {
                pass: 1,
                combatant_id: 1,
                delay: 2,
                roll: 20,
            },
        ],
    ));
    let ops: Vec<&Op> = played.iter().map(|(_, op)| op).collect();
    // The target turns to face its attacker, the panel switches to the
    // attacker, the Attack frame goes up (and holds), then the message.
    assert!(matches!(ops[0], Op::Pose { id: 2, .. }));
    assert_eq!(*ops[1], Op::Panel(0));
    assert!(matches!(
        ops[2],
        Op::Pose {
            id: 0,
            pose: IconPose::Attack,
            ..
        }
    ));
    assert_eq!(*ops[3], Op::Sound(7));
    // The pose hold is six ticks: the message lands on tick 7.
    let (pose_tick, _) = played[2];
    let message_tick = played
        .iter()
        .find(|(_, op)| matches!(op, Op::Message(PanelOp::Status { .. })))
        .unwrap()
        .0;
    assert_eq!(message_tick, pose_tick + 6, "100 ms = 6 ticks");
    // ...and the run's Attack frame comes down when the next turn is picked.
    let normal_again = played
        .iter()
        .position(|(_, op)| {
            matches!(
                op,
                Op::Pose {
                    id: 0,
                    pose: IconPose::Normal,
                    ..
                }
            )
        })
        .expect("the attacker's pose is restored");
    let panel_switch = played
        .iter()
        .position(|(_, op)| *op == Op::Panel(1))
        .unwrap();
    assert!(normal_again < panel_switch, "restore, then the next turn");
}

// --- §1.5 message composition ----------------------------------------------

/// The message script at its fullest — what the panel shows during the beat,
/// before the clear that ends it.
fn message_script(schedule: &[Instruction]) -> Vec<PanelOp> {
    let mut out = Vec::new();
    for i in schedule {
        match &i.op {
            Op::Message(PanelOp::Clear) | Op::Panel(_) => {
                if !out.is_empty() {
                    return out;
                }
            }
            Op::Message(op) => out.push(op.clone()),
            _ => {}
        }
    }
    out
}

#[test]
fn the_attack_message_is_the_display_attack_message_sequence() {
    let schedule = compose(
        &scene_board(),
        BeatClock::default(),
        &[
            ActionEvent::Attacking {
                attacker_id: 0,
                target_id: 2,
                kind: AttackKind::Behind,
            },
            ActionEvent::Dmg {
                attacker_id: 0,
                target_id: 2,
                amount: 1,
                backstab: false,
            },
        ],
    );
    let script = message_script(&schedule);
    assert_eq!(
        script,
        vec![
            PanelOp::Status {
                row: Row::At(10),
                who: 0,
                color: 0x0B,
                text: "Attacks".to_string(),
            },
            // The target is still up when its name prints, so it takes the
            // enemy colour rather than the removed one.
            PanelOp::Name {
                row: Row::At(12),
                who: 2,
                color: 0x0E,
            },
            PanelOp::Wrapped {
                row: Row::At(13),
                // Singular at exactly 1 damage, with the behind prefix.
                text: "(from behind) Hitting for 1 point of damage".to_string(),
            },
            PanelOp::Mark,
        ]
    );
}

#[test]
fn a_backstab_changes_the_verb_and_a_slay_replaces_the_damage_line() {
    let backstab = compose(
        &scene_board(),
        BeatClock::default(),
        &[
            ActionEvent::Attacking {
                attacker_id: 0,
                target_id: 2,
                kind: AttackKind::Backstab,
            },
            ActionEvent::Dmg {
                attacker_id: 0,
                target_id: 2,
                amount: 9,
                backstab: true,
            },
        ],
    );
    assert!(matches!(
        &message_script(&backstab)[0],
        PanelOp::Status { text, .. } if text == "-Backstabs-"
    ));

    let slay = compose(
        &scene_board(),
        BeatClock::default(),
        &[
            ActionEvent::Attacking {
                attacker_id: 0,
                target_id: 2,
                kind: AttackKind::Normal,
            },
            ActionEvent::Sound { id: 7 },
            ActionEvent::SlayHelpless {
                attacker_id: 0,
                target_id: 2,
            },
        ],
    );
    let script = message_script(&slay);
    assert!(matches!(&script[0], PanelOp::Status { text, .. } if text == "slays helpless"));
    assert!(matches!(&script[2], PanelOp::Wrapped { text, .. } if text == "with one cruel blow"));
}

#[test]
fn a_whole_attack_misses_once_however_many_swings_it_took() {
    // Three missing swings, one `sound_9` — the engine's own emission shape.
    let schedule = compose(
        &scene_board(),
        BeatClock::default(),
        &[
            ActionEvent::Attacking {
                attacker_id: 0,
                target_id: 2,
                kind: AttackKind::Normal,
            },
            ActionEvent::Attack {
                attacker_id: 0,
                target_id: 2,
                roll: 3,
                hit: false,
            },
            ActionEvent::Attack {
                attacker_id: 0,
                target_id: 2,
                roll: 5,
                hit: false,
            },
            ActionEvent::Attack {
                attacker_id: 0,
                target_id: 2,
                roll: 2,
                hit: false,
            },
            ActionEvent::Sound { id: 9 },
        ],
    );
    let misses = schedule
        .iter()
        .filter(
            |i| matches!(&i.op, Op::Message(PanelOp::Wrapped { text, .. }) if text == "and Misses"),
        )
        .count();
    assert_eq!(misses, 1, "one message for the attack, not one per swing");
    // Pose hold + exactly one message beat.
    assert_eq!(total_ticks(&schedule), 6 + 24);
}

#[test]
fn a_kill_continues_the_message_into_the_removal_tail() {
    let schedule = compose(
        &scene_board(),
        BeatClock::default(),
        &[
            ActionEvent::Attacking {
                attacker_id: 0,
                target_id: 2,
                kind: AttackKind::Normal,
            },
            ActionEvent::Dmg {
                attacker_id: 0,
                target_id: 2,
                amount: 12,
                backstab: false,
            },
            ActionEvent::Sound { id: 5 },
            ActionEvent::Removed {
                combatant_id: 2,
                reason: RemovalReason::Killed,
            },
        ],
    );
    let script = message_script(&schedule);
    // The damage line's rows are absolute; the removal's hang off the mark the
    // wrap cursor left (`line = gbl.textYCol + 1`).
    assert!(matches!(script[3], PanelOp::Mark));
    assert_eq!(
        script[4],
        PanelOp::Status {
            row: Row::FromMark(0),
            who: 2,
            color: 0x0C,
            text: "goes down".to_string(),
        }
    );
    assert_eq!(
        script[5],
        PanelOp::Status {
            row: Row::FromMark(2),
            who: 2,
            color: 0x0C,
            text: "is killed".to_string(),
        }
    );
    // A downed-but-dying target says so with a bare line instead.
    let dying = compose(
        &scene_board(),
        BeatClock::default(),
        &[
            ActionEvent::Attacking {
                attacker_id: 0,
                target_id: 2,
                kind: AttackKind::Normal,
            },
            ActionEvent::Dmg {
                attacker_id: 0,
                target_id: 2,
                amount: 12,
                backstab: false,
            },
            ActionEvent::Removed {
                combatant_id: 2,
                reason: RemovalReason::Downed { dying: true },
            },
        ],
    );
    assert_eq!(
        message_script(&dying)[5],
        PanelOp::Line {
            row: Row::FromMark(2),
            text: "and is Dying".to_string(),
        }
    );
}

#[test]
fn flight_and_removal_messages_take_their_own_shapes() {
    for (reason, text) in [
        (RemovalReason::Fled, "Got Away"),
        (RemovalReason::Surrendered, "Surrenders"),
    ] {
        let schedule = compose(
            &scene_board(),
            BeatClock::default(),
            &[ActionEvent::Removed {
                combatant_id: 2,
                reason,
            }],
        );
        assert!(
            matches!(&schedule[0].op, Op::Message(PanelOp::Status { text: t, .. }) if t == text),
            "{reason:?}"
        );
        // One message beat, and no death flash — `RemoveFromCombat` has none.
        assert_eq!(total_ticks(&schedule), 24);
        assert!(!schedule
            .iter()
            .any(|i| matches!(&i.op, Op::Overlay(v) if !v.is_empty())));
    }

    let panic = compose(
        &scene_board(),
        BeatClock::default(),
        &[ActionEvent::Flees {
            combatant_id: 2,
            forced: false,
        }],
    );
    assert!(
        matches!(&panic[0].op, Op::Message(PanelOp::Status { text, .. }) if text == "flees in panic")
    );
}

#[test]
fn guarding_and_the_continue_prompt_use_the_prompt_row() {
    let schedule = compose(
        &scene_board(),
        BeatClock::default(),
        &[ActionEvent::Ai {
            combatant_id: 2,
            field_15: 1,
            target_id: -1,
        }],
    );
    assert_eq!(schedule[0].op, Op::Prompt(Some("Guarding".to_string())));
    assert_eq!(schedule[0].hold, 24, "printed for one GameDelay");
    assert_eq!(schedule[1].op, Op::Prompt(None));

    let prompt = compose(
        &scene_board(),
        BeatClock::default(),
        &[ActionEvent::ContinueBattlePrompt { answered_yes: true }],
    );
    assert_eq!(
        prompt[0].op,
        Op::Prompt(Some("Continue Battle:".to_string()))
    );
}

#[test]
fn a_cure_reports_full_or_partial_from_the_post_heal_hit_points() {
    // `DescribeHealing` reads `hit_point_current == hit_point_max` AFTER the
    // heal, so the composer must apply the event to its shadow board first.
    let mut roster = vec![presented(0, Team::Party, GridPos::new(20, 12))];
    roster[0].hp_current = 4; // hp_max is 10 in the fixture
    let partial = compose(
        &board(roster.clone()),
        BeatClock::default(),
        &[ActionEvent::Healed {
            healer_id: 0,
            target_id: 0,
            amount: 3,
            kind: HealKind::Cure,
        }],
    );
    assert!(
        matches!(&partial[1].op, Op::Message(PanelOp::Status { text, .. }) if text == "is partially healed")
    );
    let full = compose(
        &board(roster),
        BeatClock::default(),
        &[ActionEvent::Healed {
            healer_id: 0,
            target_id: 0,
            amount: 6,
            kind: HealKind::Cure,
        }],
    );
    assert!(
        matches!(&full[1].op, Op::Message(PanelOp::Status { text, .. }) if text == "is fully healed")
    );
}

// --- the death flash, against a hand-computed schedule ---------------------

#[test]
fn the_death_flash_is_nine_one_tick_alternations_then_a_game_delay() {
    let schedule = compose(
        &scene_board(),
        BeatClock::default(),
        &[ActionEvent::Removed {
            combatant_id: 2,
            reason: RemovalReason::Killed,
        }],
    );
    // The victim's icon is erased before the flash plays over its cell.
    assert_eq!(
        schedule
            .iter()
            .find(|i| matches!(i.op, Op::Hidden(_)))
            .map(|i| i.op.clone()),
        Some(Op::Hidden(Some(2)))
    );
    let frames: Vec<(&Op, u32)> = schedule
        .iter()
        .filter(|i| matches!(&i.op, Op::Overlay(v) if !v.is_empty()))
        .map(|i| (&i.op, i.hold))
        .collect();
    assert_eq!(frames.len(), 9, "`for var_3 = 0; var_3 <= 8`");
    assert!(
        frames.iter().all(|(_, hold)| *hold == 1),
        "10 ms rounds to one tick"
    );
    // Alternating slot 24's Attack frame and slot 25's Normal frame.
    let sprite = |op: &Op| match op {
        Op::Overlay(v) => v[0].sprite,
        _ => unreachable!(),
    };
    assert_eq!(
        sprite(frames[0].0),
        SpriteRef::new(24, IconPose::Attack, false)
    );
    assert_eq!(
        sprite(frames[1].0),
        SpriteRef::new(25, IconPose::Normal, false)
    );
    assert_eq!(
        sprite(frames[8].0),
        SpriteRef::new(24, IconPose::Attack, false)
    );
    // 9 × 1 tick + the `GameDelay` the removal holds.
    assert_eq!(total_ticks(&schedule), 9 + 24);
}

#[test]
fn an_off_screen_death_flashes_nothing_but_still_removes() {
    // `CoordOnScreen(pos)` gates every flash cell (`ovr033.cs:565`).
    let mut roster = vec![presented(0, Team::Party, GridPos::new(1, 1))];
    roster[0].pos = GridPos::new(40, 20); // far outside the camera-at-origin window
    let schedule = compose(
        &board(roster),
        BeatClock::default(),
        &[ActionEvent::Removed {
            combatant_id: 0,
            reason: RemovalReason::Killed,
        }],
    );
    assert!(schedule.iter().all(
        |i| matches!(&i.op, Op::Overlay(v) if v.is_empty()) || !matches!(i.op, Op::Overlay(_))
    ));
    assert!(schedule
        .iter()
        .any(|i| matches!(i.op, Op::Board(ActionEvent::Removed { .. }))));
}

// --- the missile, against a hand-computed schedule -------------------------

#[test]
fn a_missile_holds_its_class_delay_per_eight_pixel_step() {
    let mut roster = vec![
        presented(0, Team::Party, GridPos::new(1, 3)),
        presented(1, Team::Monster, GridPos::new(5, 3)),
    ];
    roster[1].team = Team::Monster;
    let b = board(roster);
    // Arrow (ITEMS type 0x49): one static frame, 10 ms per step = 1 tick.
    let schedule = compose(
        &b,
        BeatClock::default(),
        &[ActionEvent::Missile {
            attacker_id: 0,
            target_id: 1,
            weapon_type: 0x49,
        }],
    );
    let flight: Vec<&Instruction> = schedule
        .iter()
        .filter(|i| matches!(&i.op, Op::Overlay(v) if !v.is_empty()))
        .collect();
    // Four cells apart on one axis: 12 sub-steps, `var_B0` of them drawn, plus
    // the landing frame.
    let dirs = missile::path_directions(GridPos::new(1, 3), GridPos::new(5, 3));
    assert_eq!(flight.len(), (dirs.len() - 2) + 1);
    assert!(flight.iter().all(|i| i.hold == 1), "10 ms = 1 tick");
    // Both launch sounds, in `DrawRangedAttack`'s order.
    assert_eq!(schedule[0].op, Op::Sound(0x0C));
    assert_eq!(schedule[1].op, Op::Sound(0x0C));

    // A hand axe (0x02) is the four-frame spin at 50 ms = 3 ticks, sound 9.
    let axe = compose(
        &b,
        BeatClock::default(),
        &[ActionEvent::Missile {
            attacker_id: 0,
            target_id: 1,
            weapon_type: 0x02,
        }],
    );
    assert_eq!(axe[1].op, Op::Sound(9));
    assert!(axe
        .iter()
        .filter(|i| matches!(&i.op, Op::Overlay(v) if !v.is_empty()))
        .all(|i| i.hold == 3));
}

// --- the fast drain --------------------------------------------------------

/// Everything a frame is drawn from — the fast-drain equivalence surface.
fn presentation_state(
    scene: &super::CombatScene,
) -> (PresentedBoard, Vec<PanelOp>, Option<String>) {
    (
        scene.board().clone(),
        scene.messages().to_vec(),
        scene.prompt().map(str::to_string),
    )
}

fn scene_for_drain() -> super::CombatScene {
    let mut monster = presented(1, Team::Monster, GridPos::new(21, 12));
    monster.team = Team::Monster;
    let snapshot = super::EntrySnapshot {
        roster: vec![presented(0, Team::Party, GridPos::new(20, 12)), monster],
        map: super::tests::floor_map(),
        camera_top_left: GridPos::new(17, 9),
    };
    super::CombatScene::new(snapshot, SceneArt::default())
}

/// A batch with every timed shape in it: a pose hold, a message beat, a
/// removal with its flash, and a prompt beat.
fn drain_batch() -> Vec<ActionEvent> {
    vec![
        ActionEvent::Pick {
            pass: 0,
            combatant_id: 0,
            delay: 3,
            roll: 50,
        },
        ActionEvent::Attacking {
            attacker_id: 0,
            target_id: 1,
            kind: AttackKind::Normal,
        },
        ActionEvent::Attack {
            attacker_id: 0,
            target_id: 1,
            roll: 18,
            hit: true,
        },
        ActionEvent::Sound { id: 7 },
        ActionEvent::Dmg {
            attacker_id: 0,
            target_id: 1,
            amount: 12,
            backstab: false,
        },
        ActionEvent::Sound { id: 5 },
        ActionEvent::Removed {
            combatant_id: 1,
            reason: RemovalReason::Killed,
        },
        ActionEvent::ContinueBattlePrompt {
            answered_yes: false,
        },
    ]
}

#[test]
fn a_drained_step_lands_exactly_where_a_played_step_does() {
    let mut played = scene_for_drain();
    played.begin_step(&drain_batch());
    let mut played_sounds = Vec::new();
    while played.is_playing() {
        played_sounds.extend(played.tick(1).iter().copied());
    }

    let mut drained = scene_for_drain();
    drained.begin_step(&drain_batch());
    let drained_sounds: Vec<_> = drained.skip().to_vec();

    assert!(!drained.is_playing(), "a skip completes the playback");
    assert_eq!(
        presentation_state(&played),
        presentation_state(&drained),
        "the drained board and text are the played ones"
    );
    // The skip is a time collapse, not a mute: every cue still comes out, in
    // order, so nothing downstream can tell the two apart.
    assert_eq!(played_sounds, drained_sounds);
    assert!(!played_sounds.is_empty());
}

#[test]
fn a_mid_playback_skip_finishes_the_rest_of_the_step() {
    let mut scene = scene_for_drain();
    scene.begin_step(&drain_batch());
    let total = scene.ticks_remaining();
    assert!(total > 30, "the batch really is long: {total}");
    for _ in 0..10 {
        scene.tick(1);
    }
    assert!(scene.is_playing());
    scene.skip();
    assert!(!scene.is_playing());

    let mut played = scene_for_drain();
    played.begin_step(&drain_batch());
    while played.is_playing() {
        played.tick(1);
    }
    assert_eq!(presentation_state(&scene), presentation_state(&played));
}

#[test]
fn ticking_in_one_lump_matches_ticking_one_at_a_time() {
    // A host may run the reel at N× by ticking faster (D-CV3's other
    // acceleration form); the frames it lands on must not depend on the
    // granularity.
    let mut lumped = scene_for_drain();
    lumped.begin_step(&drain_batch());
    let total = lumped.ticks_remaining();
    let lumped_sounds: Vec<_> = lumped.tick(total).to_vec();

    let mut stepped = scene_for_drain();
    stepped.begin_step(&drain_batch());
    let mut stepped_sounds = Vec::new();
    while stepped.is_playing() {
        stepped_sounds.extend(stepped.tick(1).iter().copied());
    }
    assert!(!lumped.is_playing());
    assert_eq!(presentation_state(&lumped), presentation_state(&stepped));
    assert_eq!(lumped_sounds, stepped_sounds);
}

// --- the burst -------------------------------------------------------------

#[test]
fn the_star_burst_repeats_four_frames_speed_plus_one_times() {
    // §1.4 by hand: 70 ms per frame = 4 ticks, four frames, and the stars
    // repeat `game_speed_var + 1` times.
    for (speed, passes) in [(0u8, 1u32), (4, 5), (9, 10)] {
        let schedule =
            burst_instructions(BeatClock::new(speed), 0, 0x0B, (6, 6), "is Healed", true);
        let frames: Vec<&Instruction> = schedule
            .iter()
            .filter(|i| matches!(&i.op, Op::Overlay(v) if !v.is_empty()))
            .collect();
        assert_eq!(frames.len() as u32, 4 * passes, "speed {speed}");
        assert!(frames.iter().all(|i| i.hold == 4));
        assert_eq!(total_ticks(&schedule), 4 * 4 * passes);
        assert_eq!(schedule[0].op, Op::Sound(4), "the stars play sound 4");
    }
    // The plain variant runs one pass and closes with a `GameDelay`.
    let plain = burst_instructions(BeatClock::default(), 0, 0x0B, (6, 6), "is affected", false);
    assert_eq!(plain[0].op, Op::Sound(3));
    assert_eq!(total_ticks(&plain), 4 * 4 + 24);
}

// --- the composer never reads a live roster --------------------------------

#[test]
fn compose_leaves_the_callers_board_untouched() {
    // D-CV2's mid-playback rule, kept structurally: the composer walks a clone.
    let before = scene_board();
    let after = before.clone();
    let _ = compose(
        &after,
        BeatClock::default(),
        &[
            ActionEvent::Move {
                combatant_id: 0,
                from_x: 20,
                from_y: 12,
                to_x: 20,
                to_y: 11,
                cost: 2,
            },
            ActionEvent::Removed {
                combatant_id: 2,
                reason: RemovalReason::Killed,
            },
        ],
    );
    assert_eq!(before, after);
    assert_eq!(
        after.combatant(2).unwrap().health_status,
        HealthStatus::Okey
    );
}

// --- the composer must not drop a board-bearing event ----------------------

#[test]
fn every_board_bearing_event_still_reaches_the_board() {
    // The failure this localizes: a beat that composes a *message* for an
    // event and forgets to schedule its board effect. The draw-parity test
    // catches it too, but only as "the presented board drifted at step N" —
    // this names the event.
    let batch = [
        ActionEvent::Move {
            combatant_id: 0,
            from_x: 20,
            from_y: 12,
            to_x: 21,
            to_y: 11,
            cost: 3,
        },
        ActionEvent::Camera {
            top_left: GridPos::new(18, 8),
        },
        ActionEvent::Attacking {
            attacker_id: 0,
            target_id: 2,
            kind: AttackKind::Normal,
        },
        ActionEvent::Dmg {
            attacker_id: 0,
            target_id: 2,
            amount: 4,
            backstab: false,
        },
        ActionEvent::Healed {
            healer_id: 1,
            target_id: 1,
            amount: 0,
            kind: HealKind::Bandage,
        },
        ActionEvent::Bled {
            combatant_id: 2,
            died: false,
        },
        ActionEvent::Removed {
            combatant_id: 2,
            reason: RemovalReason::Killed,
        },
    ];

    // What the board looks like when the events are simply applied — the
    // reconciliation surface D-CV2 asserts at every step boundary.
    let mut expected = scene_board();
    for event in batch {
        expected.apply(event);
    }

    // ...and what a played-out schedule leaves.
    let mut played = scene_board();
    let mut timeline = Timeline::default();
    timeline.load(compose(&played, BeatClock::default(), &batch));
    while timeline.is_playing() {
        let mut ops = Vec::new();
        timeline.advance(1, |op| ops.push(op.clone()));
        for op in ops {
            match op {
                Op::Camera(top_left) => played.apply(ActionEvent::Camera { top_left }),
                Op::Board(event) => played.apply(event),
                _ => {}
            }
        }
    }

    assert_eq!(played.camera_top_left(), expected.camera_top_left());
    for (a, b) in played.combatants().iter().zip(expected.combatants()) {
        assert_eq!(
            (a.pos, a.hp_current, a.health_status, a.in_combat, a.size),
            (b.pos, b.hp_current, b.health_status, b.in_combat, b.size),
            "combatant {} drifted",
            a.id
        );
    }
    assert_eq!(played.map(), expected.map(), "the body tile landed");
    assert_eq!(played.downed_tiles(), expected.downed_tiles());
}
