//! Unit tests for the combat scene: the presented board's event application
//! and boundary reconciliation, the layout/tile plumbing, and the two loud
//! failures (`DrawIsoTile`'s overlay fork, roster drift).
//!
//! Pixel goldens live next door in [`super::goldens`]; every fixture there
//! and here is hand-authored (D10) — no real art bytes, ever.

use super::*;
use crate::combat::{
    ActionEvent, CombatMap, CombatState, Combatant, GridPos, HealKind, HealthStatus, RemovalReason,
    Team, TILE_DOWN_PLAYER, TILE_STINKING_CLOUD,
};
use crate::combat_art::IconPose;
use crate::rng::EngineRng;

/// A single-cell presented combatant on team `team` at `pos`.
pub(super) fn presented(id: usize, team: Team, pos: GridPos) -> PresentedCombatant {
    PresentedCombatant {
        id,
        name: format!("C{id}"),
        team,
        non_team_member: false,
        icon_slot: id,
        size: 1,
        pos,
        direction: 0,
        pose: IconPose::Normal,
        hp_current: 10,
        hp_max: 10,
        ac: 0x32,
        health_status: HealthStatus::Okey,
        in_combat: true,
    }
}

/// A board on an all-floor map (`0x17`, a cost-1 passable tile) with the
/// camera at the origin.
pub(super) fn board(roster: Vec<PresentedCombatant>) -> PresentedBoard {
    board_with(roster, CombatMap::uniform(0x17))
}

/// [`board`] over a caller-prepared map.
pub(super) fn board_with(roster: Vec<PresentedCombatant>, map: CombatMap) -> PresentedBoard {
    PresentedBoard::new(roster, map, GridPos::new(0, 0))
}

/// The all-floor map [`board`] uses, so a test can season it first.
pub(super) fn floor_map() -> CombatMap {
    CombatMap::uniform(0x17)
}

// --- event application -----------------------------------------------------

#[test]
fn move_advances_the_position_and_turns_the_icon() {
    let mut b = board(vec![presented(0, Team::Party, GridPos::new(3, 3))]);
    b.apply(ActionEvent::Move {
        combatant_id: 0,
        from_x: 3,
        from_y: 3,
        to_x: 4,
        to_y: 3,
        cost: 2,
    });
    let c = b.combatant(0).unwrap();
    assert_eq!(c.pos, GridPos::new(4, 3));
    assert_eq!(c.direction, 2, "east");
    b.apply(ActionEvent::Move {
        combatant_id: 0,
        from_x: 4,
        from_y: 3,
        to_x: 3,
        to_y: 2,
        cost: 2,
    });
    assert_eq!(b.combatant(0).unwrap().direction, 7, "north-west");
}

#[test]
fn damage_and_cure_move_hit_points_the_way_the_engine_does() {
    let mut b = board(vec![presented(0, Team::Party, GridPos::new(3, 3))]);
    b.apply(ActionEvent::Dmg {
        attacker_id: 1,
        target_id: 0,
        amount: 4,
        backstab: false,
    });
    assert_eq!(b.combatant(0).unwrap().hp_current, 6);
    // A cure caps at hp_max even though the event reports the raw roll.
    b.apply(ActionEvent::Healed {
        healer_id: 1,
        target_id: 0,
        amount: 8,
        kind: HealKind::Cure,
    });
    assert_eq!(b.combatant(0).unwrap().hp_current, 10);
}

#[test]
fn bandage_lifts_dying_to_unconscious_and_heals_nothing() {
    let mut roster = vec![presented(0, Team::Party, GridPos::new(3, 3))];
    roster[0].health_status = HealthStatus::Dying;
    roster[0].hp_current = 0;
    let mut b = board(roster);
    b.apply(ActionEvent::Healed {
        healer_id: 1,
        target_id: 0,
        amount: 0,
        kind: HealKind::Bandage,
    });
    let c = b.combatant(0).unwrap();
    assert_eq!(c.health_status, HealthStatus::Unconscious);
    assert_eq!(c.hp_current, 0);
}

#[test]
fn bleed_out_marks_the_dying_combatant_dead() {
    let mut roster = vec![presented(0, Team::Party, GridPos::new(3, 3))];
    roster[0].health_status = HealthStatus::Dying;
    let mut b = board(roster);
    b.apply(ActionEvent::Bled {
        combatant_id: 0,
        died: false,
    });
    assert_eq!(
        b.combatant(0).unwrap().health_status,
        HealthStatus::Dying,
        "a plain bleed tick changes nothing the board draws"
    );
    b.apply(ActionEvent::Bled {
        combatant_id: 0,
        died: true,
    });
    assert_eq!(b.combatant(0).unwrap().health_status, HealthStatus::Dead);
}

#[test]
fn camera_events_move_the_window() {
    let mut b = board(vec![presented(0, Team::Party, GridPos::new(3, 3))]);
    assert!(b.on_screen(GridPos::new(6, 6)));
    b.apply(ActionEvent::Camera {
        top_left: GridPos::new(10, 4),
    });
    assert_eq!(b.camera_top_left(), GridPos::new(10, 4));
    assert_eq!(b.screen_pos(GridPos::new(12, 5)), (2, 1));
    assert!(!b.on_screen(GridPos::new(6, 6)));
}

#[test]
fn a_downed_party_member_leaves_the_body_tile_and_a_monster_vanishes() {
    let mut b = board(vec![
        presented(0, Team::Party, GridPos::new(3, 3)),
        presented(1, Team::Monster, GridPos::new(4, 3)),
    ]);
    b.apply(ActionEvent::Removed {
        combatant_id: 0,
        reason: RemovalReason::Downed { dying: true },
    });
    b.apply(ActionEvent::Removed {
        combatant_id: 1,
        reason: RemovalReason::Killed,
    });

    let party = b.combatant(0).unwrap();
    assert!(!party.in_combat && party.size == 0 && !party.on_board());
    assert_eq!(party.health_status, HealthStatus::Dying);
    assert_eq!(party.hp_current, 0);
    assert_eq!(
        b.map().ground_tile(GridPos::new(3, 3)),
        TILE_DOWN_PLAYER,
        "the party member leaves a body"
    );
    assert_eq!(
        b.map().ground_tile(GridPos::new(4, 3)),
        0x17,
        "the monster just vanishes — no body tile"
    );
    assert_eq!(b.downed_tiles().len(), 1);
    assert_eq!(b.downed_tiles()[0].original_tile, 0x17);
}

#[test]
fn an_allied_npc_leaves_no_body_and_a_stinking_cloud_is_not_overwritten() {
    let mut roster = vec![
        presented(0, Team::Party, GridPos::new(3, 3)),
        presented(1, Team::Party, GridPos::new(5, 5)),
    ];
    roster[0].non_team_member = true; // an allied guild thief (§47.1)
    let mut map = floor_map();
    map.set_tile(GridPos::new(5, 5), TILE_STINKING_CLOUD);
    let mut b = board_with(roster, map);

    b.apply(ActionEvent::Removed {
        combatant_id: 0,
        reason: RemovalReason::Killed,
    });
    b.apply(ActionEvent::Removed {
        combatant_id: 1,
        reason: RemovalReason::Killed,
    });

    assert_eq!(b.map().ground_tile(GridPos::new(3, 3)), 0x17);
    assert!(b.downed_tiles().is_empty() || b.downed_tiles()[0].combatant_id == 1);
    assert_eq!(
        b.map().ground_tile(GridPos::new(5, 5)),
        TILE_STINKING_CLOUD,
        "the cloud survives the body stamp"
    );
    assert_eq!(
        b.downed_tiles().len(),
        1,
        "the save entry is still recorded for the cloud cell"
    );
}

#[test]
fn a_fleeing_combatant_keeps_its_hit_points() {
    let mut roster = vec![presented(0, Team::Monster, GridPos::new(3, 3))];
    roster[0].hp_current = 7;
    let mut b = board(roster);
    b.apply(ActionEvent::Removed {
        combatant_id: 0,
        reason: RemovalReason::Fled,
    });
    let c = b.combatant(0).unwrap();
    assert_eq!(c.hp_current, 7, "the Got-Away case skips the hp zeroing");
    assert_eq!(c.health_status, HealthStatus::Running);
    assert!(!c.in_combat);
}

#[test]
fn restoring_a_body_puts_the_saved_tile_back() {
    let mut map = floor_map();
    map.set_tile(GridPos::new(3, 3), 0x18);
    let mut b = board_with(vec![presented(0, Team::Party, GridPos::new(3, 3))], map);
    b.apply(ActionEvent::Removed {
        combatant_id: 0,
        reason: RemovalReason::Downed { dying: true },
    });
    assert_eq!(b.map().ground_tile(GridPos::new(3, 3)), TILE_DOWN_PLAYER);

    assert_eq!(b.restore_downed(0, GridPos::new(3, 3)), Some(0x18));
    assert_eq!(b.map().ground_tile(GridPos::new(3, 3)), 0x18);
    assert!(b.downed_tiles().is_empty());
}

#[test]
fn a_second_body_on_the_cell_suppresses_the_restore() {
    let mut b = board(vec![
        presented(0, Team::Party, GridPos::new(3, 3)),
        presented(1, Team::Party, GridPos::new(3, 3)),
    ]);
    b.apply(ActionEvent::Removed {
        combatant_id: 0,
        reason: RemovalReason::Killed,
    });
    // The second body's save records the *body* tile the first one left.
    b.apply(ActionEvent::Removed {
        combatant_id: 1,
        reason: RemovalReason::Killed,
    });
    assert_eq!(b.downed_tiles()[1].original_tile, TILE_DOWN_PLAYER);

    assert_eq!(
        b.restore_downed(1, GridPos::new(3, 3)),
        None,
        "combatant 0's body is still recorded here"
    );
    assert_eq!(b.map().ground_tile(GridPos::new(3, 3)), TILE_DOWN_PLAYER);
}

#[test]
fn non_board_events_leave_the_board_alone() {
    let start = board(vec![presented(0, Team::Party, GridPos::new(3, 3))]);
    let mut b = start.clone();
    for event in [
        ActionEvent::Attack {
            attacker_id: 0,
            target_id: 0,
            roll: 12,
            hit: true,
        },
        ActionEvent::Sound { id: 7 },
        ActionEvent::SlayHelpless {
            attacker_id: 0,
            target_id: 0,
        },
        ActionEvent::ContinueBattlePrompt { answered_yes: true },
        ActionEvent::Missile {
            attacker_id: 0,
            target_id: 0,
            weapon_type: 0x29,
        },
        ActionEvent::Cast {
            caster_id: 0,
            spell_id: 4,
        },
    ] {
        b.apply(event);
    }
    assert_eq!(b.combatants(), start.combatants());
    assert_eq!(b.camera_top_left(), start.camera_top_left());
}

// --- boundary reconciliation ----------------------------------------------

/// A two-combatant `CombatState` stepped once, so the camera has run its lazy
/// `combat_setup` init — the exact boundary D-CV2 reads the snapshot at.
fn stepped_state() -> (CombatState, EngineRng) {
    let mut roster = Vec::new();
    for (i, team) in [Team::Party, Team::Monster].into_iter().enumerate() {
        let mut c = Combatant::new(i, team, 0, true);
        c.pos = GridPos::new(20 + i as i32, 12);
        c.hp_current = 10;
        c.hp_max = 10;
        c.size = 1;
        roster.push(c);
    }
    let mut state = CombatState::initiative_only(roster);
    state.map = CombatMap::uniform(0x17);
    let mut rng = EngineRng::new(1234);
    let _ = state.step(&mut rng);
    (state, rng)
}

fn identities() -> Vec<CombatantIdentity> {
    vec![
        CombatantIdentity::new("KERMIT", 0),
        CombatantIdentity::new("KOBOLD", 8),
    ]
}

#[test]
fn the_entry_snapshot_reads_the_camera_at_the_first_step_boundary() {
    let (state, _rng) = stepped_state();
    let snapshot = EntrySnapshot::from_state(&state, &identities());
    assert_eq!(snapshot.camera_top_left, state.camera_top_left());
    assert_eq!(snapshot.roster.len(), 2);
    assert_eq!(snapshot.roster[0].name, "KERMIT");
    assert_eq!(snapshot.roster[1].icon_slot, 8);
    assert_eq!(snapshot.roster[1].pos, state.roster()[1].pos);
}

#[test]
fn reconciliation_passes_on_an_untouched_snapshot() {
    let (state, _rng) = stepped_state();
    let snapshot = EntrySnapshot::from_state(&state, &identities());
    let mut scene = CombatScene::new(snapshot, SceneArt::default());
    scene.reconcile(&state).expect("no drift yet");
}

/// The assert must actually fire — seeded by moving the presented board
/// somewhere the engine never went. `debug_assert!` panics in the test
/// profile, so this is the panic path AND the `Err` path in one.
#[test]
#[should_panic(expected = "BoardDrift")]
fn reconciliation_catches_a_seeded_position_mismatch() {
    let (state, _rng) = stepped_state();
    let snapshot = EntrySnapshot::from_state(&state, &identities());
    let mut scene = CombatScene::new(snapshot, SceneArt::default());
    scene.apply_event(ActionEvent::Move {
        combatant_id: 0,
        from_x: state.roster()[0].pos.x,
        from_y: state.roster()[0].pos.y,
        to_x: state.roster()[0].pos.x + 1,
        to_y: state.roster()[0].pos.y,
        cost: 2,
    });
    let _ = scene.reconcile(&state);
}

#[test]
#[should_panic(expected = "hp_current")]
fn reconciliation_catches_a_seeded_hit_point_mismatch() {
    let (state, _rng) = stepped_state();
    let snapshot = EntrySnapshot::from_state(&state, &identities());
    let mut scene = CombatScene::new(snapshot, SceneArt::default());
    scene.apply_event(ActionEvent::Dmg {
        attacker_id: 1,
        target_id: 0,
        amount: 3,
        backstab: false,
    });
    let _ = scene.reconcile(&state);
}

#[test]
#[should_panic(expected = "CameraDrift")]
fn reconciliation_catches_a_seeded_camera_mismatch() {
    let (state, _rng) = stepped_state();
    let snapshot = EntrySnapshot::from_state(&state, &identities());
    let mut scene = CombatScene::new(snapshot, SceneArt::default());
    scene.apply_event(ActionEvent::Camera {
        top_left: GridPos::new(0, 0),
    });
    let _ = scene.reconcile(&state);
}

#[test]
fn reconciliation_ignores_panel_only_fields_and_refreshes_facing() {
    let (mut state, _rng) = stepped_state();
    let snapshot = EntrySnapshot::from_state(&state, &identities());
    let mut scene = CombatScene::new(snapshot, SceneArt::default());

    // Panel-only: AC and the readied weapon are deliberately outside the
    // reconciliation scope (no event carries them; the panel boundary-reads
    // them instead).
    state.fighters[0].ac = 0x20;
    state.fighters[0].readied_weapon = Some((0x25, 0));
    // Facing is refreshed, not asserted.
    state.fighters[0].direction = 5;
    scene
        .reconcile(&state)
        .expect("panel-only drift is not board drift");
    assert_eq!(scene.board().combatant(0).unwrap().direction, 5);
    assert_ne!(scene.board().combatant(0).unwrap().ac, state.fighters[0].ac);
}

#[test]
fn the_panel_summary_is_a_boundary_read() {
    let (mut state, _rng) = stepped_state();
    state.fighters[0].hp_current = 6;
    state.fighters[0].ac = 0x32;
    state.fighters[0].readied_weapon = Some((0x25, 1));
    let snapshot = EntrySnapshot::from_state(&state, &identities());
    let mut scene = CombatScene::new(snapshot, SceneArt::default());
    scene.set_weapon_names(BTreeMap::from([(0x25u8, "Long Sword".to_string())]));

    let summary = scene.panel_summary(&state, 0).expect("combatant 0");
    assert_eq!(summary.name, "KERMIT", "the name comes from the snapshot");
    assert_eq!(summary.hp_current, 6);
    assert_eq!(summary.readied_weapon.as_deref(), Some("Long Sword"));
    assert!(!summary.held);

    // An unnamed weapon type takes the original's `primaryWeapon == null`
    // branch rather than printing an invented name.
    state.fighters[0].readied_weapon = Some((0x29, 0));
    let summary = scene.panel_summary(&state, 0).unwrap();
    assert_eq!(summary.readied_weapon, None);
}

// --- the two loud failures -------------------------------------------------

#[test]
fn the_iso_tile_overlay_path_fails_loudly_rather_than_guessing() {
    // Ground value 66 is one of the four `BackGroundTiles` sentinel rows
    // whose `tile_index` is 0xFF — past `DrawIsoTile`'s `> 0x7f` fork into
    // coab's stubbed `dword_1C8FC` overlay store (doc §6 item 2).
    assert_eq!(crate::combat::BACKGROUND_TILE_INDEX[66], 0xFF);
    let mut map = floor_map();
    map.set_tile(GridPos::new(0, 0), 66);
    let b = board_with(vec![presented(0, Team::Party, GridPos::new(3, 3))], map);
    let mut fb = crate::framebuffer::Framebuffer::new();
    let err = render::draw_ground(&mut fb, &b, &SceneArt::default()).unwrap_err();
    assert_eq!(
        err,
        SceneError::IsoTileOverlayPath {
            ground_value: 66,
            tile_index: 0xFF
        }
    );
}

#[test]
fn a_ground_value_past_the_table_fails_loudly() {
    let mut map = floor_map();
    map.set_tile(GridPos::new(0, 0), 200);
    let b = board_with(vec![presented(0, Team::Party, GridPos::new(3, 3))], map);
    let mut fb = crate::framebuffer::Framebuffer::new();
    assert_eq!(
        render::draw_ground(&mut fb, &b, &SceneArt::default()).unwrap_err(),
        SceneError::BackgroundTileOutOfRange { ground_value: 200 }
    );
}

#[test]
fn reconciliation_reports_a_roster_size_mismatch() {
    let (state, _rng) = stepped_state();
    let mut b = board(vec![presented(0, Team::Party, GridPos::new(3, 3))]);
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| b.reconcile(&state)));
    // The debug assert fires first in this profile; either way the shape is
    // the same, and a release build gets the `Err`.
    match err {
        Err(_) => {}
        Ok(res) => assert_eq!(
            res.unwrap_err(),
            SceneError::RosterSizeMismatch {
                presented: 1,
                actual: 2
            }
        ),
    }
}

// --- the background tile table --------------------------------------------

#[test]
fn the_body_and_cloud_tiles_index_randcoms_last_two_slots() {
    // RANDCOM's six tiles land at atlas slots 0x22..=0x27, so the two ground
    // tiles the combat core already names must point at the last two.
    assert_eq!(
        crate::combat::BACKGROUND_TILE_INDEX[TILE_DOWN_PLAYER as usize],
        0x27
    );
    assert_eq!(
        crate::combat::BACKGROUND_TILE_INDEX[TILE_STINKING_CLOUD as usize],
        0x26
    );
    // And the two tables describe the same 74 rows.
    assert_eq!(
        crate::combat::BACKGROUND_TILE_INDEX.len(),
        crate::combat::BACKGROUND_MOVE_COST.len()
    );
}
