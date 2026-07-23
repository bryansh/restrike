use super::*;

// --- the combat camera (doc §36.3) -------------------------------------

/// A minimal melee roster over an open floor for exercising camera state.
fn camera_state(positions: &[(Team, GridPos)]) -> CombatState {
    let fighters = positions
        .iter()
        .enumerate()
        .map(|(i, (team, pos))| {
            Combatant::new_melee(
                i,
                *team,
                *team == Team::Monster,
                *pos,
                10,
                10,
                0,
                12,
                (1, 6, 0),
                5,
                1,
            )
        })
        .collect();
    CombatState::new(CombatMap::uniform(FLOOR), fighters)
}

#[test]
fn turn_head_resets_the_actors_own_swarm_and_guard_state() {
    // sub_33281 @028F-02A9: the acting combatant's own AttacksReceived (@028F),
    // directionChanges (@029C), and guarding (@02A9) zero at its turn head,
    // before the body.
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Monster, GridPos::new(21, 12)), // adjacent → attackable
    ]);
    s.combat_setup();
    s.combat_setup_done = true;
    s.fighters[0].attacks_received = 3; // stale swarm count
    s.fighters[0].direction_changes = 5; // stale facing accumulator
    s.fighters[0].guarding = true; // a parked guard
    s.fighters[0].delay = 5;
    s.fighters[0].attack1_left = 1;
    s.fighters[0].target = Some(1);
    let mut rng = EngineRng::new(0x1234_5678);
    s.take_turn(&mut rng, 0);
    // Cleared at the head; the actor attacked (RecalcAttacksReceived bumps
    // the TARGET, not the actor) so its own count stays 0 and it did not
    // re-guard.
    assert_eq!(s.fighters[0].attacks_received, 0);
    assert_eq!(s.fighters[0].direction_changes, 0);
    assert!(!s.fighters[0].guarding);
}

#[test]
fn recalc_accumulates_direction_changes_mod_8() {
    // sub_3F94D @194D-19D8: each swing bumps AttacksReceived and folds a
    // dirDiff into directionChanges = (directionChanges + dirDiff) % 8.
    // Target at (20,12) faces EAST (direction 2); attacker adjacent due WEST
    // at (19,12). At distance 1 the fixed-point octant classifier floors due
    // west to SW=5 (`lo(1)` = 0), so bearing target→attacker = 5. dirDiff =
    // (5 − 2 + 8) % 8 = 3 (≤ 4, no fold). Three swings: 3, 6, then 9 % 8 = 1.
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(19, 12)), // attacker (idx 0), due west
        (Team::Monster, GridPos::new(20, 12)), // target (idx 1)
    ]);
    assert_eq!(
        target_direction(s.fighters[1].pos, s.fighters[0].pos),
        5,
        "adjacent-west classifies SW at distance 1"
    );
    s.fighters[1].direction = 2; // faces east
    s.fighters[1].direction_changes = 0;
    s.fighters[1].attacks_received = 0;

    s.recalc_attacks_received(1, 0);
    assert_eq!(s.fighters[1].attacks_received, 1);
    assert_eq!(s.fighters[1].direction_changes, 3, "first: (0 + 3) % 8");

    s.recalc_attacks_received(1, 0);
    assert_eq!(s.fighters[1].attacks_received, 2);
    assert_eq!(s.fighters[1].direction_changes, 6, "second: (3 + 3) % 8");

    s.recalc_attacks_received(1, 0);
    assert_eq!(
        s.fighters[1].direction_changes, 1,
        "third: (6 + 3) % 8 = 9 % 8 wraps"
    );
}

#[test]
fn recalc_direction_diff_folds_above_four() {
    // dirDiff > 4 folds to 8 − dirDiff (@1996-19A8). Target faces N (0);
    // attacker at bearing SW (5) from the target → raw dirDiff = (5 − 0 + 8)
    // % 8 = 5, folded to 3.
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(5, 15)), // attacker (idx 0): SW of target
        (Team::Monster, GridPos::new(10, 10)), // target (idx 1)
    ]);
    // Sanity: bearing target→attacker is SW (5).
    assert_eq!(
        target_direction(s.fighters[1].pos, s.fighters[0].pos),
        5,
        "SW"
    );
    s.fighters[1].direction = 0; // faces north
    s.fighters[1].direction_changes = 0;
    s.recalc_attacks_received(1, 0);
    assert_eq!(s.fighters[1].direction_changes, 3, "folded 5 → 8 − 5 = 3");
}

/// A roster for the AttackTarget facing table: attacker (idx 0) due NORTH of
/// target (idx 1), two cells apart → bearing target→attacker = N (0), attacker
/// faces target = S (4). `on_screen` picks whether the target is inside the
/// 7×7 window.
fn facing_state(target_on_screen: bool) -> CombatState {
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(20, 10)), // attacker (idx 0), due north
        (Team::Monster, GridPos::new(20, 12)), // target (idx 1)
    ]);
    s.fighters[0].in_combat = true;
    s.fighters[1].in_combat = true;
    s.focus = false; // isolate the direction stores from any recenter
    s.map_screen_top_left = if target_on_screen {
        GridPos::new(17, 9) // window x17..23, y9..15 → both on-screen
    } else {
        GridPos::new(40, 20) // window x40..46 → target (20,12) off-screen
    };
    // Sanity on the geometry.
    assert_eq!(
        target_direction(GridPos::new(20, 12), GridPos::new(20, 10)),
        0
    );
    assert_eq!(
        target_direction(GridPos::new(20, 10), GridPos::new(20, 12)),
        4
    );
    s
}

#[test]
fn attack_target_facing_first_attack_on_screen_faces_the_attacker() {
    // Table row 1 (§36.1): AttacksReceived<2, attackType 0, on-screen — the
    // face-away store is overwritten by the on-screen draw → target FACES its
    // attacker (bearing target→attacker = 0). Attacker faces target (4).
    let mut s = facing_state(true);
    assert!(s.on_screen(1));
    s.fighters[1].attacks_received = 1; // < 2 (post-Recalc bump)
    s.fighters[1].direction = 3; // arbitrary prior facing
    s.attack_target_facing(1, 0, false);
    assert_eq!(s.fighters[1].direction, 0, "target faces its attacker");
    assert_eq!(s.fighters[0].direction, 4, "attacker faces its target");
}

#[test]
fn attack_target_facing_first_attack_off_screen_faces_away() {
    // Table row 2: AttacksReceived<2, attackType 0, off-screen — no draw, so
    // the face-away store stands → (bearing + 4) % 8 = 4.
    let mut s = facing_state(false);
    assert!(!s.on_screen(1));
    s.fighters[1].attacks_received = 1;
    s.fighters[1].direction = 3;
    s.attack_target_facing(1, 0, false);
    assert_eq!(s.fighters[1].direction, 4, "target faces away");
    assert_eq!(
        s.fighters[0].direction, 4,
        "attacker still faces its target"
    );
}

#[test]
fn attack_target_facing_subsequent_attack_on_screen_is_a_no_op() {
    // Table row 3: AttacksReceived>=2, attackType 0, on-screen — the 180° flip
    // is stored then the draw restores the old value → unchanged.
    let mut s = facing_state(true);
    s.fighters[1].attacks_received = 2; // not < 2
    s.fighters[1].direction = 3;
    s.attack_target_facing(1, 0, false);
    assert_eq!(
        s.fighters[1].direction, 3,
        "subsequent on-screen: unchanged"
    );
    assert_eq!(s.fighters[0].direction, 4);
}

#[test]
fn attack_target_facing_behind_attack_never_changes_the_target() {
    // Table row 5: attackType != 0 (a departure/behind swing) — branch 2, no
    // flip stored (attackType-gated), draw restores → target unchanged, even
    // on the first attack and on-screen.
    let mut s = facing_state(true);
    s.fighters[1].attacks_received = 1; // would be branch 1 if attackType 0
    s.fighters[1].direction = 3;
    s.attack_target_facing(1, 0, true);
    assert_eq!(
        s.fighters[1].direction, 3,
        "attackType != 0: target unchanged"
    );
    assert_eq!(
        s.fighters[0].direction, 4,
        "but the attacker still faces it"
    );
}

#[test]
fn attack_target_facing_subsequent_attack_off_screen_is_a_no_op() {
    // Table row 4: AttacksReceived>=2, off-screen — no store, no draw.
    let mut s = facing_state(false);
    s.fighters[1].attacks_received = 2;
    s.fighters[1].direction = 3;
    s.attack_target_facing(1, 0, false);
    assert_eq!(
        s.fighters[1].direction, 3,
        "subsequent off-screen: unchanged"
    );
    assert_eq!(s.fighters[0].direction, 4);
}

#[test]
fn flanking_fires_only_when_swarmed_turned_and_backs_the_attacker() {
    // §36.4 (ovr014:16AD-16E9): AttacksReceived>1 && the target's back is to
    // the attacker (target_direction(attacker,target) == target.direction) &&
    // directionChanges>4. Attacker (idx 0) due north of target (idx 1) → the
    // attacker's bearing toward the target is S (4); the target faces S (4) to
    // present its back.
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(20, 10)), // attacker (idx 0), due north
        (Team::Monster, GridPos::new(20, 12)), // target (idx 1)
    ]);
    assert_eq!(
        target_direction(GridPos::new(20, 10), GridPos::new(20, 12)),
        4
    );
    // All three satisfied → flanking.
    s.fighters[1].attacks_received = 2;
    s.fighters[1].direction = 4; // back turned to the attacker
    s.fighters[1].direction_changes = 5;
    assert!(s.is_flanking(1, 0));

    // Only one swing this turn → not swarmed.
    s.fighters[1].attacks_received = 1;
    assert!(!s.is_flanking(1, 0), "AttacksReceived must be > 1");
    s.fighters[1].attacks_received = 2;

    // Target faces the attacker (N=0), not away → not behind.
    s.fighters[1].direction = 0;
    assert!(
        !s.is_flanking(1, 0),
        "target must face away from the attacker"
    );
    s.fighters[1].direction = 4;

    // Not spun enough this turn.
    s.fighters[1].direction_changes = 4;
    assert!(!s.is_flanking(1, 0), "directionChanges must be > 4");
}

#[test]
fn can_backstab_needs_a_thief_a_listed_weapon_a_swarmed_manzised_turned_target() {
    // §36.4 (ovr014:28D7-29B9): SkillLevel(Thief)>0 && weapon∈list &&
    // AttacksReceived>1 && (field_DE&0x7F)<=1 && back turned. Same geometry as
    // flanking: attacker (idx 0) due north → back-turned target faces S (4).
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(20, 10)),   // attacker (idx 0)
        (Team::Monster, GridPos::new(20, 12)), // target (idx 1)
    ]);
    s.fighters[0].thief_skill_level = 5; // TRAVIS's T5
    s.fighters[0].weapon_readied = false; // punching → null weapon → capable
    s.fighters[1].attacks_received = 2;
    s.fighters[1].field_de = 0x01; // man-sized
    s.fighters[1].direction = 4; // back turned
    assert!(s.can_backstab(1, 0), "thief, bare hands, swarmed, turned");

    // Not a thief.
    s.fighters[0].thief_skill_level = 0;
    assert!(!s.can_backstab(1, 0), "SkillLevel(Thief) must be > 0");
    s.fighters[0].thief_skill_level = 5;

    // Large target (field_DE & 0x7F > 1).
    s.fighters[1].field_de = 0x02;
    assert!(!s.can_backstab(1, 0), "(field_DE & 0x7F) must be <= 1");
    s.fighters[1].field_de = 0x01;

    // Only one swing this turn.
    s.fighters[1].attacks_received = 1;
    assert!(!s.can_backstab(1, 0), "AttacksReceived must be > 1");
    s.fighters[1].attacks_received = 2;

    // A readied weapon NOT in the list (short bow 44) fails; a dagger (8) is
    // in the list.
    s.fighters[0].weapon_readied = true;
    s.fighters[0].loadout = Some(Loadout {
        primary_type: 44, // short bow — not a backstab weapon
        ammo_count: 10,
        unarmed_profile: (1, 2, 3),
    });
    assert!(!s.can_backstab(1, 0), "a readied short bow can't backstab");
    s.fighters[0].loadout = Some(Loadout {
        primary_type: 8, // dagger — in the list
        ammo_count: 0,
        unarmed_profile: (1, 2, 3),
    });
    assert!(s.can_backstab(1, 0), "a readied dagger can backstab");

    // Back NOT turned (faces the attacker, N=0).
    s.fighters[1].direction = 0;
    assert!(
        !s.can_backstab(1, 0),
        "the target's back must be to the attacker"
    );
}

#[test]
fn entry_init_facing_faces_the_party_heading_enemies_reversed() {
    // ovr011.cs:803-807: direction = HalfDirToIso[md/2]; enemies +4%8.
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(26, 12)),
        (Team::Monster, GridPos::new(34, 13)),
    ]);
    s.map_direction = 2; // md/2 = 1 → HalfDirToIso[1] = 2.
    s.combat_setup();
    assert_eq!(s.fighters[0].direction, 2, "party faces the heading");
    assert_eq!(s.fighters[1].direction, 6, "enemy faces back (+4)");
    // md = 0 → HalfDirToIso[0] = 7; enemy 3.
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(26, 12)),
        (Team::Monster, GridPos::new(34, 13)),
    ]);
    s.map_direction = 0;
    s.combat_setup();
    assert_eq!(s.fighters[0].direction, 7);
    assert_eq!(s.fighters[1].direction, 3);
}

#[test]
fn entry_init_facing_survives_an_out_of_range_map_direction() {
    // `HalfDirToIso` has 4 entries and `map_direction` is a `pub u8` fed by a
    // capture field or `RESTRIKE_MAP_DIR` (which parses any u8), so the
    // `md / 2` index must be masked — an out-of-range heading must not panic.
    for md in [8u8, 9, 200, 255] {
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(26, 12)),
            (Team::Monster, GridPos::new(34, 13)),
        ]);
        s.map_direction = md;
        s.combat_setup(); // must not panic
        let party = HALF_DIR_TO_ISO[(md as usize / 2) % 4] as u8;
        assert_eq!(s.fighters[0].direction, party, "md={md}");
        assert_eq!(s.fighters[1].direction, (party + 4) % 8, "md={md}");
    }
    // The four well-formed headings are unaffected by the mask.
    for (md, want) in [(0u8, 7u8), (2, 2), (4, 3), (6, 6)] {
        let mut s = camera_state(&[(Team::Party, GridPos::new(26, 12))]);
        s.map_direction = md;
        s.combat_setup();
        assert_eq!(s.fighters[0].direction, want, "md={md}");
    }
}

#[test]
fn camera_setup_centres_the_window_on_teamlist0() {
    // BattleSetup (ovr011.cs:1209): mapScreenTopLeft = TeamList[0].pos − (3,3).
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(26, 12)),
        (Team::Monster, GridPos::new(34, 13)),
    ]);
    s.combat_setup();
    assert_eq!(s.map_screen_top_left, GridPos::new(23, 9));
    // The 7×7 window is [23,29]×[9,15]: the party member is on-screen; the
    // monster at x=34 (as in combat4) starts off-screen — the camera matters.
    assert!(s.on_screen(0));
    assert!(!s.on_screen(1));
}

#[test]
fn coord_on_screen_is_the_seven_by_seven_window() {
    assert!(CombatState::coord_on_screen(0, 0));
    assert!(CombatState::coord_on_screen(6, 6));
    assert!(!CombatState::coord_on_screen(-1, 3));
    assert!(!CombatState::coord_on_screen(7, 3));
    assert!(!CombatState::coord_on_screen(3, 7));
}

#[test]
fn screen_map_check_clamps_the_centre_to_the_map_interior() {
    // The clamp bounds (ovr033.cs:286-311): centre.x ∈ [3,46], centre.y ∈ [3,21].
    let mut s = camera_state(&[(Team::Party, GridPos::new(10, 10))]);
    s.map_screen_top_left = GridPos::new(0, 0); // centre (3,3)
    assert!(s.screen_map_check(0xFF, GridPos::new(90, 90)));
    let centre = GridPos::new(
        s.map_screen_top_left.x + SCREEN_HALF,
        s.map_screen_top_left.y + SCREEN_HALF,
    );
    assert_eq!(centre, GridPos::new(46, 21), "clamps to the far corner");
    assert!(s.screen_map_check(0xFF, GridPos::new(-50, -50)));
    let centre = GridPos::new(
        s.map_screen_top_left.x + SCREEN_HALF,
        s.map_screen_top_left.y + SCREEN_HALF,
    );
    assert_eq!(centre, GridPos::new(3, 3), "clamps to the near corner");
}

#[test]
fn screen_map_check_box_test_gates_the_scroll() {
    let mut s = camera_state(&[(Team::Party, GridPos::new(20, 12))]);
    s.map_screen_top_left = GridPos::new(17, 9); // centre (20,12)
                                                 // Inside the radius-2 box → no scroll.
    assert!(!s.screen_map_check(2, GridPos::new(21, 13)));
    assert_eq!(s.map_screen_top_left, GridPos::new(17, 9));
    // Outside the box → the centre steps all the way to `pos` (the while
    // loops chase `pos`, not merely back within radius).
    assert!(s.screen_map_check(2, GridPos::new(24, 12)));
    assert_eq!(s.map_screen_top_left, GridPos::new(21, 9)); // centre (24,12)
}

#[test]
fn draw_74b3f_recenters_an_offscreen_combatant_and_stores_direction() {
    let mut s = camera_state(&[(Team::Party, GridPos::new(40, 20))]);
    s.map_screen_top_left = GridPos::new(0, 0); // (40,20) is far off-screen
    s.focus = true;
    assert!(!s.on_screen(0));
    s.draw_74b3f(0, 5);
    assert!(s.on_screen(0), "the recenter brings it on-screen");
    assert_eq!(
        s.fighters[0].direction, 5,
        "direction stored unconditionally"
    );
    // With focus off, an off-screen combatant is NOT chased (only the store).
    s.map_screen_top_left = GridPos::new(0, 0);
    s.focus = false;
    s.draw_74b3f(0, 2);
    assert!(!s.on_screen(0));
    assert_eq!(s.fighters[0].direction, 2);
}

#[test]
fn departure_attack_turns_focus_on_per_candidate() {
    // Site 7 (`sub_3E954` @`ovr014:0AE5`): each candidate iteration of the
    // departure-attack loop sets `focusCombatAreaOnPlayer = 1`. Without it an
    // off-screen monster mover keeps focus off and the step that follows
    // (`sub_3E748`) skips its focus-gated scrolls.
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(20, 12)),   // mover (idx 0)
        (Team::Monster, GridPos::new(19, 12)), // adjacent, will be departed
    ]);
    s.map_screen_top_left = GridPos::new(40, 20); // both off-screen
    s.focus = false;
    assert!(!s.on_screen(0));
    let mut rng = EngineRng::new(SEED);
    // Step EAST (2) to (21,12): distance to (19,12) becomes 2 → departed.
    s.move_step_away_attack(&mut rng, 0, 2);
    assert!(s.focus, "a departure candidate turns the camera focus on");

    // Control: no adjacent enemy → the loop never runs → focus untouched.
    let mut s = camera_state(&[
        (Team::Party, GridPos::new(20, 12)),
        (Team::Monster, GridPos::new(30, 12)), // far away
    ]);
    s.focus = false;
    let mut rng = EngineRng::new(SEED);
    s.move_step_away_attack(&mut rng, 0, 2);
    assert!(!s.focus, "no candidates → no focus write");
}

/// Attacker (idx 0) on-screen at the window centre, target (idx 1) far
/// off-screen to the east, window at (0,0). The attacker readies
/// `primary_type`. Only the missile camera can move the window here: the
/// attacker-side `draw_74b3f` recenter needs an OFF-screen attacker, the
/// target-side draw needs an ON-screen target, and the target survives.
fn sling_state(primary_type: u8) -> CombatState {
    let attacker = Combatant::new_melee(
        0,
        Team::Party,
        false,
        GridPos::new(3, 3),
        30,
        40,
        0,
        12,
        (1, 2, 0),
        5,
        1,
    );
    let target = Combatant::new_melee(
        1,
        Team::Monster,
        true,
        GridPos::new(30, 3),
        200, // survives the swing, so no CombatantKilled scroll
        40,
        0,
        12,
        (1, 2, 0),
        5,
        1,
    );
    let mut s = CombatState::new(CombatMap::uniform(FLOOR), vec![attacker, target]);
    s.item_data = Some(synth_item_table());
    s.set_loadout(
        0,
        Loadout {
            primary_type,
            ammo_count: 40,
            unarmed_profile: (1, 2, 0),
        },
    );
    s.combat_setup_done = true; // skip the lazy setup's camera/facing seed
    s.map_screen_top_left = GridPos::new(0, 0);
    s
}

#[test]
fn a_sling_primary_fires_the_missile_camera_despite_its_null_item() {
    // `sub_3F9DB` @`ovr014:1B14-1B4C`: a Sling (0x2F) / StaffSling (0x65)
    // primary fires a SECOND `sub_40BF1` with the primary itself as the
    // missile. `GetCurrentAttackItem` hands a sling a found-but-NULL item
    // (flags 0x0A, §34.2), so the item-gated first call does not fire for it —
    // without this branch a sling would scroll no camera at all.
    let mut s = sling_state(47); // Sling
    assert!(s.on_screen(0), "attacker on-screen: no attacker recenter");
    assert!(!s.on_screen(1), "target off-screen: no target-side draw");
    let before = s.map_screen_top_left;
    let mut rng = EngineRng::new(SEED);
    // A sling resolves to a null attack item — the melee-shaped call.
    s.attack_target(&mut rng, 0, 1, false, AttackItemRef::None);
    assert_ne!(
        s.map_screen_top_left, before,
        "the sling's own missile scrolls the camera toward the target"
    );

    // Control: a plain range-1 melee primary (type 30) fires no missile, so
    // the window is untouched by an otherwise identical swing.
    let mut s = sling_state(30);
    let before = s.map_screen_top_left;
    let mut rng = EngineRng::new(SEED);
    s.attack_target(&mut rng, 0, 1, false, AttackItemRef::None);
    assert_eq!(
        s.map_screen_top_left, before,
        "a melee primary fires no missile camera"
    );
}

/// Two combatants at the given cells with the window at `top_left`, for
/// exercising [`CombatState::draw_missile_camera`] directly.
fn missile_state(a: GridPos, t: GridPos, top_left: GridPos) -> CombatState {
    let mut s = camera_state(&[(Team::Party, a), (Team::Monster, t)]);
    s.map_screen_top_left = top_left;
    s
}

#[test]
fn missile_camera_returns_early_on_a_short_path() {
    // `ovr025.cs:910-915`: `var_B0 = var_AF − 2 < 2` (or `var_AF < 2`) →
    // return before any scroll. Adjacent cells span 3 pixels → var_AF = 4 →
    // var_B0 = 2, which does NOT early-return; the same cell gives var_AF = 1.
    // Use a same-cell shot for the guaranteed early return, and assert the
    // window is untouched even though one endpoint is off-screen.
    let mut s = missile_state(
        GridPos::new(30, 12),
        GridPos::new(30, 12),
        GridPos::new(0, 0),
    );
    assert!(
        !s.on_screen_pos(GridPos::new(30, 12)),
        "off-screen endpoint"
    );
    s.draw_missile_camera(0, 1);
    assert_eq!(
        s.map_screen_top_left,
        GridPos::new(0, 0),
        "a short path scrolls nothing"
    );
}

#[test]
fn missile_camera_is_a_no_op_when_both_endpoints_are_on_screen() {
    // `ovr025.cs:934-940`: both on-screen ⇒ `center1` = the current centre, so
    // the force-recenter resolves to the window it already has.
    let mut s = missile_state(
        GridPos::new(18, 10),
        GridPos::new(22, 14),
        GridPos::new(17, 9),
    );
    assert!(s.on_screen(0) && s.on_screen(1));
    s.draw_missile_camera(0, 1);
    assert_eq!(s.map_screen_top_left, GridPos::new(17, 9));
}

#[test]
fn missile_camera_force_scrolls_to_the_midpoint_on_a_short_span() {
    // `ovr025.cs:922-926/940`: an off-screen endpoint with |Δ| ≤ 6 on both
    // axes ⇒ force-scroll to `center1 = Δ/2 + attacker`. Attacker (10,12),
    // target (16,12) → Δ=(6,0) → centre (13,12) → top-left (10,9).
    let mut s = missile_state(
        GridPos::new(10, 12),
        GridPos::new(16, 12),
        GridPos::new(4, 9),
    );
    assert!(!s.on_screen(1), "the target starts off-screen");
    s.draw_missile_camera(0, 1);
    assert_eq!(s.map_screen_top_left, GridPos::new(10, 9), "centre (13,12)");
    assert!(s.on_screen(0) && s.on_screen(1), "both now in the window");
}

#[test]
fn missile_camera_anchors_the_target_on_a_long_span() {
    // `ovr025.cs:1010-1032`: an off-screen endpoint with a span > 6 ⇒ the
    // missile leaves the screen first, so the animation force-scrolls to a
    // target-anchored centre that brings the TARGET into the window.
    let mut s = missile_state(
        GridPos::new(3, 12),
        GridPos::new(40, 12),
        GridPos::new(0, 9),
    );
    assert!(!s.on_screen(1));
    s.draw_missile_camera(0, 1);
    assert!(
        s.on_screen(1),
        "a long shot ends with the target on-screen, not the midpoint"
    );
    // Near the map's right edge the target anchor is pushed back in-bounds by
    // `var_CE`, and `ScreenMapCheck`'s clamp holds the centre at x ≤ 46.
    let mut s = missile_state(
        GridPos::new(3, 12),
        GridPos::new(49, 12),
        GridPos::new(0, 9),
    );
    s.draw_missile_camera(0, 1);
    assert!(
        s.map_screen_top_left.x + SCREEN_HALF < MAP_W - SCREEN_HALF,
        "the centre stays inside ScreenMapCheck's clamp (x <= 46)"
    );
    assert!(s.on_screen(1), "an edge target still lands in the window");
}

#[test]
fn missile_path_pixel_steps_counts_stepping_path_iterations() {
    // A 2-cell horizontal shot spans 6 pixels; Step() moves x 0→6 (6 steps)
    // then the 7th call takes none → var_AF = 7 (var_B0 = 5).
    assert_eq!(
        missile_path_pixel_steps(GridPos::new(0, 0), GridPos::new(2, 0)),
        7
    );
    // A zero-length shot: the first Step() takes none → var_AF = 1 (var_B0 =
    // −1 < 2 ⇒ the missile camera early-returns).
    assert_eq!(
        missile_path_pixel_steps(GridPos::new(5, 5), GridPos::new(5, 5)),
        1
    );
}

#[test]
fn distance_and_adjacency_are_king_moves() {
    assert_eq!(grid_distance(GridPos::new(0, 0), GridPos::new(3, 1)), 3);
    assert_eq!(grid_distance(GridPos::new(5, 5), GridPos::new(5, 5)), 0);
    // Adjacency: the 8 neighbours, not self, not distance 2.
    assert!(is_adjacent(GridPos::new(5, 5), GridPos::new(6, 6)));
    assert!(is_adjacent(GridPos::new(5, 5), GridPos::new(5, 4)));
    assert!(!is_adjacent(GridPos::new(5, 5), GridPos::new(5, 5)));
    assert!(!is_adjacent(GridPos::new(5, 5), GridPos::new(5, 7)));
}

#[test]
fn setup_geometry_is_draw_free() {
    // The whole tactical subsystem must not touch the PRNG (D9). Attach a sink
    // to a shared EngineRng, run placement + movement + facing, assert zero
    // draws.
    let log = DrawLog::default();
    let mut rng = EngineRng::new(SEED);
    rng.attach_sink(log.sink());

    let mut map = CombatMap::uniform(FLOOR);
    let roster: Vec<PlacementInput> = (0..3)
        .map(|_| place_input(Team::Party))
        .chain((0..3).map(|_| place_input(Team::Monster)))
        .collect();
    let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);
    let _ = calc_moves(12);
    let _ = step_cost(&map, p[0].pos, 2);
    let _ = target_direction(p[0].pos, p[3].pos);
    let _ = grid_distance(p[0].pos, p[3].pos);

    assert_eq!(log.len(), 0, "the setup path draws nothing (D9)");
    // (The rng binding exists only to hold the sink; silence unused warnings.)
    let _ = &mut rng;
}

/// coab≠binary #19 (doc §43.1): `can_see_combatant`'s dir-1 (NE) cone bounds
/// its second half-plane by the ANTI-diagonal through the facing cell
/// (`ovr032:066F-068B`: `tx >= (fx + fy) - ty`), not coab's main diagonal
/// (`CanSeeCombatant` case 1). Anchor (10,10), facing (11,9): coab's spurious
/// wedge — west of the facing column, between the diagonals — is OUTSIDE the
/// binary's NE cone; the anti-diagonal boundary and the east side stay inside.
/// The first-true SCAN is provably unchanged (the dir-0 cone's second
/// half-plane from this anchor is `tx - ty >= 1`, a superset of the wedge's
/// `tx - ty >= 2`), so near-list tie-break dirs are unaffected — the
/// observable is the direct boolean (e.g. a NE-facing departure cone).
#[test]
fn cone_dir1_second_half_plane_is_the_anti_diagonal_bug19() {
    let b = GridPos::new(10, 10); // anchor; facing cell (11,9)
    let see = |x, y| can_see_combatant(1, GridPos::new(x, y), b);
    // coab's wedge: OUT under the binary's cone.
    assert!(
        !see(10, 7),
        "due north is not in the NE cone (coab said it was)"
    );
    assert!(!see(9, 6));
    // The anti-diagonal boundary and beyond (east side): IN.
    assert!(see(12, 8)); // tx+ty == fx+fy, ty <= fy
    assert!(see(13, 8));
    // The first disjunct is untouched.
    assert!(see(11, 8));
    assert!(see(12, 9));
    // Scan invariance: a wedge point still resolves to dir 0 first.
    assert_eq!(find_combatant_direction(GridPos::new(10, 7), b), 0);
}
