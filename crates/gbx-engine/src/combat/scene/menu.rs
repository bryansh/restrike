//! ★ **The manual combat UI** (doc §1.7/§9) — the words on the prompt row and
//! the keys that turn into [`TurnCmd`]s.
//!
//! This is presentation's half of §9's split rule: it decides **which words
//! show**, tracks the aim cursor, and prints the original's own messages. It
//! never decides whether a command may execute — [`CombatState::issue`] does
//! that, from the same cited conditions, and refuses loudly if these two ever
//! disagree.
//!
//! **Why not [`Hotbar`](crate::widgets::Hotbar):** the combat menus need the
//! `displayInput_specialKeyPressed` bit. `combat_menu` dispatches a plain `M`
//! to the *Move* word and an extended `M` (the keypad's right arrow) to a step
//! east (`ovr009.cs:184` vs `:243`), and `Hotbar` folds both onto one resolved
//! byte. [`InputEvent`] carries the distinction natively — `Char` vs `Ext` — so
//! the menus read it directly.
//!
//! Derived by reading coab for behavior (D11, never copied): `combat_menu`
//! (`ovr009.cs:148-296`), `delay_menu` (`:616-669`), `set_gamespeed`
//! (`:672-707`), `sub_33B26` (`:416-588`), `aim_menu`/`aim_sub_menu`/`Target`
//! (`ovr014.cs:1752-2220`), `yes_no` (`ovr027.cs:676-689`).

use super::{strings, FocusCursor};
use crate::combat::manual::{movement_key_direction, AimCamera, TurnCmd, TurnOutcome, TurnRefusal};
use crate::combat::{CombatState, DoneWords, GridPos, MenuWords};
use crate::input::InputEvent;

/// Which menu is on the prompt row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Stage {
    /// §9.1's `Move View Aim Use Cast Turn Quick Done`.
    Main,
    /// §9.3's movement loop — "Move/Attack, Move Left = N".
    Moving,
    /// §9.4's `Next Prev Manual [Target] Center Exit`.
    Aim,
    /// §9.4's free cursor — `[Target] Center Exit` over "(Use Cursor keys)".
    Cursor,
    /// §9.2's `Guard Delay Quit Bandage Speed Exit`.
    Done,
    /// `set_gamespeed`'s `Slower Faster Exit`.
    Speed,
    /// The movement loop's off-map `yes_no("Flee:")`.
    FleePrompt,
    /// The caster's queued spell resolving before any menu opens
    /// (`ovr009.cs:155-163`) — the host aims and resolves; no words show.
    PendingCast,
}

/// What the host owes after feeding a key to [`ManualUi::key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// The UI changed only its own state — re-render and read on.
    None,
    /// Issue this through [`CombatState::issue`], then hand the result back to
    /// [`ManualUi::note`].
    Issue(TurnCmd),
    /// `V` — open the M3 character sheet for the acting combatant
    /// (`viewPlayer`, `ovr009.cs:197`), palette swapped back around it
    /// (`ovr020.cs:240,334`). The host issues [`TurnCmd::ViewSheet`] when the
    /// sheet closes.
    OpenSheet,
}

/// The aim cursor: either walking the scan list or free over the map.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct AimState {
    /// `copy_sorted_players`' list (`ovr014.cs:2073`) — every reachable
    /// combatant, allies and self included, in the exchange-sort order.
    list: Vec<usize>,
    /// `list_index − 1` (the original's index is 1-based).
    index: usize,
    /// The free cursor's cell, once `Manual` has been pressed.
    cursor: Option<GridPos>,
    /// Where the camera's aim line was last drawn from.
    last_cell: GridPos,
}

/// The manual turn's presentation state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ManualUi {
    actor: usize,
    stage: Stage,
    /// Boundary-read legality (§9.1/§9.2), refreshed by [`ManualUi::refresh`].
    words: MenuWords,
    done: DoneWords,
    /// The actor's cell and the whole moves left — both boundary reads, both
    /// only used to compose text and place the focus box.
    actor_pos: GridPos,
    move_left: i32,
    /// `game_speed_var`, mirrored from the scene's clock for the Speed menu.
    game_speed: u8,
    aim: Option<AimState>,
    /// Whether the focused combatant may be committed at (§9.4's `Target`
    /// word) — a boundary read of [`CombatState::can_commit_aim`].
    can_commit: bool,
    /// The focused combatant's range, for the "Range = N" status line.
    range: i32,
    /// The cell the focus box sits on while aiming (the focused combatant's,
    /// or the free cursor's).
    focus_of_target: GridPos,
    /// Who is under the aim cursor, if anyone.
    cursor_occupant: Option<usize>,
    /// A direction key pressed at the **main** menu opens the movement loop
    /// *and* takes that step (`ovr009.cs:243-252`). The loop's entry snapshot
    /// has to land first, so the step waits here for one command.
    pending_step: Option<u8>,
    /// The original's own transient line ("can't go there", "Not with that
    /// weapon", "Already been targeted"), shown until the next keypress.
    message: Option<String>,
}

impl ManualUi {
    /// Opens the UI for a suspended turn. `pending_cast` is
    /// [`CombatState::pending_spell`]: a queued cast resolves before any menu
    /// is drawn, so the UI opens straight into [`Stage::PendingCast`].
    pub fn open(state: &mut CombatState, actor: usize) -> Self {
        let mut ui = ManualUi {
            actor,
            stage: if state.pending_spell(actor).is_some() {
                Stage::PendingCast
            } else {
                Stage::Main
            },
            words: MenuWords {
                move_: false,
                use_item: false,
                cast: false,
                turn_undead: false,
            },
            done: DoneWords {
                guard: false,
                bandage: false,
            },
            actor_pos: GridPos::new(0, 0),
            move_left: 0,
            game_speed: 4,
            aim: None,
            can_commit: false,
            range: 0,
            focus_of_target: GridPos::new(0, 0),
            cursor_occupant: None,
            pending_step: None,
            message: None,
        };
        ui.refresh(state);
        ui
    }

    /// The step a main-menu direction key owes, once its `BeginMove` has been
    /// issued (`ovr009.cs:243-252` opens the loop *with* the direction).
    pub fn take_pending_step(&mut self) -> Option<TurnCmd> {
        self.pending_step.take().map(TurnCmd::MoveStep)
    }

    /// **Boundary read.** Re-reads every legality condition and the actor's
    /// cell from the fight. The host calls this whenever a command has changed
    /// something — which is every time one is accepted.
    ///
    /// `&mut CombatState` only because §9.2's Bandage word *is* the
    /// `bandage(false)` scan; nothing here writes fight state.
    pub fn refresh(&mut self, state: &mut CombatState) {
        if let Some(done) = state.done_words() {
            self.done = done;
        }
        self.refresh_readonly(state);
    }

    /// The half of [`Self::refresh`] that needs no scan.
    fn refresh_readonly(&mut self, state: &CombatState) {
        self.words = state.menu_words_for(self.actor);
        self.actor_pos = state.roster()[self.actor].pos;
        self.move_left = state.roster()[self.actor].move_left;
        if let Some(aim) = &mut self.aim {
            // The list is rebuilt after every commit (`ovr014.cs:2196`);
            // rebuilding it whenever the board moved is the same thing.
            aim.list = state.aim_list(self.actor);
            if aim.index >= aim.list.len() {
                aim.index = 0;
            }
        }
        self.recompute_aim_readouts(state);
    }

    /// The three Aim readouts — where the box sits, who is under it, whether
    /// `Target` shows, and the "Range = N" number.
    fn recompute_aim_readouts(&mut self, state: &CombatState) {
        let Some(aim) = self.aim.as_ref() else {
            self.can_commit = false;
            self.cursor_occupant = None;
            self.range = 0;
            self.focus_of_target = self.actor_pos;
            return;
        };
        let (cell, occupant) = match aim.cursor {
            Some(cursor) => (cursor, occupant_at(state, cursor)),
            None => match aim.list.get(aim.index) {
                Some(&t) => (state.roster()[t].pos, Some(t)),
                None => (self.actor_pos, None),
            },
        };
        self.focus_of_target = cell;
        self.cursor_occupant = occupant;
        self.can_commit = occupant.is_some_and(|t| state.can_commit_aim(self.actor, t));
        // `getTargetRange` for a combatant, `canReachTarget(…)/2` for a bare
        // cell (`ovr014.cs:1888-1895`) — the same halved steps either way.
        self.range = match occupant {
            Some(t) => state.target_range(t, self.actor) as i32,
            None => crate::combat::get_target_range(&state.map, cell, self.actor_pos) as i32,
        };
        // The aim line is drawn from wherever the box was last.
        if let Some(aim) = self.aim.as_mut() {
            aim.last_cell = cell;
        }
    }

    /// Mirrors `game_speed_var` from the scene's clock (D-CV3 owns it).
    pub fn set_game_speed(&mut self, speed: u8) {
        self.game_speed = speed;
    }

    pub fn actor(&self) -> usize {
        self.actor
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The combatant the aim cursor is on, if any — the one whose summary the
    /// right panel shows while aiming (`ovr014.cs:1796`).
    pub fn aim_target(&self) -> Option<usize> {
        let aim = self.aim.as_ref()?;
        if aim.cursor.is_some() {
            return None;
        }
        aim.list.get(aim.index).copied()
    }

    /// The cell the grey focus box sits on
    /// (`mapToBackGroundTile.drawTargetCursor`): the aim cursor while aiming,
    /// the acting combatant otherwise (`RedrawCombatIfFocusOn(true, 2, actor)`
    /// at the turn head).
    pub fn focus_cell(&self) -> Option<GridPos> {
        match self.stage {
            Stage::Aim | Stage::Cursor => Some(self.focus_of_target),
            _ => Some(self.actor_pos),
        }
    }

    /// The aim cursor's cell — `Some` only while Aim is open.
    ///
    /// The host prefers this over [`Self::focus_cell`] for the box it draws,
    /// because the *actor's* box must ride with the icon on the **presented**
    /// board (D-CV2): a box placed from a boundary read would jump ahead of
    /// the walk it is following.
    pub fn aim_focus_cell(&self) -> Option<GridPos> {
        match self.stage {
            Stage::Aim | Stage::Cursor => Some(self.focus_of_target),
            _ => None,
        }
    }

    /// The cell the free cursor is on (its own, or the focused target's).
    fn cursor_cell(&self) -> GridPos {
        self.focus_of_target
    }

    /// The prompt row's text for the current stage.
    pub fn prompt(&self) -> String {
        if let Some(message) = &self.message {
            return message.clone();
        }
        match self.stage {
            Stage::Main => self.words.text(),
            Stage::Moving => strings::move_left_prompt(self.move_left / 2),
            Stage::Aim => {
                let mut text = String::from(strings::AIM_WORDS_HEAD);
                if self.can_commit {
                    text.push_str(strings::AIM_TARGET_WORD);
                }
                text.push_str(strings::AIM_WORDS_TAIL);
                format!("{} {}", strings::AIM_PROMPT, text)
            }
            Stage::Cursor => {
                let mut text = String::new();
                if self.can_commit {
                    text.push_str(strings::AIM_TARGET_WORD);
                }
                text.push_str(strings::CURSOR_WORDS);
                format!("{}{}", strings::CURSOR_PROMPT, text)
            }
            Stage::Done => self.done.text(),
            Stage::Speed => format!(
                "{}{}",
                strings::game_speed_prompt(self.game_speed),
                strings::game_speed_words(self.game_speed)
            ),
            Stage::FleePrompt => format!("{} {}", strings::FLEE_PROMPT, strings::YES_NO),
            Stage::PendingCast => String::new(),
        }
    }

    /// The status row ("Range = N") — Aim's only, and only while aiming.
    pub fn status(&self) -> Option<String> {
        match self.stage {
            Stage::Aim | Stage::Cursor => Some(strings::range_status(self.range)),
            _ => None,
        }
    }

    /// The focus box, sized for whatever it sits on.
    pub fn focus_cursor(&self, size: u8) -> Option<FocusCursor> {
        self.focus_cell().map(|pos| FocusCursor { pos, size })
    }

    /// Feed one key. The original's menus are modal, so exactly one stage
    /// interprets it.
    pub fn key(&mut self, ev: InputEvent) -> MenuAction {
        // Any keypress clears the transient message the last one printed —
        // `draw8x8_clear_area(0x18, …)` at the top of every loop iteration.
        self.message = None;
        match self.stage {
            Stage::Main => self.key_main(ev),
            Stage::Moving => self.key_moving(ev),
            Stage::Aim => self.key_aim(ev),
            Stage::Cursor => self.key_cursor(ev),
            Stage::Done => self.key_done(ev),
            Stage::Speed => self.key_speed(ev),
            Stage::FleePrompt => self.key_flee(ev),
            // The host drives the aim menu itself here and issues
            // `ResolvePendingCast`; no keys of our own.
            Stage::PendingCast => MenuAction::None,
        }
    }

    /// React to what the core did with the last issued command.
    pub fn note(&mut self, outcome: TurnOutcome) {
        match outcome {
            TurnOutcome::Blocked => self.message = Some(strings::CANT_GO_THERE.to_string()),
            TurnOutcome::FleePrompt => self.stage = Stage::FleePrompt,
            TurnOutcome::Continue | TurnOutcome::TurnEnded | TurnOutcome::Speed(_) => {}
        }
    }

    /// React to a refusal. Two of them are the original's own player-facing
    /// lines rather than driver bugs, and print as such; the rest are bugs the
    /// host reports.
    pub fn note_refusal(&mut self, refusal: &TurnRefusal) {
        match refusal {
            TurnRefusal::NotWithThatWeapon => {
                self.message = Some(strings::NOT_WITH_THAT_WEAPON.to_string());
            }
            TurnRefusal::DuplicateTarget { .. } => {
                self.message = Some(strings::ALREADY_BEEN_TARGETED.to_string());
            }
            _ => {}
        }
    }

    // --- §9.1 -----------------------------------------------------------

    fn key_main(&mut self, ev: InputEvent) -> MenuAction {
        // The extended-key arm (`ovr009.cs:239-296`): a direction key starts
        // the movement loop *with that step* (`sub_33B26(ref var_2, var_1, …)`).
        if let InputEvent::Ext(ext) = ev {
            if let Some(dir) = movement_key_direction(ext.ctrl_code()) {
                if self.words.move_ {
                    self.stage = Stage::Moving;
                    // The loop's entry snapshot first; the host feeds the step
                    // straight after (see `pending_step`).
                    self.pending_step = Some(dir);
                    return MenuAction::Issue(TurnCmd::BeginMove);
                }
            }
            return MenuAction::None;
        }
        let InputEvent::Char(key) = ev else {
            return MenuAction::None;
        };
        match key.to_ascii_uppercase() {
            b'M' if self.words.move_ => {
                self.stage = Stage::Moving;
                MenuAction::Issue(TurnCmd::BeginMove)
            }
            b'V' => MenuAction::OpenSheet,
            b'A' => {
                self.stage = Stage::Aim;
                self.aim = Some(AimState {
                    list: Vec::new(),
                    index: 0,
                    cursor: None,
                    last_cell: self.actor_pos,
                });
                MenuAction::None
            }
            b'U' if self.words.use_item => MenuAction::Issue(TurnCmd::UseItem),
            b'C' if self.words.cast => {
                // The spell menu itself is the host's (it prints from the
                // memorized list and then aims); the UI only opens Aim for it.
                self.stage = Stage::Aim;
                self.aim = Some(AimState {
                    list: Vec::new(),
                    index: 0,
                    cursor: None,
                    last_cell: self.actor_pos,
                });
                MenuAction::None
            }
            b'T' if self.words.turn_undead => MenuAction::Issue(TurnCmd::TurnUndead),
            b'Q' => MenuAction::Issue(TurnCmd::EngageQuickFight),
            b'D' => {
                self.stage = Stage::Done;
                MenuAction::None
            }
            b' ' => MenuAction::Issue(TurnCmd::RevokeQuickFight),
            b'2' => MenuAction::Issue(TurnCmd::ToggleAutoMagic),
            _ => MenuAction::None,
        }
    }

    // --- §9.3 -----------------------------------------------------------

    fn key_moving(&mut self, ev: InputEvent) -> MenuAction {
        match ev {
            // ESC — `case '\0'` (`ovr009.cs:444`).
            InputEvent::Escape => {
                self.stage = Stage::Main;
                MenuAction::Issue(TurnCmd::AbortMove)
            }
            // RETURN (13) — leave the loop where you stand (`:427`).
            InputEvent::Enter => {
                self.stage = Stage::Main;
                MenuAction::Issue(TurnCmd::EndMove)
            }
            InputEvent::Ext(ext) => match movement_key_direction(ext.ctrl_code()) {
                Some(dir) => MenuAction::Issue(TurnCmd::MoveStep(dir)),
                None => MenuAction::None,
            },
            // The letters are the same table (the original's switch reads the
            // one `arg_4` whether or not it came in as a ctrl code).
            InputEvent::Char(key) => match movement_key_direction(key) {
                Some(dir) => MenuAction::Issue(TurnCmd::MoveStep(dir)),
                None => MenuAction::None,
            },
            _ => MenuAction::None,
        }
    }

    fn key_flee(&mut self, ev: InputEvent) -> MenuAction {
        // `yes_no` loops until Y or N — ESC does not exit it (`ovr027.cs:682`).
        let InputEvent::Char(key) = ev else {
            return MenuAction::None;
        };
        match key.to_ascii_uppercase() {
            b'Y' => {
                self.stage = Stage::Moving;
                MenuAction::Issue(TurnCmd::Flee)
            }
            b'N' => {
                self.stage = Stage::Moving;
                MenuAction::None
            }
            _ => MenuAction::None,
        }
    }

    // --- §9.4 -----------------------------------------------------------

    fn key_aim(&mut self, ev: InputEvent) -> MenuAction {
        // An extended key commits at the cursor (`unk_41B05`, `ovr014.cs:2118`
        // — the eight direction codes open the free cursor via `Target`).
        if let InputEvent::Ext(_) = ev {
            self.stage = Stage::Cursor;
            let at = self.focus_cell().unwrap_or(self.actor_pos);
            if let Some(aim) = &mut self.aim {
                aim.cursor = Some(at);
            }
            return MenuAction::None;
        }
        if let InputEvent::Escape = ev {
            self.stage = Stage::Main;
            self.aim = None;
            return MenuAction::None;
        }
        let InputEvent::Char(key) = ev else {
            return MenuAction::None;
        };
        match key.to_ascii_uppercase() {
            b'N' => self.step_list(1),
            b'P' => self.step_list(-1),
            b'M' => {
                self.stage = Stage::Cursor;
                let at = self.focus_cell().unwrap_or(self.actor_pos);
                if let Some(aim) = &mut self.aim {
                    aim.cursor = Some(at);
                }
                MenuAction::None
            }
            b'T' if self.can_commit => match self.aim_target() {
                Some(target) => MenuAction::Issue(TurnCmd::AttackTarget { target }),
                None => MenuAction::None,
            },
            b'C' => {
                let at = self.focus_cell().unwrap_or(self.actor_pos);
                MenuAction::Issue(TurnCmd::Aim(AimCamera::Center { at }))
            }
            b'E' => {
                self.stage = Stage::Main;
                self.aim = None;
                MenuAction::None
            }
            _ => MenuAction::None,
        }
    }

    fn key_cursor(&mut self, ev: InputEvent) -> MenuAction {
        let dir = match ev {
            InputEvent::Ext(ext) => movement_key_direction(ext.ctrl_code()),
            InputEvent::Char(key) => movement_key_direction(key),
            _ => None,
        };
        if let Some(dir) = dir {
            let at = self.cursor_cell();
            let next = clamp_to_map(at.stepped(dir));
            if let Some(aim) = &mut self.aim {
                aim.cursor = Some(next);
            }
            // The scroll probes the cell the cursor is moving to, and it
            // happens *before* the move (`ovr014.cs:1875-1877`).
            return MenuAction::Issue(TurnCmd::Aim(AimCamera::Cursor { at, dir }));
        }
        match ev {
            InputEvent::Escape => {
                self.stage = Stage::Main;
                self.aim = None;
                MenuAction::None
            }
            InputEvent::Enter => self.commit_cursor(),
            InputEvent::Char(key) => match key.to_ascii_uppercase() {
                b'T' => self.commit_cursor(),
                b'C' => {
                    let at = self.cursor_cell();
                    MenuAction::Issue(TurnCmd::Aim(AimCamera::Center { at }))
                }
                b'E' => {
                    self.stage = Stage::Main;
                    self.aim = None;
                    MenuAction::None
                }
                _ => MenuAction::None,
            },
            _ => MenuAction::None,
        }
    }

    /// `Next`/`Prev` (`step_combat_list`, `ovr014.cs:2081-2113`): walk the list
    /// by index with wraparound, and draw the aim line between the old cell and
    /// the new one.
    fn step_list(&mut self, step: i32) -> MenuAction {
        let Some(aim) = &mut self.aim else {
            return MenuAction::None;
        };
        if aim.list.is_empty() {
            return MenuAction::None;
        }
        aim.cursor = None;
        let n = aim.list.len() as i32;
        aim.index = (aim.index as i32 + step).rem_euclid(n) as usize;
        let from = aim.last_cell;
        MenuAction::Issue(TurnCmd::Aim(AimCamera::Line {
            from,
            to: self.focus_of_target,
        }))
    }

    fn commit_cursor(&mut self) -> MenuAction {
        // The free cursor commits only on an occupied, legal cell — empty
        // ground is `canTargetEmptyGround == false` for a weapon
        // (`ovr014.cs:1962`).
        match (self.can_commit, self.cursor_occupant) {
            (true, Some(target)) => MenuAction::Issue(TurnCmd::AttackTarget { target }),
            // "beep and stay" (`ovr014.cs:2020-2026`): the commit is refused
            // and the cursor is right where it was.
            _ => MenuAction::None,
        }
    }

    // --- §9.2 -----------------------------------------------------------

    fn key_done(&mut self, ev: InputEvent) -> MenuAction {
        // `delay_menu`'s loop ends on ESC or `E` (`ovr009.cs:637`).
        if let InputEvent::Escape = ev {
            self.stage = Stage::Main;
            return MenuAction::None;
        }
        let InputEvent::Char(key) = ev else {
            return MenuAction::None;
        };
        match key.to_ascii_uppercase() {
            b'G' if self.done.guard => MenuAction::Issue(TurnCmd::Guard),
            b'D' => MenuAction::Issue(TurnCmd::DelayTurn),
            b'Q' => MenuAction::Issue(TurnCmd::Quit),
            b'B' if self.done.bandage => MenuAction::Issue(TurnCmd::Bandage),
            b'S' => {
                self.stage = Stage::Speed;
                MenuAction::None
            }
            b'E' => {
                self.stage = Stage::Main;
                MenuAction::None
            }
            _ => MenuAction::None,
        }
    }

    fn key_speed(&mut self, ev: InputEvent) -> MenuAction {
        if let InputEvent::Escape = ev {
            self.stage = Stage::Done;
            return MenuAction::None;
        }
        let InputEvent::Char(key) = ev else {
            return MenuAction::None;
        };
        match key.to_ascii_uppercase() {
            // 'S' = Slower = `game_speed_var++` (`ovr009.cs:697`).
            b'S' if self.game_speed < 9 => {
                MenuAction::Issue(TurnCmd::SetSpeed(self.game_speed + 1))
            }
            b'F' if self.game_speed > 0 => {
                MenuAction::Issue(TurnCmd::SetSpeed(self.game_speed - 1))
            }
            b'E' => {
                self.stage = Stage::Done;
                MenuAction::None
            }
            _ => MenuAction::None,
        }
    }
}

/// `Point.MapBoundaryTrunc()` — the free cursor cannot leave the map
/// (`ovr014.cs:1874`).
fn clamp_to_map(p: GridPos) -> GridPos {
    GridPos::new(
        p.x.clamp(0, crate::combat::MAP_W - 1),
        p.y.clamp(0, crate::combat::MAP_H - 1),
    )
}

/// `AtMapXY`'s occupant (`ovr033.cs:191`) as a roster index — the combat map
/// stores `index + 1`, with 0 for an empty cell.
fn occupant_at(state: &CombatState, pos: GridPos) -> Option<usize> {
    if !pos.in_bounds() {
        return None;
    }
    match state.map.occupant(pos) {
        0 => None,
        n => Some(n as usize - 1),
    }
}
