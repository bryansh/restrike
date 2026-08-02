//! The **combat scene** (`docs/design/combat-visualizer.md` D-CV1/D-CV2/D-CV4)
//! — the engine-side presenter that turns a fight into pixels.
//!
//! Placement is D-CV1's: the scene lives inside `gbx-engine`, holds
//! **presentation state only**, and draws through the existing primitives
//! (`draw.rs`, `text.rs`, `symbols.rs`, `combat_art.rs`). Two hosts will drive
//! it — the reel (slice 5) and the shell (slice 6) — and the headless path
//! never constructs one, so `run_combat`/`h4_replay`/the frontier guard are
//! untouched by anything here.
//!
//! **What this slice is.** Slice 3 is the *static* half of the presenter:
//!
//! - the entry-snapshot input type and the [`PresentedBoard`] it starts
//!   ([`EntrySnapshot`], [`CombatantIdentity`]);
//! - advancing that board by [`ActionEvent`]s, and reconciling it against the
//!   real roster at step boundaries ([`CombatScene::apply_event`],
//!   [`CombatScene::reconcile`]);
//! - pixel-true rendering of any given moment ([`CombatScene::render`] and
//!   the finer-grained draws in [`render`]).
//!
//! Timing, beats, message text, missiles, the death flash and sounds are
//! slice 4; floor *generation* is slice 6 (the scene renders whatever
//! [`CombatMap`] it is handed — captures carry real terrain).
//!
//! **The mid-playback rule, by construction.** D-CV2 forbids reading
//! `CombatState` during playback: `step()` has already run the whole turn, so
//! a live read shows end-of-step state (the §49 trap). The scene enforces
//! that structurally — it stores no `CombatState` and no reference to one.
//! The only `&CombatState` in this module's API appears on two explicitly
//! named **boundary** methods, [`CombatScene::reconcile`] and
//! [`CombatScene::panel_summary`], which a host may only call between steps.

mod board;
pub mod layout;
pub mod missile;
pub mod render;
pub mod strings;
pub mod time;
pub mod timeline;

#[cfg(test)]
mod goldens;
#[cfg(test)]
mod parity;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod timeline_tests;

pub use board::{DownedTile, PresentedBoard, PresentedCombatant};
pub use render::{status_string, OverlayDraw, PanelOp, PanelSummary};
pub use time::BeatClock;
pub use timeline::{Instruction, Op, Timeline};

use crate::combat::{ActionEvent, CombatMap, CombatState, GridPos};
use crate::combat_art::{CombatIcons, IconPose, TileAtlas};
use crate::framebuffer::Framebuffer;
use crate::shell::SoundEvent;
use crate::symbols::{SymbolError, SymbolSets};
use gbx_formats::font::Font;
use std::collections::BTreeMap;

/// Everything the scene can refuse to draw.
///
/// Every variant is a place where the original either cannot be transcribed
/// from what is in hand (the `DrawIsoTile` overlay fork) or where the input
/// is impossible for a well-formed fight — loud beats a guess (D11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    /// `DrawIsoTile`'s `tileIndex > 0x7f` branch (`ovr034.cs:32-35`) blits
    /// from `gbl.dword_1C8FC`, an overlay store **coab leaves stubbed** — the
    /// original behavior is unknown (doc §6 item 2). Four `BackGroundTiles`
    /// rows (66, 67, 69, 71) carry `0xFF` tile indices, so a map cell really
    /// can steer here; when one does, the scene stops instead of inventing a
    /// second art store.
    IsoTileOverlayPath { ground_value: u8, tile_index: u8 },
    /// A map cell's ground value is past the 74-row `BackGroundTiles` table
    /// (`Gbl.cs:186-257`), which the original indexes unguarded.
    BackgroundTileOutOfRange { ground_value: u8 },
    /// A tile index resolved past the 48-slot atlas — impossible for the
    /// shipped table, so this is a corrupt atlas or a bad slot computation.
    AtlasSlotOutOfRange { slot: usize },
    /// `DrawFrame_Combat` couldn't resolve an 8×8 symbol — set 4 isn't
    /// loaded (boot didn't run) or an id fell outside it.
    Symbol(SymbolError),
    /// [`CombatScene::reconcile`] found the presented board and the real
    /// roster disagreeing on an event-carried field: playback drifted from
    /// the fight it is presenting. Also fires a `debug_assert!`.
    BoardDrift {
        combatant_id: usize,
        field: &'static str,
        presented: String,
        actual: String,
    },
    /// Same, for the camera ([`ActionEvent::Camera`]'s field).
    CameraDrift { presented: GridPos, actual: GridPos },
    /// The presented roster and the real one are different lengths — the
    /// snapshot was built from a different fight.
    RosterSizeMismatch { presented: usize, actual: usize },
}

impl From<SymbolError> for SceneError {
    fn from(e: SymbolError) -> Self {
        SceneError::Symbol(e)
    }
}

/// The art one fight draws with (D-CV4): the per-fight 24×24 ground-tile
/// atlas (`gbl.dax24x24Set`) and the 26-slot combatant icon store
/// (`gbl.combat_icons`), both filled by the host's combat-entry loads —
/// [`crate::combat_art`]'s four loaders.
///
/// The scene owns them for the fight's duration and gives no opinion about
/// which slot holds what; that mapping is the host's (party icons from the
/// records, monster CPIC ids from the capture sidecar, COMSPR from boot).
#[derive(Debug, Clone, Default)]
pub struct SceneArt {
    pub tiles: TileAtlas,
    pub icons: CombatIcons,
}

impl SceneArt {
    pub fn new(tiles: TileAtlas, icons: CombatIcons) -> Self {
        SceneArt { tiles, icons }
    }
}

/// The two per-combatant facts the engine's roster cannot supply (doc §2's
/// gap table): the display **name** (`Combatant` has no name field — the
/// harnesses decode it from the capture record) and the **icon slot**
/// (`player.icon_id`; captures carry no monster CPIC ids, so M6a pins them in
/// a per-capture sidecar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatantIdentity {
    pub name: String,
    pub icon_slot: usize,
}

impl CombatantIdentity {
    pub fn new(name: impl Into<String>, icon_slot: usize) -> Self {
        CombatantIdentity {
            name: name.into(),
            icon_slot,
        }
    }
}

/// D-CV2's **entry snapshot**: the roster, the map, and the camera as read at
/// the first step boundary.
///
/// The camera read is deliberately *after* the first [`CombatState::step`]:
/// `combat_setup` initializes `mapScreenTopLeft` lazily inside that first
/// call (`ovr011.cs:1209`, `mod.rs:1437`), so reading it before would capture
/// the constructor's `(0,0)`.
#[derive(Debug, Clone)]
pub struct EntrySnapshot {
    pub roster: Vec<PresentedCombatant>,
    pub map: CombatMap,
    pub camera_top_left: GridPos,
}

impl EntrySnapshot {
    /// Builds the snapshot from a boundary read of `state`, with
    /// host-supplied identities in roster order.
    ///
    /// Call this after the first `step()` — see the type's own note about the
    /// lazily-initialized camera.
    pub fn from_state(state: &CombatState, identities: &[CombatantIdentity]) -> Self {
        let roster = state
            .roster()
            .iter()
            .enumerate()
            .map(|(i, c)| {
                // A short list would silently give a monster a party icon
                // slot; the host owns both halves of this pairing.
                debug_assert!(
                    i < identities.len(),
                    "combatant {i} has no supplied identity (name + icon slot)"
                );
                let identity = identities.get(i);
                PresentedCombatant {
                    id: c.id,
                    name: identity.map(|d| d.name.clone()).unwrap_or_default(),
                    team: c.team,
                    non_team_member: c.non_team_member,
                    icon_slot: identity.map(|d| d.icon_slot).unwrap_or(i),
                    size: c.size,
                    pos: c.pos,
                    direction: c.direction,
                    pose: IconPose::Normal,
                    hp_current: c.hp_current,
                    hp_max: c.hp_max,
                    ac: c.ac,
                    health_status: c.health_status,
                    in_combat: c.in_combat,
                }
            })
            .collect();
        EntrySnapshot {
            roster,
            map: state.map.clone(),
            camera_top_left: state.camera_top_left(),
        }
    }
}

/// The grey focus box's placement — `gbl.mapToBackGroundTile`'s
/// `drawTargetCursor` + `size` + the cell `sub_7416E` is called with
/// (`RedrawCombatIfFocusOn`, `ovr033.cs:670-680`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusCursor {
    pub pos: GridPos,
    /// The footprint the box covers — the *acting/targeted* combatant's size,
    /// which is what the original parks in `mapToBackGroundTile.size`.
    pub size: u8,
}

/// The combat scene: a presented board, the fight's art, and the focus
/// cursor. Presentation state only — no `CombatState`, no PRNG, no clock.
#[derive(Debug, Clone)]
pub struct CombatScene {
    board: PresentedBoard,
    art: SceneArt,
    focus: Option<FocusCursor>,
    weapon_names: BTreeMap<u8, String>,
    // --- the playback half (slice 4) -----------------------------------
    clock: BeatClock,
    timeline: Timeline,
    /// Boundary-read panel summaries, one per roster index — refreshed at
    /// step boundaries by [`CombatScene::refresh_panels`], selected during
    /// playback by [`Op::Panel`].
    panels: Vec<PanelSummary>,
    panel_focus: Option<usize>,
    /// The message script accumulated so far (§1.5's panel channel).
    messages: Vec<PanelOp>,
    /// The prompt row's current line (§1.5's other channel).
    prompt: Option<String>,
    /// The status row's current line ("Spell:<name>", "Range = N").
    status: Option<String>,
    /// Overlay sprites currently up — missile, burst, death flash.
    overlays: Vec<OverlayDraw>,
    /// Sound cues produced by the ticks since the last [`CombatScene::tick`].
    sounds: Vec<SoundEvent>,
}

impl CombatScene {
    /// Starts a scene from an entry snapshot and the fight's art.
    pub fn new(snapshot: EntrySnapshot, art: SceneArt) -> Self {
        CombatScene {
            board: PresentedBoard::new(snapshot.roster, snapshot.map, snapshot.camera_top_left),
            art,
            focus: None,
            weapon_names: BTreeMap::new(),
            clock: BeatClock::default(),
            timeline: Timeline::default(),
            panels: Vec::new(),
            panel_focus: None,
            messages: Vec::new(),
            prompt: None,
            status: None,
            overlays: Vec::new(),
            sounds: Vec::new(),
        }
    }

    /// The readied-weapon display names the right panel prints, keyed by
    /// `ITEMS` type (`activeItems.primaryWeapon.name`).
    ///
    /// Item *display names* are `ItemDisplayNameBuild`'s job — an M3
    /// inventory surface that combat's `Combatant` does not carry (it holds
    /// the readied weapon as `(type, plus)` only). Rather than invent a name
    /// table, the scene prints whatever the host names and otherwise takes
    /// the original's own `primaryWeapon == null` branch: no weapon line.
    pub fn set_weapon_names(&mut self, names: BTreeMap<u8, String>) {
        self.weapon_names = names;
    }

    pub fn board(&self) -> &PresentedBoard {
        &self.board
    }

    pub fn board_mut(&mut self) -> &mut PresentedBoard {
        &mut self.board
    }

    pub fn art(&self) -> &SceneArt {
        &self.art
    }

    pub fn focus(&self) -> Option<FocusCursor> {
        self.focus
    }

    /// Parks the grey focus box on a cell (`RedrawCombatIfFocusOn(true, …)`),
    /// or takes it down (`None` — `drawTargetCursor = false`).
    pub fn set_focus(&mut self, focus: Option<FocusCursor>) {
        self.focus = focus;
    }

    /// Applies one playback event to the presented board.
    pub fn apply_event(&mut self, event: ActionEvent) {
        self.board.apply(event);
    }

    /// Applies a whole step's event batch, in emission order.
    pub fn apply_events(&mut self, events: &[ActionEvent]) {
        for &event in events {
            self.board.apply(event);
        }
    }

    /// **Boundary read.** Reconciles the presented board with the real roster
    /// — see [`PresentedBoard::reconcile`] for the scope. Legal only between
    /// steps; that is the whole contract this method exists to name.
    pub fn reconcile(&mut self, state: &CombatState) -> Result<(), SceneError> {
        self.board.reconcile(state)
    }

    /// **Boundary read.** Builds the right panel's contents for `id` from the
    /// real roster: hp/AC/in-combat/health-status and the two fields no event
    /// carries — the readied weapon's name and `IsHeld()`. The display name
    /// comes from the entry snapshot (the engine has none).
    ///
    /// The original draws this summary at every turn start (`ovr009.cs:124`),
    /// which *is* a step boundary — which is why D-CV2 needs no `Readied`
    /// event and why the reconciliation assert excludes these fields.
    pub fn panel_summary(&self, state: &CombatState, id: usize) -> Option<PanelSummary> {
        let presented = self.board.combatant(id)?;
        let actual = state.roster().get(id)?;
        let mut summary = render::summary_skeleton(presented);
        summary.in_combat = actual.in_combat;
        summary.hp_current = actual.hp_current;
        summary.hp_max = actual.hp_max;
        summary.ac = actual.ac;
        summary.health_status = actual.health_status;
        summary.readied_weapon = actual
            .readied_weapon
            .and_then(|(item_type, _plus)| self.weapon_names.get(&item_type).cloned());
        summary.held = state.is_held(id);
        Some(summary)
    }

    /// `RedrawCombatScreen` (`ovr025.cs:1514-1519`): the palette swap, the
    /// combat frame, the 7×7 ground, every on-screen icon, and — when a focus
    /// cursor is parked — the grey box under the cell's occupant.
    ///
    /// This is the whole-screen paint; slice 4's incremental redraws build on
    /// the same pieces in [`render`].
    pub fn render(&self, fb: &mut Framebuffer, sets: &SymbolSets) -> Result<(), SceneError> {
        render::redraw_combat_screen(fb, &self.board, &self.art, sets)?;
        if let Some(focus) = self.focus {
            // `redrawCombatArea`'s own tail (`ovr033.cs:365`): after the icon
            // repaint it calls `sub_7416E(newPos)`, which repaints the ground
            // under the box, lays the box, and puts that cell's occupant back
            // on top.
            render::draw_focus_box(fb, &self.board, &self.art, focus.pos, focus.size, true)?;
        }
        Ok(())
    }

    /// Draws the right panel for a summary built by [`Self::panel_summary`].
    pub fn draw_panel(&self, fb: &mut Framebuffer, font: &Font, summary: &PanelSummary) {
        render::draw_panel_summary(fb, font, summary);
    }

    /// Clears the panel's message rows, the status line and the prompt line —
    /// the three surfaces slice 4 and M6c fill (§1.1/§1.5).
    pub fn clear_text_surfaces(&self, fb: &mut Framebuffer) {
        render::clear_message_region(fb);
        render::clear_status_line(fb);
        render::clear_prompt_line(fb);
    }

    /// `Color_0_8_normal` — puts the EGA registers back at fight exit
    /// (`free_combat_stuff`, `ovr009.cs:9`).
    pub fn restore_palette(&self, fb: &mut Framebuffer) {
        render::palette_normal(fb);
    }

    // === the playback timeline (slice 4, D-CV3) ==========================

    /// The fight's `game_speed_var` clock.
    pub fn clock(&self) -> BeatClock {
        self.clock
    }

    /// Sets `game_speed_var` (the Speed menu's 0–9, `ovr009.cs:672-704`).
    /// Affects only how long beats are **held** — never what a frame contains.
    pub fn set_game_speed(&mut self, speed: u8) {
        self.clock.set_game_speed(speed);
    }

    /// **Boundary read.** Caches every combatant's [`PanelSummary`] so the
    /// timeline can switch the right panel mid-playback without touching
    /// `CombatState` (D-CV2: the panel draws at turn start, a step boundary).
    /// Call it at each boundary, beside [`Self::reconcile`].
    pub fn refresh_panels(&mut self, state: &CombatState) {
        self.panels = (0..self.board.combatants().len())
            .map(|id| {
                self.panel_summary(state, id).unwrap_or_else(|| {
                    render::summary_skeleton(
                        self.board.combatant(id).expect("id is a roster index"),
                    )
                })
            })
            .collect();
    }

    /// Loads a step's buffered [`ActionEvent`] batch as this step's schedule
    /// (D-CV2). Anything left of the previous step's schedule is dropped — the
    /// lockstep invariant says the host drained it before calling `step()`
    /// again, so there should be nothing left.
    pub fn begin_step(&mut self, events: &[ActionEvent]) {
        debug_assert!(
            !self.timeline.is_playing(),
            "D-CV2 lockstep: step N's playback must drain before step N+1 is composed"
        );
        let schedule = timeline::compose(&self.board, self.clock, events);
        self.timeline.load(schedule);
    }

    /// Is a step's playback still running?
    pub fn is_playing(&self) -> bool {
        self.timeline.is_playing()
    }

    /// Ticks still owed by the current schedule.
    pub fn ticks_remaining(&self) -> u32 {
        self.timeline.ticks_remaining()
    }

    /// Advances playback by `dt_ticks` and returns the sound cues those ticks
    /// produced (D-UI1's `Frame::sounds`). The scene never sleeps and never
    /// reads a clock — the host's tick rate is the only pacing there is.
    pub fn tick(&mut self, dt_ticks: u32) -> &[SoundEvent] {
        self.sounds.clear();
        let mut pending = Vec::new();
        self.timeline
            .advance(dt_ticks, |op| pending.push(op.clone()));
        for op in pending {
            self.apply(op);
        }
        &self.sounds
    }

    /// **The fast drain** (D-CV3's open skip door): collapse the rest of this
    /// step's beats to zero ticks and jump to the state the schedule would
    /// have reached. Every remaining op is applied, sounds included, so
    /// nothing downstream can tell a skipped step from a played one — the
    /// difference is wall time only. The D-CV2 lockstep invariant is
    /// *satisfied* by this: playback completes, just instantly.
    pub fn skip(&mut self) -> &[SoundEvent] {
        self.sounds.clear();
        let pending: Vec<Op> = self.timeline.drain_pending().map(|i| i.op).collect();
        for op in pending {
            self.apply(op);
        }
        &self.sounds
    }

    /// The sound cues the last [`Self::tick`] or [`Self::skip`] produced.
    pub fn sounds(&self) -> &[SoundEvent] {
        &self.sounds
    }

    /// The panel summary currently on screen, if the timeline has selected one.
    pub fn panel_focus(&self) -> Option<&PanelSummary> {
        self.panel_focus.and_then(|id| self.panels.get(id))
    }

    /// The message script currently displayed (§1.5's panel channel).
    pub fn messages(&self) -> &[PanelOp] {
        &self.messages
    }

    /// The prompt row's current line, if any.
    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }

    /// The status row's current line, if any.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// The overlay sprites currently up.
    pub fn overlays(&self) -> &[OverlayDraw] {
        &self.overlays
    }

    fn apply(&mut self, op: Op) {
        match op {
            Op::Camera(top_left) => self.board.apply(ActionEvent::Camera { top_left }),
            Op::Board(event) => self.board.apply(event),
            Op::Pose {
                id,
                pose,
                direction,
            } => self.board.set_pose(id, pose, direction),
            Op::Hidden(id) => self.board.set_icon_suppressed(id),
            Op::Panel(id) => {
                // `CombatDisplayPlayerSummary` clears the whole panel region,
                // messages included (`ovr025.cs:294`).
                self.panel_focus = Some(id);
                self.messages.clear();
            }
            Op::Focus(cursor) => self.focus = cursor,
            Op::Overlay(draws) => self.overlays = draws,
            Op::Message(PanelOp::Clear) => self.messages.clear(),
            Op::Message(other) => self.messages.push(other),
            Op::Prompt(text) => self.prompt = text,
            Op::Status(text) => self.status = text,
            Op::Sound(id) => self.sounds.push(SoundEvent(id)),
            Op::Wait => {}
        }
    }

    /// The whole combat screen at the playback cursor: the map (ground, icons,
    /// focus box, overlays), the right panel and its messages, the status row
    /// and the prompt row.
    ///
    /// A full repaint every frame, which is what makes the presented screen a
    /// pure function of the presented state — coab reaches the same pixels by
    /// saving and restoring video RAM around each overlay.
    pub fn render_frame(
        &self,
        fb: &mut Framebuffer,
        sets: &SymbolSets,
        font: &Font,
    ) -> Result<(), SceneError> {
        self.render(fb, sets)?;
        render::draw_overlays(fb, &self.art, &self.overlays);
        if let Some(summary) = self.panel_focus() {
            render::draw_panel_summary(fb, font, summary);
        }
        render::draw_panel_messages(fb, font, &self.board, &self.messages);
        match &self.status {
            Some(text) => render::draw_status_line(fb, font, text),
            None => render::clear_status_line(fb),
        }
        match &self.prompt {
            Some(text) => render::draw_prompt(fb, font, text),
            None => render::clear_prompt_line(fb),
        }
        Ok(())
    }
}
