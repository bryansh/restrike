//! ★ **The manual turn** (`docs/design/combat-visualizer.md` §9, D-CV5) — the
//! player's half of a fight, as commands the core executes.
//!
//! When [`CombatState::step`] returns [`CombatStep::AwaitPlayerTurn`], the fight
//! is parked at exactly the point `DoPlayerCombatTurn` hands the turn to
//! `combat_menu` (`ovr009.cs:134`): the turn head has run, the camera has moved,
//! and nothing else will happen until a driver issues [`TurnCmd`]s through
//! [`CombatState::issue`].
//!
//! ## The split rule (§9)
//!
//! > **Presentation decides which words show; the core decides whether a command
//! > executes.**
//!
//! Both sides transcribe the same cited conditions — the scene reads them from
//! [`MenuWords`]/[`DoneWords`], the core re-checks them inside `issue` — and a
//! command that fails the check comes back as a [`TurnRefusal`] rather than
//! quietly doing nothing. A refusal is a *driver* bug (a word that should not
//! have been offered), so it is loud by construction: an `Err` a host cannot
//! ignore.
//!
//! ## What executes
//!
//! Nothing here re-implements a mechanic. Every command lands on the primitive
//! the AI already drives and the captures already pin: `sub_3E748` for a step,
//! `move_step_away_attack` for the departure swing, `attack_target` for a swing,
//! `TryGuarding`, `flee_battle`'s §28 ladder, `spell_menu3` for a cast,
//! `bandage`. That is what makes a manual turn "plain draws" (D-CV5) and what
//! lets one staged capture close the whole path.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/ovr009.cs` `combat_menu` (`:313-360` the word builder, `:148-296`
//!   the dispatcher), `delay_menu` (`:616-669`), `set_gamespeed` (`:672-707`),
//!   `sub_33B26` (`:416-588` the movement loop), `sub_33F03` (`:591-614` the
//!   walk-into-enemy attack), `SetPlayerQuickFight` (`:712-720`).
//! - `engine/ovr014.cs` `aim_menu`/`aim_sub_menu`/`Target`/`sub_411D8`
//!   (`:1752-2220`), `copy_sorted_players` (`:2073`), `step_combat_list`
//!   (`:2081`), `can_attack_target` (`:1717`), `turns_undead` (`:607`).
//! - `engine/ovr033.cs` `sub_7515A` (`:614-668`) — the ESC restore's re-test.
//! - `engine/ovr010.cs` `process_input_in_monsters_turn` (`:703-754`) — SPACE.

use super::{
    build_sorted_combatants, ActionEvent, AttackItemRef, CombatState, GridPos, NearTarget, Phase,
};
use crate::rng::EngineRng;

/// One command from the driver — §9.1's main menu, §9.2's Done submenu, §9.3's
/// movement loop and §9.4's Aim, as a single vocabulary.
///
/// The two words that are pure presentation (`View`'s character sheet and Aim's
/// Next/Prev/Manual/Center cursor motion) still appear here where they touch
/// core state at all: `ViewSheet` runs the original's post-sheet
/// `reclac_attacks`, and the cursor never does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnCmd {
    // --- §9.1, the main menu ------------------------------------------------
    /// `M` → `sub_33B26`'s entry (`ovr009.cs:184`): take the movement loop's
    /// `movesBackup`/`dirBackup`/`pos` snapshot and open the loop.
    BeginMove,
    /// One iteration of the movement loop with a direction key
    /// (G/H/I/K/M/O/P/Q → 7/0/1/6/2/5/4/3, `ovr009.cs:459-489`).
    MoveStep(u8),
    /// RETURN (13) — leave the loop keeping the position (`ovr009.cs:427`'s
    /// `arg_4 != 13` guard), then the `move < 2 ⇒ move = 0` tail (`:585`).
    EndMove,
    /// ESC (`'\0'`, `ovr009.cs:444-456`) — restore the entry moves and facing,
    /// put the combatant back on its entry cell, and let `sub_7515A`'s re-test
    /// decide whether the turn survives.
    AbortMove,
    /// `V` → `viewPlayer` then `reclac_attacks` (`ovr009.cs:197-204`). The M3
    /// character sheet itself is the host's; this is the core half.
    ViewSheet,
    /// `U` → `PlayerItemsMenu` (`ovr009.cs:208-215`). The item-use path is not
    /// modeled: the command trips its stub and refuses (§9.1's Use row).
    UseItem,
    /// `C` → `spell_menu3` (`ovr009.cs:219`). `targets` is what the aim menu
    /// picked, in pick order — the manual arm of `ovr014.target`'s per-target
    /// loop (`ovr014.cs:1322-1358`), which draws nothing.
    CastSpell { spell_id: u8, targets: Vec<usize> },
    /// The queued cast resolving at its caster's **next** turn
    /// (`combat_menu`'s head, `ovr009.cs:155-163`): before any menu is drawn,
    /// `actions.spell_id` fires through `sub_5D2E1(true, QuickFight.False, id)`
    /// and the turn ends. A host that finds [`CombatState::pending_spell`] set
    /// on an opening manual turn runs the aim menu and issues this — there is
    /// no menu to offer.
    ResolvePendingCast { targets: Vec<usize> },
    /// `T` → `turns_undead` (`ovr009.cs:222-226`) — a cited stub until a
    /// capture drives it (§9.1's Turn row).
    TurnUndead,
    /// `Q` → `SetPlayerQuickFight` + `PlayerQuickFight` (`ovr009.cs:175-182`):
    /// the AI finishes this turn and every later one.
    EngageQuickFight,
    /// SPACE inside the menu (`ovr009.cs:229-236`): revoke auto-fight for every
    /// player-controlled combatant. The menu stays open.
    RevokeQuickFight,
    /// `'2'` inside the menu (`ovr009.cs:253-264`) — the auto-magic toggle.
    ToggleAutoMagic,

    // --- §9.4, Aim ----------------------------------------------------------
    /// Aim's `Target` commit (`sub_411D8` with `showRange`, `ovr014.cs:1806`):
    /// swing at the focused combatant with the readied weapon.
    AttackTarget { target: usize },

    // --- §9.2, the Done submenu --------------------------------------------
    /// `G` → `guarding(player)`; ends the turn.
    Guard,
    /// `D` → `actions.delay = 1`; ends the turn.
    DelayTurn,
    /// `Q` → `clear_actions`; ends the turn.
    Quit,
    /// `B` → `bandage(true)` + `clear_actions`; ends the turn.
    Bandage,
    /// `S` → `set_gamespeed` (`ovr009.cs:672-707`). Does **not** end the turn.
    SetSpeed(u8),

    // --- §9.3's off-map fork ------------------------------------------------
    /// The "Flee:" prompt answered `Y` (`ovr009.cs:508-514`) → `flee_battle`'s
    /// §28 ladder. Ends the turn whether or not the escape succeeds.
    Flee,
}

/// What an accepted [`TurnCmd`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    /// The turn continues — the driver re-opens whichever menu it came from.
    Continue,
    /// The turn is over: the core is back in its selection loop and the host
    /// resumes calling [`CombatState::step`].
    TurnEnded,
    /// The step cost more than the moves left — "can't go there"
    /// (`ovr009.cs:533`). The movement loop continues.
    Blocked,
    /// The step left the map (`ground_tile == 0`, `ovr009.cs:507`): the driver
    /// owes the player a "Flee:" yes/no and, on `Y`, a [`TurnCmd::Flee`].
    /// Nothing has moved.
    FleePrompt,
    /// `game_speed_var := n`. The speed lives in the scene's clock (D-CV3 — it
    /// is a presentation quantity, and the core has no clock), so the core
    /// validates the value and hands it back for the host to apply.
    Speed(u8),
}

impl TurnOutcome {
    /// Did this command end the turn?
    pub fn turn_ended(self) -> bool {
        matches!(self, TurnOutcome::TurnEnded)
    }
}

/// Why the core refused a command.
///
/// Every variant means the driver offered something the original's own
/// conditions would not have offered — the §9 rule is that this is loud, not a
/// silent no-op. Nothing here is a *player* error: an illegal keypress is
/// filtered by the menu (the word is absent) or answered with the original's own
/// message ("can't go there", "Not with that weapon"), both of which are
/// [`TurnOutcome`]s, not refusals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnRefusal {
    /// No manual turn is suspended — the fight is not waiting on anyone.
    NoManualTurn,
    /// The word's §9.1/§9.2 condition is false for the acting combatant.
    WordUnavailable { word: &'static str },
    /// A movement command outside an open movement loop.
    NotMoving,
    /// [`TurnCmd::BeginMove`] with a loop already open.
    AlreadyMoving,
    /// The `actions.move > 1` loop gate (`ovr009.cs:425`): a single remaining
    /// half-move cannot step.
    NoMovesLeft,
    /// A direction outside 0..=7.
    BadDirection(u8),
    /// A roster index that is not a live combatant.
    NoSuchTarget { target: usize },
    /// `can_attack_target`'s ally branch (`ovr014.cs:1721-1746`) — the "Attack
    /// Ally: " prompt and its team-flip are not modeled.
    AllyTarget { target: usize },
    /// The aim commit failed its range/reach/weapon legality (§9.4) — the
    /// original never shows `Target` here, so reaching it is a driver bug.
    IllegalTarget { target: usize },
    /// `sub_33F03`'s "Not with that weapon" (`ovr009.cs:594-597`): a pure-ranged
    /// weapon cannot swing at the cell you walked into.
    NotWithThatWeapon,
    /// A cast of something the combatant has not memorized.
    SpellNotMemorized { spell_id: u8 },
    /// More aim picks than `(field_6 & 3) + 1` (`ovr014.cs:1322`).
    TooManyTargets { max: usize },
    /// The same combatant picked twice — the original prints "Already been
    /// targeted" and re-aims instead (`ovr014.cs:1343`).
    DuplicateTarget { target: usize },
    /// A modeled command that reaches an unmodeled engine path. The core has
    /// already emitted the matching [`ActionEvent::StubTripped`].
    Unmodeled { stub: &'static str },
    /// `set_gamespeed` walks `game_speed_var` one step at a time inside 0..=9
    /// (`ovr009.cs:695-706`).
    BadSpeed(u8),
}

/// §9.1's word list (`combat_menu`, `ovr009.cs:313-345`) — which words the menu
/// prints for the acting combatant, in the original's order.
///
/// `View`, `Aim`, `Quick` and `Done` are unconditional and have no field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuWords {
    /// `player.actions.move > 0`.
    pub move_: bool,
    /// `player.items.Count > 0`.
    pub use_item: bool,
    /// `spellList.HasSpells() && actions.can_cast && !area.can_cast_spells` —
    /// the area flag is a per-area cast BAN when set (§4.1.1's inverted name).
    pub cast: bool,
    /// `SkillLevel(Cleric) > 0 && !actions.hasTurnedUndead`.
    pub turn_undead: bool,
}

impl MenuWords {
    /// The menu line the original builds (`ovr009.cs:315-343`), words in its
    /// order and separated by single spaces.
    pub fn text(&self) -> String {
        let mut out = String::new();
        if self.move_ {
            out.push_str("Move ");
        }
        out.push_str("View Aim ");
        if self.use_item {
            out.push_str("Use ");
        }
        if self.cast {
            out.push_str("Cast ");
        }
        if self.turn_undead {
            out.push_str("Turn ");
        }
        out.push_str("Quick Done");
        out
    }
}

/// §9.2's word list (`delay_menu`, `ovr009.cs:619-635`).
///
/// `Delay`, `Quit`, `Speed` and `Exit` are unconditional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoneWords {
    /// `!is_weapon_ranged(player) || is_weapon_ranged_melee(player)` — a
    /// pure-ranged weapon cannot guard (the §34 `TryGuarding` rule, now
    /// player-facing).
    pub guard: bool,
    /// `bandage(false)` — a real team member is dying somewhere.
    pub bandage: bool,
}

impl DoneWords {
    /// The submenu line (`ovr009.cs:619-635`).
    pub fn text(&self) -> String {
        let mut out = String::new();
        if self.guard {
            out.push_str("Guard ");
        }
        out.push_str("Delay Quit ");
        if self.bandage {
            out.push_str("Bandage ");
        }
        out.push_str("Speed Exit");
        out
    }
}

/// The open manual turn — the actor and, while the player is walking, the
/// movement loop's entry backups (`sub_33B26`'s `movesBackup`, `dirBackup` and
/// `pos`, `ovr009.cs:418-420`).
///
/// The backups live in the core because the ESC restore is a core mutation: a
/// driver holding them would be a second source of truth for fight state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ManualTurn {
    actor: usize,
    movement: Option<MovementBackup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct MovementBackup {
    moves: i32,
    direction: u8,
    pos: GridPos,
}

impl ManualTurn {
    pub(super) fn new(actor: usize) -> Self {
        ManualTurn {
            actor,
            movement: None,
        }
    }

    /// Whose turn is open (a roster index).
    pub fn actor(&self) -> usize {
        self.actor
    }

    /// Is the §9.3 movement loop open?
    pub fn is_moving(&self) -> bool {
        self.movement.is_some()
    }
}

/// The movement keys (`sub_33B26`'s switch, `ovr009.cs:459-489`) — the same
/// eight the aim cursor uses (`Target`, `ovr014.cs:2040-2070`).
///
/// Returns the `MapDirectionDelta` index, or `None` for any other key (coab's
/// `default: dir = 8`, i.e. "no direction").
pub fn movement_key_direction(key: u8) -> Option<u8> {
    Some(match key.to_ascii_uppercase() {
        b'H' => 0,
        b'I' => 1,
        b'M' => 2,
        b'Q' => 3,
        b'P' => 4,
        b'O' => 5,
        b'K' => 6,
        b'G' => 7,
        _ => return None,
    })
}

impl CombatState {
    /// The suspended manual turn, if any ([`CombatStep::AwaitPlayerTurn`]).
    pub fn manual_turn(&self) -> Option<&ManualTurn> {
        self.manual.as_ref()
    }

    /// §9.1's words for the open manual turn (`None` if none is open).
    pub fn menu_words(&self) -> Option<MenuWords> {
        let actor = self.manual.as_ref()?.actor;
        Some(self.menu_words_for(actor))
    }

    /// §9.1's words for any combatant — the same conditions `combat_menu`
    /// builds from (`ovr009.cs:315-343`).
    pub fn menu_words_for(&self, actor: usize) -> MenuWords {
        let f = &self.fighters[actor];
        MenuWords {
            move_: f.move_left > 0,
            use_item: f.has_items,
            cast: !f.memorized_list.is_empty() && f.can_cast && !self.area_can_cast_spells,
            turn_undead: f.skill_level_cleric > 0 && !f.has_turned_undead,
        }
    }

    /// §9.2's words for the open manual turn (`None` if none is open).
    ///
    /// Takes `&mut self` because the Bandage word's condition *is* the
    /// `bandage(false)` scan (`ovr025.cs:1628`) — a display-only pass that
    /// bandages nobody, but the one predicate only that function can answer.
    pub fn done_words(&mut self) -> Option<DoneWords> {
        let actor = self.manual.as_ref()?.actor;
        Some(DoneWords {
            guard: !self.is_weapon_ranged(actor) || self.is_weapon_ranged_melee(actor),
            bandage: self.bandage(None, false),
        })
    }

    /// ★ **Execute one turn command** (D-CV5's semantic commands).
    ///
    /// `Ok(_)` means the original would have done exactly this; `Err(_)` means
    /// the driver asked for something the original's own conditions forbid.
    /// Every accepted command runs through the primitives the AI turn uses, so
    /// a manual turn's draws are the same draws a QuickFight turn makes.
    pub fn issue(&mut self, rng: &mut EngineRng, cmd: TurnCmd) -> Result<TurnOutcome, TurnRefusal> {
        let Some(manual) = self.manual.as_ref() else {
            return Err(TurnRefusal::NoManualTurn);
        };
        let actor = manual.actor;
        match cmd {
            TurnCmd::BeginMove => self.begin_move(actor),
            TurnCmd::MoveStep(dir) => self.move_step(rng, actor, dir),
            TurnCmd::EndMove => self.end_move(actor),
            TurnCmd::AbortMove => self.abort_move(actor),
            TurnCmd::ViewSheet => {
                // `viewPlayer()` is the M3 sheet (the host's); its combat-side
                // tail is `reclac_attacks` (`ovr009.cs:199`) — the same
                // recompute a ready/unready inside the sheet would need.
                // `viewPlayer` returning true (a turn spent from inside the
                // sheet) is an M3 surface that cannot yet happen from here.
                self.reclac_attacks(actor);
                Ok(TurnOutcome::Continue)
            }
            TurnCmd::UseItem => {
                self.trip(actor, "item-use");
                Err(TurnRefusal::Unmodeled { stub: "item-use" })
            }
            TurnCmd::CastSpell { spell_id, targets } => {
                self.cast_spell(rng, actor, spell_id, &targets)
            }
            TurnCmd::ResolvePendingCast { targets } => {
                self.resolve_pending_cast(rng, actor, &targets)
            }
            TurnCmd::TurnUndead => {
                if !self.menu_words_for(actor).turn_undead {
                    return Err(TurnRefusal::WordUnavailable { word: "Turn" });
                }
                // `turns_undead` (`ovr014.cs:607`) is draw-bearing (a d12 and a
                // d20 before its `FindLowestE9Target` walk) and nothing has ever
                // driven it: no capture in the guard turns undead, and the
                // engine has no `field_E9` target scan. Guessing it would put
                // unproven draws in the stream of every fight a cleric plays.
                self.trip(actor, "turn-undead");
                Err(TurnRefusal::Unmodeled {
                    stub: "turn-undead",
                })
            }
            TurnCmd::EngageQuickFight => {
                // `SetPlayerQuickFight` (`ovr009.cs:712-720`) then
                // `PlayerQuickFight` (`:181`): the AI takes it from here.
                self.set_player_quick_fight(actor);
                self.melee_ai_turn(rng, actor);
                Ok(self.end_turn())
            }
            TurnCmd::RevokeQuickFight => {
                // `ovr009.cs:229-236` — no `health_status` term at this site,
                // unlike `sub_36269`'s sweep (`ovr010.cs:733`). Transcribed as
                // written, both times.
                for f in &mut self.fighters {
                    if !f.npc {
                        f.quick_fight = false;
                    }
                }
                Ok(TurnOutcome::Continue)
            }
            TurnCmd::ToggleAutoMagic => {
                self.auto_pcs_cast_magic = !self.auto_pcs_cast_magic;
                Ok(TurnOutcome::Continue)
            }
            TurnCmd::AttackTarget { target } => self.aim_commit(rng, actor, target),
            TurnCmd::Guard => {
                if !self.done_words().expect("a turn is open").guard {
                    return Err(TurnRefusal::WordUnavailable { word: "Guard" });
                }
                // `guarding(player)` (`ovr025.cs:1338`) is `TryGuarding`'s body:
                // clear_actions, then set the flag. The word's own condition has
                // already excluded the pure-ranged case `TryGuarding` re-tests.
                self.try_guarding(actor);
                Ok(self.end_turn())
            }
            TurnCmd::DelayTurn => {
                // `actions.delay = 1` (`ovr009.cs:650`) — the combatant stays in
                // the pick pool at the bottom of the order.
                self.fighters[actor].delay = 1;
                Ok(self.end_turn())
            }
            TurnCmd::Quit => {
                self.clear_actions(actor);
                Ok(self.end_turn())
            }
            TurnCmd::Bandage => {
                if !self.done_words().expect("a turn is open").bandage {
                    return Err(TurnRefusal::WordUnavailable { word: "Bandage" });
                }
                self.bandage(Some(actor), true);
                self.clear_actions(actor);
                Ok(self.end_turn())
            }
            TurnCmd::SetSpeed(n) => {
                if n > 9 {
                    return Err(TurnRefusal::BadSpeed(n));
                }
                Ok(TurnOutcome::Speed(n))
            }
            TurnCmd::Flee => {
                // The "Flee:" answer (`ovr009.cs:508-514`): `arg_0 = true` FIRST,
                // then `flee_battle` — the turn ends whether the fleer got away
                // or the §28 ladder blocked the escape.
                self.flee_battle(rng, actor);
                Ok(self.end_turn())
            }
        }
    }

    /// `SetPlayerQuickFight(player)` (`sub_3432F`, `ovr009.cs:712-720`): raise
    /// the flag and drop a held target that is now on the actor's own team.
    fn set_player_quick_fight(&mut self, actor: usize) {
        self.fighters[actor].quick_fight = true;
        if let Some(t) = self.fighters[actor].target {
            if self.fighters[t].team == self.fighters[actor].team {
                self.fighters[actor].target = None;
            }
        }
    }

    /// Close the manual turn: the core returns to its selection loop and the
    /// host goes back to [`CombatState::step`].
    fn end_turn(&mut self) -> TurnOutcome {
        self.manual = None;
        self.phase = Phase::Selecting;
        TurnOutcome::TurnEnded
    }

    fn trip(&mut self, actor: usize, stub: &'static str) {
        let id = self.fighters[actor].id;
        self.emit(ActionEvent::StubTripped {
            combatant_id: id,
            stub,
        });
    }

    // === §9.3 — the movement loop (`sub_33B26`, `ovr009.cs:416-588`) ========

    /// The loop's entry (`ovr009.cs:418-420`): snapshot moves, facing and cell.
    fn begin_move(&mut self, actor: usize) -> Result<TurnOutcome, TurnRefusal> {
        if !self.menu_words_for(actor).move_ {
            return Err(TurnRefusal::WordUnavailable { word: "Move" });
        }
        let f = &self.fighters[actor];
        let backup = MovementBackup {
            moves: f.move_left,
            direction: f.direction,
            pos: f.pos,
        };
        let manual = self.manual.as_mut().expect("checked by `issue`");
        if manual.movement.is_some() {
            return Err(TurnRefusal::AlreadyMoving);
        }
        manual.movement = Some(backup);
        Ok(TurnOutcome::Continue)
    }

    /// One pass of the loop body with a direction key (`ovr009.cs:425-583`).
    fn move_step(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        dir: u8,
    ) -> Result<TurnOutcome, TurnRefusal> {
        if dir > 7 {
            return Err(TurnRefusal::BadDirection(dir));
        }
        let manual = self.manual.as_ref().expect("checked by `issue`");
        if manual.movement.is_none() {
            return Err(TurnRefusal::NotMoving);
        }
        // The loop condition (`ovr009.cs:425`): `actions.move > 1`. One
        // remaining half-move cannot step — transcribe the `> 1` exactly.
        if self.fighters[actor].move_left <= 1 {
            return Err(TurnRefusal::NoMovesLeft);
        }

        // `draw_74B3F(false, Icon.Normal, dir, player)` (`ovr009.cs:495`): the
        // facing write happens BEFORE the fork, so a blocked step, a flee
        // prompt and a swing all turn the icon first.
        self.draw_74b3f(actor, dir);

        let (ground_tile, target_index) = self.ground_info_dir(actor, dir);
        if target_index > 0 {
            // `sub_33F03` (`ovr009.cs:500`) — walk into an occupied cell.
            return self.walk_into(rng, actor, (target_index - 1) as usize);
        }
        if ground_tile == 0 {
            // Off the map (`ovr009.cs:503-514`): the driver owes a "Flee:".
            return Ok(TurnOutcome::FleePrompt);
        }

        // The affordability pre-check (`ovr009.cs:518-531`).
        //
        // ★ Cited asymmetry, flagged for the audit: this site multiplies by 3
        // when `(dir / 2) < 1` — i.e. for directions 0 and 1 — where
        // `sub_3E748`'s own deduction (`ovr014.cs:266`, and our
        // [`CombatState::sub_3e748`]) uses the diagonal test `dir & 1`. coab
        // writes the two idioms differently in the two functions, which is why
        // this is transcribed rather than unified: the pre-check can pass a
        // diagonal that then costs more, and `sub_3E748` clamps the remainder
        // to zero when it does. Only a manual-turn capture can settle whether
        // the binary really reads `cmp dir,2`.
        let base = super::ground_tile_move_cost(ground_tile) as i32;
        let cost = if dir < 2 { base * 3 } else { base * 2 };
        let cost = if base == 0xFF { 0xFFFF } else { cost };
        if cost > self.fighters[actor].move_left {
            return Ok(TurnOutcome::Blocked); // "can't go there"
        }

        // The step itself, through the proven primitives (`ovr009.cs:537-566`).
        self.move_step_away_attack(rng, actor, dir);
        if !self.fighters[actor].in_combat {
            self.clear_actions(actor);
            return Ok(self.end_turn());
        }
        if self.fighters[actor].move_left > 0 {
            self.sub_3e748(rng, actor, self.fighters[actor].direction);
        }
        if !self.fighters[actor].in_combat {
            self.clear_actions(actor);
            return Ok(self.end_turn());
        }
        // `in_poison_cloud(1, player)` (`ovr009.cs:561`) — no cloud subsystem;
        // draw-free and inert exactly as everywhere else in the engine.
        if self.is_held(actor) {
            self.clear_actions(actor);
            return Ok(self.end_turn());
        }
        Ok(TurnOutcome::Continue)
    }

    /// `sub_33F03` (`ovr009.cs:591-614`): the cell you walked into is occupied.
    fn walk_into(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        target: usize,
    ) -> Result<TurnOutcome, TurnRefusal> {
        if self.is_weapon_ranged(actor) && !self.is_weapon_ranged_melee(actor) {
            // "Not with that weapon" — the loop continues in the original; the
            // core reports it so the driver can print the line.
            return Err(TurnRefusal::NotWithThatWeapon);
        }
        self.can_attack_target(actor, target)?;
        self.fighters[actor].target = Some(target);
        if self.try_sweep_attack(target, actor) {
            return Ok(self.end_turn());
        }
        self.recalc_attacks_received(target, actor);
        // `AttackTarget(null, 0, target, player)` — a null weapon: the swing is
        // melee whatever is readied (the pure-ranged case was refused above).
        let ended = self.attack_target(rng, actor, target, false, AttackItemRef::None);
        Ok(if ended {
            self.end_turn()
        } else {
            TurnOutcome::Continue
        })
    }

    /// RETURN (`ovr009.cs:427`) — or the loop running out of moves. Closes the
    /// session and runs the tail (`:585-587`).
    fn end_move(&mut self, actor: usize) -> Result<TurnOutcome, TurnRefusal> {
        let manual = self.manual.as_mut().expect("checked by `issue`");
        if manual.movement.take().is_none() {
            return Err(TurnRefusal::NotMoving);
        }
        // `if (player.actions.move < 2) player.actions.move = 0;` — a leftover
        // half-move is spent, which is what takes the Move word off the menu.
        if self.fighters[actor].move_left < 2 {
            self.fighters[actor].move_left = 0;
        }
        Ok(TurnOutcome::Continue)
    }

    /// ESC (`ovr009.cs:444-456`): moves and facing restored, the combatant put
    /// back on its entry cell, and `sub_7515A`'s re-test deciding the turn.
    fn abort_move(&mut self, actor: usize) -> Result<TurnOutcome, TurnRefusal> {
        let manual = self.manual.as_mut().expect("checked by `issue`");
        let Some(backup) = manual.movement.take() else {
            return Err(TurnRefusal::NotMoving);
        };
        self.fighters[actor].move_left = backup.moves;
        // `sub_7515A(false, pos, player)` (`ovr033.cs:614-640`) writes the map
        // cell back and re-tests it: occupied, off-map or impassable and the
        // combatant's footprint is dropped (`CombatMap[i].size = 0`) and the
        // turn ends. Our occupancy is derived from `in_combat`, so a footprint
        // with no combatant behind it is not representable — the case needs
        // another mover to have taken the cell mid-turn, which cannot happen
        // inside one player's movement loop. If it ever does, it says so.
        let restored = backup.pos;
        let blocked = {
            let occupant = self.map.occupant(restored);
            let taken = occupant != 0 && occupant != (actor + 1) as u16;
            taken || !super::map_in_bounds(restored) || self.map.move_cost(restored) == 0xFF
        };
        self.fighters[actor].pos = restored;
        self.rebuild_occupancy();
        // D-CV2: the presented board rewinds from this event, never from a
        // state read (§9.3 — "the scene replays it from the abort event").
        self.emit(ActionEvent::MoveAborted {
            combatant_id: actor,
            to_x: restored.x,
            to_y: restored.y,
        });
        self.fighters[actor].direction = backup.direction;
        if blocked {
            self.trip(actor, "esc-restore-blocked");
            return Ok(self.end_turn());
        }
        if self.fighters[actor].move_left < 2 {
            self.fighters[actor].move_left = 0;
        }
        Ok(TurnOutcome::Continue)
    }

    // === §9.4 — Aim ========================================================

    /// Aim's cycling list (`copy_sorted_players`, `ovr014.cs:2073`):
    /// `Rebuild_SortedCombatantList(attacker, 0x7F, p => true)` — **every**
    /// reachable combatant, both teams and the attacker itself, in the
    /// `sub_73033` exchange-sort order (steps, then direction).
    ///
    /// `Next`/`Prev` walk this list by index with wraparound
    /// (`step_combat_list`, `ovr014.cs:2081-2113`), which is why the order is
    /// the list's and not a re-sort per keypress.
    pub fn aim_list(&self, actor: usize) -> Vec<usize> {
        build_sorted_combatants(&self.map, &self.range_combatants(), actor, 0x7F, false)
            .into_iter()
            .map(|t: NearTarget| t.idx)
            .collect()
    }

    /// The aim cursor's maximum range (`aim_menu`, `ovr014.cs:2131-2147`): the
    /// readied weapon's `ITEMS` range minus one, with bare hands (and every
    /// degenerate value) clamped to 1 — which is what makes a melee weapon's
    /// legality plain adjacency.
    ///
    /// This is the same computation the AI's attack range makes
    /// (`ovr010.cs:562-572`), down to the sanitize, so it *is*
    /// [`CombatState::weapon_range`]. Aim's third `maxRange == 0xff` arm is
    /// unreachable in `i32` space for the same reason that one records
    /// (review finding #4: the binary sanitizes bytes).
    pub fn aim_max_range(&self, actor: usize) -> i32 {
        self.weapon_range(actor)
    }

    /// Whether Aim would offer `Target` for this pairing — `aim_sub_menu`'s
    /// word condition (`ovr014.cs:1765-1791`) and `Target`'s own `can_target`
    /// gates (`ovr014.cs:1930-1960`), which agree on every term that matters
    /// here.
    pub fn can_commit_aim(&self, actor: usize, target: usize) -> bool {
        if target == actor || target >= self.fighters.len() {
            return false;
        }
        if !self.can_see_target(target) {
            return false;
        }
        if self.target_range(target, actor) as i32 > self.aim_max_range(actor) {
            return false;
        }
        if !self.is_weapon_ranged(actor) {
            return true;
        }
        // A ranged attacker needs ammo, and needs to be un-cornered unless its
        // weapon doubles as a melee one (`ovr014.cs:1776-1786`).
        self.get_current_attack_item(actor).found
            && (self.build_near(actor, 1, false).is_empty() || self.is_weapon_ranged_melee(actor))
    }

    /// Aim's `Target` commit (`sub_411D8`, `ovr014.cs:1806-1857`).
    fn aim_commit(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        target: usize,
    ) -> Result<TurnOutcome, TurnRefusal> {
        if target >= self.fighters.len() || !self.fighters[target].in_combat {
            return Err(TurnRefusal::NoSuchTarget { target });
        }
        if !self.can_commit_aim(actor, target) {
            return Err(TurnRefusal::IllegalTarget { target });
        }
        self.can_attack_target(actor, target)?;
        if self.try_sweep_attack(target, actor) {
            self.clear_actions(actor);
            return Ok(self.end_turn());
        }
        self.recalc_attacks_received(target, actor);
        // `ovr014.cs:1832-1845`: the shot's item, with the ranged-melee weapon
        // at range 0 falling back to a null (melee) swing. The `== 0` is the
        // original's own constant — `getTargetRange` is halved steps, so an
        // adjacent target reads 1 and this fallback is unreachable for it.
        let ranged_item = if self.is_weapon_ranged(actor) {
            let item = self.get_current_attack_item(actor);
            if item.found
                && self.is_weapon_ranged_melee(actor)
                && self.target_range(target, actor) == 0
            {
                AttackItemRef::None
            } else {
                item.item
            }
        } else {
            AttackItemRef::None
        };
        let ended = self.attack_target(rng, actor, target, false, ranged_item);
        Ok(if ended {
            self.end_turn()
        } else {
            TurnOutcome::Continue
        })
    }

    /// `can_attack_target` (`sub_40F1F`, `ovr014.cs:1717-1749`): an
    /// opposite-team target is always attackable; an ALLY opens the "Attack
    /// Ally: " prompt, whose `Y` flips every conscious NPC to the enemy team —
    /// a whole betrayal mechanic with no capture behind it.
    fn can_attack_target(&mut self, actor: usize, target: usize) -> Result<(), TurnRefusal> {
        if self.fighters[target].team != self.fighters[actor].team {
            return Ok(());
        }
        self.trip(actor, "attack-ally");
        Err(TurnRefusal::AllyTarget { target })
    }

    // === §9.1's Cast =======================================================

    /// `spell_menu3(out casting_spell, 0, 0)` (`ovr009.cs:219`) with the spell
    /// and its targets already picked by the menu and the aim cursor.
    ///
    /// The manual arm of the targeting loop (`ovr014.cs:1322-1358`) draws
    /// **nothing** — each target came from an aim commit, not from
    /// `find_target`'s d(count) — which is the whole difference between a
    /// player's cast and the AI's.
    fn cast_spell(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
    ) -> Result<TurnOutcome, TurnRefusal> {
        if !self.menu_words_for(actor).cast {
            return Err(TurnRefusal::WordUnavailable { word: "Cast" });
        }
        if !self.fighters[actor].memorized_list.contains(&spell_id) {
            return Err(TurnRefusal::SpellNotMemorized { spell_id });
        }
        self.validate_manual_targets(actor, spell_id, targets)?;
        if targets.is_empty() {
            // `gbl.spellTargets.Count == 0 ⇒ castSpell = false` (`ovr014.cs:1360`):
            // an aborted aim is not a cast, and the turn is not spent.
            return Ok(TurnOutcome::Continue);
        }
        let ended = self.spell_menu3_manual(rng, actor, spell_id, targets);
        Ok(if ended {
            self.end_turn()
        } else {
            TurnOutcome::Continue
        })
    }

    /// `combat_menu`'s pending-cast head (`ovr009.cs:155-163`): the spell queued
    /// by a previous "Begins Casting" resolves, `clear_actions` runs, and the
    /// turn is over — the menu never opens.
    ///
    /// The AI's twin is `melee_ai_turn`'s step 4 (`ovr010.cs:60-66`); the only
    /// difference is this one's `QuickFight.False`, which is why the targets
    /// come in from the aim menu instead of `find_target`'s dice.
    fn resolve_pending_cast(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        targets: &[usize],
    ) -> Result<TurnOutcome, TurnRefusal> {
        let Some(spell_id) = self.fighters[actor].pending_spell else {
            return Err(TurnRefusal::WordUnavailable {
                word: "pending cast",
            });
        };
        self.validate_manual_targets(actor, spell_id, targets)?;
        self.fighters[actor].pending_spell = None;
        self.sub_5d2e1_manual(rng, actor, spell_id, targets);
        self.clear_actions(actor);
        Ok(self.end_turn())
    }

    /// The shared half of [`Self::cast_spell`] and [`Self::resolve_pending_cast`]:
    /// the aim picks must satisfy the same rules the manual targeting loop
    /// enforces (`ovr014.cs:1322-1358`) — a transcribed spell, a shape the
    /// per-target loop actually handles, at most `(field_6 & 3) + 1` picks, all
    /// live and all distinct.
    fn validate_manual_targets(
        &mut self,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
    ) -> Result<(), TurnRefusal> {
        let Some(entry) = super::spells::spell_entry(spell_id) else {
            // The lazy-transcription rule (doc §41.2): an untranscribed row is
            // a stub, not a guess.
            self.trip(actor, "spell-entry");
            return Err(TurnRefusal::Unmodeled {
                stub: "spell-entry",
            });
        };
        let nibble = entry.field_6 & 0x0F;
        if nibble == 0 || nibble == 5 || (8..=0x0F).contains(&nibble) {
            // Self / budgeted-multi / area shapes: the manual arm needs the
            // same shape machinery the AI arm stubs (doc §41.3).
            self.trip(actor, "spell-target-shape");
            return Err(TurnRefusal::Unmodeled {
                stub: "spell-target-shape",
            });
        }
        let max = (entry.field_6 & 3) as usize + 1;
        if targets.len() > max {
            return Err(TurnRefusal::TooManyTargets { max });
        }
        for (i, &t) in targets.iter().enumerate() {
            if t >= self.fighters.len() || !self.fighters[t].in_combat {
                return Err(TurnRefusal::NoSuchTarget { target: t });
            }
            if targets[..i].contains(&t) {
                return Err(TurnRefusal::DuplicateTarget { target: t });
            }
        }
        Ok(())
    }
}
