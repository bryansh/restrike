//! The UI shell state machine (D-UI2, task deliverable 3): `Shell`,
//! `VmPhase`, the flow plans (`BootFlow`/`LookFlow`/`StepFlow`) with chain
//! checkpoints and resume-after-chain, the persistent `chained`/
//! `party_killed` engine state, and the walk-loop's world-menu dispatch.
//! The VM itself is [`crate::vm_stub::StubVm`] this session (real
//! `EclMachine` binding is step 4) — every "run a vector" site pulls from a
//! test-scripted step sequence.
//!
//! **Fable review finding, addressed explicitly (binding for this session,
//! per the task brief):** the design doc's prose says every blocking site is
//! a Widget parked in `VmPhase::Gate` or `WorldMenu` — but the locked-door
//! menu lives in a [`StepFlow`] stage (`StepStage::DoorInteraction`), which
//! is neither: no VM vector is running during a door prompt at all. The fix
//! applied here is the doc's own suggested alternative, "Gate generalizes to
//! flows": [`VmPhase::Gate`] is not exclusive to `VectorRun`s — any flow
//! stage may park a `Widget` in it directly (`StepStage::DoorInteraction`
//! does exactly this, with no VM involvement whatsoever). There is nowhere
//! left for a blocking interaction to hide outside this one mechanism.
//!
//! Derived by reading coab for behavior (D11, never copied) — see
//! `movement.rs`'s citations for `ovr015.cs`/`ovr031.cs`; this module's own
//! citations are to `engine/ovr003.cs` `sub_29758` (the walk loop,
//! `:2230-2396`) and `sub_29677` (the chain runner, `:2180-2227`), pinned to
//! exact sequencing by this session's research pass.

use crate::framebuffer::Framebuffer;
use crate::input::InputQueue;
use crate::movement::{
    attempt_bash, attempt_knock, attempt_pick, build_door_hotbar, move_party_forward,
    position_time_text, try_step_forward, wall_door_flags, DoorState, DoorStepFlags, Facing,
    GameClock, PartyPredicates, WorldMenuCommand,
};
use crate::text::{JobStatus, TextCursor, TextJob, TextPacer, NORMAL_BOTTOM};
use crate::vm_stub::StubVm;
use crate::widgets::{Delay, Hotbar, PressAnyKey, Widget, WidgetOutcome};
use gbx_formats::font::Font;
use gbx_formats::geo::GeoBlock;
use gbx_vm::{BlockId, Effect, Exit, Request, VmRng, VmStep};
use std::collections::VecDeque;

/// One audio cue this tick (D-UI1's `Frame::sounds` — M8 synthesizes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SoundEvent(pub u8);

/// The Fuel watchdog (D-UI2's obligations table): a vector run steps at
/// most this many times per tick before yielding, so a `GOTO`-self script
/// can't hang the app.
const STEP_BUDGET: u32 = 10_000;

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

/// One vector's execution against [`StubVm`]: pumps steps, buffers `Effect`s
/// into an ordered presentation queue, drains that queue (pacing text
/// through [`TextJob`], gating on pagination) before any `Request`'s Widget
/// opens — the D-VM3 ordering obligation, mechanically enforced by this
/// struct's own phase order.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VectorRun {
    phase: VmPhase,
    queue: VecDeque<Effect>,
    current_job: Option<TextJob>,
    pending: Option<PendingOutcome>,
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
    pub vm: &'a mut StubVm,
    pub input: &'a mut InputQueue,
    pub dt_ticks: u32,
    pub state: &'a mut EngineState,
    pub geo: &'a GeoBlock,
    pub party: &'a mut dyn PartyPredicates,
    pub rng: &'a mut dyn VmRng,
    pub fb: &'a mut Framebuffer,
    pub font: &'a Font,
    pub cursor: &'a mut TextCursor,
    pub pacer: &'a mut TextPacer,
    pub sounds: &'a mut Vec<SoundEvent>,
}

/// `Request` -> `Widget` (design doc's table, M2 slice). Engine-owned
/// interactions (world menu, door menu, pagination) never go through this —
/// only a real `VectorRun`'s `Request` does.
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

impl VectorRun {
    /// `machine.enter` + the first pump (`RunVector(n)`, §1.6).
    pub fn start(vm: &mut StubVm) -> Self {
        vm.enter();
        VectorRun {
            phase: VmPhase::Pump,
            queue: VecDeque::new(),
            current_job: None,
            pending: None,
        }
    }

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

    /// Pumps up to [`STEP_BUDGET`] steps. Returns `true` once a `Request`
    /// or `Done` is pending (phase advances to `Present`), `false` if the
    /// budget ran out first (the Fuel watchdog, D-UI2's obligations table).
    fn tick_pump(&mut self, ctx: &mut FlowCtx) -> bool {
        for _ in 0..STEP_BUDGET {
            match ctx.vm.advance() {
                VmStep::Continue => continue,
                VmStep::Effect(e) => self.queue.push_back(e),
                VmStep::Request(r) => {
                    self.pending = Some(PendingOutcome::Request(r));
                    break;
                }
                VmStep::Done(exit) => {
                    self.pending = Some(PendingOutcome::Exit(exit));
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
                // Picture/animation *rendering* is step 5 scope; the effect
                // is consumed (drained) here so the queue-before-gate
                // ordering obligation still holds for it.
                Effect::Picture(_) | Effect::ClearPicture | Effect::AnimationFrame => {}
            }
        }

        match self
            .pending
            .take()
            .expect("Present entered with no pending outcome")
        {
            PendingOutcome::Exit(exit) => PresentTick::Done(exit),
            PendingOutcome::Request(request) => {
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
                self.phase = VmPhase::Present;
                return true;
            }
            return false;
        }

        if matches!(outcome, WidgetOutcome::Pending) {
            return false;
        }
        // Any resolution: the pending Request is answered (the real Reply
        // value is unneeded — StubVm's script already bakes in the
        // consequence, see vm_stub.rs's doc comment) and pumping resumes.
        self.pending = None;
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
    pub fn start(vm: &mut StubVm) -> Self {
        ChainRunner {
            run: VectorRun::start(vm),
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<ChainRunnerOutcome> {
        match self.run.tick(ctx) {
            RunTick::Working => None,
            RunTick::Done(Exit::Ended) => Some(ChainRunnerOutcome::Finished),
            RunTick::Done(Exit::ChainTo(id)) => Some(ChainRunnerOutcome::ChainedAgain(id)),
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
            ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
            ctx.state.ecl_block_id = id.0;
            ctx.state.last_selected_player = ctx.state.selected_player;
            *chain = Some(ChainRunner::start(ctx.vm));
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
        }
    }

    fn search_mode(&self) -> bool {
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
    pub fn start(vm: &mut StubVm, state: &mut EngineState) -> Self {
        state.last_selected_player = state.selected_player; // `:2232`
        BootFlow {
            stage: BootStage::EntryVector,
            run: Some(VectorRun::start(vm)),
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
                let run = self.run.as_mut().expect("EntryVector always has a run");
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        ctx.state.last_ecl_block_id = ctx.state.ecl_block_id; // `:2292-2294`
                        self.run = None;
                        self.stage = BootStage::PostChainResume;
                        None
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        // NEWECL's own write (`ovr003.cs:488`): the *old*
                        // id lands in LastEclBlockId before the swap.
                        ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
                        ctx.state.ecl_block_id = id.0;
                        ctx.state.chained = true;
                        self.run = None;
                        self.chain = Some(ChainRunner::start(ctx.vm));
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
    pub fn start(vm: &mut StubVm, state: &mut EngineState) -> Self {
        let backup = state.search_flags & 1;
        state.search_flags = 1;
        LookFlow {
            stage: LookStage::RunVector2,
            run: Some(VectorRun::start(vm)),
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
                let run = self.run.as_mut().expect("RunVector2 always has a run");
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        self.run = None;
                        self.stage = LookStage::RestoreSearchFlags;
                        None
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
                        ctx.state.ecl_block_id = id.0;
                        ctx.state.chained = true;
                        self.run = None;
                        self.chain = Some(ChainRunner::start(ctx.vm));
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
    pub fn start(vm: &mut StubVm, state: &mut EngineState) -> Self {
        StepFlow {
            stage: StepStage::RunVector1,
            run: Some(VectorRun::start(vm)),
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
                let run = self.run.as_mut().expect("RunVector1 always has a run");
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        self.run = None;
                        self.last_pos = ctx.state.pos;
                        self.stage = StepStage::DoorInteraction;
                        None
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
                        ctx.state.ecl_block_id = id.0;
                        ctx.state.chained = true;
                        self.run = None;
                        self.chain = Some(ChainRunner::start(ctx.vm));
                        None
                    }
                }
            }
            StepStage::DoorInteraction => self.tick_door_interaction(ctx),
            StepStage::RunVector2 => {
                let run = self.run.as_mut().expect("RunVector2 always has a run");
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        self.run = None;
                        self.stage = StepStage::Done;
                        Some(())
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
                        ctx.state.ecl_block_id = id.0;
                        ctx.state.chained = true;
                        self.run = None;
                        self.chain = Some(ChainRunner::start(ctx.vm));
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
        self.run = Some(VectorRun::start(ctx.vm));
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
    WorldMenu { menu: Widget },
    Look(LookFlow),
    Step(StepFlow),
    GameOver,
}

impl Shell {
    pub fn boot(vm: &mut StubVm, state: &mut EngineState) -> Self {
        Shell::Boot(BootFlow::start(vm, state))
    }

    /// `main_3d_world_menu`'s entry bookkeeping (`ovr015.cs:352`): zeroes
    /// `field_592` on *every* entry, no exceptions — the required
    /// "field_592 zeroing at menu entry" test target.
    fn enter_world_menu(state: &mut EngineState) -> Shell {
        state.field_592 = 0;
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
        match self {
            Shell::Boot(b) => {
                matches!(b.run.as_ref().map(|r| &r.phase), Some(VmPhase::Gate(_)))
                    || matches!(
                        b.chain.as_ref().map(|c| &c.run.phase),
                        Some(VmPhase::Gate(_))
                    )
            }
            Shell::WorldMenu { .. } => true,
            Shell::Look(l) => {
                matches!(l.run.as_ref().map(|r| &r.phase), Some(VmPhase::Gate(_)))
                    || matches!(
                        l.chain.as_ref().map(|c| &c.run.phase),
                        Some(VmPhase::Gate(_))
                    )
            }
            Shell::Step(s) => {
                s.door_widget.is_some()
                    || matches!(s.run.as_ref().map(|r| &r.phase), Some(VmPhase::Gate(_)))
                    || matches!(
                        s.chain.as_ref().map(|c| &c.run.phase),
                        Some(VmPhase::Gate(_))
                    )
            }
            Shell::GameOver => false,
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
                    *self = Self::enter_world_menu(ctx.state);
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
                    *self = Self::enter_world_menu(ctx.state);
                }
            }
            Shell::Step(flow) => {
                if flow.tick(ctx).is_some() {
                    *self = Self::enter_world_menu(ctx.state);
                }
            }
            Shell::GameOver => {}
        }
    }

    fn dispatch_world_menu_key(&mut self, key: u8, ctx: &mut FlowCtx) {
        use WorldMenuCommand::*;
        let cmd = crate::movement::world_menu_command(key, ctx.state.area_view_allowed);
        match cmd {
            ToggleAreaView => {
                ctx.state.area_map_shown = !ctx.state.area_map_shown;
                *self = Self::enter_world_menu(ctx.state);
            }
            NotHere => {
                // A timed status wait inside the menu (§1.6): parked as a
                // Delay widget, same interaction layer as everything else.
                *self = Shell::WorldMenu {
                    menu: Widget::Delay(Delay::new(24)),
                };
            }
            Cast | View | Encamp => {
                // M3 stub: status text only, stays in the menu (task scope
                // cut — TryEncamp's vector 3/4 dance is M3 party/camp UI).
                *self = Self::enter_world_menu(ctx.state);
            }
            ToggleSearch => {
                ctx.state.search_flags ^= 1;
                *self = Self::enter_world_menu(ctx.state);
            }
            Look => {
                ctx.state.search_flags |= 2;
                ctx.state.clock.advance(true);
                *self = Shell::Look(LookFlow::start(ctx.vm, ctx.state));
            }
            Forward => {
                ctx.state.tried_to_exit_map =
                    try_step_forward(ctx.geo, ctx.state.pos, ctx.state.facing);
                *self = Shell::Step(StepFlow::start(ctx.vm, ctx.state));
            }
            TurnLeft => {
                ctx.state.facing = ctx.state.facing.turn_left();
                ctx.sounds.push(SoundEvent(crate::movement::SOUND_A));
                *self = Self::enter_world_menu(ctx.state);
            }
            TurnRight => {
                ctx.state.facing = ctx.state.facing.turn_right();
                ctx.sounds.push(SoundEvent(crate::movement::SOUND_A));
                *self = Self::enter_world_menu(ctx.state);
            }
            TurnAround => {
                ctx.state.facing = ctx.state.facing.turn_around(); // no sound (research finding)
                *self = Self::enter_world_menu(ctx.state);
            }
            ScrollParty(_) | None => {
                *self = Self::enter_world_menu(ctx.state);
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
    use crate::rng::EngineRng;
    use gbx_formats::font;
    use gbx_vm::VmString;

    fn open_geo() -> GeoBlock {
        GeoBlock::parse(&vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE]).unwrap()
    }

    fn marker_font() -> Font {
        let data = vec![0xFFu8; font::GLYPH_COUNT * font::GLYPH_BYTES];
        font::decode(&data)
    }

    struct Harness {
        vm: StubVm,
        input: InputQueue,
        state: EngineState,
        geo: GeoBlock,
        party: DefaultPartyPredicates,
        rng: EngineRng,
        fb: Framebuffer,
        font: Font,
        cursor: TextCursor,
        pacer: TextPacer,
        sounds: Vec<SoundEvent>,
    }

    impl Harness {
        fn new() -> Self {
            Harness {
                vm: StubVm::new(),
                input: InputQueue::new(),
                state: EngineState::new(),
                geo: open_geo(),
                party: DefaultPartyPredicates::default(),
                rng: EngineRng::new(1),
                fb: Framebuffer::new(),
                font: marker_font(),
                cursor: TextCursor::new(),
                pacer: TextPacer::new(4),
                sounds: Vec::new(),
            }
        }

        fn ctx(&mut self) -> FlowCtx<'_> {
            FlowCtx {
                vm: &mut self.vm,
                input: &mut self.input,
                dt_ticks: 1,
                state: &mut self.state,
                geo: &self.geo,
                party: &mut self.party,
                rng: &mut self.rng,
                fb: &mut self.fb,
                font: &self.font,
                cursor: &mut self.cursor,
                pacer: &mut self.pacer,
                sounds: &mut self.sounds,
            }
        }
    }

    fn ended() -> VmStep {
        VmStep::Done(Exit::Ended)
    }

    #[test]
    fn boot_reaches_world_menu_with_no_chain() {
        let mut h = Harness::new();
        h.vm.script_call(vec![ended()]);
        let mut shell = Shell::boot(&mut h.vm, &mut h.state);
        for _ in 0..10 {
            if matches!(shell, Shell::WorldMenu { .. }) {
                break;
            }
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(matches!(shell, Shell::WorldMenu { .. }));
    }

    #[test]
    fn boot_resume_after_chain_clears_reload_flag_only_after_the_chain_finishes() {
        let mut h = Harness::new();
        h.state.reload_ecl_and_pictures = true;
        h.vm.script_call(vec![VmStep::Done(Exit::ChainTo(BlockId(2)))]);
        h.vm.script_call(vec![ended()]);
        let mut shell = Shell::boot(&mut h.vm, &mut h.state);

        // First tick: the entry vector chains — reload flag must still be set.
        {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            h.state.reload_ecl_and_pictures,
            "must not clear before the chain resolves"
        );
        assert!(h.state.chained);

        for _ in 0..10 {
            if matches!(shell, Shell::WorldMenu { .. }) {
                break;
            }
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(matches!(shell, Shell::WorldMenu { .. }));
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
        h.vm.script_call(vec![ended()]); // boot entry vector
        let mut shell = Shell::boot(&mut h.vm, &mut h.state);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
            if matches!(shell, Shell::WorldMenu { .. }) {
                break;
            }
        }
        assert!(matches!(shell, Shell::WorldMenu { .. }));

        h.vm.script_call(vec![ended()]); // vector 1
        h.vm.script_call(vec![ended()]); // vector 2
                                         // Forward is driven by the extended "up" key (resolves through
                                         // accept_ext's ctrl-code table to 'H'), not a literal typed 'h'.
        h.input
            .push_all(&[crate::input::InputEvent::Ext(crate::input::ExtKey::Up)]);
        let start_pos = h.state.pos;
        for _ in 0..20 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
            if matches!(shell, Shell::WorldMenu { .. }) && h.state.pos != start_pos {
                break;
            }
        }
        assert_ne!(
            h.state.pos, start_pos,
            "an open square must let the party step forward"
        );
        assert!(matches!(shell, Shell::WorldMenu { .. }));
    }

    #[test]
    fn party_killed_unwinds_to_game_over_and_resets_the_flag() {
        let mut h = Harness::new();
        h.vm.script_call(vec![ended()]);
        let mut shell = Shell::boot(&mut h.vm, &mut h.state);
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
        let shell = Shell::enter_world_menu(&mut h.state);
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
        let _ = Shell::enter_world_menu(&mut h.state);
        assert_eq!(h.state.field_592, 0);
    }

    #[test]
    fn no_vector_pumps_while_a_gate_is_open() {
        // Mechanical D-UI7 property: whenever `gate_open()` is true, a
        // direct `StubVm::advance()` call must not be reachable from
        // `Shell::tick` — proven here by observing that a widget requiring
        // several ticks to resolve never lets the underlying run advance
        // (StubVm has no more scripted steps, so any accidental advance
        // would panic).
        let mut h = Harness::new();
        h.vm.script_call(vec![VmStep::Request(Request::Combat), ended()]);
        let mut shell = Shell::boot(&mut h.vm, &mut h.state);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            shell.gate_open(),
            "Combat's PressAnyKey stub must be parked"
        );
        // Ticking repeatedly with no input must not panic (would happen if
        // tick_gate ever fell through to advance() while pending).
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
        h.vm.script_call(vec![ended()]);
        let shell = Shell::boot(&mut h.vm, &mut h.state);
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
        h.vm.script_call(vec![ended()]); // boot entry
        let mut shell = Shell::boot(&mut h.vm, &mut h.state);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
            if matches!(shell, Shell::WorldMenu { .. }) {
                break;
            }
        }
        h.vm.script_call(vec![ended()]); // vector 2 (Look)
        h.input.push_all(&[crate::input::InputEvent::Char(b'l')]);
        for _ in 0..10 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
            if matches!(shell, Shell::WorldMenu { .. }) && h.state.search_flags <= 1 {
                break;
            }
        }
        assert!(matches!(shell, Shell::WorldMenu { .. }));
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
        h.vm.script_call(vec![ended()]); // vector 1
        let mut flow = StepFlow::start(&mut h.vm, &mut h.state);
        {
            let mut ctx = h.ctx();
            let _ = flow.tick(&mut ctx); // pump vector 1 to Done
        }
        {
            let mut ctx = h.ctx();
            let _ = flow.tick(&mut ctx); // door interaction opens the menu
        }
        assert!(
            flow.door_widget.is_some(),
            "the Bash/Exit menu must be parked directly"
        );
    }

    #[test]
    fn combat_request_maps_to_press_any_key_stub() {
        let options = vec![VmString::from_bytes(*b"Yes")];
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

        let world_menu = Shell::enter_world_menu(&mut h.state);
        assert!(matches!(
            round_trip_shell(&world_menu),
            Shell::WorldMenu { .. }
        ));

        h.vm.script_call(vec![ended()]);
        let step = Shell::Step(StepFlow::start(&mut h.vm, &mut h.state));
        assert!(matches!(round_trip_shell(&step), Shell::Step(_)));

        h.vm.script_call(vec![ended()]);
        let look = Shell::Look(LookFlow::start(&mut h.vm, &mut h.state));
        assert!(matches!(round_trip_shell(&look), Shell::Look(_)));

        h.vm.script_call(vec![ended()]);
        let boot = Shell::boot(&mut h.vm, &mut h.state);
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
                0,
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
        h.vm.script_call(vec![ended()]); // boot entry
        let mut shell = Shell::boot(&mut h.vm, &mut h.state);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
            if matches!(shell, Shell::WorldMenu { .. }) {
                break;
            }
        }
        // Facing North at y=0: stepping forward would exit the 16x16 grid.
        assert_eq!(h.state.pos, (0, 0));
        assert_eq!(h.state.facing, Facing::North);
        h.vm.script_call(vec![ended()]); // vector 1
        h.vm.script_call(vec![ended()]); // vector 2
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
        h.vm.script_call(vec![ended()]); // vector 1
        h.vm.script_call(vec![ended()]); // vector 2 (door interaction proceeds synchronously)
        let mut flow = StepFlow::start(&mut h.vm, &mut h.state);
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
        h.vm.script_call(vec![ended()]); // vector 1
        h.vm.script_call(vec![ended()]); // vector 2 (after the door attempt resolves)
        let mut flow = StepFlow::start(&mut h.vm, &mut h.state);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            let _ = flow.tick(&mut ctx);
            if flow.door_widget.is_some() {
                break;
            }
        }
        assert!(flow.door_widget.is_some());
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
        let mut h = Harness::new();
        h.vm.script_call(vec![ended()]); // boot entry
        let mut shell = Shell::boot(&mut h.vm, &mut h.state);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
            if matches!(shell, Shell::WorldMenu { .. }) {
                break;
            }
        }

        h.vm.script_call(vec![VmStep::Done(Exit::ChainTo(BlockId(9)))]); // vector 2 chains
        h.vm.script_call(vec![ended()]); // chain runner's entry vector
        h.input.push_all(&[crate::input::InputEvent::Char(b'l')]);
        for _ in 0..15 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
            if matches!(shell, Shell::WorldMenu { .. }) && h.state.search_flags <= 1 {
                break;
            }
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
}
