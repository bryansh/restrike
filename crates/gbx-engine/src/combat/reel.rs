//! **The reel's input contract** (`docs/design/combat-visualizer.md` D-CV1
//! item 2, §4 M6a) — everything a host must hand over to replay one captured
//! fight, and the one place a `CombatState` is built from it.
//!
//! D-CV1 pins the dependency direction as a settled door: `gbx-oracle` →
//! `gbx-engine`, never the reverse. So `.gbxtrace` parsing stays in the oracle
//! and the **assembly product** lands here, engine-side, as plain data:
//! [`ReelInput`]. The oracle's `replay` module builds one from a capture (roster
//! records, terrain, the knob/icon sidecar); [`Engine::new_reel`] consumes it.
//!
//! [`Engine::new_reel`]: crate::engine::Engine::new_reel
//!
//! **One assembly path, not two.** The harnesses (`h4_replay`, `h4_turndiff`,
//! the frontier guard) and the reel all reach a live `CombatState` through
//! [`build_state`] — the same record decode, the same knob application, the same
//! loadout install, in the same order. That is what makes "the reel is h4_replay
//! with pixels" a structural claim rather than a hope: if the reel's fight ever
//! diverged from the harness's, the two would have had to build different
//! states, and there is only one builder.
//!
//! The input half of the module draws nothing. The **playback** half ([`Reel`])
//! owns the fight, the scene and the capture's draw stream, and is what
//! `engine.rs` ticks — see [`crate::combat::scene`] for the presenter itself.

use super::scene::{CombatScene, CombatantIdentity, EntrySnapshot, SceneArt};
use super::{
    combat_state_from_records, ActionEvent, ActionSink, CombatMap, CombatState, CombatStep,
    GridPos, Loadout, RecordCombatant,
};
use crate::combat_art::{
    self, CombatArtLoadError, CombatIcons, COMBAT_ICON_SLOTS, PARTY_ICON_SLOTS,
};
use crate::framebuffer::Framebuffer;
use crate::party::IconInfo;
use crate::rng::{EngineRng, RngDraw, RngSink};
use crate::shell::SoundEvent;
use crate::symbols::SymbolSets;
use gbx_formats::affects::AffectRecord;
use gbx_formats::font::Font;
use gbx_formats::game_data::GameData;
use gbx_formats::items::ItemDataTable;
use gbx_formats::save_orig::{decode_char_record, SaveParseError};
use gbx_rules::flavor::Flavor;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub use super::Team;

/// The full `0x1A6` combat record every roster slot carries (`Player`'s
/// on-disk size — the oracle's `COMBAT_RECORD_LEN` names the same number from
/// the wire side, deliberately without a shared dependency).
pub const COMBAT_RECORD_LEN: usize = 0x1A6;

/// Open-floor fallback tile (`0x17` = passable floor, move cost 1).
///
/// Used only when a capture predates the `combat_entry.terrain` field. Terrain
/// is load-bearing for movement (doc §14), so every modern capture builds its
/// map from the captured ground grid instead.
pub const FALLBACK_FLOOR: u8 = 0x17;

/// Icon slots `0..8` are the party; `8..` are assigned per monster type at
/// LOADMONSTER time (`gbl.monster_icon_id` starts at 8, `ovr008.cs:98` /
/// `ovr003.cs:763`, and `CMD_LoadMonster` stamps it onto every copy before
/// incrementing, `ovr003.cs:259-293`).
pub const MONSTER_FIRST_ICON_SLOT: u8 = 8;

/// One roster slot's replay input: where it stood, whose side it was on, and
/// the bytes it was.
///
/// Owned rather than borrowed (unlike [`RecordCombatant`]) because a
/// [`ReelInput`] outlives the capture text it was parsed from — the reel host
/// keeps it for the length of the fight.
#[derive(Debug, Clone, PartialEq)]
pub struct ReelCombatant {
    pub team: Team,
    pub pos: GridPos,
    /// The full `0x1A6` combat record.
    pub record: Vec<u8>,
    /// The live affect chain as the staging hook captured it (doc §44.2/§47.7),
    /// order preserved — find-FIRST is order-observable.
    pub affects: Vec<AffectRecord>,
}

/// The per-fight input knobs a capture cannot always carry.
///
/// Three of these (`map_direction`, `area_field_58c`, `area_field_6e4`) ARE
/// emitted by modern staging hooks and the capture's value wins where present;
/// the rest (`auto_cast*`, `continue_battle`) are recordings of keypresses that
/// never made it into the snapshot and stay hand-pinned per capture (doc §38,
/// §48). Defaults are the documented pre-capture fallbacks the replay harnesses
/// have always used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReelKnobs {
    /// `gbl.mapDirection` — the flee HEADING (doc §29). Default 2 (E), the
    /// provisional geometry-matched value, capture-confirmed for the bar.
    pub map_direction: u8,
    /// `gbl.AutoPCsCastMagic` at entry (`BattleSetup` resets it false,
    /// `ovr011.cs:1186`).
    pub auto_cast: bool,
    /// Mid-fight '2' presses as 0-based global turn ordinals (doc §38).
    pub auto_cast_toggles: Vec<u32>,
    /// "Continue Battle:" occurrences answered 'Y', 0-based (doc §48).
    pub continue_battle: Vec<u16>,
    /// `area2.field_58C` — the `FleeCheck_001` gate-2 morale threshold (doc
    /// §28). Default 99, the measured bar value under which the natural rout
    /// is mathematically impossible.
    pub area_field_58c: i32,
    /// `area2.field_6E4` — the PARTY-gated area movement modifier, in the
    /// ENGINE domain (coab≠binary #21, doc §45; the §47 byte-bridge converts
    /// a capture's raw word).
    pub area_field_6e4: i32,
}

impl Default for ReelKnobs {
    fn default() -> Self {
        ReelKnobs {
            map_direction: 2,
            auto_cast: false,
            auto_cast_toggles: Vec::new(),
            continue_battle: Vec::new(),
            area_field_58c: 99,
            area_field_6e4: 0,
        }
    }
}

/// The art pins a capture cannot supply (doc §2's gap table, §4 M6a).
///
/// `combat_entry` carries no monster CPIC ids — LOADMONSTER's third operand is
/// read on the live path only (`format.rs:430-437`) — so the 15 closed captures
/// hand-pin them exactly as their loadouts are pinned. Party icons need no pin:
/// `CHEAD[head_icon]` + `CBODY[weapon_icon]` + `icon_colours` all ride in the
/// record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReelArt {
    /// The `CPIC{area}.DAX` suffix — `gbl.game_area` at the fight
    /// (`chead_cbody_comspr_icon`'s `fileText + game_area`, `ovr034.cs:79`).
    pub cpic_area: u8,
    /// Icon slot (`>= 8`) → the CPIC block LOADMONSTER put there.
    pub monster_blocks: BTreeMap<u8, u8>,
    /// `SetupGroundTiles`' fork (`ovr011.cs:757-768`): DUNGCOM or WILDCOM.
    pub in_dungeon: bool,
}

impl Default for ReelArt {
    fn default() -> Self {
        ReelArt {
            cpic_area: 2,
            monster_blocks: BTreeMap::new(),
            in_dungeon: true,
        }
    }
}

/// One draw the capture recorded, as the reel checks it: `(before, after)`
/// always, plus the `Random(n)` operand when the capture carried one
/// (`ss_sp_words[3]`).
///
/// The equality surface is the harnesses' (doc §14's lesson: `(before, after)`
/// alone is draw-COUNT equality for a pure LCG, so the operand is part of the
/// comparison whenever both sides have it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedDraw {
    pub before: u32,
    pub after: u32,
    pub operand: Option<u16>,
}

/// Everything [`Engine::new_reel`] needs to replay one captured fight with
/// pixels.
///
/// [`Engine::new_reel`]: crate::engine::Engine::new_reel
#[derive(Debug, Clone)]
pub struct ReelInput {
    /// A human label for diagnostics — the capture basename.
    pub label: String,
    /// The replay seed (`combat_entry.rng_state`).
    pub rng_state: u32,
    /// The roster in `TeamList` (== initiative draw) order. Verbatim.
    pub combatants: Vec<ReelCombatant>,
    /// The captured ground grid (`mapToBackGroundTile`, 50×25 row-major);
    /// `None` falls back to [`FALLBACK_FLOOR`].
    pub terrain: Option<Vec<u8>>,
    pub knobs: ReelKnobs,
    /// Per-capture ranged loadouts by roster index (doc §34.1/§45/§48).
    pub loadouts: Vec<(usize, Loadout)>,
    /// The resident `ITEMS` table — required by any capture with a loadout.
    pub item_data: Option<ItemDataTable>,
    pub art: ReelArt,
    /// `game_speed_var` for playback (`seg001.cs:274`'s default is 4). Affects
    /// how long beats are held, never what a frame contains (D-CV3).
    pub game_speed: u8,
    /// D-CV3's host tick multiplier: how many scene ticks one engine tick
    /// advances. `1` is the faithful rate; a bigger number runs the reel fast
    /// without changing a single frame.
    pub tick_multiplier: u32,
    /// The capture's post-`combat_entry` draw stream. The reel asserts equality
    /// against this **live, while rendering** (D-CV1: "the reel is h4_replay
    /// with pixels"). Empty disables the check — for a reel over a fight that
    /// has no capture behind it.
    pub expected_draws: Vec<ExpectedDraw>,
}

impl ReelInput {
    /// A minimal input: a roster, no terrain, default knobs/art, no capture to
    /// check against. Hosts fill in the rest.
    pub fn new(label: impl Into<String>, rng_state: u32, combatants: Vec<ReelCombatant>) -> Self {
        ReelInput {
            label: label.into(),
            rng_state,
            combatants,
            terrain: None,
            knobs: ReelKnobs::default(),
            loadouts: Vec::new(),
            item_data: None,
            art: ReelArt::default(),
            game_speed: 4,
            tick_multiplier: 1,
            expected_draws: Vec::new(),
        }
    }

    /// The map this input replays on — the captured terrain, or open floor.
    pub fn map(&self) -> CombatMap {
        match &self.terrain {
            Some(ground) => CombatMap::from_ground(ground.clone()),
            None => CombatMap::uniform(FALLBACK_FLOOR),
        }
    }
}

/// **The one place a replay's `CombatState` is built.**
///
/// Decodes the roster records, lays the captured terrain, applies every knob,
/// installs the `ITEMS` table and the per-capture loadouts — in exactly the
/// order the H4 harnesses have always applied them, because they now call this.
///
/// The returned state has no sinks attached and has not stepped: the caller
/// owns the PRNG and the observation seams.
pub fn build_state(input: &ReelInput, flavor: &dyn Flavor) -> Result<CombatState, SaveParseError> {
    let entries: Vec<RecordCombatant> = input
        .combatants
        .iter()
        .map(|c| RecordCombatant {
            team: c.team,
            pos: c.pos,
            record: &c.record,
            affects: c.affects.clone(),
        })
        .collect();

    let mut state = combat_state_from_records(&entries, input.map(), flavor)?;
    state.area_field_58c = input.knobs.area_field_58c;
    state.map_direction = input.knobs.map_direction;
    state.auto_pcs_cast_magic = input.knobs.auto_cast;
    state.auto_cast_toggles = input.knobs.auto_cast_toggles.clone();
    state.continue_battle_yes = input.knobs.continue_battle.clone();
    state.area_field_6e4 = input.knobs.area_field_6e4;
    // §34.1: the `ITEMS` table first, then the rows that read it. A `None`
    // loadout list leaves every combatant exactly as today's engine — the
    // melee-identical path the un-armed captures ride.
    state.item_data = input.item_data.clone();
    for &(id, loadout) in &input.loadouts {
        state.set_loadout(id, loadout);
    }
    Ok(state)
}

/// Compare one of our draws against the capture's, on the harnesses' surface.
///
/// `(before, after)` always; the `Random(n)` operand additionally, but only
/// when **both** sides carry one — a capture from a hook that didn't record
/// `ss_sp_words` falls back to the chain alone for that draw.
pub fn draws_agree(ours: &crate::rng::RngDraw, capture: &ExpectedDraw) -> bool {
    let operand_ok = match (ours.n, capture.operand) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    ours.before == capture.before && ours.after == capture.after && operand_ok
}

/// Which combat mechanic drew, inferred from the `Random(n)` operand — the
/// honest die tells the mechanic (§2/§4/§9 draw map). Diagnostic only.
pub fn mechanic_for(operand: Option<u16>) -> &'static str {
    match operand {
        Some(6) => "initiative d6 (CalculateInitiative)",
        Some(100) => "d100 (FindNextCombatant selection, or FleeCheck/advance morale)",
        Some(20) => "d20 (to-hit PC_CanHitTarget, or a saving throw)",
        Some(7) => "d7 (QuickFight AI mode-gate / wand-scan / spell-priority)",
        Some(0) => "random(0) edge draw",
        Some(_) => "damage die (weapon/monster attack dice)",
        None => "unknown (operand not recorded)",
    }
}

// === the playback host ====================================================

/// Everything that can stop a reel before its first frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ReelError {
    /// A roster record didn't decode.
    Record(SaveParseError),
    /// The fight's art wouldn't load.
    Art(CombatArtLoadError),
    /// A monster's icon slot has no CPIC pin in the input's [`ReelArt`].
    ///
    /// Captures carry no monster icon ids, so an unpinned capture reaches here
    /// rather than drawing its enemies as empty slots — a silent, watchable
    /// -looking failure is the worst outcome for a tool whose whole job is
    /// showing you the fight.
    UnpinnedMonsterIcon { slot: u8, name: String },
    /// A record's `icon_id` fell outside `gbl.combat_icons[26]`.
    IconSlotOutOfRange { slot: u8, name: String },
    /// An empty roster, or a fight that is over before it starts.
    NothingToPlay,
}

impl std::fmt::Display for ReelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReelError::Record(e) => write!(f, "a roster record did not decode: {e:?}"),
            ReelError::Art(e) => write!(f, "combat art did not load: {e:?}"),
            ReelError::UnpinnedMonsterIcon { slot, name } => write!(
                f,
                "{name} uses combat icon slot {slot} but the capture sidecar pins no CPIC block \
                 for it — add a `monster_icons` row (combat-visualizer.md §4 M6a)"
            ),
            ReelError::IconSlotOutOfRange { slot, name } => write!(
                f,
                "{name}'s record asks for combat icon slot {slot}, past the {COMBAT_ICON_SLOTS}-slot store"
            ),
            ReelError::NothingToPlay => write!(f, "the reel's fight is over before it begins"),
        }
    }
}

impl std::error::Error for ReelError {}

impl From<SaveParseError> for ReelError {
    fn from(e: SaveParseError) -> Self {
        ReelError::Record(e)
    }
}
impl From<CombatArtLoadError> for ReelError {
    fn from(e: CombatArtLoadError) -> Self {
        ReelError::Art(e)
    }
}

/// What a reel has done so far — for a host that wants to show progress, and
/// for the smoke test that asserts a capture played to its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReelProgress {
    pub label: String,
    /// `CombatState::step` calls made.
    pub steps: u64,
    /// Draws checked against the capture (== draws our fight has made).
    pub draws_checked: usize,
    /// Draws the capture recorded. `0` when there is no capture behind this
    /// reel and the equality check is off.
    pub draws_expected: usize,
    /// Is the fight over AND its last step's playback drained?
    pub finished: bool,
    /// Ticks still owed by the current step's schedule.
    pub ticks_remaining: u32,
}

/// Buffers one `step()`'s [`ActionEvent`]s — D-CV2's render feed.
struct BatchSink(Rc<RefCell<Vec<ActionEvent>>>);
impl ActionSink for BatchSink {
    fn on_action(&mut self, event: ActionEvent) {
        self.0.borrow_mut().push(event);
    }
}

/// Taps every draw at the engine seam — the same tap `h4_replay` attaches.
struct DrawTap(Rc<RefCell<Vec<RngDraw>>>);
impl RngSink for DrawTap {
    fn on_draw(&mut self, draw: RngDraw) {
        self.0.borrow_mut().push(draw);
    }
}

/// ★ **The reel** — h4_replay with pixels (D-CV1 item 2).
///
/// It owns the replayed `CombatState`, its PRNG, a [`CombatScene`] over the
/// fight's real art, and the capture's draw stream. Each host tick advances the
/// current step's playback by one beat; when that playback drains, the reel
/// reconciles at the boundary, runs the next `step()`, and **checks every draw
/// that step made against the capture, before the next frame is drawn**.
///
/// **A divergence panics.** That is deliberate and is the whole point of the
/// live check: a fight that has stopped being the captured fight must not keep
/// scrolling prettily past. The panic carries `h4_replay`'s full diagnostic —
/// index, both sides' `(before, after, operand)`, the matched draw before it,
/// and the inferred mechanic.
///
/// The two D-CV contracts it honours:
/// - **D-CV2 lockstep** — step N's playback drains completely before `step()`
///   is called for N+1. There is no pipelining and no rollback.
/// - **D-CV3 tick clock** — the scene consumes whole ticks and never reads a
///   clock. A host may run faster by raising `tick_multiplier`; frames are
///   unchanged, only wall time is.
pub struct Reel {
    label: String,
    state: CombatState,
    scene: CombatScene,
    rng: EngineRng,
    batch: Rc<RefCell<Vec<ActionEvent>>>,
    draws: Rc<RefCell<Vec<RngDraw>>>,
    expected: Vec<ExpectedDraw>,
    /// How many of our draws have been checked (== how many we have made).
    checked: usize,
    tick_multiplier: u32,
    /// `step()` has returned `Ended`; the final batch may still be playing.
    ended: bool,
    /// …and it has drained. Further ticks only redraw the last frame.
    finished: bool,
    steps: u64,
    sounds: Vec<SoundEvent>,
}

impl Reel {
    /// Builds a reel: loads the fight's art, builds the fight, takes D-CV2's
    /// entry snapshot after the first `step()`, and loads that step's playback.
    ///
    /// `boot_icons` is the boot-loaded COMSPR store (`seg001.cs:312-317` —
    /// missiles, effects and the grey focus box); the party and monster slots
    /// are filled here, which is `BattleSetup`'s own division of labour.
    pub fn new(
        data: &GameData,
        boot_icons: CombatIcons,
        input: &ReelInput,
        flavor: &dyn Flavor,
    ) -> Result<Self, ReelError> {
        if input.combatants.is_empty() {
            return Err(ReelError::NothingToPlay);
        }
        let (art, identities) = load_fight_art(data, boot_icons, input)?;
        Reel::with_art(art, identities, input, flavor)
    }

    /// The same reel over art a host already holds — the seam [`Reel::new`] is
    /// built on, and the one a fixture fight uses (an empty [`SceneArt`] draws
    /// nothing, `ovr034.cs:92`, which is exactly right for a test about the
    /// draw *stream*).
    pub fn with_art(
        art: SceneArt,
        identities: Vec<CombatantIdentity>,
        input: &ReelInput,
        flavor: &dyn Flavor,
    ) -> Result<Self, ReelError> {
        if input.combatants.is_empty() {
            return Err(ReelError::NothingToPlay);
        }
        let mut state = build_state(input, flavor)?;
        let batch = Rc::new(RefCell::new(Vec::new()));
        state.attach_action_sink(Box::new(BatchSink(Rc::clone(&batch))));

        let draws = Rc::new(RefCell::new(Vec::new()));
        let mut rng = EngineRng::new(input.rng_state);
        rng.attach_sink(Box::new(DrawTap(Rc::clone(&draws))));

        let mut reel = Reel {
            label: input.label.clone(),
            // A placeholder scene, replaced below — `EntrySnapshot` must be
            // read AFTER the first step (the camera initializes lazily inside
            // `combat_setup` on that call, `mod.rs:1437`).
            scene: CombatScene::new(
                EntrySnapshot {
                    roster: Vec::new(),
                    map: CombatMap::uniform(FALLBACK_FLOOR),
                    camera_top_left: GridPos::new(0, 0),
                },
                SceneArt::default(),
            ),
            state,
            rng,
            batch,
            draws,
            expected: input.expected_draws.clone(),
            checked: 0,
            tick_multiplier: input.tick_multiplier.max(1),
            ended: false,
            finished: false,
            steps: 0,
            sounds: Vec::new(),
        };

        let first = reel.state.step(&mut reel.rng);
        reel.steps = 1;
        reel.verify_new_draws();
        if first == CombatStep::Ended {
            return Err(ReelError::NothingToPlay);
        }

        reel.scene = CombatScene::new(EntrySnapshot::from_state(&reel.state, &identities), art);
        reel.scene.set_game_speed(input.game_speed);
        reel.scene.refresh_panels(&reel.state);
        reel.scene
            .reconcile(&reel.state)
            .expect("the entry snapshot was just read from this very state");
        let events = reel.take_batch();
        reel.scene.begin_step(&events);
        Ok(reel)
    }

    /// Advances one host tick and redraws.
    ///
    /// Either the current step's playback advances by [`Self::tick_multiplier`]
    /// beats, or — if it has drained — the reel reconciles at the step boundary
    /// and runs the next `step()`, checking its draws against the capture.
    /// Returns this tick's sound cues (D-UI1's `Frame::sounds`).
    pub fn tick(&mut self, fb: &mut Framebuffer, sets: &SymbolSets, font: &Font) -> &[SoundEvent] {
        self.sounds.clear();
        if !self.finished {
            if self.scene.is_playing() {
                let cues = self.scene.tick(self.tick_multiplier);
                self.sounds.extend_from_slice(cues);
            } else {
                self.advance_step();
            }
        }
        self.scene
            .render_frame(fb, sets, font)
            .unwrap_or_else(|e| panic!("{}: the reel failed to render: {e:?}", self.label));
        &self.sounds
    }

    /// The step boundary: the only two `CombatState` reads the scene is allowed
    /// (D-CV2), then the next step and its draw check.
    fn advance_step(&mut self) {
        self.scene.reconcile(&self.state).unwrap_or_else(|e| {
            panic!(
                "{}: the presented board drifted from the fight at step {} — {e:?}",
                self.label, self.steps
            )
        });
        self.scene.refresh_panels(&self.state);

        if self.ended {
            self.finished = true;
            self.check_length();
            return;
        }
        let step = self.state.step(&mut self.rng);
        self.steps += 1;
        self.verify_new_draws();
        if step == CombatStep::Ended {
            self.ended = true;
        }
        let events = self.take_batch();
        self.scene.begin_step(&events);
    }

    fn take_batch(&self) -> Vec<ActionEvent> {
        std::mem::take(&mut *self.batch.borrow_mut())
    }

    /// ★ **The live capture-equality assert.** Checks every draw made since the
    /// last call against the capture's stream, on the harnesses' surface.
    ///
    /// No-op when the reel has no capture behind it (`expected_draws` empty) —
    /// that is a reel over a synthetic fight, which has nothing to be equal to.
    fn verify_new_draws(&mut self) {
        if self.expected.is_empty() {
            self.checked = self.draws.borrow().len();
            return;
        }
        let draws = self.draws.borrow();
        while self.checked < draws.len() {
            let i = self.checked;
            let ours = draws[i];
            match self.expected.get(i) {
                Some(c) if draws_agree(&ours, c) => {}
                Some(c) => {
                    let context = if i > 0 {
                        let po = draws[i - 1];
                        let pc = self.expected[i - 1];
                        format!(
                            "\n  draw #{} (context, matched): ours ({:#010x}->{:#010x}, n={:?}) | \
                             capture ({:#010x}->{:#010x}, op={:?})",
                            i - 1,
                            po.before,
                            po.after,
                            po.n,
                            pc.before,
                            pc.after,
                            pc.operand
                        )
                    } else {
                        String::new()
                    };
                    panic!(
                        "\n=== REEL DIVERGENCE ({}) at draw #{i}, step {} ==={context}\n  \
                         ours   : before={:#010x} after={:#010x} n={:?}\n  \
                         capture: before={:#010x} after={:#010x} op={:?}\n  \
                         inferred mechanic (ours): {} | (capture): {}\n  \
                         {i}/{} draws matched before divergence. The pins are ground truth: \
                         a reel that diverges where h4_replay closes is an ASSEMBLY bug \
                         (sidecar knobs, loadouts, terrain), not a combat bug.\n",
                        self.label,
                        self.steps,
                        ours.before,
                        ours.after,
                        ours.n,
                        c.before,
                        c.after,
                        c.operand,
                        mechanic_for(ours.n),
                        mechanic_for(c.operand),
                        self.expected.len(),
                    );
                }
                None => panic!(
                    "\n=== REEL DIVERGENCE ({}) at draw #{i}, step {} ===\n  \
                     our fight drew MORE than the capture ({} draws). First extra: \
                     ({:#010x}->{:#010x}, n={:?}), mechanic {}.\n",
                    self.label,
                    self.steps,
                    self.expected.len(),
                    ours.before,
                    ours.after,
                    ours.n,
                    mechanic_for(ours.n),
                ),
            }
            self.checked += 1;
        }
    }

    /// The other half of length equality: our fight must not end early.
    fn check_length(&self) {
        if self.expected.is_empty() {
            return;
        }
        assert_eq!(
            self.checked,
            self.expected.len(),
            "\n=== REEL DIVERGENCE ({}) on `length` ===\n  our fight ENDED EARLY \
             ({} draws) vs the capture ({}). First missing capture draw: mechanic {}.\n",
            self.label,
            self.checked,
            self.expected.len(),
            mechanic_for(self.expected.get(self.checked).and_then(|d| d.operand)),
        );
    }

    /// Has the fight ended AND its last step's playback drained?
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn progress(&self) -> ReelProgress {
        ReelProgress {
            label: self.label.clone(),
            steps: self.steps,
            draws_checked: self.checked,
            draws_expected: self.expected.len(),
            finished: self.finished,
            ticks_remaining: self.scene.ticks_remaining(),
        }
    }

    /// The fight being replayed — a **boundary** read for a host that wants to
    /// inspect the roster between steps (the inspector's debug pane). Never
    /// read it mid-playback: `step()` has already run the whole turn (D-CV2).
    pub fn state(&self) -> &CombatState {
        &self.state
    }

    pub fn scene(&self) -> &CombatScene {
        &self.scene
    }

    /// D-CV3's host tick multiplier: how many scene beats one host tick
    /// advances. `1` is the original's pace; bigger runs the reel fast without
    /// changing a single frame.
    pub fn set_tick_multiplier(&mut self, n: u32) {
        self.tick_multiplier = n.max(1);
    }

    /// `game_speed_var` (the original's Speed menu, 0–9).
    pub fn set_game_speed(&mut self, speed: u8) {
        self.scene.set_game_speed(speed);
    }
}

/// Loads the fight's art and derives each roster slot's identity.
///
/// Which store an icon comes from is the record's own answer, not a guess:
/// `CMD_LoadMonster` stamps `gbl.monster_icon_id` (8 and up) onto every monster
/// it loads (`ovr003.cs:259-293`) while party members carry their party order,
/// so `icon_id < 8` **is** the party/monster fork — and it puts the allied
/// team-0 NPCs (loaded by LOADMONSTER, so slot 8+) on the CPIC path where they
/// belong.
fn load_fight_art(
    data: &GameData,
    boot_icons: CombatIcons,
    input: &ReelInput,
) -> Result<(SceneArt, Vec<CombatantIdentity>), ReelError> {
    let mut icons = boot_icons;
    let mut identities = Vec::with_capacity(input.combatants.len());
    let mut loaded: Vec<u8> = Vec::new();

    for c in &input.combatants {
        let record = decode_char_record(&c.record)?;
        let slot = record.icon_id;
        if slot as usize >= COMBAT_ICON_SLOTS {
            return Err(ReelError::IconSlotOutOfRange {
                slot,
                name: record.name.clone(),
            });
        }
        identities.push(CombatantIdentity::new(record.name.clone(), slot as usize));
        if loaded.contains(&slot) {
            continue;
        }
        let icon = if (slot as usize) < PARTY_ICON_SLOTS {
            // `LoadPlayerCombatIcon` (`ovr017.cs:86-122`): CHEAD[head] merged
            // into CBODY[weapon], then the `icon_colours` nibble recolor.
            combat_art::load_party_icon(
                data,
                &IconInfo {
                    head_icon: record.head_icon,
                    weapon_icon: record.weapon_icon,
                    icon_id: record.icon_id,
                    icon_size: record.icon_size,
                    colours: record.icon_colours,
                },
                true,
            )?
        } else {
            let block = *input.art.monster_blocks.get(&slot).ok_or_else(|| {
                ReelError::UnpinnedMonsterIcon {
                    slot,
                    name: record.name.clone(),
                }
            })?;
            combat_art::load_monster_icon(data, input.art.cpic_area, block)?
        };
        icons.set(slot as usize, icon);
        loaded.push(slot);
    }

    let tiles = combat_art::load_ground_tiles(data, input.art.in_dungeon)?;
    Ok((SceneArt::new(tiles, icons), identities))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::HealthStatus;
    use gbx_rules::adnd1::flavor_impl::Adnd1;
    use gbx_rules::pack::RuleSet;

    /// A hand-authored `0x1A6` record (D10 — no game bytes anywhere near
    /// this): a name, hp, AC, a 1d6 attack profile and an icon assignment.
    /// The field offsets are `save_orig::decode_char_record`'s.
    fn synthetic_record(name: &str, hp: u8, icon_slot: u8, monster: bool) -> Vec<u8> {
        let mut r = vec![0u8; COMBAT_RECORD_LEN];
        r[0] = name.len() as u8;
        r[1..1 + name.len()].copy_from_slice(name.as_bytes());
        r[0x78] = hp; // hit_point_max
        r[0x1a4] = hp; // hit_point_current
        r[0x19a] = 0x30; // ac
        r[0x199] = 40; // hitBonus
        r[0x11e] = 1; // attack-1 dice count
        r[0x120] = 6; // attack-1 dice size
        r[0xde] = 0x01; // size 1
        r[0x143] = icon_slot;
        r[0x144] = 1; // icon_size
        r[0xf7] = if monster { 0x80 } else { 0x00 }; // control_morale: NPC bit
        r
    }

    fn two_sided_input() -> ReelInput {
        let combatants = vec![
            ReelCombatant {
                team: Team::Party,
                pos: GridPos::new(20, 12),
                record: synthetic_record("ALPHA", 20, 0, false),
                affects: Vec::new(),
            },
            ReelCombatant {
                team: Team::Monster,
                pos: GridPos::new(24, 12),
                record: synthetic_record("BRUTE", 14, 8, true),
                affects: Vec::new(),
            },
        ];
        ReelInput::new("synthetic.gbxtrace", 0x0C0F_FEE0, combatants)
    }

    #[test]
    fn build_state_decodes_the_roster_in_order() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let input = two_sided_input();
        let state = build_state(&input, &flavor).expect("synthetic records decode");
        let roster = state.roster();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].team, Team::Party);
        assert_eq!(roster[0].pos, GridPos::new(20, 12));
        assert_eq!(roster[0].hp_current, 20);
        assert_eq!(roster[1].team, Team::Monster);
        assert_eq!(roster[1].hp_current, 14);
        assert_eq!(roster[1].health_status, HealthStatus::Okey);
    }

    #[test]
    fn build_state_applies_every_knob() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let mut input = two_sided_input();
        input.knobs = ReelKnobs {
            map_direction: 6,
            auto_cast: true,
            auto_cast_toggles: vec![3, 17],
            continue_battle: vec![0],
            area_field_58c: 75,
            area_field_6e4: -3,
        };
        let state = build_state(&input, &flavor).expect("records decode");
        assert_eq!(state.map_direction, 6);
        assert!(state.auto_pcs_cast_magic);
        assert_eq!(state.auto_cast_toggles, vec![3, 17]);
        assert_eq!(state.continue_battle_yes, vec![0]);
        assert_eq!(state.area_field_58c, 75);
        assert_eq!(state.area_field_6e4, -3);
    }

    #[test]
    fn the_default_knobs_are_the_documented_pre_capture_fallbacks() {
        // These four numbers are load-bearing history: every replay before the
        // hooks emitted them rode exactly these values.
        let k = ReelKnobs::default();
        assert_eq!(k.map_direction, 2);
        assert_eq!(k.area_field_58c, 99);
        assert_eq!(k.area_field_6e4, 0);
        assert!(!k.auto_cast);
        assert!(k.auto_cast_toggles.is_empty());
        assert!(k.continue_battle.is_empty());
    }

    #[test]
    fn terrain_builds_the_map_and_absence_falls_back_to_open_floor() {
        let mut input = two_sided_input();
        assert_eq!(input.map().ground_tile(GridPos::new(0, 0)), FALLBACK_FLOOR);

        let mut ground = vec![FALLBACK_FLOOR; 50 * 25];
        ground[12 * 50 + 22] = 1; // a wall between the two combatants
        input.terrain = Some(ground);
        assert_eq!(input.map().ground_tile(GridPos::new(22, 12)), 1);
    }

    #[test]
    fn a_loadout_row_lands_on_its_roster_index() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let mut input = two_sided_input();
        input.loadouts = vec![(
            1,
            Loadout {
                ranged: Some((44, 0)),
                ammo_count: 7,
                ammo_readied: true,
                melee: None,
                unarmed_profile: (1, 8, 0),
                entry_ranged_readied: true,
            },
        )];
        let state = build_state(&input, &flavor).expect("records decode");
        assert!(
            state.roster()[0].loadout.is_none(),
            "index 0 carries no row"
        );
        assert_eq!(state.roster()[1].ammo, 7);
        assert_eq!(state.roster()[1].readied_weapon, Some((44, 0)));
    }

    // === the reel host ====================================================

    /// A fixture fight with enough combatants on both sides to run several
    /// rounds of walking and trading. Party icons in slots 0..2, one monster
    /// type in slot 8 — the real shape of every capture.
    fn fixture_reel_input() -> ReelInput {
        let mut combatants = Vec::new();
        for (i, (team, x, y, hp)) in [
            (Team::Party, 20, 11, 18),
            (Team::Party, 20, 12, 22),
            (Team::Party, 20, 13, 14),
            (Team::Monster, 26, 11, 12),
            (Team::Monster, 26, 12, 10),
            (Team::Monster, 26, 13, 11),
        ]
        .into_iter()
        .enumerate()
        {
            let monster = team == Team::Monster;
            let slot = if monster { 8 } else { i as u8 };
            combatants.push(ReelCombatant {
                team,
                pos: GridPos::new(x, y),
                record: synthetic_record(
                    if monster { "BRUTE" } else { "ALPHA" },
                    hp,
                    slot,
                    monster,
                ),
                affects: Vec::new(),
            });
        }
        let mut input = ReelInput::new("fixture.gbxtrace", 0x0C0F_FEE0, combatants);
        input.art.monster_blocks.insert(8, 4);
        input
    }

    fn fixture_reel(input: &ReelInput) -> Reel {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let identities: Vec<CombatantIdentity> = input
            .combatants
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let r = decode_char_record(&c.record).expect("fixture record decodes");
                CombatantIdentity::new(format!("{}{i}", r.name), r.icon_id as usize)
            })
            .collect();
        Reel::with_art(SceneArt::default(), identities, input, &flavor)
            .expect("the fixture fight starts")
    }

    fn fixture_sets() -> SymbolSets {
        let mut sets = SymbolSets::new();
        sets.load(
            4,
            gbx_formats::image::ImageBlock {
                height: 8,
                width_cols: 1,
                x_pos: 0,
                y_pos: 0,
                field_9: [0; 8],
                items: (0..40)
                    .map(|i| gbx_formats::image::DecodedItem {
                        pixels: vec![(i % 16) as u8; 64],
                    })
                    .collect(),
            },
        );
        sets
    }

    fn fixture_font() -> Font {
        use gbx_formats::font;
        let mut data = Vec::with_capacity(font::GLYPH_COUNT * font::GLYPH_BYTES);
        for j in 0..font::GLYPH_COUNT {
            data.extend_from_slice(&[j as u8; font::GLYPH_BYTES]);
        }
        font::decode(&data)
    }

    /// Runs a reel to completion, returning its final progress. Panics (loudly,
    /// by design) on any capture divergence along the way.
    fn play(reel: &mut Reel) -> ReelProgress {
        let sets = fixture_sets();
        let font = fixture_font();
        let mut fb = Framebuffer::new();
        let mut ticks = 0u32;
        while !reel.is_finished() {
            reel.tick(&mut fb, &sets, &font);
            ticks += 1;
            assert!(ticks < 200_000, "the fixture reel must finish");
        }
        assert!(ticks > 60, "a real fight takes more than a second of beats");
        reel.progress()
    }

    /// The headless draw stream of the same fight — what a capture of it would
    /// have recorded.
    fn headless_draws(input: &ReelInput) -> Vec<ExpectedDraw> {
        use crate::rng::RngSink;
        struct Tap(Rc<RefCell<Vec<RngDraw>>>);
        impl RngSink for Tap {
            fn on_draw(&mut self, d: RngDraw) {
                self.0.borrow_mut().push(d);
            }
        }
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let mut state = build_state(input, &flavor).expect("records decode");
        let taken = Rc::new(RefCell::new(Vec::new()));
        let mut rng = EngineRng::new(input.rng_state);
        rng.attach_sink(Box::new(Tap(Rc::clone(&taken))));
        while state.step(&mut rng) != CombatStep::Ended {}
        let out = taken
            .borrow()
            .iter()
            .map(|d| ExpectedDraw {
                before: d.before,
                after: d.after,
                operand: d.n,
            })
            .collect();
        out
    }

    #[test]
    fn a_reel_plays_its_fight_to_the_end() {
        let input = fixture_reel_input();
        let progress = play(&mut fixture_reel(&input));
        assert!(progress.finished);
        assert!(progress.steps > 5, "steps: {}", progress.steps);
        assert_eq!(progress.ticks_remaining, 0);
        assert_eq!(progress.label, "fixture.gbxtrace");
    }

    #[test]
    fn the_reel_checks_every_draw_against_the_capture() {
        // ★ The M6a claim, in miniature: give the reel the fight's own draw
        // stream as if it were a capture, and it plays through checking all of
        // them. The 15-capture version of this is the local-tier reel smoke.
        let mut input = fixture_reel_input();
        let expected = headless_draws(&input);
        assert!(expected.len() > 100, "the fixture fight is substantial");
        input.expected_draws = expected.clone();

        let progress = play(&mut fixture_reel(&input));
        assert!(progress.finished);
        assert_eq!(progress.draws_expected, expected.len());
        assert_eq!(
            progress.draws_checked,
            expected.len(),
            "every draw was checked, and the lengths agree"
        );
    }

    #[test]
    #[should_panic(expected = "REEL DIVERGENCE")]
    fn a_diverging_draw_stops_the_reel_loudly() {
        // A reel that has stopped being the captured fight must not scroll
        // prettily past — perturbing one operand deep in the stream is enough.
        let mut input = fixture_reel_input();
        let mut expected = headless_draws(&input);
        let i = expected.len() / 2;
        expected[i].operand = Some(expected[i].operand.unwrap_or(0).wrapping_add(1));
        input.expected_draws = expected;
        play(&mut fixture_reel(&input));
    }

    #[test]
    #[should_panic(expected = "drew MORE than the capture")]
    fn drawing_past_the_capture_stops_the_reel_loudly() {
        let mut input = fixture_reel_input();
        let mut expected = headless_draws(&input);
        expected.truncate(50);
        input.expected_draws = expected;
        play(&mut fixture_reel(&input));
    }

    #[test]
    #[should_panic(expected = "ENDED EARLY")]
    fn ending_before_the_capture_stops_the_reel_loudly() {
        let mut input = fixture_reel_input();
        let mut expected = headless_draws(&input);
        let tail = expected[expected.len() - 1];
        expected.push(tail);
        input.expected_draws = expected;
        play(&mut fixture_reel(&input));
    }

    #[test]
    fn the_tick_multiplier_changes_wall_time_and_nothing_else() {
        // D-CV3's open speed door: the host may tick faster; the fight, the
        // draws and the final roster are identical — only the number of host
        // ticks it took differs.
        let mut input = fixture_reel_input();
        input.expected_draws = headless_draws(&input);

        let slow = play(&mut fixture_reel(&input));
        let mut fast_input = input.clone();
        fast_input.tick_multiplier = 8;
        let fast = play(&mut fixture_reel(&fast_input));

        assert_eq!(slow.steps, fast.steps);
        assert_eq!(slow.draws_checked, fast.draws_checked);
        assert!(slow.finished && fast.finished);
    }

    #[test]
    fn an_empty_roster_has_nothing_to_play() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let input = ReelInput::new("empty.gbxtrace", 1, Vec::new());
        assert!(matches!(
            Reel::with_art(SceneArt::default(), Vec::new(), &input, &flavor).err(),
            Some(ReelError::NothingToPlay)
        ));
    }

    #[test]
    fn an_unpinned_monster_icon_is_refused_before_any_art_loads() {
        // The failure mode this catches is the nastiest one a viewer can have:
        // an unpinned monster would otherwise render as an empty slot — a
        // fight that LOOKS fine but is not showing you the enemy.
        let mut input = fixture_reel_input();
        input.art.monster_blocks.clear();
        let empty = GameData::from_files([] as [(String, Vec<u8>); 0]);
        // Roster order matters: the party icons would try to load first, so
        // this input starts at the monster.
        input.combatants.retain(|c| c.team == Team::Monster);
        let err = load_fight_art(&empty, CombatIcons::new(), &input).unwrap_err();
        assert_eq!(
            err,
            ReelError::UnpinnedMonsterIcon {
                slot: 8,
                name: "BRUTE".into()
            }
        );
        assert!(err.to_string().contains("monster_icons"));
    }

    #[test]
    fn an_icon_slot_past_the_store_is_refused() {
        let mut input = fixture_reel_input();
        input.combatants = vec![ReelCombatant {
            team: Team::Monster,
            pos: GridPos::new(24, 12),
            record: synthetic_record("HUGE", 10, 40, true),
            affects: Vec::new(),
        }];
        let empty = GameData::from_files([] as [(String, Vec<u8>); 0]);
        assert_eq!(
            load_fight_art(&empty, CombatIcons::new(), &input).unwrap_err(),
            ReelError::IconSlotOutOfRange {
                slot: 40,
                name: "HUGE".into()
            }
        );
    }

    #[test]
    fn draw_equality_uses_the_operand_only_when_both_sides_have_one() {
        use crate::rng::RngDraw;
        let ours = RngDraw {
            before: 1,
            after: 2,
            n: Some(20),
            result: Some(3),
        };
        assert!(draws_agree(
            &ours,
            &ExpectedDraw {
                before: 1,
                after: 2,
                operand: Some(20)
            }
        ));
        assert!(
            !draws_agree(
                &ours,
                &ExpectedDraw {
                    before: 1,
                    after: 2,
                    operand: Some(6)
                }
            ),
            "a differing operand is a divergence even when the chain matches"
        );
        assert!(
            draws_agree(
                &ours,
                &ExpectedDraw {
                    before: 1,
                    after: 2,
                    operand: None
                }
            ),
            "a capture with no recorded operand falls back to the chain"
        );
        assert!(!draws_agree(
            &ours,
            &ExpectedDraw {
                before: 1,
                after: 3,
                operand: Some(20)
            }
        ));
    }
}
