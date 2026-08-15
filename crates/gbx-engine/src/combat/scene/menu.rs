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
    /// ★ `can_attack_target`'s `yes_no("Attack Ally: ")` (`ovr014.cs:1725`) —
    /// opened when the core refuses a commit at a team-mate. `Y` re-issues the
    /// refused command behind a [`TurnCmd::ConfirmAttackAlly`]; `N` goes back
    /// to where the commit came from, which is what `result = false` does.
    AllyPrompt,
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
    /// ★ `U` — open the acting combatant's item list (`PlayerItemsMenu`,
    /// `ovr009.cs:203-211`). Roll-credits slice 9a: the picker is the host's,
    /// because the records live on the roster; it answers with
    /// [`ManualUi::arm_item`].
    OpenItems,
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
    /// ★ The command the core refused with [`TurnRefusal::AllyTarget`], held
    /// while [`Stage::AllyPrompt`] asks — re-issued behind a
    /// [`TurnCmd::ConfirmAttackAlly`] if the player answers `Y`. The original
    /// asks its `yes_no` *inside* `can_attack_target`, so from the outside the
    /// one keypress becomes two commands, exactly as a main-menu direction key
    /// becomes `BeginMove` + `MoveStep`.
    pending_retry: Option<TurnCmd>,
    /// The refused command itself, parked while the question is on screen. It
    /// only becomes a [`Self::pending_retry`] once the player answers `Y` —
    /// arming it earlier would re-issue it (unconfirmed, refused again) before
    /// the prompt was ever read.
    ally_held: Option<TurnCmd>,
    /// Where [`Stage::AllyPrompt`] returns on `N` — the stage the refused
    /// command came from.
    ally_prompt_return: Stage,
    /// The original's own transient line ("can't go there", "Not with that
    /// weapon", "Already been targeted"), shown until the next keypress.
    message: Option<String>,
    /// `gbl.menuSelectedWord` — the selected menu word. GLOBAL in the
    /// original (the last-resolved word stays selected into the next menu,
    /// across turns): the host seeds it at open and persists it after keys.
    selected: usize,
    /// ★ Roll-credits slice 9a: the item the host's picker chose and the
    /// spell it resolved to (`item.affect_2 & 0x7F`, `ovr020.cs:993-997`),
    /// held while the aim menu picks targets. The commit then issues
    /// [`TurnCmd::UseItem`] instead of an attack, and the host reads the item
    /// index back out to spend the charge.
    pending_item: Option<(usize, u8)>,
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
            pending_retry: None,
            ally_held: None,
            ally_prompt_return: Stage::Main,
            message: None,
            selected: 0,
            pending_item: None,
        };
        ui.refresh(state);
        ui
    }

    /// The second command a single keypress owes, if any — the movement loop's
    /// entry step (`ovr009.cs:243-252` opens the loop *with* the direction), or
    /// the commit re-issued behind a confirmed `"Attack Ally: "`
    /// (`ovr014.cs:1725-1746`). The host issues it right behind the first.
    /// ★ The host's item picker answering `U` (`MenuAction::OpenItems`): the
    /// chosen record's index on the roster member and the spell it resolves
    /// to. The UI drops straight into the aim menu, exactly as `Cast` does,
    /// and the commit issues [`TurnCmd::UseItem`].
    ///
    /// A `spell_id` of 0 (`UseMagicItem`'s own "no spell" answer,
    /// `ovr020.cs:999`) just returns to the main menu.
    pub fn arm_item(&mut self, item: usize, spell_id: u8) {
        if spell_id == 0 {
            self.stage = Stage::Main;
            self.pending_item = None;
            return;
        }
        self.pending_item = Some((item, spell_id));
        self.stage = Stage::Aim;
        self.aim = Some(AimState {
            list: Vec::new(),
            index: 0,
            cursor: None,
            last_cell: self.actor_pos,
        });
    }

    /// The item index a committed [`TurnCmd::UseItem`] spent, taken once —
    /// the host's cue to burn the charge (`ovr020.cs:1064-1085`).
    pub fn take_used_item(&mut self) -> Option<usize> {
        self.pending_item.take().map(|(item, _)| item)
    }

    pub fn take_follow_up(&mut self) -> Option<TurnCmd> {
        self.pending_retry
            .take()
            .or_else(|| self.pending_step.take().map(TurnCmd::MoveStep))
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
        if self.aiming() {
            Some(self.focus_of_target)
        } else {
            Some(self.actor_pos)
        }
    }

    /// Is the aim cursor on screen? `Target`'s `"Attack Ally: "` question is
    /// asked from *inside* `sub_411D8`, before it clears `drawTargetCursor`
    /// (`ovr014.cs:1806-1821`), so the box stays on the target while the
    /// player answers.
    fn aiming(&self) -> bool {
        match self.stage {
            Stage::Aim | Stage::Cursor => true,
            Stage::AllyPrompt => {
                matches!(self.ally_prompt_return, Stage::Aim | Stage::Cursor)
            }
            _ => false,
        }
    }

    /// The aim cursor's cell — `Some` only while Aim is open.
    ///
    /// The host prefers this over [`Self::focus_cell`] for the box it draws,
    /// because the *actor's* box must ride with the icon on the **presented**
    /// board (D-CV2): a box placed from a boundary read would jump ahead of
    /// the walk it is following.
    pub fn aim_focus_cell(&self) -> Option<GridPos> {
        self.aiming().then_some(self.focus_of_target)
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
            // `yes_no(defaultMenuColors, "Attack Ally: ")` (`ovr014.cs:1725`)
            // — the prompt already carries its own trailing space.
            Stage::AllyPrompt => format!("{}{}", strings::ATTACK_ALLY_PROMPT, strings::YES_NO),
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
        // `displayInput`'s selection machinery (`ovr027.cs:215-268`): the
        // ','/'.' cycle, and Enter resolving the highlighted word — only in
        // the stages that show MENU WORDS. The loops with their own Enter
        // semantics (Moving ends the walk, Cursor commits) are untouched.
        if matches!(
            self.stage,
            Stage::Main | Stage::Aim | Stage::Done | Stage::Speed
        ) {
            match ev {
                InputEvent::Char(b',') => {
                    self.cycle_selection(-1);
                    return MenuAction::None;
                }
                InputEvent::Char(b'.') => {
                    self.cycle_selection(1);
                    return MenuAction::None;
                }
                InputEvent::Enter => {
                    return match self.selected_hotkey() {
                        Some(key) => self.key(InputEvent::Char(key)),
                        None => MenuAction::None,
                    };
                }
                InputEvent::Char(c) if c.is_ascii_alphanumeric() => {
                    // `select_word_starting_with` before the stage handler
                    // resolves it — the selection lands on what you picked.
                    self.select_word_starting_with(c.to_ascii_uppercase());
                }
                _ => {}
            }
        }
        match self.stage {
            Stage::Main => self.key_main(ev),
            Stage::Moving => self.key_moving(ev),
            Stage::Aim => self.key_aim(ev),
            Stage::Cursor => self.key_cursor(ev),
            Stage::Done => self.key_done(ev),
            Stage::Speed => self.key_speed(ev),
            Stage::FleePrompt => self.key_flee(ev),
            Stage::AllyPrompt => self.key_ally_prompt(ev),
            // The host drives the aim menu itself here and issues
            // `ResolvePendingCast`; no keys of our own.
            Stage::PendingCast => MenuAction::None,
        }
    }

    /// The selected word's span within the CURRENT prompt, for the inverse-
    /// video render — `None` for non-menu stages and transient messages.
    pub fn selected_span(&self) -> Option<(usize, usize)> {
        if !matches!(
            self.stage,
            Stage::Main | Stage::Aim | Stage::Done | Stage::Speed
        ) || self.message.is_some()
        {
            return None;
        }
        let text = self.prompt();
        let words = crate::widgets::build_words(&text);
        if words.is_empty() {
            return None;
        }
        Some(words[self.selected.min(words.len() - 1)])
    }

    /// `gbl.menuSelectedWord` in/out — the host persists it across turns.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn set_selected(&mut self, index: usize) {
        self.selected = index;
    }

    fn cycle_selection(&mut self, dir: i32) {
        let words = crate::widgets::build_words(&self.prompt());
        let n = words.len() as i32;
        if n == 0 {
            return;
        }
        let cur = (self.selected.min(words.len() - 1)) as i32;
        self.selected = ((cur + dir).rem_euclid(n)) as usize;
    }

    /// pub(crate) for the sibling test module.
    pub(crate) fn selected_hotkey(&self) -> Option<u8> {
        let text = self.prompt();
        let words = crate::widgets::build_words(&text);
        if words.is_empty() {
            return None;
        }
        let (start, _) = words[self.selected.min(words.len() - 1)];
        text.as_bytes().get(start).copied()
    }

    fn select_word_starting_with(&mut self, upper: u8) {
        let text = self.prompt();
        if let Some(i) = crate::widgets::build_words(&text)
            .iter()
            .position(|&(s, _)| text.as_bytes().get(s).copied() == Some(upper))
        {
            self.selected = i;
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

    /// React to a refusal of `cmd`. Two of them are the original's own
    /// player-facing lines rather than driver bugs and print as such; one is a
    /// question the original asks mid-commit and this UI must now ask; the rest
    /// are bugs the host reports.
    pub fn note_refusal(&mut self, cmd: &TurnCmd, refusal: &TurnRefusal) {
        match refusal {
            TurnRefusal::NotWithThatWeapon => {
                self.message = Some(strings::NOT_WITH_THAT_WEAPON.to_string());
            }
            TurnRefusal::DuplicateTarget { .. } => {
                self.message = Some(strings::ALREADY_BEEN_TARGETED.to_string());
            }
            // ★ `can_attack_target`'s `yes_no("Attack Ally: ")`
            // (`ovr014.cs:1725`). The core cannot block on a modal, so it
            // refuses an unconfirmed ally target and the menu asks here; `Y`
            // re-issues `cmd` behind a `ConfirmAttackAlly`.
            TurnRefusal::AllyTarget { .. } => {
                self.ally_prompt_return = self.stage;
                self.stage = Stage::AllyPrompt;
                self.ally_held = Some(cmd.clone());
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
            b'U' if self.words.use_item => MenuAction::OpenItems,
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

    /// `yes_no(defaultMenuColors, "Attack Ally: ")` (`ovr014.cs:1725` →
    /// `ovr027.cs:676-689`): loops until `Y` or `N`, ESC included — the
    /// original's `yes_no` has no escape.
    fn key_ally_prompt(&mut self, ev: InputEvent) -> MenuAction {
        let InputEvent::Char(key) = ev else {
            return MenuAction::None;
        };
        match key.to_ascii_uppercase() {
            // `result = true` plus the betrayal flip — the core does both once
            // the consent is armed, and the held command follows immediately
            // (`take_follow_up`).
            b'Y' => {
                self.stage = self.ally_prompt_return;
                self.pending_retry = self.ally_held.take();
                MenuAction::Issue(TurnCmd::ConfirmAttackAlly)
            }
            // `result = false`: nothing happens and the menu it came from
            // re-prompts.
            b'N' => {
                self.stage = self.ally_prompt_return;
                self.ally_held = None;
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
                Some(target) => MenuAction::Issue(self.commit_cmd(target)),
                None => MenuAction::None,
            },
            b'C' => {
                let at = self.focus_cell().unwrap_or(self.actor_pos);
                MenuAction::Issue(TurnCmd::Aim(AimCamera::Center { at }))
            }
            b'E' => {
                self.stage = Stage::Main;
                self.aim = None;
                self.pending_item = None;
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
                self.pending_item = None;
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
            (true, Some(target)) => MenuAction::Issue(self.commit_cmd(target)),
            // "beep and stay" (`ovr014.cs:2020-2026`): the commit is refused
            // and the cursor is right where it was.
            _ => MenuAction::None,
        }
    }

    /// ★ What an aim commit issues: an ordinary attack, or — when the host's
    /// item picker armed one — `UseMagicItem`'s cast
    /// ([`ManualUi::arm_item`], roll-credits slice 9a). One pick is all the
    /// shipped combat items need: the Wand of Fireballs' row is an AREA shape
    /// (`field_6` nibble `8..=0xE`), which `validate_manual_targets` caps at
    /// exactly one.
    fn commit_cmd(&self, target: usize) -> TurnCmd {
        match self.pending_item {
            Some((_, spell_id)) => TurnCmd::UseItem {
                spell_id,
                targets: vec![target],
            },
            None => TurnCmd::AttackTarget { target },
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
