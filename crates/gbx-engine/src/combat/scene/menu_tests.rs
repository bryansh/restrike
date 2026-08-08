//! ★ **The manual UI's fixture goldens** (doc §9.6): the conditional words on
//! the prompt row, the movement prompt's halved count, the aim cursor and its
//! focus box, and the key→[`TurnCmd`] table.
//!
//! Everything here is text and state — no pixels. The pixel side is already
//! pinned by slice 3/4's goldens (the prompt row is drawn by the same
//! `draw_prompt` they cover), and what M6c adds is *which string goes there*.

use super::menu::{ManualUi, MenuAction, Stage};
use crate::combat::manual::{AimCamera, TurnCmd, TurnOutcome};
use crate::combat::{CombatMap, CombatState, Combatant, GridPos, Team};
use crate::input::{ExtKey, InputEvent};
use crate::rng::EngineRng;

const SEED: u32 = 0x0C0F_FEE0;
const FLOOR: u8 = 0x17;

/// A PC at (4,4) with a monster adjacent east, suspended on the PC's turn.
fn open_fight() -> (CombatState, EngineRng, ManualUi) {
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
    let mut state = CombatState::new(CombatMap::uniform(FLOOR), vec![pc, foe]);
    let mut rng = EngineRng::new(SEED);
    state.set_interactive(true);
    for _ in 0..64 {
        if let crate::combat::CombatStep::AwaitPlayerTurn { .. } = state.step(&mut rng) {
            let ui = ManualUi::open(&mut state, 0);
            return (state, rng, ui);
        }
    }
    panic!("no manual turn opened");
}

#[test]
fn the_main_menu_prints_only_the_words_whose_conditions_hold() {
    let (mut state, _rng, mut ui) = open_fight();
    assert_eq!(ui.prompt(), "Move View Aim Quick Done");

    state.fighters[0].has_items = true;
    state.fighters[0].memorized_list = vec![0x0F];
    state.fighters[0].skill_level_cleric = 4;
    ui.refresh(&mut state);
    assert_eq!(ui.prompt(), "Move View Aim Use Cast Turn Quick Done");

    state.fighters[0].move_left = 0;
    state.fighters[0].can_cast = false; // §45's disruption
    state.fighters[0].has_turned_undead = true;
    ui.refresh(&mut state);
    assert_eq!(ui.prompt(), "View Aim Use Quick Done");
}

#[test]
fn the_done_submenu_prints_its_own_conditional_words() {
    let (mut state, _rng, mut ui) = open_fight();
    assert_eq!(ui.key(InputEvent::Char(b'D')), MenuAction::None);
    assert_eq!(ui.stage(), Stage::Done);
    assert_eq!(ui.prompt(), "Guard Delay Quit Speed Exit");

    // A dying team member splices Bandage in.
    let mut ally = state.fighters[0].clone();
    ally.id = 2;
    ally.pos = GridPos::new(4, 5);
    ally.health_status = crate::combat::HealthStatus::Dying;
    state.fighters.push(ally);
    ui.refresh(&mut state);
    assert_eq!(ui.prompt(), "Guard Delay Quit Bandage Speed Exit");

    // Exit goes back to the main menu without ending the turn.
    assert_eq!(ui.key(InputEvent::Char(b'E')), MenuAction::None);
    assert_eq!(ui.stage(), Stage::Main);
}

#[test]
fn the_speed_menu_shows_the_original_prompt_and_its_two_gated_words() {
    let (_state, _rng, mut ui) = open_fight();
    ui.key(InputEvent::Char(b'D'));
    ui.key(InputEvent::Char(b'S'));
    assert_eq!(ui.stage(), Stage::Speed);
    assert_eq!(ui.prompt(), "GameSpeed (4) : Slower Faster Exit");

    // Slower increments `game_speed_var` (longer delays) — and at 9 the word
    // is gone; at 0 it is Faster that goes.
    assert_eq!(
        ui.key(InputEvent::Char(b'S')),
        MenuAction::Issue(TurnCmd::SetSpeed(5))
    );
    ui.set_game_speed(9);
    assert_eq!(ui.prompt(), "GameSpeed (9) : Faster Exit");
    assert_eq!(
        ui.key(InputEvent::Char(b'S')),
        MenuAction::None,
        "Slower is not offered at 9, and the core would refuse it"
    );
    ui.set_game_speed(0);
    assert_eq!(ui.prompt(), "GameSpeed (0) : Slower Exit");
    assert_eq!(
        ui.key(InputEvent::Char(b'F')),
        MenuAction::None,
        "nor Faster at 0"
    );
}

#[test]
fn the_movement_prompt_counts_whole_moves_not_halves() {
    let (mut state, _rng, mut ui) = open_fight();
    state.fighters[0].move_left = 9;
    ui.refresh(&mut state);
    assert_eq!(
        ui.key(InputEvent::Char(b'M')),
        MenuAction::Issue(TurnCmd::BeginMove)
    );
    assert_eq!(ui.stage(), Stage::Moving);
    // 9 half-moves = 4 whole ones (the original's own integer division).
    assert_eq!(ui.prompt(), "Move/Attack, Move Left = 4 ");
}

#[test]
fn a_direction_key_at_the_main_menu_opens_the_loop_and_owes_its_step() {
    let (_state, _rng, mut ui) = open_fight();
    // Kp6 / Right → ctrl code 'M' → direction 2 (east).
    assert_eq!(
        ui.key(InputEvent::Ext(ExtKey::Kp6)),
        MenuAction::Issue(TurnCmd::BeginMove)
    );
    assert_eq!(ui.stage(), Stage::Moving);
    assert_eq!(ui.take_follow_up(), Some(TurnCmd::MoveStep(2)));
    assert_eq!(ui.take_follow_up(), None, "owed exactly once");
}

#[test]
fn the_movement_loop_maps_every_direction_key_and_its_two_exits() {
    let (_state, _rng, mut ui) = open_fight();
    ui.key(InputEvent::Char(b'M'));
    for (key, dir) in [
        (b'G', 7u8),
        (b'H', 0),
        (b'I', 1),
        (b'K', 6),
        (b'M', 2),
        (b'O', 5),
        (b'P', 4),
        (b'Q', 3),
    ] {
        assert_eq!(
            ui.key(InputEvent::Char(key)),
            MenuAction::Issue(TurnCmd::MoveStep(dir))
        );
    }
    assert_eq!(
        ui.key(InputEvent::Enter),
        MenuAction::Issue(TurnCmd::EndMove)
    );
    assert_eq!(ui.stage(), Stage::Main);
    ui.key(InputEvent::Char(b'M'));
    assert_eq!(
        ui.key(InputEvent::Escape),
        MenuAction::Issue(TurnCmd::AbortMove)
    );
    assert_eq!(ui.stage(), Stage::Main);
}

#[test]
fn the_flee_prompt_only_answers_yes_or_no() {
    let (_state, _rng, mut ui) = open_fight();
    ui.key(InputEvent::Char(b'M'));
    ui.note(crate::combat::TurnOutcome::FleePrompt);
    assert_eq!(ui.stage(), Stage::FleePrompt);
    assert_eq!(ui.prompt(), "Flee: Yes No");
    // `yes_no` loops until Y or N — ESC is not an answer.
    assert_eq!(ui.key(InputEvent::Escape), MenuAction::None);
    assert_eq!(ui.stage(), Stage::FleePrompt);
    assert_eq!(ui.key(InputEvent::Char(b'X')), MenuAction::None);
    assert_eq!(ui.key(InputEvent::Char(b'N')), MenuAction::None);
    assert_eq!(ui.stage(), Stage::Moving, "N returns to the loop");
    ui.note(crate::combat::TurnOutcome::FleePrompt);
    assert_eq!(
        ui.key(InputEvent::Char(b'Y')),
        MenuAction::Issue(TurnCmd::Flee)
    );
}

#[test]
fn blocked_and_refused_steps_print_the_originals_own_lines() {
    let (_state, _rng, mut ui) = open_fight();
    ui.key(InputEvent::Char(b'M'));
    ui.note(crate::combat::TurnOutcome::Blocked);
    assert_eq!(ui.prompt(), "can't go there");
    // The next keypress clears it.
    ui.key(InputEvent::Char(b'X'));
    assert_eq!(ui.prompt(), "Move/Attack, Move Left = 12 ");

    ui.note_refusal(
        &TurnCmd::MoveStep(2),
        &crate::combat::TurnRefusal::NotWithThatWeapon,
    );
    assert_eq!(ui.prompt(), "Not with that weapon");
    ui.key(InputEvent::Char(b'X'));
    ui.note_refusal(
        &TurnCmd::AttackTarget { target: 1 },
        &crate::combat::TurnRefusal::DuplicateTarget { target: 1 },
    );
    assert_eq!(ui.prompt(), "Already been targeted");
}

// === §9.4, Aim ==========================================================

#[test]
fn aim_shows_target_only_when_the_commit_is_legal() {
    let (mut state, _rng, mut ui) = open_fight();
    assert_eq!(ui.key(InputEvent::Char(b'A')), MenuAction::None);
    ui.refresh(&mut state);
    assert_eq!(ui.stage(), Stage::Aim);
    // ★ The list opens on the ATTACKER: `copy_sorted_players` filters nothing
    // (`p => true`) and the self-entry reaches at steps 0, so it sorts first
    // (`ovr014.cs:2157` steps the list by 0). You cannot target yourself, so
    // the word is absent until Next moves off.
    assert_eq!(ui.aim_target(), Some(0));
    assert_eq!(ui.prompt(), "Aim: Next Prev Manual Center Exit");
    assert_eq!(ui.focus_cell(), Some(GridPos::new(4, 4)));

    ui.key(InputEvent::Char(b'N'));
    ui.refresh(&mut state);
    // The adjacent monster is the next entry, and bare hands reach it.
    assert_eq!(ui.aim_target(), Some(1));
    assert_eq!(ui.prompt(), "Aim: Next Prev Manual Target Center Exit");
    assert_eq!(ui.status().as_deref(), Some("Range = 1  "));
    assert_eq!(ui.focus_cell(), Some(GridPos::new(5, 4)));

    // Push it out of reach: the word goes, and the range readout follows.
    state.fighters[1].pos = GridPos::new(9, 4);
    state.rebuild_occupancy();
    ui.refresh(&mut state);
    assert_eq!(ui.prompt(), "Aim: Next Prev Manual Center Exit");
    assert_eq!(ui.status().as_deref(), Some("Range = 5  "));
}

#[test]
fn next_and_prev_walk_the_scan_list_and_draw_the_aim_line() {
    let (mut state, _rng, mut ui) = open_fight();
    let mut ally = state.fighters[0].clone();
    ally.id = 2;
    ally.pos = GridPos::new(4, 6);
    state.fighters.push(ally);
    state.rebuild_occupancy();
    ui.key(InputEvent::Char(b'A'));
    ui.refresh(&mut state);

    let first = ui.aim_target().expect("a focused target");
    let action = ui.key(InputEvent::Char(b'N'));
    assert!(
        matches!(
            action,
            MenuAction::Issue(TurnCmd::Aim(AimCamera::Line { .. }))
        ),
        "cycling draws the original's aim line: {action:?}"
    );
    ui.refresh(&mut state);
    let second = ui.aim_target().expect("a focused target");
    assert_ne!(first, second, "Next moved the focus");
    ui.key(InputEvent::Char(b'P'));
    ui.refresh(&mut state);
    assert_eq!(ui.aim_target(), Some(first), "Prev came back");
}

#[test]
fn manual_opens_a_free_cursor_that_scrolls_and_clamps() {
    let (mut state, _rng, mut ui) = open_fight();
    ui.key(InputEvent::Char(b'A'));
    ui.refresh(&mut state);
    ui.key(InputEvent::Char(b'N')); // off the self-entry, onto the monster
    ui.refresh(&mut state);
    assert_eq!(ui.key(InputEvent::Char(b'M')), MenuAction::None);
    assert_eq!(ui.stage(), Stage::Cursor);
    assert_eq!(ui.prompt(), "(Use Cursor keys) Target Center Exit");
    assert_eq!(
        ui.aim_target(),
        None,
        "a free cursor has no list target, even over one"
    );

    // A direction key moves the cursor and scrolls the window.
    let action = ui.key(InputEvent::Ext(ExtKey::Kp4)); // 'K' → west
    assert_eq!(
        action,
        MenuAction::Issue(TurnCmd::Aim(AimCamera::Cursor {
            at: GridPos::new(5, 4),
            dir: 6
        }))
    );
    ui.refresh(&mut state);
    assert_eq!(ui.focus_cell(), Some(GridPos::new(4, 4)));

    // Over the acting combatant itself, `Target` is not offered.
    assert_eq!(ui.prompt(), "(Use Cursor keys) Center Exit");
    assert_eq!(ui.key(InputEvent::Char(b'T')), MenuAction::None);
}

#[test]
fn target_commits_at_the_focused_combatant() {
    let (mut state, _rng, mut ui) = open_fight();
    ui.key(InputEvent::Char(b'A'));
    ui.refresh(&mut state);
    ui.key(InputEvent::Char(b'N')); // off the self-entry
    ui.refresh(&mut state);
    assert_eq!(
        ui.key(InputEvent::Char(b'T')),
        MenuAction::Issue(TurnCmd::AttackTarget { target: 1 })
    );
}

#[test]
fn center_recentres_and_exit_leaves_aim() {
    let (mut state, _rng, mut ui) = open_fight();
    ui.key(InputEvent::Char(b'A'));
    ui.refresh(&mut state);
    ui.key(InputEvent::Char(b'N')); // off the self-entry
    ui.refresh(&mut state);
    assert_eq!(
        ui.key(InputEvent::Char(b'C')),
        MenuAction::Issue(TurnCmd::Aim(AimCamera::Center {
            at: GridPos::new(5, 4)
        }))
    );
    assert_eq!(ui.key(InputEvent::Char(b'E')), MenuAction::None);
    assert_eq!(ui.stage(), Stage::Main);
    assert_eq!(
        ui.focus_cell(),
        Some(GridPos::new(4, 4)),
        "back on the actor"
    );
}

// === the rest of §9.1's keys ============================================

#[test]
fn the_main_menu_routes_its_remaining_words() {
    let (mut state, _rng, mut ui) = open_fight();
    assert_eq!(ui.key(InputEvent::Char(b'V')), MenuAction::OpenSheet);
    assert_eq!(
        ui.key(InputEvent::Char(b'Q')),
        MenuAction::Issue(TurnCmd::EngageQuickFight)
    );
    assert_eq!(
        ui.key(InputEvent::Char(b' ')),
        MenuAction::Issue(TurnCmd::RevokeQuickFight)
    );
    assert_eq!(
        ui.key(InputEvent::Char(b'2')),
        MenuAction::Issue(TurnCmd::ToggleAutoMagic)
    );
    // Use and Turn are silent while their conditions are false...
    assert_eq!(ui.key(InputEvent::Char(b'U')), MenuAction::None);
    assert_eq!(ui.key(InputEvent::Char(b'T')), MenuAction::None);
    // ...and route once they hold.
    state.fighters[0].has_items = true;
    state.fighters[0].skill_level_cleric = 3;
    ui.refresh(&mut state);
    assert_eq!(
        ui.key(InputEvent::Char(b'U')),
        MenuAction::Issue(TurnCmd::UseItem)
    );
    assert_eq!(
        ui.key(InputEvent::Char(b'T')),
        MenuAction::Issue(TurnCmd::TurnUndead)
    );
}

#[test]
fn a_pending_cast_opens_with_no_menu_at_all() {
    let (mut state, mut rng, _ui) = open_fight();
    state.fighters[0].memorized_list = vec![0x17]; // hold person queues
    state
        .issue(
            &mut rng,
            TurnCmd::CastSpell {
                spell_id: 0x17,
                targets: vec![1],
            },
        )
        .unwrap();
    // Its next turn: `combat_menu`'s head resolves the queue instead of
    // drawing words (`ovr009.cs:155-163`).
    let actor = loop {
        match state.step(&mut rng) {
            crate::combat::CombatStep::AwaitPlayerTurn { combatant_id } => break combatant_id,
            crate::combat::CombatStep::Ended => panic!("the fight ended too soon"),
            _ => {}
        }
    };
    let ui = ManualUi::open(&mut state, actor);
    assert_eq!(ui.stage(), Stage::PendingCast);
    assert_eq!(ui.prompt(), "");
}

// --- `displayInput`'s selection machinery (`gbl.menuSelectedWord`) ---------

#[test]
fn comma_and_period_cycle_the_selection_and_enter_resolves_it() {
    let (_state, _rng, mut ui) = open_fight();
    // The main menu always carries Move? -> at minimum "View Aim Quick Done".
    let span0 = ui.selected_span().expect("a menu shows a selection");
    assert_eq!(ui.key(InputEvent::Char(b'.')), MenuAction::None);
    let span1 = ui.selected_span().expect("still a menu");
    assert_ne!(span0, span1, "'.' moved the selection");
    assert_eq!(ui.key(InputEvent::Char(b',')), MenuAction::None);
    assert_eq!(
        ui.selected_span().expect("back where it started"),
        span0,
        "',' cycles the other way"
    );
    // Enter resolves the SELECTED word: select 'Q'uick, then Enter.
    ui.key(InputEvent::Char(b'q'));
    let mut ui2 = ui.clone();
    // Whatever Quick resolved to, Enter on a fresh Quick-selected menu
    // resolves identically (Enter == the highlighted word's hotkey).
    assert_eq!(ui.selected_hotkey(), Some(b'Q'));
    assert_eq!(
        ui2.key(InputEvent::Enter),
        MenuAction::Issue(TurnCmd::EngageQuickFight),
    );
}

#[test]
fn the_selection_index_carries_into_submenus_like_the_original() {
    let (_state, _rng, mut ui) = open_fight();
    // Pressing 'D' lands the selection on "Done" (index 4 of
    // "Move View Aim Quick Done") and opens the submenu — and
    // `gbl.menuSelectedWord` is GLOBAL: the original does not reset it for
    // the new menu, so index 4 of "Guard Delay Quit Speed Exit" (no dying
    // teammate in this duel, so no Bandage) is "Exit".
    ui.key(InputEvent::Char(b'd'));
    assert_eq!(ui.selected_hotkey(), Some(b'E'));
}

#[test]
fn transient_messages_render_without_a_selection_span() {
    let (_state, _rng, mut ui) = open_fight();
    assert!(ui.selected_span().is_some());
    ui.note(TurnOutcome::Blocked); // "CAN'T GO THERE"
    assert!(
        ui.selected_span().is_none(),
        "a transient message is a prompt, not a menu"
    );
}

#[test]
fn the_selection_seeds_from_the_hosts_memory() {
    let (_state, _rng, mut ui) = open_fight();
    ui.set_selected(3);
    assert_eq!(ui.selected_index(), 3);
    let words = gbx_len_of_menu(&ui);
    assert!(words > 3, "the main menu has at least four words");
}

fn gbx_len_of_menu(ui: &ManualUi) -> usize {
    crate::widgets::build_words(&ui.prompt()).len()
}

// === `can_attack_target`'s "Attack Ally: " (ovr014.cs:1717-1749) ==========

/// The core refuses an unconfirmed ally commit; the UI turns that into the
/// original's own `yes_no` prompt rather than swallowing it.
#[test]
fn an_ally_refusal_opens_the_attack_ally_prompt() {
    let (_state, _rng, mut ui) = open_fight();
    let cmd = TurnCmd::AttackTarget { target: 2 };
    ui.note_refusal(&cmd, &crate::combat::TurnRefusal::AllyTarget { target: 2 });
    assert_eq!(ui.stage(), Stage::AllyPrompt);
    assert_eq!(ui.prompt(), "Attack Ally: Yes No");
}

/// `Y` → `ConfirmAttackAlly`, and the refused command follows it as the
/// keypress's second half (the host issues `take_follow_up` right behind).
#[test]
fn answering_yes_arms_the_confirmation_and_re_issues_the_commit() {
    let (_state, _rng, mut ui) = open_fight();
    ui.key(InputEvent::Char(b'a')); // open Aim so the return stage is real
    assert_eq!(ui.stage(), Stage::Aim);
    let cmd = TurnCmd::AttackTarget { target: 2 };
    ui.note_refusal(&cmd, &crate::combat::TurnRefusal::AllyTarget { target: 2 });

    assert_eq!(
        ui.key(InputEvent::Char(b'y')),
        MenuAction::Issue(TurnCmd::ConfirmAttackAlly)
    );
    assert_eq!(ui.stage(), Stage::Aim, "back where the commit came from");
    assert_eq!(ui.take_follow_up(), Some(cmd));
    assert_eq!(ui.take_follow_up(), None, "owed exactly once");
}

/// `N` → nothing happens and the aim menu re-prompts (`result = false`,
/// `ovr014.cs:1727`). ESC does not answer a `yes_no` (`ovr027.cs:682`).
#[test]
fn answering_no_drops_the_commit_and_esc_does_not_answer_at_all() {
    let (_state, _rng, mut ui) = open_fight();
    ui.key(InputEvent::Char(b'a'));
    let cmd = TurnCmd::AttackTarget { target: 2 };
    ui.note_refusal(&cmd, &crate::combat::TurnRefusal::AllyTarget { target: 2 });

    assert_eq!(ui.key(InputEvent::Escape), MenuAction::None);
    assert_eq!(ui.stage(), Stage::AllyPrompt, "yes_no has no escape");

    assert_eq!(ui.key(InputEvent::Char(b'n')), MenuAction::None);
    assert_eq!(ui.stage(), Stage::Aim);
    assert_eq!(ui.take_follow_up(), None, "N drops the held commit");
}
