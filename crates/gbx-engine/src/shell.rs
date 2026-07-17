//! The UI shell state machine (D-UI2, task deliverable 3) driven by the
//! real `EclMachine` (M2 step 4, task deliverables 1-3): `Shell`, `VmPhase`,
//! the flow plans (`BootFlow`/`LookFlow`/`StepFlow`) with chain checkpoints
//! and resume-after-chain, the persistent `chained`/`party_killed` engine
//! state, and the walk-loop's world-menu dispatch.
//!
//! **Fable review finding, addressed explicitly (binding since M2 step 3):**
//! the design doc's prose says every blocking site is a Widget parked in
//! `VmPhase::Gate` or `WorldMenu` — but the locked-door menu lives in a
//! [`StepFlow`] stage (`StepStage::DoorInteraction`), which is neither: no
//! VM vector is running during a door prompt at all. The fix applied here is
//! the doc's own suggested alternative, "Gate generalizes to flows":
//! [`VmPhase::Gate`] is not exclusive to `VectorRun`s — any flow stage may
//! park a `Widget` in it directly (`StepStage::DoorInteraction` does exactly
//! this, with no VM involvement whatsoever). There is nowhere left for a
//! blocking interaction to hide outside this one mechanism.
//!
//! Derived by reading coab for behavior (D11, never copied) — see
//! `movement.rs`'s citations for `ovr015.cs`/`ovr031.cs`, `vmhost.rs`'s for
//! the ScriptMemory/EngineServices/`load_ecl_dax` research pass; this
//! module's own citations are to `engine/ovr003.cs` `sub_29758` (the walk
//! loop, `:2230-2396`) and `sub_29677` (the chain runner, `:2180-2227`).

use crate::framebuffer::Framebuffer;
use crate::input::InputQueue;
use crate::movement::{
    attempt_bash, attempt_knock, attempt_pick, build_door_hotbar, move_party_forward,
    position_time_text, try_step_forward, wall_door_flags, DoorState, DoorStepFlags, Facing,
    GameClock, PartyPredicates, WorldMenuCommand,
};
use crate::rng::EngineRng;
use crate::text::{JobStatus, TextCursor, TextJob, TextPacer, NORMAL_BOTTOM};
use crate::vmhost::{describe_halt, load_ecl_block, EngineVmHost, HaltRecord, VmMemoryState};
use crate::widgets::{Delay, Hotbar, PressAnyKey, Widget, WidgetOutcome};
use gbx_formats::font::Font;
use gbx_formats::game_data::GameData;
use gbx_formats::geo::GeoBlock;
use gbx_vm::{BlockId, EclMachine, Effect, Exit, Reply, Request, VmStep, COTAB};
use std::collections::VecDeque;

/// One audio cue this tick (D-UI1's `Frame::sounds` — M8 synthesizes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SoundEvent(pub u8);

/// The Fuel watchdog (D-UI2's obligations table): a vector run steps at
/// most this many times per tick before yielding, so a `GOTO`-self script
/// can't hang the app.
const STEP_BUDGET: u32 = 10_000;

/// The dialect's vector-table indices this session's flows fire
/// (`docs/design/vm-scriptmemory.md` §1's table, 0-indexed as
/// `EclMachine::vector` takes it — confirmed by `frontends/cli/run_script.rs`'s
/// own `vector.unwrap_or(4)` default).
const VECTOR_RUN_ADDR_1: usize = 0;
const VECTOR_SEARCH_LOCATION: usize = 1;
const VECTOR_ENTRY_POINT: usize = 4;

/// What a flow's cursor is doing right now (D-UI2, generalized — see this
/// module's top doc comment). `Pump`/`Present` only ever occur inside a
/// [`VectorRun`]; `Gate` is the shared park point for both VM-sourced and
/// purely engine-owned widgets.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VmPhase {
    Pump,
    Present,
    Gate(Widget),
}

/// Whatever a vector run yielded once its activation stack stops (a
/// `Request` or `Done`) — remembered across the `Present` drain so the run
/// knows what to do once the presentation queue is empty.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum PendingOutcome {
    Request(Request),
    Exit(Exit),
}

/// One vector's execution against the real [`EclMachine`]: pumps steps,
/// buffers `Effect`s into an ordered presentation queue, drains that queue
/// (pacing text through [`TextJob`], gating on pagination) before any
/// `Request`'s Widget opens — the D-VM3 ordering obligation, mechanically
/// enforced by this struct's own phase order.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VectorRun {
    phase: VmPhase,
    queue: VecDeque<Effect>,
    current_job: Option<TextJob>,
    pending: Option<PendingOutcome>,
    /// Set by [`VectorRun::tick_gate`] once a parked widget resolves;
    /// consumed by the next [`VectorRun::tick_pump`] call, which then calls
    /// `machine.resume(reply, ...)` instead of `machine.step(...)` exactly
    /// once.
    pending_reply: Option<Reply>,
}

/// One tick's result from [`VectorRun::tick`].
#[derive(Debug, Clone, PartialEq)]
pub enum RunTick {
    Working,
    Done(Exit),
}

/// [`VectorRun::tick_present`]'s internal result.
enum PresentTick {
    Working,
    OpenedGate,
    Done(Exit),
}

/// Everything a flow stage needs for one tick — bundled so `Shell`/flow
/// methods don't thread a dozen parameters individually.
pub struct FlowCtx<'a> {
    pub machine: &'a mut EclMachine,
    pub vm_memory: &'a mut VmMemoryState,
    pub data: &'a GameData,
    pub game_area: u8,
    pub input: &'a mut InputQueue,
    pub dt_ticks: u32,
    pub state: &'a mut EngineState,
    pub geo: &'a GeoBlock,
    pub party: &'a mut dyn PartyPredicates,
    /// The real party roster (M3 step 6): the party-facing screens
    /// (character sheet, camp, training, shops) read and mutate it. Distinct
    /// from [`Self::party`], which is the M2 combat/door-predicate abstraction.
    pub roster: &'a mut crate::party::Party,
    /// The loaded rules pack (M3 step 6): derived numbers, training XP
    /// thresholds, spell slots, prices — the flavor's tables.
    pub rules: &'a gbx_rules::pack::RuleSet,
    /// Host-injected view of the save slots (M3 step 6 deliverable 3): the
    /// save/load screen renders from this, never the filesystem (D8).
    pub slots: &'a crate::saveload::SlotDirectory,
    /// Where the save/load screen deposits its chosen action for the host to
    /// fulfill after the tick (D8: the core does no file I/O itself).
    pub io_request: &'a mut Option<crate::saveload::SaveLoadRequest>,
    pub rng: &'a mut EngineRng,
    pub fb: &'a mut Framebuffer,
    pub font: &'a Font,
    pub cursor: &'a mut TextCursor,
    pub pacer: &'a mut TextPacer,
    pub sounds: &'a mut Vec<SoundEvent>,
    /// Resident 8×8 symbol sets + wallset slots (step 5, task deliverable
    /// 1): `load_walldef`'s real target, and `crate::corridor`'s texture
    /// source.
    pub symbols: &'a mut crate::symbols::SymbolSets,
    /// The three boot-loaded `SKY` blocks (moon/sun/horizon, `boot.rs`'s
    /// `BootAssets::sky`) — read-only after boot, `crate::corridor`'s
    /// backdrop source (step 5, task deliverable 2).
    pub sky: &'a [gbx_formats::image::ImageBlock; 3],
}

/// `Request` -> `Widget` (design doc's table, M2 slice). Engine-owned
/// interactions (world menu, door menu, pagination) never go through this —
/// only a real `VectorRun`'s `Request` does. Only the `Request` variants
/// `gbx-vm`'s interpreter can currently emit are handled
/// (`HorizontalMenu`/`Delay`/`Combat`) — `VerticalMenu`/`InputNumber`/
/// `InputString`/`SelectPlayer` await their opcodes (`0x15`/`0x0F`/`0x10`/
/// `0x39`) landing in the interpreter; `ListMenu`/`TextEntry` already exist
/// and are ready (step 3), this is purely a `gbx-vm` coverage gap, docketed.
fn widget_for_request(request: &Request) -> Widget {
    match request {
        Request::HorizontalMenu { options } => {
            let text = options
                .iter()
                .map(|s| String::from_utf8_lossy(&s.0).into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            let mut hotbar = Hotbar::new(text);
            hotbar.accept_ext = true;
            hotbar.ext_scrolls_party = true;
            // `sub_317AA`'s validkeys (`ovr008.cs:1167`, this session's
            // research): '0'-'9','A'-'Z'.
            hotbar.valid_keys = Some((b'0'..=b'9').chain(b'A'..=b'Z').collect());
            Widget::Hotbar(hotbar)
        }
        // `game_speed_var`-scaled duration is engine-owned and not on the
        // wire (host.rs's doc comment); placeholder tick count pending the
        // real value (docketed, same spirit as the "Not Here" 24-tick wait).
        Request::Delay => Widget::Delay(Delay::new(24)),
        // M2 stub (design doc's Request table): paint a stub + wait for any
        // key. The real paint is step 5's rendering scope; the flow-control
        // shape (park, resolve, resume) is what this session proves.
        Request::Combat => Widget::PressAnyKey(PressAnyKey),
    }
}

/// Transcript-mode's (M2 step 8) request label — content, not widget shape:
/// a `HorizontalMenu`'s joined option text (the same text a player reads),
/// or a fixed descriptive label for the non-textual requests.
fn describe_request(request: &Request) -> String {
    match request {
        Request::HorizontalMenu { options } => {
            let text = options
                .iter()
                .map(|s| String::from_utf8_lossy(&s.0).into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            format!("menu: {text}")
        }
        Request::Delay => "delay".to_string(),
        // Reached only for the DEFERRED non-combat COMBAT branch (no monsters
        // loaded → shop/temple/AfterCombat dispatch, `CMD_Combat` ovr003:974);
        // the real-combat branch resolves in `tick_present` before a widget is
        // ever built, so it never reaches here.
        Request::Combat => "combat (non-combat branch: deferred)".to_string(),
    }
}

/// The party's world facing → coab's `mapDirection` (0/2/4/6 = N/E/S/W), the
/// axis `place_combatants` offsets the monster team along and `sub_304B4`
/// casts its LoS ray down.
fn facing_to_map_dir(facing: crate::movement::Facing) -> u8 {
    use crate::movement::Facing;
    match facing {
        Facing::North => 0,
        Facing::East => 2,
        Facing::South => 4,
        Facing::West => 6,
    }
}

/// The equipped-weapon damage die the party fights with until the `.swg`
/// `ItemData` weapon records are decoded (FD-29's weapon clause, M5-adjacent):
/// a documented 1d8 (a longsword). Flagged provisional — real per-member
/// weapon dice replace this when items land.
const DEFAULT_PARTY_WEAPON_DIE: (u8, u8, u8) = (1, 8, 0);

/// Map the live party roster into the combat runner's team-0 inputs (M4 combat
/// #6). Only living members (`hit_point_current > 0`) enter the fight. Each
/// member's raw AC, THAC0 (as the to-hit bonus, matching the monster path),
/// current HP, and movement come straight off the record; the weapon die is
/// [`DEFAULT_PARTY_WEAPON_DIE`] pending item decode.
fn party_combat_stats(members: &[crate::party::Character]) -> Vec<crate::combat::PartyCombatStats> {
    members
        .iter()
        .filter(|c| c.hit_point_current > 0)
        .map(|c| crate::combat::PartyCombatStats {
            hp: c.hit_point_current as i32,
            raw_ac: c.combat.ac as u8,
            hit_bonus: c.combat.thac0_base as i32,
            movement: c.combat.movement as i32,
            dice: DEFAULT_PARTY_WEAPON_DIE,
            npc: c.control_morale >= 0x80,
        })
        .collect()
}

/// Run the `COMBAT` opcode's real-combat branch (`CMD_Combat` else-branch,
/// `ovr003.cs:1004` → `MainCombatLoop`) from the shell/tick path (M4 combat
/// #6). Called only when monsters were loaded; assembles the roster (party
/// team 0 + the script-loaded monsters team 1), derives the terrain +
/// encounter distance from the current area (all draw-free — no draw is added
/// before the fight's first initiative d6), runs the unified [`CombatState`]
/// to a victor, then **consumes** the pending roster. A party wipe sets
/// `party_killed` (the game-over signal). XP/treasure
/// (`AfterCombatExpAndTreasure`) is deferred.
///
/// Runs entirely off `FlowCtx` — the `VmHost` borrow was already released when
/// `step()` yielded `Request::Combat`, so this never blocks the VM host (D8).
fn run_pending_combat(ctx: &mut FlowCtx) -> crate::combat::EncounterOutcome {
    let party = party_combat_stats(&ctx.roster.members);
    let monsters = std::mem::take(&mut ctx.state.pending_combat.monsters);
    let map_dir = facing_to_map_dir(ctx.state.facing);
    let in_dungeon = matches!(ctx.state.game_state, GameState::DungeonMap);
    let (px, py) = (ctx.state.pos.0 as i32, ctx.state.pos.1 as i32);
    let dist = crate::combat::encounter_distance(ctx.geo, map_dir, px, py, in_dungeon);
    let map = crate::combat::provisional_combat_map(ctx.geo);
    let result = crate::combat::run_encounter(&party, &monsters, map, map_dir, dist, ctx.rng);
    ctx.state.pending_combat.clear();
    if result.outcome == crate::combat::CombatOutcome::MonstersWin {
        ctx.state.party_killed = true;
    }
    result
}

/// The inverse of [`widget_for_request`]'s `HorizontalMenu` case: maps a
/// resolved Hotbar key back to a `Reply::Selection` index. Implementation
/// note (flagged): finds the first option whose leading byte (uppercased)
/// matches the resolved key — exact for every menu this session's flows
/// construct (each option is its own hotkey-selectable word, per real
/// HORIZONTAL MENU option text), but not a byte-exact replication of
/// `sub_317AA`'s own index bookkeeping (out of scope — the original tracks
/// the option index directly rather than re-deriving it from the key).
fn resolve_horizontal_menu_reply(options: &[gbx_vm::VmString], key: u8) -> Reply {
    let upper = key.to_ascii_uppercase();
    let idx = options
        .iter()
        .position(|opt| opt.0.first().map(|b| b.to_ascii_uppercase()) == Some(upper))
        .unwrap_or(0);
    Reply::Selection(idx as u8)
}

/// `machine.vector(index)` + `machine.enter(addr)`, or `None` if that vector
/// is unresolved in the resident block (an empty/malformed block — treated
/// as "nothing to run," not a panic; real CotAB data never hits this per
/// M1's census).
fn enter_vector(machine: &mut EclMachine, index: usize) -> Option<VectorRun> {
    let addr = machine.vector(index)?;
    machine.enter(addr);
    Some(VectorRun {
        phase: VmPhase::Pump,
        queue: VecDeque::new(),
        current_job: None,
        pending: None,
        pending_reply: None,
    })
}

impl VectorRun {
    /// Advances by one tick — internally loops through phase transitions
    /// (Pump -> Present -> Gate -> Pump -> ...) making maximal progress,
    /// per D-UI1's "bounded state advance" model: a tick only *actually*
    /// pauses at a genuine wait (a parked widget needing input, or a
    /// `TextJob` that's spent this tick's character budget), never on an
    /// artificial one-phase-per-tick rule.
    ///
    /// The character-pacing budget (D-UI1's fractional accumulator) is
    /// drawn from `ctx.pacer` **exactly once** here, regardless of how many
    /// phase transitions this call makes — `tick_present` spends it on at
    /// most one `TextJob::advance` call, so a cascade of same-tick effects
    /// (e.g. an instant `PrintReturn` immediately followed by a `Print`)
    /// can never double-dip the per-tick character rate.
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> RunTick {
        let tick_ms = 1000.0 / crate::input::TICK_HZ as f64;
        let mut budget = Some(ctx.pacer.tick(tick_ms));
        loop {
            match &self.phase {
                VmPhase::Pump => {
                    if !self.tick_pump(ctx) {
                        return RunTick::Working; // exhausted the step budget without resolving
                    }
                    // pending is now set; phase became Present — keep going.
                }
                VmPhase::Present => match self.tick_present(ctx, &mut budget) {
                    PresentTick::Working => return RunTick::Working,
                    PresentTick::Done(exit) => return RunTick::Done(exit),
                    PresentTick::OpenedGate => {} // loop: let the Gate arm run this same tick
                },
                VmPhase::Gate(_) => {
                    if !self.tick_gate(ctx) {
                        return RunTick::Working; // still gated, or paginating
                    }
                    // resumed to Pump/Present — keep going this same tick.
                }
            }
        }
    }

    /// Pumps up to [`STEP_BUDGET`] steps against the real `EclMachine`.
    /// Returns `true` once a `Request` or `Done` is pending (phase advances
    /// to `Present`), `false` if the budget ran out first (the Fuel
    /// watchdog, D-UI2's obligations table). A `VmError` invokes the M2 halt
    /// policy (task deliverable 4): logged to `vm_memory.halts`, the run
    /// treated as `Done(Ended)` for flow purposes — never a hard failure.
    fn tick_pump(&mut self, ctx: &mut FlowCtx) -> bool {
        for _ in 0..STEP_BUDGET {
            // Constructed inline (not via a helper function) so the borrow
            // checker sees these as disjoint field reborrows of `*ctx` —
            // `ctx.machine` stays reachable alongside `host` only because
            // this happens within the same scope, not across a call
            // boundary (a `&mut FlowCtx`-taking helper would opaquely
            // borrow the whole struct from the caller's view).
            let mut host = EngineVmHost {
                state: &mut *ctx.state,
                vm: &mut *ctx.vm_memory,
                geo: ctx.geo,
                party: &mut *ctx.party,
                rng: &mut *ctx.rng,
                sounds: &mut *ctx.sounds,
                data: ctx.data,
                game_area: ctx.game_area,
                symbols: &mut *ctx.symbols,
            };
            let result = if let Some(reply) = self.pending_reply.take() {
                ctx.machine.resume(reply, &mut host)
            } else {
                ctx.machine.step(&mut host)
            };
            match result {
                Ok(VmStep::Continue) => continue,
                Ok(VmStep::Effect(e)) => self.queue.push_back(e),
                Ok(VmStep::Request(r)) => {
                    self.pending = Some(PendingOutcome::Request(r));
                    break;
                }
                Ok(VmStep::Done(exit)) => {
                    self.pending = Some(PendingOutcome::Exit(exit));
                    break;
                }
                Err(err) => {
                    ctx.vm_memory.halts.push(describe_halt(&err));
                    self.pending = Some(PendingOutcome::Exit(Exit::Ended));
                    break;
                }
            }
        }
        if self.pending.is_some() {
            self.phase = VmPhase::Present;
            true
        } else {
            false
        }
    }

    /// `budget` is this external tick's character allowance, spent on at
    /// most one `TextJob::advance` call (see [`VectorRun::tick`]'s doc
    /// comment) — once taken, later jobs started the same tick get `0`
    /// characters and simply wait for the next external tick.
    fn tick_present(&mut self, ctx: &mut FlowCtx, budget: &mut Option<u32>) -> PresentTick {
        loop {
            if let Some(job) = &mut self.current_job {
                let this_budget = budget.take().unwrap_or(0);
                match job.advance(this_budget, ctx.fb, ctx.font, ctx.cursor) {
                    JobStatus::Continuing => return PresentTick::Working,
                    JobStatus::NeedsKey => {
                        self.phase = VmPhase::Gate(Widget::PressAnyKey(PressAnyKey));
                        return PresentTick::OpenedGate;
                    }
                    JobStatus::Done => {
                        self.current_job = None;
                        continue;
                    }
                }
            }
            let Some(effect) = self.queue.pop_front() else {
                break;
            };
            match effect {
                Effect::Print { text, clear_first } => {
                    let text = String::from_utf8_lossy(&text.0).into_owned();
                    ctx.vm_memory
                        .transcript
                        .push(crate::vmhost::TranscriptEntry::Print {
                            text: text.clone(),
                            clear_first,
                        });
                    self.current_job = Some(TextJob::new(
                        &text,
                        10,
                        NORMAL_BOTTOM,
                        clear_first,
                        ctx.cursor,
                        ctx.fb,
                    ));
                }
                Effect::PrintReturn => {
                    ctx.cursor.row += 1;
                    ctx.cursor.col = NORMAL_BOTTOM.x_start;
                }
                Effect::Sound(variant) => ctx.sounds.push(SoundEvent(variant)),
                // Picture/animation *rendering* is out of M2's scope
                // (pictures land in a later milestone); the effect is
                // consumed (drained) here so the queue-before-gate ordering
                // obligation still holds for it. `CMD_Picture`'s own
                // redraw-flag side effects (`spriteChanged`/`can_draw_bigpic`
                // — `ovr003.cs:320,348`, this session's redraw-flag
                // consolidation research) are real regardless of whether
                // the picture itself draws, so they're tracked here.
                Effect::Picture(_) => {
                    ctx.vm_memory.sprite_changed = true;
                    ctx.vm_memory.can_draw_bigpic = true;
                }
                Effect::ClearPicture | Effect::AnimationFrame => {}
            }
        }

        match self
            .pending
            .take()
            .expect("Present entered with no pending outcome")
        {
            PendingOutcome::Exit(exit) => PresentTick::Done(exit),
            PendingOutcome::Request(request) => {
                // COMBAT (0x24) real-combat branch (`CMD_Combat` else, monsters
                // loaded): run the fight here in the shell/tick path — the
                // `VmHost` borrow was released when `step()` yielded the
                // request, so this never blocks it (D8) — then resume the
                // script with its outcome. No player input is needed (the fight
                // is all-AI this slice), so it bypasses the widget/gate
                // entirely and pumps straight on this same tick.
                if matches!(request, Request::Combat) && ctx.state.pending_combat.monsters_loaded {
                    let result = run_pending_combat(ctx);
                    let label = match result.outcome {
                        crate::combat::CombatOutcome::PartyWins => "party wins",
                        crate::combat::CombatOutcome::MonstersWin => "party wiped",
                        crate::combat::CombatOutcome::Stalemate => "stalemate",
                    };
                    ctx.vm_memory
                        .transcript
                        .push(crate::vmhost::TranscriptEntry::Request(format!(
                            "combat: {label} ({} round(s))",
                            result.rounds
                        )));
                    self.pending_reply = Some(Reply::Combat);
                    self.phase = VmPhase::Pump;
                    return PresentTick::OpenedGate;
                }
                ctx.vm_memory
                    .transcript
                    .push(crate::vmhost::TranscriptEntry::Request(describe_request(
                        &request,
                    )));
                let widget = widget_for_request(&request);
                self.pending = Some(PendingOutcome::Request(request));
                self.phase = VmPhase::Gate(widget);
                PresentTick::OpenedGate
            }
        }
    }

    /// Ticks the parked Gate widget. Returns `true` once it resumed pumping
    /// (or resumed presenting, for a nested pagination gate) — `false` while
    /// still waiting on input.
    fn tick_gate(&mut self, ctx: &mut FlowCtx) -> bool {
        let VmPhase::Gate(widget) = &mut self.phase else {
            unreachable!("tick_gate called outside Gate phase")
        };
        let outcome = widget.tick(ctx.input, ctx.dt_ticks);

        // A PressAnyKey gate nested under a paginating TextJob: release the
        // job and resume presenting, rather than resuming the VM.
        if self.current_job.is_some() {
            if matches!(outcome, WidgetOutcome::Done) {
                if let Some(job) = &mut self.current_job {
                    job.release(ctx.fb);
                }
                // The original drains the whole keyboard buffer right after
                // the pagination keypress (`clear_keyboard`, seg041.cs:211;
                // design doc §1.4/D-UI3 named this the caller's obligation —
                // it was never wired in steps 3/4). Without it, keys typed
                // behind the gating keypress leak to the next widget, where
                // Enter selects the highlighted first word ("Area" in the
                // world menu). Found via the step-5 four-facings demo.
                ctx.input.clear();
                self.phase = VmPhase::Present;
                return true;
            }
            return false;
        }

        if matches!(outcome, WidgetOutcome::Pending) {
            return false;
        }

        // Any resolution: build the real Reply matching the pending
        // Request, then resume pumping.
        let Some(PendingOutcome::Request(request)) = self.pending.take() else {
            unreachable!("Gate phase without a pending Request")
        };
        let reply = match (&request, outcome) {
            (Request::HorizontalMenu { options }, WidgetOutcome::Hotbar(key)) => {
                resolve_horizontal_menu_reply(options, key)
            }
            (Request::Delay, _) => Reply::Delay,
            (Request::Combat, _) => Reply::Combat,
            _ => Reply::Selection(0), // unreachable in practice; a safe fallback, not a panic
        };
        self.pending_reply = Some(reply);
        self.phase = VmPhase::Pump;
        true
    }
}

/// The chain runner (`sub_29677`, `ovr003.cs:2180-2227`): re-entered at
/// every `ChainCheckpoint` while `chained` stays set, running the newly
/// resident block's entry vector each round.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChainRunner {
    run: VectorRun,
}

pub enum ChainRunnerOutcome {
    ChainedAgain(BlockId),
    Finished,
}

impl ChainRunner {
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<ChainRunnerOutcome> {
        match self.run.tick(ctx) {
            RunTick::Working => None,
            RunTick::Done(Exit::Ended) => Some(ChainRunnerOutcome::Finished),
            RunTick::Done(Exit::ChainTo(id)) => Some(ChainRunnerOutcome::ChainedAgain(id)),
        }
    }
}

/// `Exit::ChainTo` bookkeeping shared by every checkpoint (§1.6): commits
/// `LastEclBlockId` (NEWECL's own old-id write, `ovr003.cs:488`), sets
/// `chained`, loads the new block via `load_ecl_dax`'s mapping
/// (`vmhost.rs`), and starts its entry vector. Returns `None` if the chain
/// already fully resolved this same tick (a load failure or an unresolved
/// entry vector — both loudly diagnosed via `vm_memory.halts`, never a
/// silent stall or a panic on bad/missing content).
fn begin_chain(ctx: &mut FlowCtx, id: BlockId) -> Option<ChainRunner> {
    ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
    ctx.state.ecl_block_id = id.0;
    ctx.state.chained = true;

    let bytes = match load_ecl_block(ctx.data, ctx.game_area, id.0) {
        Ok(bytes) => bytes,
        Err(err) => {
            ctx.vm_memory.halts.push(HaltRecord {
                pc: 0,
                opcode: 0,
                description: format!("NEWECL to block {} failed to load: {err:?}", id.0),
            });
            ctx.state.chained = false;
            return None;
        }
    };
    *ctx.machine = EclMachine::load_block(bytes, &COTAB).unwrap_or_else(|never| match never {});

    match enter_vector(ctx.machine, VECTOR_ENTRY_POINT) {
        Some(run) => Some(ChainRunner { run }),
        None => {
            ctx.state.chained = false;
            None
        }
    }
}

/// Runs `chain`, handling the "chained again" re-entry loop; on `Finished`,
/// commits the bookkeeping every checkpoint shares (`chained` clears,
/// `LastEclBlockId` commits, `LastSelectedPlayer` saves — §1.6) and reports
/// completion so the owning flow can resume its suspended plan. `None`
/// means still working (call again next tick); `Some(())` means resume.
fn drive_chain(chain: &mut Option<ChainRunner>, ctx: &mut FlowCtx) -> Option<()> {
    let runner = chain.as_mut()?;
    match runner.tick(ctx) {
        None => None,
        Some(ChainRunnerOutcome::ChainedAgain(id)) => {
            ctx.state.last_selected_player = ctx.state.selected_player;
            *chain = begin_chain(ctx, id);
            if chain.is_none() {
                ctx.state.chained = false;
                ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
                return Some(());
            }
            None
        }
        Some(ChainRunnerOutcome::Finished) => {
            ctx.state.chained = false;
            ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
            *chain = None;
            Some(())
        }
    }
}

/// The persistent, serializable engine state carried across ticks (D-UI2's
/// "Engine state carried" list, M2 slice — rendering caches/redraw flags
/// are step-5 scope and not modeled here).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EngineState {
    pub pos: (u8, u8),
    pub facing: Facing,
    /// bit 0 = search mode; bit 1 = transient "Look in progress" marker
    /// (`search_flags |= 2` on `'L'`, cleared by [`LookFlow`]'s restore).
    pub search_flags: u8,
    /// `block_area_view == 0`-equivalent: whether the area map is available
    /// at all in the resident block (a per-area config, M6 scope in full —
    /// defaults `true`).
    pub area_view_allowed: bool,
    /// `mapAreaDisplay`: whether the area map is currently being shown.
    pub area_map_shown: bool,
    /// The persistent `vmFlag01` equivalent (D-UI2): survives across
    /// `WorldMenu`, suppresses `LastEclBlockId` commits while set.
    pub chained: bool,
    pub party_killed: bool,
    pub selected_player: u8,
    pub last_selected_player: u8,
    pub ecl_block_id: u8,
    pub last_ecl_block_id: u8,
    pub tried_to_exit_map: bool,
    /// `area2_ptr.field_592`: `< 0xFF` gates `locked_door`'s whole
    /// interaction; zeroed at every world-menu entry.
    pub field_592: u8,
    pub door_flags: DoorStepFlags,
    pub clock: GameClock,
    pub reload_ecl_and_pictures: bool,
    /// `gbl.game_state`/`gbl.last_game_state` (`inDungeon`'s write hook,
    /// this session's research): only `DungeonMap`/`WildernessMap` are
    /// modeled (M2 slice — the other `game_state` values are M3+ screens).
    pub game_state: GameState,
    pub last_game_state: GameState,
    /// `area2_ptr.HeadBlockId`: `0xFF` = no specific portrait head (reset by
    /// `vm_init_ecl`); M4/combat scope carries this even though M2 draws
    /// nothing from it yet.
    pub head_block_id: u8,
    /// The pending-combat monster roster the `LOAD MONSTER`/`CLEARMONSTERS`
    /// opcodes accumulate and `COMBAT` consumes (coab `gbl.TeamList` monster
    /// half + `monstersLoaded`/`monster_icon_id`, M4 combat #6). Transient
    /// combat-setup state — **not serialized** (`#[serde(skip)]`): a save is
    /// never taken mid-setup, so the `.rsav` golden is unaffected (the field
    /// deserializes back to [`crate::monster::PendingCombat::default`]).
    #[serde(skip)]
    pub pending_combat: crate::monster::PendingCombat,
}

/// `gbl.game_state`'s M2 slice (`Classes/Gbl.cs`'s `GameState` enum —
/// `WildernessMap`/`DungeonMap` are the only two this session's `inDungeon`
/// write hook can produce).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GameState {
    WildernessMap,
    DungeonMap,
}

impl EngineState {
    pub fn new() -> Self {
        EngineState {
            pos: (0, 0),
            facing: Facing::North,
            search_flags: 0,
            area_view_allowed: true,
            area_map_shown: false,
            chained: false,
            party_killed: false,
            selected_player: 0,
            last_selected_player: 0,
            ecl_block_id: 1,
            last_ecl_block_id: 0,
            tried_to_exit_map: false,
            field_592: 0,
            door_flags: DoorStepFlags::all_true(),
            clock: GameClock::default(),
            reload_ecl_and_pictures: false,
            game_state: GameState::DungeonMap,
            last_game_state: GameState::DungeonMap,
            head_block_id: 0xFF,
            pending_combat: crate::monster::PendingCombat::default(),
        }
    }

    pub(crate) fn search_mode(&self) -> bool {
        self.search_flags & 1 != 0
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self::new()
    }
}

// --- BootFlow (block-entry preamble, `sub_29758`'s header, `:2230-2313`) ---

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum BootStage {
    EntryVector,
    PostChainResume,
    Done,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BootFlow {
    stage: BootStage,
    run: Option<VectorRun>,
    chain: Option<ChainRunner>,
}

impl BootFlow {
    pub fn start(machine: &mut EclMachine, state: &mut EngineState) -> Self {
        state.last_selected_player = state.selected_player; // `:2232`
        let run = enter_vector(machine, VECTOR_ENTRY_POINT);
        BootFlow {
            stage: BootStage::EntryVector,
            run,
            chain: None,
        }
    }

    /// `Some(())` once the flow is done (caller transitions to `WorldMenu`).
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if self.chain.is_some() {
            drive_chain(&mut self.chain, ctx)?;
            self.stage = BootStage::PostChainResume;
        }

        match self.stage {
            BootStage::EntryVector => {
                let Some(run) = self.run.as_mut() else {
                    // The entry vector was unresolved at construction — an
                    // immediate no-op, matching an empty block.
                    self.stage = BootStage::PostChainResume;
                    return None;
                };
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        ctx.state.last_ecl_block_id = ctx.state.ecl_block_id; // `:2292-2294`
                        self.run = None;
                        self.stage = BootStage::PostChainResume;
                        None
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        self.run = None;
                        match begin_chain(ctx, id) {
                            Some(runner) => self.chain = Some(runner),
                            None => self.stage = BootStage::PostChainResume,
                        }
                        None
                    }
                }
            }
            // The `LoadPic`/`RedrawView`-equivalent + `reload_ecl_and_pictures`
            // clear (`:2298-2313`) — rendering itself is step-5 scope; this
            // session's observable contract is the flag clear happening
            // strictly after any chain resolves (the resume-after-chain
            // shape D-UI2 calls mandatory).
            BootStage::PostChainResume => {
                ctx.state.reload_ecl_and_pictures = false;
                self.stage = BootStage::Done;
                Some(())
            }
            BootStage::Done => Some(()),
        }
    }
}

// --- LookFlow (the 'L' sub-loop's Look branch, `ovr003.cs`'s
// `search_flags>1` while-loop body) ---

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum LookStage {
    RunVector2,
    RestoreSearchFlags,
    Done,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LookFlow {
    stage: LookStage,
    run: Option<VectorRun>,
    chain: Option<ChainRunner>,
    search_flags_backup: u8,
}

impl LookFlow {
    /// Caller (the world-menu dispatch) has already set `search_flags |= 2`
    /// and advanced the clock (`'L'` handler, §1.6) before calling this.
    pub fn start(machine: &mut EclMachine, state: &mut EngineState) -> Self {
        let backup = state.search_flags & 1;
        state.search_flags = 1;
        let run = enter_vector(machine, VECTOR_SEARCH_LOCATION);
        LookFlow {
            stage: LookStage::RunVector2,
            run,
            chain: None,
            search_flags_backup: backup,
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if self.chain.is_some() {
            drive_chain(&mut self.chain, ctx)?;
            self.stage = LookStage::RestoreSearchFlags;
        }

        match self.stage {
            LookStage::RunVector2 => {
                let Some(run) = self.run.as_mut() else {
                    self.stage = LookStage::RestoreSearchFlags;
                    return None;
                };
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        self.run = None;
                        self.stage = LookStage::RestoreSearchFlags;
                        None
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        self.run = None;
                        match begin_chain(ctx, id) {
                            Some(runner) => self.chain = Some(runner),
                            None => self.stage = LookStage::RestoreSearchFlags,
                        }
                        None
                    }
                }
            }
            LookStage::RestoreSearchFlags => {
                // Bit 1 ("Look pending") never survives a Look, regardless
                // of what vector 2 did — confirmed by this session's
                // research (only bit 0 is ever backed up/restored).
                ctx.state.search_flags = self.search_flags_backup;
                self.stage = LookStage::Done;
                Some(())
            }
            LookStage::Done => Some(()),
        }
    }
}

// --- StepFlow (the forward/step sequence, `sub_29758`'s tail) ---

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum StepStage {
    RunVector1,
    DoorInteraction,
    RunVector2,
    Done,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StepFlow {
    stage: StepStage,
    run: Option<VectorRun>,
    chain: Option<ChainRunner>,
    /// The Fable-review fix in concrete form: a Widget parked directly by
    /// this flow stage, no VM involved. `None` once resolved or if the
    /// door menu never opened (no options / solid / already open).
    door_widget: Option<Widget>,
    last_pos: (u8, u8),
}

impl StepFlow {
    pub fn start(machine: &mut EclMachine, state: &mut EngineState) -> Self {
        let run = enter_vector(machine, VECTOR_RUN_ADDR_1);
        StepFlow {
            stage: StepStage::RunVector1,
            run,
            chain: None,
            door_widget: None,
            last_pos: state.pos,
        }
    }

    /// Whether the Bash/Pick/Knock/Exit menu is currently parked — a
    /// test/demo introspection seam (the Fable-review fix is otherwise
    /// invisible from outside `shell.rs`: no VM run is active while this is
    /// `true`).
    pub fn door_widget_is_some(&self) -> bool {
        self.door_widget.is_some()
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if self.chain.is_some() {
            drive_chain(&mut self.chain, ctx)?;
            // The chained-mid-flow rule: the pending step (door interaction)
            // is abandoned entirely (§1.6) — resume goes straight past it.
            self.stage = StepStage::Done;
            return Some(());
        }

        match self.stage {
            StepStage::RunVector1 => {
                let Some(run) = self.run.as_mut() else {
                    self.last_pos = ctx.state.pos;
                    self.stage = StepStage::DoorInteraction;
                    return None;
                };
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        self.run = None;
                        self.last_pos = ctx.state.pos;
                        self.stage = StepStage::DoorInteraction;
                        None
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        self.run = None;
                        match begin_chain(ctx, id) {
                            Some(runner) => self.chain = Some(runner),
                            None => self.stage = StepStage::Done,
                        }
                        None
                    }
                }
            }
            StepStage::DoorInteraction => self.tick_door_interaction(ctx),
            StepStage::RunVector2 => {
                let Some(run) = self.run.as_mut() else {
                    self.stage = StepStage::Done;
                    return Some(());
                };
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        self.run = None;
                        self.stage = StepStage::Done;
                        Some(())
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        self.run = None;
                        match begin_chain(ctx, id) {
                            Some(runner) => self.chain = Some(runner),
                            None => self.stage = StepStage::Done,
                        }
                        None
                    }
                }
            }
            StepStage::Done => Some(()),
        }
    }

    /// `locked_door` (`ovr015.cs:468-593`) — no VM run of any kind; a
    /// direct Widget park (the Fable review fix, see module doc comment).
    fn tick_door_interaction(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if let Some(widget) = &mut self.door_widget {
            match widget.tick(ctx.input, ctx.dt_ticks) {
                WidgetOutcome::Pending => return None,
                WidgetOutcome::Hotbar(key) => {
                    let state_flag = wall_door_flags(
                        ctx.geo
                            .square(ctx.state.pos.0 as usize, ctx.state.pos.1 as usize),
                        ctx.state.facing,
                    );
                    let door_state = DoorState::from_flag(state_flag);
                    let moved = match key.to_ascii_uppercase() {
                        b'B' => {
                            attempt_bash(door_state, ctx.party, &mut ctx.state.door_flags, ctx.rng)
                        }
                        b'P' => {
                            attempt_pick(door_state, ctx.party, &mut ctx.state.door_flags, ctx.rng)
                        }
                        b'K' => attempt_knock(ctx.party),
                        _ => false, // Exit (or anything else): no effect
                    };
                    if moved {
                        let facing = ctx.state.facing;
                        let search = ctx.state.search_mode();
                        move_party_forward(
                            &mut ctx.state.pos,
                            facing,
                            search,
                            &mut ctx.state.door_flags,
                            &mut ctx.state.clock,
                        );
                    }
                    self.door_widget = None;
                }
                _ => self.door_widget = None, // any other widget outcome: treat as exit
            }
        } else if ctx.state.field_592 < 0xFF {
            let square = ctx
                .geo
                .square(ctx.state.pos.0 as usize, ctx.state.pos.1 as usize);
            let flag = wall_door_flags(square, ctx.state.facing);
            match DoorState::from_flag(flag) {
                DoorState::Open => {
                    let facing = ctx.state.facing;
                    let search = ctx.state.search_mode();
                    move_party_forward(
                        &mut ctx.state.pos,
                        facing,
                        search,
                        &mut ctx.state.door_flags,
                        &mut ctx.state.clock,
                    )
                }
                DoorState::Solid => {}
                DoorState::Locked | DoorState::Unpickable => {
                    if let Some(hotbar) = build_door_hotbar(&ctx.state.door_flags, ctx.party) {
                        self.door_widget = Some(Widget::Hotbar(hotbar));
                        return None;
                    }
                    // No option available at all: silent no-op (research
                    // finding — the original shows no menu here either).
                }
            }
        } else {
            ctx.state.field_592 = 0;
        }

        if ctx.state.pos != self.last_pos {
            ctx.sounds.push(SoundEvent(crate::movement::SOUND_A));
        }
        self.run = enter_vector(ctx.machine, VECTOR_SEARCH_LOCATION);
        self.stage = StepStage::RunVector2;
        None
    }
}

/// The UI shell (D-UI2): `Boot`/`WorldMenu`/`Step`/`GameOver`, plus `Look`
/// as its own explicit variant (the `'L'` sub-loop, distinct from
/// `WorldMenu` and `Step` — both this session's required resume-after-chain
/// sites live here and in [`BootFlow`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Shell {
    Boot(BootFlow),
    WorldMenu {
        menu: Widget,
    },
    Look(LookFlow),
    Step(StepFlow),
    GameOver,
    /// The M3 step-6 party-facing menu screens (character sheet, camp,
    /// save/load, training, shops) — additive, no VM vector runs here; each
    /// is a parked-widget screen (`crate::screens`).
    Screen(crate::screens::Screen),
}

impl Shell {
    pub fn boot(machine: &mut EclMachine, state: &mut EngineState) -> Self {
        Shell::Boot(BootFlow::start(machine, state))
    }

    /// `main_3d_world_menu`'s entry bookkeeping (`ovr015.cs:352`): zeroes
    /// `field_592` on *every* entry, no exceptions — the required
    /// "field_592 zeroing at menu entry" test target. Also recomposes the
    /// viewport (`crate::corridor::redraw_view`) — a deliberate, documented
    /// simplification of the original's sparser, flag-gated `RedrawView`
    /// call sites (step 5, task deliverable 4's design note): since the
    /// composited result is deterministic and immediate-mode redraws are
    /// idempotent (D-UI4), redrawing every time the player can see the
    /// world menu again is behaviorally equivalent to the original's own
    /// call-site choreography without needing to model the save-load-only
    /// `reload_ecl_and_pictures` gate this session's research found the
    /// original's boot-recompose path actually depends on.
    fn enter_world_menu(ctx: &mut FlowCtx) -> Shell {
        ctx.state.field_592 = 0;
        crate::corridor::redraw_view(ctx);
        let mut hotbar = Hotbar::new("Area Cast View Encamp Search Look");
        hotbar.accept_ext = true;
        Shell::WorldMenu {
            menu: Widget::Hotbar(hotbar),
        }
    }

    /// The D-UI7 mechanical property: true whenever a Widget is parked
    /// anywhere in the current state (a `Gate`, `WorldMenu`'s own menu, or a
    /// `StepFlow`'s door menu) — no vector may be pumped while this holds.
    pub fn gate_open(&self) -> bool {
        fn run_gated(run: &Option<VectorRun>) -> bool {
            matches!(run.as_ref().map(|r| &r.phase), Some(VmPhase::Gate(_)))
        }
        fn chain_gated(chain: &Option<ChainRunner>) -> bool {
            matches!(chain.as_ref().map(|c| &c.run.phase), Some(VmPhase::Gate(_)))
        }
        match self {
            Shell::Boot(b) => run_gated(&b.run) || chain_gated(&b.chain),
            Shell::WorldMenu { .. } => true,
            Shell::Look(l) => run_gated(&l.run) || chain_gated(&l.chain),
            Shell::Step(s) => s.door_widget.is_some() || run_gated(&s.run) || chain_gated(&s.chain),
            Shell::GameOver => false,
            // A screen always has a parked widget (its command bar/list); no
            // VM vector ever runs while one is open.
            Shell::Screen(_) => true,
        }
    }

    /// Advances the whole shell by one tick.
    pub fn tick(&mut self, ctx: &mut FlowCtx) {
        if ctx.state.party_killed {
            *self = Shell::GameOver;
            ctx.state.party_killed = false;
            return;
        }

        match self {
            Shell::Boot(flow) => {
                if flow.tick(ctx).is_some() {
                    ctx.state.last_selected_player = ctx.state.selected_player;
                    *self = Self::enter_world_menu(ctx);
                }
            }
            Shell::WorldMenu { menu } => {
                let outcome = menu.tick(ctx.input, ctx.dt_ticks);
                let WidgetOutcome::Hotbar(key) = outcome else {
                    return; // Pending, or a party-scroll outcome — handled below
                };
                ctx.state.last_selected_player = ctx.state.selected_player; // `:2319`/`:2353`
                if !ctx.state.chained {
                    ctx.state.last_ecl_block_id = ctx.state.ecl_block_id; // `:2321-2324`
                }
                self.dispatch_world_menu_key(key, ctx);
            }
            Shell::Look(flow) => {
                if flow.tick(ctx).is_some() {
                    ctx.state.last_selected_player = ctx.state.selected_player; // `:2353`
                    *self = Self::enter_world_menu(ctx);
                }
            }
            Shell::Step(flow) => {
                if flow.tick(ctx).is_some() {
                    *self = Self::enter_world_menu(ctx);
                }
            }
            Shell::GameOver => {}
            Shell::Screen(screen) => {
                use crate::screens::ScreenTransition;
                match screen.tick(ctx) {
                    ScreenTransition::Stay => {}
                    ScreenTransition::Exit => *self = Self::enter_world_menu(ctx),
                    ScreenTransition::To(next) => *self = Shell::Screen(next),
                }
            }
        }
    }

    fn dispatch_world_menu_key(&mut self, key: u8, ctx: &mut FlowCtx) {
        use WorldMenuCommand::*;
        let cmd = crate::movement::world_menu_command(key, ctx.state.area_view_allowed);
        match cmd {
            ToggleAreaView => {
                ctx.state.area_map_shown = !ctx.state.area_map_shown;
                *self = Self::enter_world_menu(ctx);
            }
            NotHere => {
                // A timed status wait inside the menu (§1.6): parked as a
                // Delay widget, same interaction layer as everything else.
                *self = Shell::WorldMenu {
                    menu: Widget::Delay(Delay::new(24)),
                };
            }
            View => {
                // The character sheet / party view (`ovr020.viewPlayer`),
                // returning to the walk loop on Exit (M3 step 6 deliverable 1).
                *self = Shell::Screen(crate::screens::Screen::PartyView(
                    crate::screens::PartyView::new(ctx, crate::screens::ReturnTo::World),
                ));
            }
            Encamp => {
                // The camp menu (`ovr016.MakeCamp`) — M3 step 6 deliverable 2.
                // (TryEncamp's vector 3/4 area-script dance is out of scope;
                // this enters the menu directly.)
                *self = Shell::Screen(crate::screens::Screen::Camp(crate::screens::Camp::new(ctx)));
            }
            Cast => {
                // M3 stub: casting is M5. Status text only, stays in the menu.
                *self = Self::enter_world_menu(ctx);
            }
            ToggleSearch => {
                ctx.state.search_flags ^= 1;
                *self = Self::enter_world_menu(ctx);
            }
            Look => {
                ctx.state.search_flags |= 2;
                ctx.state.clock.advance(true);
                *self = Shell::Look(LookFlow::start(ctx.machine, ctx.state));
            }
            Forward => {
                ctx.state.tried_to_exit_map =
                    try_step_forward(ctx.geo, ctx.state.pos, ctx.state.facing);
                *self = Shell::Step(StepFlow::start(ctx.machine, ctx.state));
            }
            TurnLeft => {
                ctx.state.facing = ctx.state.facing.turn_left();
                ctx.sounds.push(SoundEvent(crate::movement::SOUND_A));
                *self = Self::enter_world_menu(ctx);
            }
            TurnRight => {
                ctx.state.facing = ctx.state.facing.turn_right();
                ctx.sounds.push(SoundEvent(crate::movement::SOUND_A));
                *self = Self::enter_world_menu(ctx);
            }
            TurnAround => {
                ctx.state.facing = ctx.state.facing.turn_around(); // no sound (research finding)
                *self = Self::enter_world_menu(ctx);
            }
            ScrollParty(_) | None => {
                *self = Self::enter_world_menu(ctx);
            }
        }
    }

    /// The status line every command refreshes (§1.6): `"X,Y DIR HH:MM"`
    /// (+ `" search"`).
    pub fn status_line(state: &EngineState) -> String {
        position_time_text(state.pos, state.facing, &state.clock, state.search_mode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::DefaultPartyPredicates;
    use crate::test_support::{ecl_game_data, exit_only_block, labeled_block, simple_block};
    use gbx_formats::font;
    use gbx_vm::test_support::EclBuilder;

    fn open_geo() -> GeoBlock {
        GeoBlock::parse(&vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE]).unwrap()
    }

    fn marker_font() -> Font {
        let data = vec![0xFFu8; font::GLYPH_COUNT * font::GLYPH_BYTES];
        font::decode(&data)
    }

    /// A trivial single-item 8×8 `ImageBlock` — a fixture stand-in for a
    /// `SKY` block (moon/sun/horizon) this module's flow-control tests
    /// never actually render.
    fn empty_sky_block() -> gbx_formats::image::ImageBlock {
        gbx_formats::image::ImageBlock {
            height: 8,
            width_cols: 1,
            x_pos: 0,
            y_pos: 0,
            field_9: [0; 8],
            items: vec![gbx_formats::image::DecodedItem {
                pixels: vec![0; 64],
            }],
        }
    }

    const GAME_AREA: u8 = 2;

    struct Harness {
        machine: EclMachine,
        vm_memory: VmMemoryState,
        data: GameData,
        input: InputQueue,
        state: EngineState,
        geo: GeoBlock,
        party: DefaultPartyPredicates,
        roster: crate::party::Party,
        rules: gbx_rules::pack::RuleSet,
        slots: crate::saveload::SlotDirectory,
        io_request: Option<crate::saveload::SaveLoadRequest>,
        rng: EngineRng,
        fb: Framebuffer,
        font: Font,
        cursor: TextCursor,
        pacer: TextPacer,
        sounds: Vec<SoundEvent>,
        symbols: crate::symbols::SymbolSets,
        sky: [gbx_formats::image::ImageBlock; 3],
    }

    impl Harness {
        /// `blocks`' id `1` becomes the initial resident block; every id
        /// (including `1`) is also reachable via NEWECL/chaining through
        /// `data` (`"ECL2.DAX"`, matching `GAME_AREA`).
        fn with_blocks(blocks: Vec<(u8, EclBuilder)>) -> Self {
            let data = ecl_game_data(GAME_AREA, blocks);
            let initial = load_ecl_block(&data, GAME_AREA, 1).expect("block 1 must load");
            let machine = EclMachine::load_block(initial, &COTAB).unwrap_or_else(|e| match e {});
            Harness {
                machine,
                vm_memory: VmMemoryState::new(),
                data,
                input: InputQueue::new(),
                state: EngineState::new(),
                geo: open_geo(),
                party: DefaultPartyPredicates::default(),
                roster: crate::party::Party::default(),
                rules: gbx_rules::pack::RuleSet::load(),
                slots: crate::saveload::SlotDirectory::new(),
                io_request: None,
                rng: EngineRng::new(1),
                fb: Framebuffer::new(),
                font: marker_font(),
                cursor: TextCursor::new(),
                pacer: TextPacer::new(4),
                sounds: Vec::new(),
                symbols: crate::symbols::SymbolSets::new(),
                sky: [empty_sky_block(), empty_sky_block(), empty_sky_block()],
            }
        }

        fn new() -> Self {
            Self::with_blocks(vec![(1, exit_only_block())])
        }

        fn ctx(&mut self) -> FlowCtx<'_> {
            FlowCtx {
                machine: &mut self.machine,
                vm_memory: &mut self.vm_memory,
                data: &self.data,
                game_area: GAME_AREA,
                input: &mut self.input,
                dt_ticks: 1,
                state: &mut self.state,
                geo: &self.geo,
                party: &mut self.party,
                roster: &mut self.roster,
                rules: &self.rules,
                slots: &self.slots,
                io_request: &mut self.io_request,
                rng: &mut self.rng,
                fb: &mut self.fb,
                font: &self.font,
                cursor: &mut self.cursor,
                pacer: &mut self.pacer,
                sounds: &mut self.sounds,
                symbols: &mut self.symbols,
                sky: &self.sky,
            }
        }
    }

    /// Ticks at least once, then up to `max_ticks` times total, stopping as
    /// soon as `done` holds — always ticks first so a call starting already
    /// in the target state (e.g. queued input meant to move `shell` *out*
    /// of `WorldMenu` and back) still gets a chance to consume that input,
    /// rather than returning immediately without ticking at all.
    fn tick_until(
        shell: &mut Shell,
        h: &mut Harness,
        max_ticks: u32,
        done: impl Fn(&Shell) -> bool,
    ) {
        for _ in 0..max_ticks {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
            if done(shell) {
                return;
            }
        }
        assert!(done(shell), "did not converge within {max_ticks} ticks");
    }

    #[test]
    fn boot_reaches_world_menu_with_no_chain() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
    }

    #[test]
    fn boot_resume_after_chain_clears_reload_flag_only_after_the_chain_finishes() {
        let newecl = simple_block(|b| {
            b.op(0x20).imm_byte(2); // NEWECL block 2
        });
        let mut h = Harness::with_blocks(vec![(1, newecl), (2, exit_only_block())]);
        h.state.reload_ecl_and_pictures = true;
        let mut shell = Shell::boot(&mut h.machine, &mut h.state);

        {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            h.state.reload_ecl_and_pictures,
            "must not clear before the chain resolves"
        );
        assert!(h.state.chained);

        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        assert!(
            !h.state.reload_ecl_and_pictures,
            "must clear once resumed post-chain"
        );
        assert!(!h.state.chained);
        assert_eq!(h.state.ecl_block_id, 2);
    }

    #[test]
    fn world_menu_forward_into_open_square_moves_and_returns_to_world_menu() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });

        // Forward is driven by the extended "up" key (resolves through
        // accept_ext's ctrl-code table to 'H'), not a literal typed 'h'.
        h.input
            .push_all(&[crate::input::InputEvent::Ext(crate::input::ExtKey::Up)]);
        let start_pos = h.state.pos;
        tick_until(&mut shell, &mut h, 20, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        assert_ne!(
            h.state.pos, start_pos,
            "an open square must let the party step forward"
        );
    }

    #[test]
    fn party_killed_unwinds_to_game_over_and_resets_the_flag() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        h.state.party_killed = true;
        let mut ctx = h.ctx();
        shell.tick(&mut ctx);
        assert!(matches!(shell, Shell::GameOver));
        assert!(!h.state.party_killed, "the flag resets on unwind");
    }

    #[test]
    fn world_menu_with_chained_set_is_a_reachable_valid_state() {
        // The M3-camp-case invariant (D-UI2): WorldMenu can run with
        // `chained` still set — the flag survives the menu, only cleared at
        // the next step's checkpoint.
        let mut h = Harness::new();
        h.state.chained = true;
        let shell = Shell::enter_world_menu(&mut h.ctx());
        assert!(matches!(shell, Shell::WorldMenu { .. }));
        assert!(
            h.state.chained,
            "WorldMenu entry must not itself clear chained"
        );
    }

    #[test]
    fn field_592_zeroes_on_every_world_menu_entry() {
        let mut h = Harness::new();
        h.state.field_592 = 0xFF;
        let _ = Shell::enter_world_menu(&mut h.ctx());
        assert_eq!(h.state.field_592, 0);
    }

    #[test]
    fn no_vector_pumps_while_a_gate_is_open() {
        // Mechanical D-UI7 property: a widget requiring several ticks to
        // resolve must keep `gate_open()` true and never advance the
        // machine on its own (proven here by ticking many times with no
        // input and observing the gate never silently closes).
        let combat = simple_block(|b| {
            b.op(0x24); // COMBAT — a Request the interpreter can really emit
            b.op(0x00); // EXIT (after the reply resumes)
        });
        let mut h = Harness::with_blocks(vec![(1, combat)]);
        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            shell.gate_open(),
            "Combat's PressAnyKey stub must be parked"
        );
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            shell.gate_open(),
            "must still be gated with no input supplied"
        );
    }

    #[test]
    fn shell_state_round_trips_through_serde_mid_boot() {
        let mut h = Harness::new();
        let shell = Shell::boot(&mut h.machine, &mut h.state);
        let json = serde_json::to_string(&shell).expect("Shell must serialize");
        let restored: Shell = serde_json::from_str(&json).expect("Shell must deserialize");
        assert!(matches!(restored, Shell::Boot(_)));
    }

    #[test]
    fn widget_round_trips_through_serde() {
        let widget = Widget::Hotbar(Hotbar::new("Yes No"));
        let json = serde_json::to_string(&widget).unwrap();
        let restored: Widget = serde_json::from_str(&json).unwrap();
        assert_eq!(widget, restored);
    }

    #[test]
    fn look_flow_restores_search_flags_after_resolving() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        h.input.push_all(&[crate::input::InputEvent::Char(b'l')]);
        tick_until(&mut shell, &mut h, 15, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        assert_eq!(h.state.search_flags, 0, "bit 1 must never survive a Look");
    }

    #[test]
    fn door_menu_parks_directly_in_step_flow_not_via_vmphase_gate_over_a_vector() {
        // The Fable review finding, proven structurally: a locked door with
        // no VM run active still reports a parked widget.
        let mut geo_data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        // Square (0,0): North wall present, door state 2 (locked).
        geo_data[2] = 5 << 4;
        geo_data[2 + 3 * 256] = 0b10; // door_north = 2
        let geo = GeoBlock::parse(&geo_data).unwrap();

        let mut h = Harness::new();
        h.geo = geo;
        h.party.can_pick = false;
        h.party.can_knock = false;
        let mut flow = StepFlow::start(&mut h.machine, &mut h.state);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            let _ = flow.tick(&mut ctx);
            if flow.door_widget_is_some() {
                break;
            }
        }
        assert!(
            flow.door_widget_is_some(),
            "the Bash/Exit menu must be parked directly"
        );
    }

    #[test]
    fn combat_request_maps_to_press_any_key_stub() {
        let options = vec![gbx_vm::VmString::from_bytes(*b"Yes")];
        let w = widget_for_request(&Request::HorizontalMenu { options });
        assert!(matches!(w, Widget::Hotbar(_)));
        let w = widget_for_request(&Request::Combat);
        assert!(matches!(w, Widget::PressAnyKey(_)));
        let w = widget_for_request(&Request::Delay);
        assert!(matches!(w, Widget::Delay(_)));
    }

    fn round_trip_shell(shell: &Shell) -> Shell {
        let json = serde_json::to_string(shell).expect("Shell must serialize");
        serde_json::from_str(&json).expect("Shell must deserialize")
    }

    #[test]
    fn every_shell_variant_round_trips_through_serde() {
        let mut h = Harness::new();

        assert!(matches!(
            round_trip_shell(&Shell::GameOver),
            Shell::GameOver
        ));

        let world_menu = Shell::enter_world_menu(&mut h.ctx());
        assert!(matches!(
            round_trip_shell(&world_menu),
            Shell::WorldMenu { .. }
        ));

        let step = Shell::Step(StepFlow::start(&mut h.machine, &mut h.state));
        assert!(matches!(round_trip_shell(&step), Shell::Step(_)));

        let look = Shell::Look(LookFlow::start(&mut h.machine, &mut h.state));
        assert!(matches!(round_trip_shell(&look), Shell::Look(_)));

        let boot = Shell::boot(&mut h.machine, &mut h.state);
        assert!(matches!(round_trip_shell(&boot), Shell::Boot(_)));
    }

    #[test]
    fn every_widget_variant_round_trips_through_serde() {
        fn round_trip(w: &Widget) -> Widget {
            let json = serde_json::to_string(w).unwrap();
            serde_json::from_str(&json).unwrap()
        }
        let variants = vec![
            Widget::Hotbar(Hotbar::new("Yes No")),
            Widget::ListMenu(crate::widgets::ListMenu::new(
                vec![crate::widgets::ListItem::Entry("x".into())],
                3,
            )),
            Widget::TextEntry(crate::widgets::TextEntry::new("Name?", 10, false)),
            Widget::PressAnyKey(PressAnyKey),
            Widget::Delay(Delay::new(5)),
        ];
        for w in variants {
            assert_eq!(round_trip(&w), w);
        }
    }

    #[test]
    fn forward_at_the_grid_edge_sets_tried_to_exit_map() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        // Facing North at y=0: stepping forward would exit the 16x16 grid.
        assert_eq!(h.state.pos, (0, 0));
        assert_eq!(h.state.facing, Facing::North);
        h.input
            .push_all(&[crate::input::InputEvent::Ext(crate::input::ExtKey::Up)]);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            h.state.tried_to_exit_map,
            "stepping off the grid must set the flag"
        );
    }

    #[test]
    fn solid_wall_blocks_movement_with_no_menu() {
        let mut geo_data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        geo_data[2] = 5 << 4; // North wall present, door state 0 = solid
        let geo = GeoBlock::parse(&geo_data).unwrap();

        let mut h = Harness::new();
        h.geo = geo;
        let mut flow = StepFlow::start(&mut h.machine, &mut h.state);
        let start_pos = h.state.pos;
        for _ in 0..5 {
            let mut ctx = h.ctx();
            if flow.tick(&mut ctx).is_some() {
                break;
            }
        }
        assert_eq!(h.state.pos, start_pos, "a solid wall must block the step");
    }

    #[test]
    fn unpickable_door_pick_never_succeeds_via_the_full_shell_path() {
        let mut geo_data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        geo_data[2] = 5 << 4; // North wall present
        geo_data[2 + 3 * 256] = 0b11; // door_north = 3 (unpickable)
        let geo = GeoBlock::parse(&geo_data).unwrap();

        let mut h = Harness::new();
        h.geo = geo;
        h.party.can_pick = true;
        h.party.pick_succeeds = true; // would succeed if it ever rolled
        let mut flow = StepFlow::start(&mut h.machine, &mut h.state);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            let _ = flow.tick(&mut ctx);
            if flow.door_widget_is_some() {
                break;
            }
        }
        assert!(flow.door_widget_is_some());
        h.input.push_all(&[crate::input::InputEvent::Char(b'p')]);
        let start_pos = h.state.pos;
        for _ in 0..5 {
            let mut ctx = h.ctx();
            if flow.tick(&mut ctx).is_some() {
                break;
            }
        }
        assert_eq!(
            h.state.pos, start_pos,
            "an unpickable door never opens via Pick"
        );
        assert!(
            !h.state.door_flags.can_pick,
            "Pick is disabled after the attempt regardless"
        );
    }

    #[test]
    fn chain_during_look_resumes_at_restore_search_flags_not_abandoned() {
        // Block 1: vector[4] (entry) is a trivial EXIT so Boot reaches
        // WorldMenu normally; vector[1] (SearchLocationAddr, the one Look's
        // vector 2 fires) is a separate label that NEWECLs to block 9 —
        // proving the chain fires specifically from the Look site.
        let block1 = labeled_block(["entry", "search", "entry", "entry", "entry"], |b| {
            b.label("entry");
            b.op(0x00); // EXIT
            b.label("search");
            b.op(0x20).imm_byte(9); // NEWECL block 9
        });
        let mut h = Harness::with_blocks(vec![(1, block1), (9, exit_only_block())]);

        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });

        h.input.push_all(&[crate::input::InputEvent::Char(b'l')]);
        for _ in 0..15 {
            if matches!(shell, Shell::WorldMenu { .. }) && h.state.ecl_block_id == 9 {
                break;
            }
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(matches!(shell, Shell::WorldMenu { .. }));
        assert_eq!(h.state.ecl_block_id, 9);
        assert!(
            !h.state.chained,
            "the chain runner must finish and clear the flag"
        );
        assert_eq!(
            h.state.search_flags, 0,
            "search_flags restore must still run after the chain resolves (resume-after-chain)"
        );
    }

    // --- M3 step 6: party-facing menu screens (View/Camp/Magic) ---

    use crate::screens::Screen;

    fn test_char(name: &str) -> crate::party::Character {
        use gbx_formats::save_orig::{decode_char_record, CHAR_RECORD_SIZE};
        let mut bytes = vec![0u8; CHAR_RECORD_SIZE];
        bytes[0] = name.len() as u8;
        bytes[1..1 + name.len()].copy_from_slice(name.as_bytes());
        let rec = decode_char_record(&bytes).unwrap();
        crate::party::character_from_record(&rec, vec![], vec![])
    }

    fn char_key(c: u8) -> crate::input::InputEvent {
        crate::input::InputEvent::Char(c)
    }

    /// Boots to the world menu with a two-member roster.
    fn boot_with_party() -> (Shell, Harness) {
        let mut h = Harness::new();
        h.roster.members = vec![test_char("Aran"), test_char("Bink")];
        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        (shell, h)
    }

    #[test]
    fn world_menu_view_opens_the_party_view_and_exit_returns() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'v')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::PartyView(_)))
        });
        // The command bar for a no-money character is just "Exit"; 'E' resolves
        // it and returns to the walk-loop world menu.
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
    }

    #[test]
    fn world_menu_encamp_opens_camp_and_exit_returns() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'e')]); // camp Exit ("Exit" is the sole 'E' word)
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
    }

    #[test]
    fn camp_view_returns_to_camp_not_the_world_menu() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'v')]); // camp View
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::PartyView(_)))
        });
        // Escape leaves the sheet — but back to camp, not the world menu.
        h.input.push_all(&[crate::input::InputEvent::Escape]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
    }

    #[test]
    fn camp_magic_opens_the_magic_submenu_and_returns() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'm')]); // Magic
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Magic(_)))
        });
        h.input.push_all(&[char_key(b'e')]); // Magic Exit → camp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
    }

    #[test]
    fn camp_rest_stays_in_camp_and_does_not_touch_spell_state() {
        // FD-25: rest reports without mutating spell state this milestone.
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        let before = h.roster.members[0].magic.clone();
        h.input.push_all(&[char_key(b'r')]); // Rest
                                             // Rest stays in camp (a status line, no transition).
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(matches!(shell, Shell::Screen(Screen::Camp(_))));
        assert_eq!(
            h.roster.members[0].magic, before,
            "rest must not fake a spell-slot restoration (FD-25)"
        );
    }

    fn feed_and_settle(shell: &mut Shell, h: &mut Harness, key: u8) {
        h.input.push_all(&[char_key(key)]);
        for _ in 0..4 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
    }

    #[test]
    fn camp_save_opens_saveload_and_emits_a_save_request() {
        use crate::saveload::SaveLoadRequest;
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b's')]); // Save → SaveLoad
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::SaveLoad(_)))
        });
        feed_and_settle(&mut shell, &mut h, b's'); // choose Save action
        h.input.push_all(&[char_key(b'a')]); // pick slot A
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        assert_eq!(h.io_request, Some(SaveLoadRequest::Save('A')));
    }

    #[test]
    fn saveload_load_emits_load_for_restrike_and_import_for_original() {
        use crate::saveload::{SaveLoadRequest, SlotStatus};
        let (mut shell, mut h) = boot_with_party();
        h.slots.set('B', SlotStatus::RestrikeSave);
        h.slots.set('C', SlotStatus::OriginalSave);

        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b's')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::SaveLoad(_)))
        });
        feed_and_settle(&mut shell, &mut h, b'l'); // Load action
                                                   // A restrike slot → Load; an original slot → ImportOriginal.
        feed_and_settle(&mut shell, &mut h, b'b');
        assert_eq!(h.io_request, Some(SaveLoadRequest::Load('B')));
        // Emitting returned us to camp (ReturnTo::Camp).
        assert!(matches!(shell, Shell::Screen(Screen::Camp(_))));

        // Re-open save/load from camp and test the original-import path.
        h.io_request = None;
        h.input.push_all(&[char_key(b's')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::SaveLoad(_)))
        });
        feed_and_settle(&mut shell, &mut h, b'l');
        feed_and_settle(&mut shell, &mut h, b'c');
        assert_eq!(h.io_request, Some(SaveLoadRequest::ImportOriginal('C')));
    }

    #[test]
    fn training_screen_levels_up_an_eligible_member() {
        use crate::screens::{ReturnTo, Training};
        let mut h = Harness::new();
        let mut fighter = test_char("Gareth");
        fighter.class_level = [0; 8];
        fighter.class_level[2] = 1; // fighter level 1
        fighter.exp = 3000; // > the 2001 needed for L1→L2
        fighter.hit_dice = 1;
        fighter.multiclass_level = 0;
        fighter.stats.con.current = 12; // a valid CON (no HP adjustment)
        fighter.money.gold = 2000;
        fighter.hit_point_max = 20;
        fighter.hit_point_current = 20;
        h.roster.members = vec![fighter];

        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        shell = Shell::Screen(Screen::Training(Training::new(0, ReturnTo::World)));

        h.input.push_all(&[char_key(b't')]); // Train
        for _ in 0..4 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.roster.members[0].class_level[2], 2, "fighter leveled up");
        assert_eq!(h.roster.members[0].exp, 3000, "exp not consumed");

        h.input.push_all(&[char_key(b'e')]); // Exit → world menu
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
    }

    #[test]
    fn shop_screen_buys_an_item_and_updates_money_and_weight() {
        use crate::screens::{Screen, Shop as ShopScreen};
        use crate::shop::{Shop, ShopItem};

        let mut h = Harness::new();
        let mut buyer = test_char("Rich");
        buyer.money.gold = 100;
        buyer.combat.weight = 0;
        h.roster.members = vec![buyer];
        h.state.selected_player = 0;

        let mut shell = Shell::boot(&mut h.machine, &mut h.state);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });

        let shop = Shop::new(vec![ShopItem::synthetic("Dagger", 2, 10)], 0x00);
        shell = Shell::Screen(Screen::Shop(ShopScreen::new(shop)));

        // Buy → enter the item list.
        h.input.push_all(&[char_key(b'b')]);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        // Select the first (highlighted) item.
        h.input.push_all(&[crate::input::InputEvent::Enter]);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.roster.members[0].items.len(), 1, "item bought");
        assert_eq!(h.roster.members[0].combat.weight, 10, "encumbrance updated");
        assert_eq!(
            crate::money::gold_worth(&h.roster.members[0].money, &h.rules),
            98,
            "paid 2 gp"
        );
    }

    #[test]
    fn party_view_scrolls_between_members() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'v')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::PartyView(_)))
        });
        assert_eq!(h.state.selected_player, 0);
        // Down (ctrl 'P') advances to the next member.
        h.input
            .push_all(&[crate::input::InputEvent::Ext(crate::input::ExtKey::Down)]);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.state.selected_player, 1);
    }
}
