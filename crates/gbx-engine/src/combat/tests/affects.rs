use super::*;

// === the affect substrate (doc §39.2) ==================================

/// A two-fighter state (party 0, monster 1) with an attached [`ActionLog`],
/// so a test can drive the affect API and read the emitted tripwires.
fn affect_world() -> (CombatState, ActionLog) {
    let mut state = CombatState::new(CombatMap::uniform(0x17), vec![party(0, 0), monster(1)]);
    let log = ActionLog::default();
    state.attach_action_sink(log.sink());
    (state, log)
}

/// The `stub` names of every `StubTripped` emitted, in order.
fn stubs(log: &ActionLog) -> Vec<&'static str> {
    log.events()
        .into_iter()
        .filter_map(|e| match e {
            ActionEvent::StubTripped { stub, .. } => Some(stub),
            _ => None,
        })
        .collect()
}

fn aff(kind: u8, call_table: bool) -> AffectRecord {
    AffectRecord {
        kind,
        minutes: 0,
        data: 0,
        call_affect_table: call_table,
    }
}

#[test]
fn add_affect_appends_at_the_tail() {
    let mut f = party(0, 0);
    f.add_affect(0x01, 10, 1, false); // bless
    f.add_affect(0x0B, 0, 0, false); // charm_person
    f.add_affect(0x01, 20, 2, true); // bless again (a second instance)
    let kinds: Vec<u8> = f.affects.iter().map(|a| a.kind).collect();
    assert_eq!(kinds, vec![0x01, 0x0B, 0x01], "add appends in call order");
    assert_eq!(f.affects[2].minutes, 20);
    assert!(f.affects[2].call_affect_table);
}

#[test]
fn find_affect_returns_the_first_match() {
    let mut f = party(0, 0);
    f.affects = vec![aff(0x01, false), aff(0x33, false), aff(0x01, true)];
    // Two bless (0x01) instances; find returns the FIRST (call_table false).
    let found = f.find_affect(0x01).unwrap();
    assert!(!found.call_affect_table);
    assert!(f.has_affect(0x33));
    assert!(f.find_affect(0x99).is_none());
}

#[test]
fn remove_affect_drops_only_one_instance() {
    let (mut state, log) = affect_world();
    state.fighters[0].affects = vec![aff(0x33, false), aff(0x33, false), aff(0x01, false)];
    state.remove_affect(0, 0x33);
    let kinds: Vec<u8> = state.fighters[0].affects.iter().map(|a| a.kind).collect();
    assert_eq!(kinds, vec![0x33, 0x01], "exactly one snake_charm removed");
    // A plain (non-call_table, non-stat) removal fires no side tripwire.
    assert!(stubs(&log).is_empty());
}

#[test]
fn remove_affect_side_tripwire_on_call_table_and_stat_kinds() {
    // call_affect_table set → "affect-remove-side".
    let (mut state, log) = affect_world();
    state.fighters[0].affects = vec![aff(0x01, true)];
    state.remove_affect(0, 0x01);
    assert_eq!(stubs(&log), vec!["affect-remove-side"]);

    // A STAT_RECOMPUTE kind (friends 0x0E → CHA) trips even without call_table.
    let (mut state, log) = affect_world();
    state.fighters[0].affects = vec![aff(0x0E, false)];
    state.remove_affect(0, 0x0E);
    assert_eq!(stubs(&log), vec!["affect-remove-side"]);

    // STR-recompute kinds too (enlarge/strength/strength_spell).
    for &k in &[0x0C_u8, 0x26, 0x92] {
        let (mut state, log) = affect_world();
        state.fighters[0].affects = vec![aff(k, false)];
        state.remove_affect(0, k);
        assert_eq!(stubs(&log), vec!["affect-remove-side"], "kind {k:#x}");
    }
}

#[test]
fn remove_combat_affects_strips_the_table_and_keeps_the_rest() {
    let (mut state, log) = affect_world();
    // faerie_fire (stripped), bless (kept), reduce (stripped), berserk (kept —
    // not in the table).
    state.fighters[0].affects = vec![
        aff(0x07, false),
        aff(0x01, false),
        aff(0x0D, false),
        aff(0x4D, false),
    ];
    state.remove_combat_affects(0);
    let kinds: Vec<u8> = state.fighters[0].affects.iter().map(|a| a.kind).collect();
    assert_eq!(kinds, vec![0x01, 0x4D], "only table kinds stripped");
    // control_morale != PC_Berzerk → no berserk quirk despite the affect.
    assert!(stubs(&log).is_empty());
}

#[test]
fn remove_combat_affects_berserk_quirk_tripwire() {
    let (mut state, log) = affect_world();
    state.fighters[0].affects = vec![aff(0x4D, false)]; // berserk survives the strip
    state.fighters[0].control_morale = 0xB3; // PC_Berzerk
    state.remove_combat_affects(0);
    assert_eq!(stubs(&log), vec!["affect-berserk"]);
}

#[test]
fn remove_attackers_affects_strips_its_four() {
    let (mut state, _log) = affect_world();
    state.fighters[0].affects = vec![
        aff(0x0D, false), // reduce (stripped)
        aff(0x01, false), // bless (kept)
        aff(0x3A, false), // clear_movement (stripped)
        aff(0x8B, false), // affect_8b (stripped)
        aff(0x90, false), // owlbear_hug_round_attack (stripped)
    ];
    state.remove_attackers_affects(0);
    let kinds: Vec<u8> = state.fighters[0].affects.iter().map(|a| a.kind).collect();
    assert_eq!(kinds, vec![0x01]);
}

#[test]
fn remove_invisibility_clears_every_instance() {
    let (mut state, _log) = affect_world();
    state.fighters[0].affects = vec![aff(0x19, false), aff(0x01, false), aff(0x19, true)];
    state.remove_invisibility(0);
    let kinds: Vec<u8> = state.fighters[0].affects.iter().map(|a| a.kind).collect();
    assert_eq!(kinds, vec![0x01], "both invisibility affects gone");
}

#[test]
fn dispatch_trips_on_a_found_affect() {
    let (mut state, log) = affect_world();
    // snake_charm (0x33) is in PlayerRestrained's list.
    state.fighters[0].affects = vec![aff(0x33, false)];
    state.check_affects_effect(0, CheckType::PlayerRestrained);
    assert_eq!(stubs(&log), vec!["affect-effect"]);
}

#[test]
fn dispatch_on_empty_lists_is_a_total_no_op() {
    let (mut state, log) = affect_world();
    for ty in [
        CheckType::None,
        CheckType::Visibility,
        CheckType::PlayerRestrained,
        CheckType::Type10,
        CheckType::Type16,
        CheckType::Morale,
        CheckType::Movement,
        CheckType::Type19,
        CheckType::SpecialAttacks,
        CheckType::Type5,
        CheckType::Type11,
        CheckType::Death,
        CheckType::Type14,
        CheckType::Type15,
        CheckType::Confusion,
    ] {
        state.check_affects_effect(0, ty);
        state.check_affects_effect(1, ty);
    }
    assert!(stubs(&log).is_empty(), "no affects → no trips, no draws");
}

#[test]
fn calc_affect_effect_radius_carrier_scan() {
    // The actor (0) lacks prayer; a team-mate carrier (1) holds it → the
    // radius scan finds it and trips. Prayer (0x31) is a RADIUS kind.
    let (mut state, log) = affect_world();
    state.fighters[1].affects = vec![aff(0x31, false)];
    state.calc_affect_effect(0, 0x31);
    assert_eq!(stubs(&log), vec!["affect-effect"]);

    // A non-radius kind not on the actor is NOT sourced from a carrier.
    let (mut state, log) = affect_world();
    state.fighters[1].affects = vec![aff(0x01, false)]; // bless — not a radius kind
    state.calc_affect_effect(0, 0x01);
    assert!(stubs(&log).is_empty());
}

#[test]
fn dispatch_id_lists_have_the_expected_case_lengths() {
    // Guards the transcription against an accidental edit: the 24 case lengths
    // from coab ovr024.cs:140-375.
    let lens: Vec<usize> = [
        CheckType::None,
        CheckType::Visibility,
        CheckType::Type2,
        CheckType::Type3,
        CheckType::SpecialAttacks,
        CheckType::Type5,
        CheckType::PreDamage,
        CheckType::PlayerRestrained,
        CheckType::Type8,
        CheckType::MagicResistance,
        CheckType::Type10,
        CheckType::Type11,
        CheckType::SavingThrow,
        CheckType::Death,
        CheckType::Type14,
        CheckType::Type15,
        CheckType::Type16,
        CheckType::Morale,
        CheckType::Movement,
        CheckType::Type19,
        CheckType::FireShield,
        CheckType::Confusion,
        CheckType::Type22,
        CheckType::Type23,
    ]
    .iter()
    .map(|t| t.affect_ids().len())
    .collect();
    assert_eq!(
        lens,
        vec![0, 4, 7, 7, 6, 16, 21, 7, 5, 12, 10, 8, 16, 3, 11, 5, 7, 3, 3, 5, 2, 1, 1, 1]
    );
}

// === the §47.7 handlers — troll regen + the dwarf racial hit adjusts ====

/// The on-hit troll cascade (`sp_regenerate` @`ovr013:1FCC`, Type_5): a
/// troll carrying `troll_regen` 0x65 gains `regenerate` 0x3B + `regen_3_hp`
/// 0x62 (the ADD-time dispatch) ONCE — a second hit finds 0x62 and adds
/// nothing. Then every round-end Type_19 heals +3, capped at max.
#[test]
fn troll_regen_cascade_and_round_end_tick() {
    let (mut state, log) = affect_world();
    state.fighters[1].hp_max = 40;
    state.fighters[1].hp_current = 14;
    state.fighters[1].affects.push(aff(0x65, false));

    state.check_affects_effect(1, CheckType::Type5); // the on-hit dispatch
    let kinds: Vec<u8> = state.fighters[1].affects.iter().map(|a| a.kind).collect();
    assert_eq!(kinds, vec![0x65, 0x3B, 0x62], "the cascade, tail-appended");

    state.check_affects_effect(1, CheckType::Type5); // a second hit
    assert_eq!(state.fighters[1].affects.len(), 3, "already regenerating");

    for _ in 0..3 {
        state.check_affects_effect(1, CheckType::Type19); // round ends
    }
    assert_eq!(state.fighters[1].hp_current, 23, "+3 per round end");
    state.fighters[1].hp_current = 39;
    state.check_affects_effect(1, CheckType::Type19);
    assert_eq!(state.fighters[1].hp_current, 40, "capped at hp_max");
    assert!(
        stubs(&log).is_empty(),
        "the whole cascade is handled, no trips"
    );
}

/// The death strip nets exactly ONE `regen_3_hp` left on the corpse: the
/// table removes `regenerate` 0x3B (whose Remove-handler re-adds a 0x62 at
/// the tail) before it removes the FIRST 0x62 (§47.7 — the order is
/// load-bearing).
#[test]
fn death_strip_regen_dance_leaves_one_regen_3hp() {
    let (mut state, log) = affect_world();
    state.fighters[1].affects.push(aff(0x65, false));
    state.check_affects_effect(1, CheckType::Type5);
    state.remove_combat_affects(1);
    let kinds: Vec<u8> = state.fighters[1].affects.iter().map(|a| a.kind).collect();
    assert_eq!(
        kinds,
        vec![0x65, 0x62],
        "0x3B stripped, its re-added 0x62 survives"
    );
    assert!(
        stubs(&log).is_empty(),
        "the 0x3B remove-side is handled, no trip"
    );
}

/// The dwarf racial hit handlers (§47.7), live inside the hit roll:
/// `dwarf_and_gnome_vs_giants` 0x2F (Type_16 on the TARGET) subtracts 4 when
/// the attacker is a size-class-2 giant/troll; `dwarf_vs_orc` 0x1A (Type_10
/// on the ATTACKER) adds 1 when the attacker's held target is orc-class
/// (`field_14B & 4`). Gates verified both ways.
#[test]
fn dwarf_racial_hit_adjustments() {
    let (mut state, _log) = affect_world();
    // [1] = a troll-shaped attacker; [0] = a dwarf with both racials.
    state.fighters[1].monster_type = 10;
    state.fighters[1].field_de = 0x82;
    state.fighters[1].field_14b = 0x0E;
    state.fighters[0].affects.push(aff(0x2F, false));
    state.fighters[0].affects.push(aff(0x1A, false));

    // Troll attacks dwarf: Type_16 on the target fires the −4.
    state.attack_roll = 10;
    state.selected_attacker = 1;
    state.check_affects_effect(0, CheckType::Type16);
    assert_eq!(
        state.attack_roll, 6,
        "vs-giants −4 (giant/troll, size-class 2)"
    );

    // A size-1 attacker: no penalty.
    state.fighters[1].field_de = 0x01;
    state.attack_roll = 10;
    state.check_affects_effect(0, CheckType::Type16);
    assert_eq!(state.attack_roll, 10, "man-sized attacker: gate closed");

    // Dwarf attacks the orc-class troll: Type_10 on the attacker fires +1.
    state.fighters[0].target = Some(1);
    state.attack_roll = 10;
    state.check_affects_effect(0, CheckType::Type10);
    assert_eq!(state.attack_roll, 11, "vs-orc +1 (field_14B & 4)");

    // A non-orc-class target: no bonus.
    state.fighters[1].field_14b = 0;
    state.attack_roll = 10;
    state.check_affects_effect(0, CheckType::Type10);
    assert_eq!(state.attack_roll, 10, "non-orc target: gate closed");
}
