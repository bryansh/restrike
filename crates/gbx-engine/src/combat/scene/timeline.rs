//! **The playback timeline** (`docs/design/combat-visualizer.md` §1.4–1.5,
//! D-CV2/D-CV3): a step's buffered [`ActionEvent`] batch, scheduled over ticks.
//!
//! Two halves, deliberately separated so each is testable on its own:
//!
//! - [`compose`] turns a batch into a flat list of [`Instruction`]s — a
//!   presentation mutation plus the number of ticks the result is *held*.
//!   This is where every §1.4 beat and every §1.5 message lives.
//! - [`Timeline`] plays that list back: it applies zero-hold instructions
//!   until it reaches one that holds, then counts ticks. It never sleeps and
//!   never reads a clock (D9).
//!
//! **Determinism is per-tick.** `game_speed_var` changes hold *lengths*, never
//! frame content; the fast-drain ([`Timeline::skip`]) collapses the remaining
//! holds to zero and applies every remaining instruction, so a skipped step
//! and a played-out step end on identical presentation state — D-CV3's open
//! skip door, and the lockstep invariant is satisfied by it (playback still
//! completes, just instantly).
//!
//! **The composer reads no `CombatState`.** It walks a *shadow* copy of the
//! presented board forward as it composes, so a beat that needs post-event
//! state (which message `DescribeHealing` picks, where a missile's endpoints
//! are) reads the shadow rather than the live roster — D-CV2's mid-playback
//! rule, kept structurally.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/ovr014.cs:904-1008` (`AttackTarget`) — the pose block: the
//!   target's redraw, the attacker's summary, the attacker's Attack frame,
//!   `SysDelay(100)`, then the missile, then the swings, then the restore.
//! - `engine/ovr014.cs:113-223` (`DisplayAttackMessage`) — the whole §1.5
//!   attack sequence, including which prints precede which `GameDelay`.
//! - `engine/ovr033.cs:534-611` (`CombatantKilled`) — background erase, death
//!   sound, nine 10 ms alternations, body tile, `GameDelay`, removal.
//! - `engine/ovr024.cs:618-647` (`RemoveFromCombat`) — the flee/surrender
//!   shape: one panel message with its own `GameDelay`, no flash.
//! - `engine/ovr025.cs:775-784` (`string_print01`) — the prompt-row beat.
//! - `engine/ovr025.cs:1118-1172` (`MagicAttackDisplay`) — the on-target
//!   burst.
//! - `engine/ovr023.cs:741-762` (`sub_5D2E1`) — the cast's projectile.
//! - `engine/ovr009.cs:118-124` — the turn head's focus box and summary.

use super::missile::{self, FlightStep, MissileClass, SpriteRef};
use super::render::{OverlayDraw, PanelOp, Row};
use super::strings;
use super::time::{self, BeatClock};
use super::{FocusCursor, PresentedBoard};
use crate::combat::{
    size_footprint, sound, target_direction, ActionEvent, AttackKind, GridPos, HealKind,
    HealthStatus, RemovalReason,
};
use crate::combat_art::IconPose;

/// The two COMSPR icon slots `CombatantKilled` alternates between
/// (`ovr033.cs:557-558`): slot 24's Attack frame and slot 25's Normal frame —
/// the latter being the same grey box the focus cursor draws from.
const DEATH_FLASH_A: SpriteRef = SpriteRef::new(24, IconPose::Attack, false);
const DEATH_FLASH_B: SpriteRef = SpriteRef::new(25, IconPose::Normal, false);

/// The panel's first message row (`DisplayPlayerStatusString(_, 10, …)`).
const MESSAGE_ROW: usize = 10;
/// The row `DisplayAttackMessage` prints the target's name on
/// (`ovr014.cs:130`).
const TARGET_NAME_ROW: usize = 12;
/// The row its damage/miss line wraps from (`line + 1`, `ovr014.cs:133`).
const DAMAGE_TEXT_ROW: usize = 13;

/// One presentation mutation. Applied atomically at the head of its
/// [`Instruction`]; what it leaves on screen is then held.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// The window jumped (`ScreenMapCheck`) — a single full repaint, never a
    /// scroll animation (§1.2).
    Camera(GridPos),
    /// Advance the presented board by an event (position, hp, status,
    /// removal, body tile).
    Board(ActionEvent),
    /// `draw_74B3F(_, iconState, direction, combatant)` — pose and facing.
    Pose {
        id: usize,
        pose: IconPose,
        direction: u8,
    },
    /// `RedrawPlayerBackground(idx)` (`ovr033.cs:556`): the combatant's icon
    /// stops drawing while its cell shows bare ground — what the death flash
    /// plays over. `None` puts every icon back.
    Hidden(Option<usize>),
    /// `CombatDisplayPlayerSummary(player)` — the right panel. Clears the
    /// whole panel region, messages included (`ovr025.cs:294`).
    Panel(usize),
    /// `RedrawCombatIfFocusOn(draw, _, player)` — the grey focus box.
    Focus(Option<FocusCursor>),
    /// The overlay sprites currently up (missile, burst, death flash).
    Overlay(Vec<OverlayDraw>),
    /// One print into the panel's message region.
    Message(PanelOp),
    /// The prompt row (0x18); `None` is `ClearPromptAreaNoUpdate`.
    Prompt(Option<String>),
    /// The status row (0x17); `None` is its own clear.
    Status(Option<String>),
    /// A cue for this tick's `Frame.sounds` (D-UI1).
    Sound(u8),
    /// Nothing changes — the frame already on screen is simply held. This is
    /// what a bare `GameDelay()` is: the prints that precede it are the beat,
    /// the delay is how long they stay up.
    Wait,
}

/// An [`Op`] plus how long its result is held, in whole ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct Instruction {
    pub op: Op,
    pub hold: u32,
}

impl Instruction {
    fn now(op: Op) -> Self {
        Instruction { op, hold: 0 }
    }
    fn held(op: Op, hold: u32) -> Self {
        Instruction { op, hold }
    }
}

/// The playback cursor over one step's composed instructions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timeline {
    instructions: Vec<Instruction>,
    cursor: usize,
    /// Ticks still owed on the instruction at `cursor - 1`.
    remaining: u32,
}

impl Timeline {
    /// Loads a step's schedule, replacing whatever was left of the last one.
    pub fn load(&mut self, instructions: Vec<Instruction>) {
        self.instructions = instructions;
        self.cursor = 0;
        self.remaining = 0;
    }

    /// Whether any instruction is still unapplied or unheld.
    pub fn is_playing(&self) -> bool {
        self.remaining > 0 || self.cursor < self.instructions.len()
    }

    /// How many `tick(1)` calls it takes to finish this schedule from where
    /// it stands.
    ///
    /// A freshly loaded schedule costs one extra tick over the sum of its
    /// holds: the first tick *applies* the opening batch (nothing is on screen
    /// to hold yet) and only then starts counting the first hold down.
    pub fn ticks_remaining(&self) -> u32 {
        let holds: u32 = self.instructions[self.cursor..]
            .iter()
            .map(|i| i.hold)
            .sum();
        if self.remaining == 0 {
            if self.cursor < self.instructions.len() {
                1 + holds
            } else {
                0
            }
        } else {
            self.remaining + holds
        }
    }

    /// The instructions still to apply — what a [`skip`](Self::skip) drains.
    pub(super) fn drain_pending(&mut self) -> std::vec::Drain<'_, Instruction> {
        self.remaining = 0;
        let from = self.cursor;
        self.cursor = self.instructions.len();
        self.instructions.drain(from..)
    }

    /// Advances by `dt` ticks, calling `apply` for every instruction whose
    /// head is reached.
    ///
    /// One tick is: spend a tick of the current hold, and if that empties it,
    /// apply instructions until the next one that holds. An instruction
    /// applied on tick *T* with hold *h* is therefore on screen for frames
    /// *T*..*T+h-1* — exactly *h* frames, which is what "held for `h` ticks"
    /// has to mean for `ms_to_ticks`'s floor to keep a speed-0 message
    /// visible.
    pub(super) fn advance(&mut self, dt: u32, mut apply: impl FnMut(&Op)) {
        for _ in 0..dt {
            if self.remaining > 0 {
                self.remaining -= 1;
            }
            while self.remaining == 0 {
                let Some(instruction) = self.instructions.get(self.cursor) else {
                    break;
                };
                self.cursor += 1;
                apply(&instruction.op);
                self.remaining = instruction.hold;
            }
        }
    }
}

/// The shadow board the composer walks forward, plus the running state a beat
/// needs from the beats before it.
struct Composer<'a> {
    board: PresentedBoard,
    clock: BeatClock,
    out: Vec<Instruction>,
    /// The open swing run — `Attacking` opens it, the next structural event
    /// closes it (`ovr014.cs:729-873`'s `AttackTarget01` body).
    run: Option<AttackRun>,
    /// Where a missile flight left the presented window, so the `Camera` the
    /// engine emits right after can be checked against it.
    flight_camera: Option<GridPos>,
    /// The cast being resolved, waiting for its `SpellTarget` run to finish
    /// before the projectile can be aimed.
    cast: Option<PendingCast>,
    events: &'a [ActionEvent],
    at: usize,
}

#[derive(Debug, Clone, Copy)]
struct AttackRun {
    attacker: usize,
    target: usize,
    kind: AttackKind,
    /// `AttackType.Slay` — `SlayHelpless` replaces the run's message fork.
    slay: bool,
}

#[derive(Debug, Clone, Copy)]
struct PendingCast {
    caster: usize,
    spell_id: u8,
    last_target: Option<usize>,
}

/// Turns one step's event batch into its tick schedule.
///
/// `board` is the presented board **as playback will find it** — the composer
/// clones it and walks the clone forward, so the caller's board is untouched.
pub fn compose(
    board: &PresentedBoard,
    clock: BeatClock,
    events: &[ActionEvent],
) -> Vec<Instruction> {
    let mut c = Composer {
        board: board.clone(),
        clock,
        out: Vec::new(),
        run: None,
        flight_camera: None,
        cast: None,
        events,
        at: 0,
    };
    while c.at < events.len() {
        let event = events[c.at];
        c.at += 1;
        c.event(event);
    }
    c.finish_cast();
    c.out
}

impl Composer<'_> {
    fn push(&mut self, op: Op) {
        self.out.push(Instruction::now(op));
    }

    fn hold(&mut self, op: Op, ticks: u32) {
        self.out.push(Instruction::held(op, ticks));
    }

    /// Applies an event to the shadow board **and** schedules it for playback.
    fn board_op(&mut self, event: ActionEvent) {
        self.board.apply(event);
        self.push(Op::Board(event));
    }

    fn pos(&self, id: usize) -> GridPos {
        self.board
            .combatant(id)
            .map(|c| c.pos)
            .unwrap_or(GridPos::new(0, 0))
    }

    fn event(&mut self, event: ActionEvent) {
        match event {
            // --- structural, untimed -------------------------------------
            ActionEvent::Camera { top_left } => {
                if let Some(flight) = self.flight_camera.take() {
                    // The flight recomputed this window presentation-locally
                    // (§1.4); the engine's own port is the authority. They are
                    // the same transcription, so a disagreement is a bug in one
                    // of them, not a case to paper over.
                    debug_assert_eq!(
                        flight, top_left,
                        "the missile flight's window disagrees with the engine's camera port"
                    );
                }
                self.board.apply(event);
                self.push(Op::Camera(top_left));
            }
            ActionEvent::Move { .. } => {
                // §1.4: a movement step has no timer at all — a redraw (the
                // radius the engine already applied) and a step sound, which
                // arrives as its own `Sound`.
                self.board_op(event);
            }
            ActionEvent::Bled { .. } => {
                // The round-end bleed tick displays nothing (`ovr009.cs:373-381`
                // mutates in silence); the presented board still has to follow it.
                self.board_op(event);
            }
            ActionEvent::Sound { id } => {
                if id == sound::MISS {
                    // `var_11 == false` (`ovr014.cs:859`): the whole attack
                    // missed. One message for the run, then the sound.
                    self.miss_message();
                }
                self.push(Op::Sound(id));
            }

            // --- the turn head (`ovr009.cs:118-124`) ----------------------
            ActionEvent::Pick { combatant_id, .. } => {
                self.close_run();
                let cursor = self.board.combatant(combatant_id).map(|c| FocusCursor {
                    pos: c.pos,
                    size: c.size,
                });
                self.push(Op::Focus(cursor));
                self.push(Op::Panel(combatant_id));
            }
            ActionEvent::Ai {
                combatant_id,
                target_id,
                ..
            } => {
                self.close_run();
                if target_id < 0 {
                    // `guarding(player)` (`ovr025.cs:1336-1339`) — the turn ended
                    // with nobody to reach. The event is the turn's summary, so
                    // this beat plays at the turn's tail rather than at the
                    // `try_guarding` call inside it; nothing draws in between.
                    let _ = combatant_id;
                    self.prompt_beat(strings::GUARDING);
                }
            }

            // --- the swing sequence --------------------------------------
            ActionEvent::Attacking {
                attacker_id,
                target_id,
                kind,
            } => {
                self.close_run();
                self.run = Some(AttackRun {
                    attacker: attacker_id,
                    target: target_id,
                    kind,
                    slay: false,
                });
                self.attack_pose(attacker_id, target_id);
            }
            ActionEvent::Attack { .. } | ActionEvent::Save { .. } | ActionEvent::Init { .. } => {}
            ActionEvent::Dmg { amount, .. } => {
                // `damage_player(actualDamage, target)` runs INSIDE
                // `DisplayAttackMessage` (`ovr014.cs:168`), after the text is
                // built and before it prints — so the hit points move first
                // and the message reports the roll either way.
                self.board_op(event);
                self.hit_message(amount);
            }
            ActionEvent::SlayHelpless { .. } => {
                if let Some(run) = self.run.as_mut() {
                    run.slay = true;
                }
                self.slay_message();
            }
            ActionEvent::Missile {
                attacker_id,
                target_id,
                weapon_type,
            } => {
                let dir = target_direction(self.pos(target_id), self.pos(attacker_id));
                let class = missile::missile_class(weapon_type, dir);
                self.push(Op::Sound(missile::LAUNCH_SOUND));
                self.push(Op::Sound(class.class_sound));
                self.flight(attacker_id, target_id, &class);
            }

            // --- casting --------------------------------------------------
            ActionEvent::BeginsCasting { caster_id } => {
                self.close_run();
                self.status_beat(caster_id, strings::BEGINS_CASTING.to_string());
            }
            ActionEvent::Cast {
                caster_id,
                spell_id,
            } => {
                self.close_run();
                // `DisplayCaseSpellText` (`ovr023.cs:3117-3122`): the panel
                // message with its own `GameDelay`, then the status row names
                // the spell — and that line stays up for the cast's duration.
                self.status_beat(caster_id, strings::CASTS_A_SPELL.to_string());
                self.push(Op::Status(Some(format!(
                    "{}{}",
                    strings::SPELL_PREFIX,
                    strings::spell_name(spell_id)
                ))));
                self.cast = Some(PendingCast {
                    caster: caster_id,
                    spell_id,
                    last_target: None,
                });
            }
            ActionEvent::SpellTarget { target_id } => {
                if let Some(cast) = self.cast.as_mut() {
                    cast.last_target = Some(target_id);
                }
            }
            ActionEvent::Healed {
                target_id, kind, ..
            } => {
                self.finish_cast();
                self.board_op(event);
                let text = match kind {
                    // `bandage(true)` (`ovr025.cs:1645`).
                    HealKind::Bandage => strings::IS_BANDAGED.to_string(),
                    // `DescribeHealing` (`ovr025.cs:1250-1257`) — chosen from
                    // the hit points AFTER the heal, which the shadow board
                    // above has already applied.
                    HealKind::Cure => {
                        let full = self
                            .board
                            .combatant(target_id)
                            .map(|c| c.hp_current >= c.hp_max)
                            .unwrap_or(false);
                        if full {
                            strings::IS_FULLY_HEALED.to_string()
                        } else {
                            strings::IS_PARTIALLY_HEALED.to_string()
                        }
                    }
                };
                self.status_beat(target_id, text);
            }

            // --- leaving the board ----------------------------------------
            ActionEvent::Removed {
                combatant_id,
                reason,
            } => self.removal(combatant_id, reason),
            ActionEvent::Flees {
                combatant_id,
                forced,
            } => {
                self.close_run();
                let text = if forced {
                    strings::IS_FORCED_TO_FLEE
                } else {
                    strings::FLEES_IN_PANIC
                };
                self.status_beat(combatant_id, text.to_string());
            }

            // --- round boundaries ----------------------------------------
            ActionEvent::ContinueBattlePrompt { .. } => {
                self.close_run();
                // `yes_no(colors, "Continue Battle:")` (`ovr009.cs:407`). Under
                // a schedule (replay, or any non-interactive driver) the prompt
                // is still *displayed*, for one message beat; D-CV5's
                // suspension replaces the schedule as the answer source without
                // changing what is drawn.
                self.prompt_beat(strings::CONTINUE_BATTLE);
            }

            // Diagnostics, never presentation.
            ActionEvent::Morale { .. } | ActionEvent::StubTripped { .. } => {}
        }
    }

    // --- beats -------------------------------------------------------------

    /// `displayPlayerName`'s colour for `who` **right now** (`ovr025.cs:827-838`),
    /// captured off the shadow board so it records the state the print
    /// actually happened in.
    fn name_color(&self, who: usize) -> u8 {
        self.board
            .combatant(who)
            .map(|c| super::render::name_color(c.in_combat, c.team))
            .unwrap_or(super::layout::NAME_COLOR_PARTY)
    }

    /// `string_print01(text)` (`ovr025.cs:775-784`): clear the prompt row,
    /// print, `GameDelay`, clear.
    fn prompt_beat(&mut self, text: &str) {
        let delay = self.clock.game_delay();
        self.hold(Op::Prompt(Some(text.to_string())), delay);
        self.push(Op::Prompt(None));
    }

    /// `GameDelay()` — hold whatever the beat just drew.
    fn game_delay(&mut self) {
        let delay = self.clock.game_delay();
        self.hold(Op::Wait, delay);
    }

    /// `DisplayPlayerStatusString(true, 10, text, who)` — the panel message
    /// with its own `GameDelay` and clear (`ovr025.cs:786-810`).
    fn status_beat(&mut self, who: usize, text: String) {
        let color = self.name_color(who);
        self.push(Op::Message(PanelOp::Status {
            row: Row::At(MESSAGE_ROW),
            who,
            color,
            text,
        }));
        self.game_delay();
        self.push(Op::Message(PanelOp::Clear));
    }

    /// `AttackTarget`'s pose block (`ovr014.cs:918-940`): the target turns to
    /// face its attacker, the panel switches to the attacker, the attacker's
    /// Attack frame goes up, and the whole thing holds 100 ms.
    fn attack_pose(&mut self, attacker: usize, target: usize) {
        let (a, t) = (self.pos(attacker), self.pos(target));
        self.push(Op::Pose {
            id: target,
            pose: IconPose::Normal,
            direction: target_direction(t, a),
        });
        self.push(Op::Panel(attacker));
        self.hold(
            Op::Pose {
                id: attacker,
                pose: IconPose::Attack,
                direction: target_direction(a, t),
            },
            time::ms_to_ticks(time::ATTACK_POSE_MS),
        );
    }

    /// Closes an open swing run: the attacker's Attack frame comes down
    /// (`draw_74B3F(_, Normal, …)`, `ovr014.cs:1002-1003`).
    fn close_run(&mut self) {
        let Some(run) = self.run.take() else {
            return;
        };
        let direction = self
            .board
            .combatant(run.attacker)
            .map(|c| c.direction)
            .unwrap_or(0);
        self.push(Op::Pose {
            id: run.attacker,
            pose: IconPose::Normal,
            direction,
        });
    }

    /// The verb `DisplayAttackMessage` opens with (`ovr014.cs:117-127`).
    fn verb(run: &AttackRun) -> &'static str {
        if run.slay {
            strings::SLAYS_HELPLESS
        } else if run.kind == AttackKind::Backstab {
            strings::BACKSTABS
        } else {
            strings::ATTACKS
        }
    }

    /// The head of every `DisplayAttackMessage`: attacker + verb on row 10,
    /// target's name on row 12 (`ovr014.cs:129-132`).
    fn message_head(&mut self, run: &AttackRun) {
        let attacker_color = self.name_color(run.attacker);
        let target_color = self.name_color(run.target);
        self.push(Op::Message(PanelOp::Status {
            row: Row::At(MESSAGE_ROW),
            who: run.attacker,
            color: attacker_color,
            text: Self::verb(run).to_string(),
        }));
        self.push(Op::Message(PanelOp::Name {
            row: Row::At(TARGET_NAME_ROW),
            who: run.target,
            color: target_color,
        }));
    }

    /// The tail every `DisplayAttackMessage` shares: wrap the damage/miss
    /// line, mark where it ended, hold one `GameDelay`, and clear unless a
    /// removal is about to continue the message (`ovr014.cs:177-223`).
    fn message_tail(&mut self, run: AttackRun, text: String) {
        self.push(Op::Message(PanelOp::Wrapped {
            row: Row::At(DAMAGE_TEXT_ROW),
            text,
        }));
        // `line = gbl.textYCol + 1` (`ovr014.cs:180`) is captured BEFORE the
        // `GameDelay`, and the removal tail hangs off it.
        self.push(Op::Message(PanelOp::Mark));
        self.game_delay();
        if !self.removal_follows(run.target) {
            self.push(Op::Message(PanelOp::Clear));
        }
    }

    /// Does this target's `Removed` arrive before the next swing or turn? The
    /// original knows because `DisplayAttackMessage` reads `target.in_combat`
    /// after applying the damage; the composer looks ahead instead, which is
    /// the same question asked of the same batch.
    fn removal_follows(&self, target: usize) -> bool {
        for event in &self.events[self.at..] {
            match event {
                ActionEvent::Removed { combatant_id, .. } => return *combatant_id == target,
                ActionEvent::Sound { .. } | ActionEvent::StubTripped { .. } => {}
                _ => return false,
            }
        }
        false
    }

    fn hit_message(&mut self, damage: i32) {
        let Some(run) = self.run else { return };
        self.message_head(&run);
        let mut text = String::new();
        if run.kind == AttackKind::Behind {
            text.push_str(strings::FROM_BEHIND);
        }
        text.push_str(&strings::hitting_for(damage));
        self.message_tail(run, text);
    }

    fn miss_message(&mut self) {
        let Some(run) = self.run else { return };
        self.message_head(&run);
        let mut text = String::new();
        if run.kind == AttackKind::Behind {
            text.push_str(strings::FROM_BEHIND);
        }
        text.push_str(strings::AND_MISSES);
        self.message_tail(run, text);
    }

    /// `DisplayAttackMessage(true, 1, hp + 5, Slay, …)` (`ovr014.cs:751`) —
    /// "slays helpless" over "with one cruel blow", which replaces the
    /// "Hitting for …" line rather than prefixing it (`ovr014.cs:147-151`).
    fn slay_message(&mut self) {
        let Some(run) = self.run else { return };
        self.message_head(&run);
        self.message_tail(run, strings::ONE_CRUEL_BLOW.to_string());
    }

    /// Both removal shapes.
    fn removal(&mut self, id: usize, reason: RemovalReason) {
        match reason {
            // `RemoveFromCombat` (`ovr024.cs:618-647`): one panel message with
            // its own `GameDelay`, then the combatant simply vanishes.
            RemovalReason::Fled | RemovalReason::Surrendered => {
                self.close_run();
                let text = if reason == RemovalReason::Fled {
                    strings::GOT_AWAY
                } else {
                    strings::SURRENDERS
                };
                self.status_beat(id, text.to_string());
                self.board_op(ActionEvent::Removed {
                    combatant_id: id,
                    reason,
                });
            }
            // `DisplayAttackMessage`'s removal tail (`ovr014.cs:188-207`) into
            // `CombatantKilled` (`ovr033.cs:546-604`).
            RemovalReason::Killed | RemovalReason::Downed { .. } => {
                let dying = self
                    .board
                    .combatant(id)
                    .map(|c| c.health_status == HealthStatus::Dying)
                    .unwrap_or(false)
                    || matches!(reason, RemovalReason::Downed { dying: true });
                // The tail prints under `target.in_combat == false`
                // (`ovr014.cs:188`), so both names take the removed colour —
                // unlike the row-12 name above, which printed while the target
                // was still up.
                let color = crate::combat::scene::layout::NAME_COLOR_REMOVED;
                self.push(Op::Message(PanelOp::Status {
                    row: Row::FromMark(0),
                    who: id,
                    color,
                    text: strings::GOES_DOWN.to_string(),
                }));
                if dying {
                    self.push(Op::Message(PanelOp::Line {
                        row: Row::FromMark(2),
                        text: strings::AND_IS_DYING.to_string(),
                    }));
                } else if reason == RemovalReason::Killed {
                    self.push(Op::Message(PanelOp::Status {
                        row: Row::FromMark(2),
                        who: id,
                        color,
                        text: strings::IS_KILLED.to_string(),
                    }));
                }
                self.death_flash(id);
                // The body tile lands before the `GameDelay` and `size = 0`
                // after it (`ovr033.cs:578-598`), but the victim's icon is
                // already erased and the last flash frame covers the cell, so
                // applying the whole removal at the head of that hold shows the
                // same pixels — and leaves the presented board exactly where
                // reconciliation expects it.
                let delay = self.clock.game_delay();
                self.board.apply(ActionEvent::Removed {
                    combatant_id: id,
                    reason,
                });
                self.hold(
                    Op::Board(ActionEvent::Removed {
                        combatant_id: id,
                        reason,
                    }),
                    delay,
                );
                self.push(Op::Overlay(Vec::new()));
                self.push(Op::Hidden(None));
                self.push(Op::Message(PanelOp::Clear));
            }
        }
    }

    /// `CombatantKilled`'s flash (`ovr033.cs:556-575`): erase the victim's
    /// icon, then nine 10 ms alternations of two COMSPR frames over every
    /// on-screen cell of its footprint.
    fn death_flash(&mut self, id: usize) {
        let Some(c) = self.board.combatant(id) else {
            return;
        };
        let (size, pos) = (c.size, c.pos);
        let cells: Vec<(i32, i32)> = size_footprint(size, pos)
            .into_iter()
            .filter(|cell| self.board.on_screen(*cell))
            .map(|cell| {
                let (sx, sy) = self.board.screen_pos(cell);
                (sx * 3, sy * 3)
            })
            .collect();
        self.push(Op::Hidden(Some(id)));
        let hold = time::ms_to_ticks(time::DEATH_FLASH_MS);
        for i in 0..time::DEATH_FLASH_FRAMES {
            let sprite = if i % 2 == 0 {
                DEATH_FLASH_A
            } else {
                DEATH_FLASH_B
            };
            let draws = cells
                .iter()
                .map(|&(cell_x, cell_y)| OverlayDraw {
                    sprite,
                    cell_x,
                    cell_y,
                })
                .collect();
            self.hold(Op::Overlay(draws), hold);
        }
    }

    /// `draw_missile_attack` as instructions — the window jumps and the
    /// per-8-pixel sprite holds [`missile::plan_flight`] produced.
    fn flight(&mut self, attacker: usize, target: usize, class: &MissileClass) {
        let steps = missile::plan_flight(
            self.pos(attacker),
            self.pos(target),
            self.board.camera_top_left(),
            class,
        );
        if steps.is_empty() {
            return;
        }
        let hold = time::ms_to_ticks(class.step_ms);
        let mut camera = self.board.camera_top_left();
        for step in steps {
            match step {
                FlightStep::Camera { top_left } => {
                    camera = top_left;
                    self.board.apply(ActionEvent::Camera { top_left });
                    self.push(Op::Camera(top_left));
                }
                FlightStep::Frame {
                    cell_x,
                    cell_y,
                    frame,
                } => {
                    let sprite = class.frames[frame.min(class.frames.len() - 1)];
                    self.hold(
                        Op::Overlay(vec![OverlayDraw {
                            sprite,
                            cell_x,
                            cell_y,
                        }]),
                        hold,
                    );
                }
            }
        }
        self.push(Op::Overlay(Vec::new()));
        self.flight_camera = Some(camera);
    }

    /// The cast's projectile (`sub_5D2E1`, `ovr023.cs:741-768`): the caster's
    /// Attack frame, the per-spell cast sound, a 30 ms four-frame flight to
    /// the LAST picked target, then the caster's frame back down.
    ///
    /// It has no event of its own — the projectile is a consequence of `Cast`
    /// plus the `SpellTarget` run, and `gbl.targetPos` is the last target
    /// added (`spells.rs`'s own note).
    fn finish_cast(&mut self) {
        let Some(cast) = self.cast.take() else { return };
        let Some(target) = cast.last_target else {
            // `Spell Aborted` (`ovr023.cs:795`) — the QuickFight cast that
            // found nothing to aim at.
            self.prompt_beat(strings::SPELL_ABORTED);
            return;
        };
        let (c, t) = (self.pos(cast.caster), self.pos(target));
        self.push(Op::Pose {
            id: cast.caster,
            pose: IconPose::Attack,
            direction: target_direction(c, t),
        });
        self.push(Op::Sound(missile::cast_sound(cast.spell_id)));
        let class = missile::spell_projectile_class();
        self.flight(cast.caster, target, &class);
        // The flight's own window is transient here — no `Camera` event
        // follows a spell projectile (`sub_5D2E1` scrolls through the same
        // `draw_missile_camera` port only on the weapon path), so there is
        // nothing to check it against.
        self.flight_camera = None;
        let direction = self
            .board
            .combatant(cast.caster)
            .map(|c| c.direction)
            .unwrap_or(0);
        self.push(Op::Pose {
            id: cast.caster,
            pose: IconPose::Normal,
            direction,
        });
    }
}

/// `MagicAttackDisplay`'s on-target burst (`ovr025.cs:1118-1172`) as
/// instructions: the sound, the panel message, then four 70 ms frames
/// repeated `game_speed_var + 1` times for the stars variant or once for the
/// plain one — the plain one closing with a `GameDelay` (`ovr025.cs:1170`).
///
/// **Reached by no modeled event yet.** Every `MagicAttackDisplay` caller is a
/// spell/affect handler whose effect is unmodeled (charm, paralyze, confusion,
/// turn-undead) or which our engine reports through a different beat —
/// `SpellCureLight` prints `DescribeHealing`'s message, not a burst. The beat
/// is built and tested here because §1.4 specifies it and because the handler
/// that first needs it should find it, not re-derive it.
pub fn burst_instructions(
    clock: BeatClock,
    who: usize,
    color: u8,
    cell: (i32, i32),
    text: &str,
    stars: bool,
) -> Vec<Instruction> {
    let mut out = Vec::new();
    let icon_slot = if stars { 0x16 } else { 0x17 };
    // `PlaySound(sound_4)` for the stars, `sound_3` otherwise
    // (`ovr025.cs:1131-1138`).
    out.push(Instruction::now(Op::Sound(if stars { 4 } else { 3 })));
    out.push(Instruction::now(Op::Message(PanelOp::Status {
        row: Row::At(MESSAGE_ROW),
        who,
        color,
        text: text.to_string(),
    })));
    let passes = if stars { clock.star_burst_passes() } else { 1 };
    let hold = time::ms_to_ticks(time::BURST_FRAME_MS);
    for _ in 0..passes {
        for frame in 0..time::BURST_FRAMES as usize {
            let sprite = missile::spin_frame(icon_slot, frame);
            out.push(Instruction::held(
                Op::Overlay(vec![OverlayDraw {
                    sprite,
                    cell_x: cell.0,
                    cell_y: cell.1,
                }]),
                hold,
            ));
        }
    }
    if !stars {
        // `if (loops == 0) GameDelay();` (`ovr025.cs:1169-1172`) — the plain
        // variant holds its last frame for a message beat.
        out.push(Instruction::held(Op::Wait, clock.game_delay()));
    }
    out.push(Instruction::now(Op::Overlay(Vec::new())));
    out
}
