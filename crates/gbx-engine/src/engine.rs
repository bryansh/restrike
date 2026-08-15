//! `Engine::new` + `tick` (D-UI1, task deliverable 1): the tick core that
//! ties the framebuffer, text system, input queue, [`Shell`] state machine,
//! and the real [`EclMachine`] together into one `tick(input) -> Frame`
//! loop (M2 step 4 — step 3's `StubVm` is gone from production).
//!
//! The M2 session's hardcoded resident map ([`DEFAULT_GEO_FILE`]/
//! [`DEFAULT_GEO_BLOCK`]) is Tilverton City (`GEO2.DAX` block 1, per
//! `SOURCES.md`'s GEO row and `restrike map`'s prior verification against
//! *Cluebook.pdf*), and the resident ECL area ([`GAME_AREA`]/
//! [`INITIAL_ECL_BLOCK`]) is `ECL2.DAX` block 1 — the same area's `ECL2.DAX`
//! block 1 this session's demo/M1's `run-script` already validated against
//! real CotAB data. Real block/ECL *selection logic* (picking a different
//! area) is step 5+ scope; this session always starts on the one map/block.

use crate::boot::{self, BootError};
use crate::framebuffer::Framebuffer;
use crate::input::InputEvent;
use crate::movement::DefaultPartyPredicates;
use crate::rng::EngineRng;
use crate::shell::{EngineState, FlowCtx, Shell, SoundEvent};
use crate::symbols::SymbolSets;
use crate::text::{draw_string, TextCursor, TextPacer};
use crate::vmhost::{load_ecl_block, VmMemoryState};
use gbx_formats::font::Font;
use gbx_formats::game_data::GameData;
use gbx_formats::geo::GeoBlock;
use gbx_formats::image::{DecodedItem, ImageBlock};
use gbx_rules::pack::{RuleSet, VerifyReport};
use gbx_vm::{EclMachine, COTAB};

/// `GEO2.DAX` block 1 — Tilverton City (this session's fixed resident map).
pub const DEFAULT_GEO_FILE: &str = "GEO2.DAX";
pub const DEFAULT_GEO_BLOCK: u8 = 1;

/// `gbl.game_area`'s **boot/import default** — no longer the engine's one
/// area (roll-credits D-S1b): the live value is
/// [`EngineState::game_area`](crate::shell::EngineState::game_area), which a
/// script moves with `SAVE <n>, 0x7F12`.
///
/// `2` is the gameplay boot value (`seg001.cs:142` sets `game_area = 2` on
/// the non-demo path; `InitFirst`/`InitAgain`'s `game_area = 1` at `:276`/
/// `:369` are the title/demo-loop resets around it), and it is the area the
/// playthrough starts in — Tilverton, `ECL2.DAX`/`GEO2.DAX`. An imported
/// original save overrides it from its own `game_area` byte, and a `.rsav`
/// restore from the saved cell.
pub const GAME_AREA: u8 = 2;
/// The walk loop's default resident block when none was previously visited
/// (`area_ptr.LastEclBlockId == 0 -> EclBlockId = 1`, §1.6) — this session's
/// fixed boot block.
pub const INITIAL_ECL_BLOCK: u8 = 1;

/// A borrowed view of the engine-owned framebuffer + this tick's sounds
/// (D-UI1). Frontends only present + scale; palette expansion to RGBA is a
/// frontend/shared-helper concern, never engine state.
pub struct Frame<'a> {
    pub pixels: &'a [u8; crate::framebuffer::WIDTH * crate::framebuffer::HEIGHT],
    pub palette: &'a [[u8; 3]; 16],
    pub sounds: &'a [SoundEvent],
    pub serial: u64,
}

impl Frame<'_> {
    /// The D-UI7 golden surface, hex-encoded: `SHA-256(pixels ‖ palette)` —
    /// identical algorithm to [`Framebuffer::hash_hex`], exposed here so
    /// golden tests don't need their own `Framebuffer` handle.
    pub fn hash_hex(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.pixels);
        for rgb in self.palette {
            hasher.update(rgb);
        }
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// The tick core (D-UI1): owns the framebuffer, palette, PRNG, `GameData`
/// derived assets, input queue, UI shell state, and the real `EclMachine`.
pub struct Engine {
    fb: Framebuffer,
    font: Font,
    geo: GeoBlock,
    data: GameData,
    pub(crate) shell: Shell,
    pub(crate) state: EngineState,
    /// `pub(crate)` for the same reason `shell`/`state` are: the crate's own
    /// tests drive it. Slice 1's overland-exit acceptance parks the shell on a
    /// chosen address inside the resident block
    /// ([`crate::shell::boot_at_address`]).
    pub(crate) machine: EclMachine,
    pub(crate) vm_memory: VmMemoryState,
    party_predicates: DefaultPartyPredicates,
    /// The party roster (D-SAVE11/task deliverable 2) — empty until
    /// original-save import (D-SAVE5) or (a later session's) new-game
    /// character creation populates it; carried verbatim by `.rsav`
    /// save/restore (D-SAVE3).
    pub(crate) party: crate::party::Party,
    pub(crate) rng: EngineRng,
    input: crate::input::InputQueue,
    cursor: TextCursor,
    pacer: TextPacer,
    sounds: Vec<SoundEvent>,
    serial: u64,
    last_hash: Option<[u8; 32]>,
    /// The PRNG seed this engine was built/restored with — provenance only
    /// (D-SAVE4), recorded in every `.rsav`'s header alongside
    /// [`Engine::tick_count`]. Never re-seeds the live PRNG; resume always
    /// continues from the saved PRNG *state* (`self.rng`).
    boot_seed: u64,
    /// Ticks advanced since boot/restore — provenance only (D-SAVE4), a
    /// session/tick coordinate for pairing a save with an input trace
    /// during H5 debugging. Not load-bearing for resume.
    tick_count: u64,
    /// Resident 8×8 symbol sets + wallset slots (step 5's `load_walldef`
    /// deliverable) — `load_walldef`'s real target and `crate::corridor`'s
    /// texture source. Persistent across ticks (LOAD PIECES may reload it
    /// mid-game).
    symbol_sets: SymbolSets,
    /// The three boot-loaded `SKY` blocks (moon/sun/horizon) — read-only
    /// after boot.
    sky: [ImageBlock; 3],
    /// Boot's 26-slot combat icon store, thirteen `COMSPR` missile/effect/
    /// focus-box icons already in it (`seg001.cs:312-317`). Kept resident so
    /// the shell's combat host (M6 slice 6) has them at fight entry; a fixture
    /// engine that skips `boot()` carries an empty store, and an unloaded slot
    /// simply draws nothing (`ovr034.cs:92`).
    combat_icons: crate::combat_art::CombatIcons,
    /// The picture layer's decoded-asset cache (`crate::picture`): the
    /// resident PIC animation, BIGPIC, HEAD and BODY blocks, keyed by block
    /// id exactly as the original keys its own `lastDaxBlockId`/
    /// `bigpic_block_id`/`current_head_id`/`current_body_id`. Derivable from
    /// `EngineState::picture` + `data`, so it is not serialized and simply
    /// re-decodes on demand after a restore.
    pictures: crate::picture::PictureCache,
    /// D-RP4's verify-on-load report, computed once at boot against `data`
    /// and retained for the `verify_report` getter (boot diagnostics, the
    /// `restrike verify` CLI subcommand, and the inspector). Advisory only
    /// — never blocks or fails boot, never serialized into saves.
    verify_report: VerifyReport,
    /// The loaded rules pack (M3 step 6): the flavor's tables, kept resident
    /// so the party-facing screens (training XP thresholds, spell slots,
    /// prices, derived numbers) can read them each tick without re-parsing.
    /// Not serialized — reloaded from the embedded packs on restore/import.
    rules: RuleSet,
    /// Host-injected save-slot directory (M3 step 6 deliverable 3): the
    /// save/load screen renders from this; the host sets it by scanning the
    /// save dir (`saveload_fs::scan_slot_directory`). Transient, not saved.
    slots: crate::saveload::SlotDirectory,
    /// The save/load screen's pending action, taken by the host after a tick
    /// (D8: the core never does file I/O). Transient, not saved.
    io_request: Option<crate::saveload::SaveLoadRequest>,
    /// ★ Host-injected list of `.guy` character files (roll-credits slice 9c),
    /// the `Add Character to Party` picker's source. Transient, not saved.
    char_files: crate::chr_file::CharFileDirectory,
    /// ★ The character-file action creation/Remove/Add left for the host.
    /// Transient, not saved.
    char_io_request: Option<crate::chr_file::CharFileRequest>,
    /// **Watch mode** (`combat-visualizer.md` D-CV1 item 2): when set by
    /// [`Engine::new_reel`], every tick advances the captured fight's playback
    /// instead of the UI shell. `None` for a normally-booted engine, which is
    /// what keeps the walk loop and every existing golden untouched.
    reel: Option<Box<crate::combat::reel::Reel>>,
    /// A host→player notice with a tick budget ([`Engine::report_host_notice`])
    /// — the save/load outcome the frontend now has and the core never can
    /// (D8). Transient, never serialized, and drawn only when a frontend put
    /// one here, so every committed golden is untouched by its existence.
    host_notice: Option<HostNotice>,
    /// ★ `print_and_exit()` raised as a request (roll-credits slice 9b): the
    /// start menu's `Exit to DOS` and copy protection's third failure. Latched
    /// until the host reads it with [`Engine::quit_requested`]; never
    /// serialized, and nothing in the core acts on it.
    quit_requested: bool,
}

/// [`Engine::report_host_notice`]'s payload: the line to show and how many
/// ticks it has left.
struct HostNotice {
    text: String,
    ticks_left: u32,
}

/// How long a host notice stays on screen (~5s at 60 Hz) unless the player
/// presses a key first. Long enough to read a save-path error, short enough
/// that it never becomes furniture.
const HOST_NOTICE_TICKS: u32 = 300;

/// A trivial single-item `ImageBlock` fixture — [`Engine::new_fixture`]'s
/// stand-in for the boot-loaded `SKY` blocks (moon/sun/horizon) when a
/// fixture skips `boot()`'s real asset decode; the corridor renderer only
/// ever reads these outdoors, and no M2 fixture test currently exercises
/// that path.
fn dummy_sky() -> [ImageBlock; 3] {
    let block = ImageBlock {
        height: 8,
        width_cols: 1,
        x_pos: 0,
        y_pos: 0,
        field_9: [0; 8],
        items: vec![DecodedItem {
            pixels: vec![0; 64],
        }],
    };
    [block.clone(), block.clone(), block]
}

/// The fields `.rsav` restore (`save.rs`, D-SAVE3) and original-save import
/// (task deliverable 4) both reconstruct from stored/re-fetched state, as
/// opposed to `Engine::build`'s fresh-boot defaults (`party_predicates`/
/// `input`/`sounds`/`serial`/`last_hash` — none of which D-SAVE3 lists as
/// save-relevant, so [`Engine::assemble`] always starts them fresh).
pub(crate) struct AssembledEngine {
    pub fb: Framebuffer,
    pub font: Font,
    pub geo: GeoBlock,
    pub data: GameData,
    pub shell: Shell,
    pub state: EngineState,
    pub machine: EclMachine,
    pub vm_memory: VmMemoryState,
    pub party: crate::party::Party,
    pub rng: EngineRng,
    pub cursor: TextCursor,
    pub pacer: TextPacer,
    pub symbol_sets: SymbolSets,
    pub sky: [ImageBlock; 3],
    pub combat_icons: crate::combat_art::CombatIcons,
    pub verify_report: VerifyReport,
    /// Provenance continuity (D-SAVE4): a restored engine's *next* `.rsav`
    /// still reports where this session's PRNG stream/tick coordinate came
    /// from, rather than resetting to `(0, 0)`.
    pub boot_seed: u64,
    pub tick_count: u64,
}

impl Engine {
    /// Boots from real `GameData`: the D-UI1/`boot.rs` asset slice, this
    /// session's hardcoded resident GEO block, and the initial resident ECL
    /// block (`ECL{GAME_AREA}.DAX` block [`INITIAL_ECL_BLOCK`]).
    pub fn new(data: GameData, seed: u32) -> Result<Self, BootError> {
        let assets = boot::boot(&data)?;
        let geo_bytes = data.block(DEFAULT_GEO_FILE, DEFAULT_GEO_BLOCK)?;
        let geo = GeoBlock::parse(&geo_bytes)?;
        Ok(Self::build(
            assets.font,
            assets.symbol_sets,
            assets.sky,
            assets.combat_icons,
            geo,
            data,
            seed,
        ))
    }

    /// ★ **The front door** (roll-credits slice 9b): boots to the TITLE.DAX
    /// sequence, the Play-Demo prompt, the copy-protection challenge and
    /// `startGameMenu` — `seg001.PROGRAM`'s own order — instead of dropping
    /// straight into the resident block's entry vector.
    ///
    /// This is what a player launching the game gets. [`Engine::new`] keeps
    /// its bare-boot meaning (`--slot none`, and every fixture in the tree),
    /// and `import_original` keeps its straight-to-the-world meaning
    /// (`--slot X`, the power-user shortcut) — so no existing golden, walk
    /// trace or capture moves because this exists.
    ///
    /// `menuSelectedWord = 1` is `InitFirst`'s (`seg001.cs:297`), and it is
    /// load-bearing at the very next prompt: `BuildInputKeys("Play Demo")`
    /// yields two words, so word 1 — **Demo** — is the one `<Enter>` picks.
    pub fn new_front_door(data: GameData, seed: u32) -> Result<Self, BootError> {
        let mut engine = Engine::new(data, seed)?;
        engine.state.menu_selected_word = 1;
        // `Engine::build` paints the exploration frame for the walk loop; the
        // title screen owns the whole display instead.
        engine.fb.clear(0);
        engine.shell = Shell::FrontDoor(Box::default());
        Ok(engine)
    }

    /// Parks this engine on `startGameMenu` — where `loadGameMenu` returns
    /// (`ovr018.cs:223-228`).
    ///
    /// The host calls this after replacing the engine from a load the START
    /// MENU asked for, so the player lands back on the menu with the loaded
    /// party in front of them rather than mid-game (`saveload_fs::fulfill`
    /// does it automatically when the outgoing engine was on the front door).
    pub fn park_at_start_menu(&mut self) {
        self.shell = Shell::FrontDoor(Box::new(crate::front_door::FrontDoor::start_menu()));
    }

    /// Whether this engine is parked anywhere on the front door — the bit
    /// `saveload_fs::fulfill` carries across an engine replacement.
    pub fn at_front_door(&self) -> bool {
        matches!(self.shell, Shell::FrontDoor(_))
    }

    /// `seg043.print_and_exit()` as a request (D8): `true` once `Exit to DOS`
    /// has been confirmed, or copy protection has ejected the session. Latched
    /// — the host decides when to act.
    pub fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    /// D-RC4/D4: `true` restores the original copy-protection challenge (the
    /// player answers it); the default `false` pre-fills the answer.
    pub fn set_copy_protection_faithful(&mut self, faithful: bool) {
        self.state.copy_protection_faithful = faithful;
    }

    /// ★ **Watch mode** (`combat-visualizer.md` D-CV1 item 2): boots an engine
    /// that replays one captured fight *with pixels* instead of running the
    /// walk loop.
    ///
    /// This is `h4_replay` with a screen attached. The engine owns the
    /// `CombatState`, the `CombatScene` and the framebuffer exactly as it owns
    /// the shell in normal play, and exports the same `tick(&[InputEvent]) ->
    /// Frame` surface — so a frontend stays a thin presenter (D-UI6): it parses
    /// the capture with `gbx-oracle`, hands over the assembled
    /// [`ReelInput`](crate::combat::reel::ReelInput), and presents Frames.
    ///
    /// **Capture equality is asserted live**, between the step that made a draw
    /// and the frame that shows it — a reel that stops being the captured fight
    /// panics with `h4_replay`'s diagnostic rather than scrolling past.
    ///
    /// `data` must be a real CotAB set: the reel draws real art (CHEAD/CBODY,
    /// CPIC, DUNGCOM/RANDCOM) and boots the same resident assets normal play
    /// does. The PRNG seed is the capture's `rng_state`, so the engine-level
    /// `boot_seed` provenance names the fight being watched.
    pub fn new_reel(
        data: GameData,
        input: crate::combat::reel::ReelInput,
    ) -> Result<Self, ReelHostError> {
        use gbx_rules::adnd1::flavor_impl::Adnd1;

        let mut engine = Engine::new(data, input.rng_state)?;
        // Boot's COMSPR store (missiles, effects, the grey focus box) is now
        // resident — the shell's combat host needs it too, so the reel reads
        // the same copy instead of re-running the load.
        let boot_icons = engine.combat_icons.clone();
        let reel = {
            let flavor = Adnd1::new(&engine.rules);
            crate::combat::reel::Reel::new(&engine.data, boot_icons, &input, &flavor)?
        };
        engine.reel = Some(Box::new(reel));
        Ok(engine)
    }

    /// The reel's progress, when this engine is in watch mode.
    pub fn reel_progress(&self) -> Option<crate::combat::reel::ReelProgress> {
        self.reel.as_ref().map(|r| r.progress())
    }

    /// The reel itself (a **boundary** read — see
    /// [`Reel::state`](crate::combat::reel::Reel::state)).
    pub fn reel(&self) -> Option<&crate::combat::reel::Reel> {
        self.reel.as_deref()
    }

    /// The synthetic-fixture seam (task deliverable: hash goldens driven
    /// from a hand-authored `GeoBlock` + step-2 fixture assets, D10) —
    /// skips `boot()`'s asset decode, but `data` must still contain a real
    /// `ECL{GAME_AREA}.DAX` block [`INITIAL_ECL_BLOCK`] (`EclBuilder`-
    /// authored, see `shell.rs`'s test module for the synthetic-DAX
    /// pattern) since the real `EclMachine` always needs real bytecode to
    /// load. Panics if `symbol_sets` lacks set 4, or if that block can't be
    /// loaded — both fixture-authoring bugs, not runtime conditions
    /// `Engine::new`'s real-data path needs to handle gracefully.
    pub fn new_fixture(
        font: Font,
        symbol_sets: SymbolSets,
        geo: GeoBlock,
        data: GameData,
        seed: u32,
    ) -> Self {
        Self::build(
            font,
            symbol_sets,
            dummy_sky(),
            crate::combat_art::CombatIcons::new(),
            geo,
            data,
            seed,
        )
    }

    fn build(
        font: Font,
        symbol_sets: SymbolSets,
        sky: [ImageBlock; 3],
        combat_icons: crate::combat_art::CombatIcons,
        geo: GeoBlock,
        data: GameData,
        seed: u32,
    ) -> Self {
        let mut fb = Framebuffer::new();
        crate::frames::draw8x8_03(&mut fb, &symbol_sets)
            .expect("Engine::build: symbol set 4 must be loaded for the exploration frame");

        let mut state = EngineState::new();
        // The fresh-boot area (`seg001.cs:142`); everything below reads it
        // from the state, never the const.
        state.game_area = GAME_AREA;
        state.game_area_backup = GAME_AREA;
        let initial = load_ecl_block(&data, state.game_area, INITIAL_ECL_BLOCK)
            .expect("Engine::build: the initial resident ECL block must load");
        let mut machine =
            EclMachine::load_block(initial, &COTAB).unwrap_or_else(|never| match never {});
        state.ecl_block_id = INITIAL_ECL_BLOCK;
        let mut vm_memory = VmMemoryState::new();
        let shell = Shell::boot(&mut machine, &mut state, &mut vm_memory);

        // D-RP4: runs immediately after asset loads, never blocks or fails
        // boot -- RuleSet::load() panics only on a malformed *embedded*
        // pack (a shipped bug the D-RP7 CI suite already catches), and
        // verify() itself always returns a report, never an error. The
        // RuleSet is kept resident (M3 step 6) rather than dropped.
        let rules = RuleSet::load();
        let verify_report = rules.verify(&data);

        Engine {
            fb,
            font,
            geo,
            data,
            shell,
            state,
            machine,
            vm_memory,
            party_predicates: DefaultPartyPredicates::default(),
            party: crate::party::Party::default(),
            rng: EngineRng::new(seed),
            input: crate::input::InputQueue::new(),
            cursor: TextCursor::new(),
            pacer: TextPacer::new(4),
            sounds: Vec::new(),
            serial: 0,
            last_hash: None,
            // Provenance header field stays u64 (D-OR1): the u32 live seed is
            // zero-extended so `ContainerHeader.seed` and its byte layout don't
            // churn. Resume never reads this — it restores `SaveState.prng`.
            boot_seed: seed as u64,
            tick_count: 0,
            symbol_sets,
            sky,
            combat_icons,
            pictures: crate::picture::PictureCache::new(),
            verify_report,
            rules,
            slots: crate::saveload::SlotDirectory::new(),
            io_request: None,
            char_files: crate::chr_file::CharFileDirectory::new(),
            char_io_request: None,
            reel: None,
            host_notice: None,
            quit_requested: false,
        }
    }

    /// Assembles a live `Engine` from already-reconstructed state (`save.rs`'s
    /// `rebuild_engine`, shared by `.rsav` restore and original-save import
    /// — both are "given engine state + `GameData`, produce a running
    /// `Engine`"). Redraws the static exploration-frame background
    /// (`build()`'s own first step) so a restored engine's framebuffer isn't
    /// left blank before the next tick's redraw.
    pub(crate) fn assemble(a: AssembledEngine) -> Self {
        let mut fb = a.fb;
        crate::frames::draw8x8_03(&mut fb, &a.symbol_sets)
            .expect("Engine::assemble: symbol set 4 must be loaded for the exploration frame");
        // A save taken mid-scene carries the picture layer (D-SAVE3): put the
        // picture back before the first tick, since nothing else will —
        // `redraw_view` would only *clear* it. A picture whose asset is
        // missing simply doesn't draw; restore is not the place to be loud.
        let mut pictures = crate::picture::PictureCache::new();
        let _ = crate::picture::compose_into(
            &mut fb,
            &a.symbol_sets,
            &a.data,
            a.state.game_area,
            &a.state.picture,
            &mut pictures,
            0,
        );
        Engine {
            fb,
            font: a.font,
            geo: a.geo,
            data: a.data,
            shell: a.shell,
            state: a.state,
            machine: a.machine,
            vm_memory: a.vm_memory,
            party_predicates: DefaultPartyPredicates::default(),
            party: a.party,
            rng: a.rng,
            input: crate::input::InputQueue::new(),
            cursor: a.cursor,
            pacer: a.pacer,
            sounds: Vec::new(),
            serial: 0,
            last_hash: None,
            boot_seed: a.boot_seed,
            tick_count: a.tick_count,
            symbol_sets: a.symbol_sets,
            sky: a.sky,
            combat_icons: a.combat_icons,
            // Not carried in the save: re-decoded on demand from
            // `state.picture` + `data` (`crate::picture::PictureCache`), and
            // already warm from the restore-time compose above.
            pictures,
            verify_report: a.verify_report,
            // Reloaded from the embedded packs (not carried in the save/import).
            rules: RuleSet::load(),
            slots: crate::saveload::SlotDirectory::new(),
            io_request: None,
            char_files: crate::chr_file::CharFileDirectory::new(),
            char_io_request: None,
            reel: None,
            host_notice: None,
            quit_requested: false,
        }
    }

    /// Encodes this engine's full restorable state (D-SAVE3) as a `.rsav`
    /// file — `save.rs`'s `encode`, header'd with `self.boot_seed`/
    /// `self.tick_count` as provenance (D-SAVE4) and `data`'s fingerprint
    /// (D-SAVE2, load-bearing).
    pub fn save(&self) -> Vec<u8> {
        let state = crate::save::SaveState {
            ecl: self.machine.snapshot(),
            shell: self.shell.clone(),
            state: self.state.clone(),
            palette: *self.fb.palette(),
            cursor: self.cursor,
            pacer: self.pacer,
            windows: self.vm_memory.snapshot(),
            party: self.party.clone(),
            prng: self.rng.clone(),
        };
        crate::save::encode(&state, &self.data, self.boot_seed, self.tick_count)
    }

    /// Decodes a `.rsav` file and rebuilds a live `Engine` from it
    /// (D-SAVE2's full reject-not-migrate chain: container version, save-
    /// format version, flavor, then `data`'s fingerprint — see
    /// `save::SaveError` for every rejection's diagnostic).
    pub fn restore(bytes: &[u8], data: GameData) -> Result<Self, crate::save::SaveError> {
        let (header, state) = crate::save::load(bytes, &data)?;
        crate::save::rebuild_engine(&header, state, data)
    }

    /// Attaches a PRNG trace observer to the live engine (D-OR3, task
    /// deliverable 3) — the differential-capture seam. `gbx-oracle` provides
    /// the `.gbxtrace`-writing [`crate::rng::RngSink`] implementation; the
    /// engine core never depends on it. Inert by default: with no sink
    /// attached, the tick loop's draws pay nothing, and `save`/`restore` and
    /// the `.rsav` golden are unaffected (the sink is never serialized).
    /// Returns any previously attached sink.
    pub fn attach_rng_sink(
        &mut self,
        sink: Box<dyn crate::rng::RngSink>,
    ) -> Option<Box<dyn crate::rng::RngSink>> {
        self.rng.attach_sink(sink)
    }

    /// Detaches and returns the PRNG trace observer, if any (end of capture).
    pub fn take_rng_sink(&mut self) -> Option<Box<dyn crate::rng::RngSink>> {
        self.rng.take_sink()
    }

    /// Test/demo seam (the M3 party-predicate seam, task deliverable 4):
    /// mutable access to the party-predicate stand-in (bash/pick/knock
    /// availability and rolls) so a scripted trace can exercise every door
    /// path ahead of M3's real party model.
    pub fn party_predicates_mut(&mut self) -> &mut DefaultPartyPredicates {
        &mut self.party_predicates
    }

    /// The party roster (task deliverable 2) — read by a save/character
    /// screen. Empty until original-save import (D-SAVE5) populates it.
    pub fn party(&self) -> &crate::party::Party {
        &self.party
    }

    /// Injects the host's view of the save slots (M3 step 6 deliverable 3),
    /// obtained by scanning the save dir (`saveload_fs::scan_slot_directory`).
    /// The save/load screen renders from this — the core never scans itself.
    pub fn set_slot_directory(&mut self, slots: crate::saveload::SlotDirectory) {
        self.slots = slots;
    }

    /// The current injected slot directory (for the inspector / a frontend
    /// that wants to reflect what the screen sees).
    pub fn slot_directory(&self) -> &crate::saveload::SlotDirectory {
        &self.slots
    }

    /// The loaded rules pack (M3 step 6) — for a frontend/demo that drives the
    /// party-facing logic (training eligibility, prices) outside the tick loop.
    pub fn rules(&self) -> &RuleSet {
        &self.rules
    }

    /// Opens the training-hall screen for the current party (M3 step 6
    /// deliverable 4) — a frontend/demo entry point for stepping onto a town
    /// training-hall tile (the ECL trigger that would auto-open it is M6).
    /// Returns to the walk-loop world menu on exit.
    pub fn open_training(&mut self) {
        self.shell = Shell::Screen(crate::screens::Screen::Training(
            crate::screens::Training::new(
                self.state.selected_player,
                crate::screens::ReturnTo::World,
            ),
        ));
    }

    /// Opens the character sheet on the currently-selected member
    /// (`viewPlayer`, `ovr020.cs:236`) — the same screen camp's `View` word
    /// reaches, as a direct entry point for a frontend, a demo, or a test that
    /// wants the sheet's own leaves (Items since roll-credits slice 8) without
    /// walking the camp menu to get there. Returns to the world menu on exit.
    pub fn open_party_view(&mut self) {
        self.shell = Shell::Screen(crate::screens::Screen::PartyView(
            crate::screens::PartyView::from_index(
                self.state.selected_player,
                crate::screens::ReturnTo::World,
            ),
        ));
    }

    /// Opens `CityShop` over a caller-supplied stock — the direct entry point,
    /// beside [`Engine::open_party_view`]. The script's own route is
    /// `EnterShop` + `COMBAT`, which parks a
    /// [`ShopHost`](crate::shop_screen::ShopHost) inside the suspended vector
    /// run instead; both run the same screen.
    ///
    /// `CityShop` empties the pool on entry (`ovr007.cs:168`), so this does
    /// too.
    pub fn enter_shop(&mut self, shop: crate::shop::Shop) {
        self.state.pooled_money.clear();
        self.shell = Shell::Screen(crate::screens::Screen::Shop(Box::new(
            crate::shop_screen::ShopHost::new(shop, &self.party),
        )));
    }

    /// Takes the save/load screen's pending action, if any — the host calls
    /// this after each tick and fulfills it (write/restore/import) via
    /// `saveload_fs`. Clears the request so it fires exactly once.
    pub fn take_io_request(&mut self) -> Option<crate::saveload::SaveLoadRequest> {
        self.io_request.take()
    }

    /// ★ Injects the host's view of the save directory's `.guy` character
    /// files (roll-credits slice 9c) — `Add Character to Party` renders from
    /// this, exactly as the save/load screen renders from the slot directory.
    pub fn set_char_file_directory(&mut self, files: crate::chr_file::CharFileDirectory) {
        self.char_files = files;
    }

    pub fn char_file_directory(&self) -> &crate::chr_file::CharFileDirectory {
        &self.char_files
    }

    /// ★ Takes the pending character-file action (write a `.guy`, or load
    /// one), if any — the host fulfills it via
    /// [`crate::saveload_fs::fulfill_char_file`]. Clears it, so it fires once.
    pub fn take_char_file_request(&mut self) -> Option<crate::chr_file::CharFileRequest> {
        self.char_io_request.take()
    }

    /// ★ `AddPlayer`'s join step (`ovr018.cs:1488-1548`): applies the party
    /// legality gate, and on success assigns the icon slot and appends the
    /// member. `Err` carries the original's own refusal words, which the
    /// caller is expected to show.
    pub fn add_character(&mut self, mut ch: crate::party::Character) -> Result<(), &'static str> {
        if let Some(why) = crate::chr_file::join_refusal(&self.party, &ch) {
            return Err(why);
        }
        crate::chr_file::assign_icon_id(&self.party, &mut ch);
        self.party.members.push(ch);
        self.state.party_size = self.party.members.len() as u8;
        Ok(())
    }

    /// ★ Shows a host-authored line to the **player** for
    /// [`HOST_NOTICE_TICKS`] ticks (or until the next keypress).
    ///
    /// D8 keeps file I/O outside the tick core, which means the *outcome* of a
    /// save/load only ever exists on the host side — and the M6 forensics rule
    /// is that a report no frontend shows is not a report. Bryan's 2026-08-03
    /// "ghost fight" was three silent failures stacked; a save that quietly
    /// doesn't write would be the same bug wearing a different hat. So the
    /// frontend hands the sentence back and the engine paints it, over
    /// whatever is on screen, in the exploration text window's first row
    /// (`NORMAL_BOTTOM.y_start`, where the game's own prose goes).
    ///
    /// Never called by the core itself: a golden that doesn't set a notice
    /// draws exactly what it drew before.
    pub fn report_host_notice(&mut self, text: impl Into<String>) {
        self.host_notice = Some(HostNotice {
            text: text.into(),
            ticks_left: HOST_NOTICE_TICKS,
        });
    }

    /// The currently-showing host notice, if any (frontend/test seam).
    pub fn host_notice(&self) -> Option<&str> {
        self.host_notice.as_ref().map(|n| n.text.as_str())
    }

    /// ★ Recomposes the exploration screen from current state — the host's
    /// post-`.rsav`-restore repaint.
    ///
    /// A restored engine's framebuffer carries the static frame and the
    /// picture layer ([`Engine::assemble`]) but **not** the 3D viewport: the
    /// walk loop only ever paints that at world-menu entry
    /// ([`Shell::enter_world_menu`]), and a restore lands *inside* an
    /// already-entered menu, so nothing would repaint it until the player
    /// walked. The original has the same problem and the same answer — its
    /// boot-recompose path is gated on `reload_ecl_and_pictures`, the flag
    /// **only a save-load sets** (`enter_world_menu`'s design note).
    ///
    /// Drawing only: no shell transition, no `field_592` zeroing, no state
    /// mutation beyond the picture layer's own hide-on-redraw. A no-op while a
    /// full-screen [`Shell::Screen`], a fight or the reel owns the display —
    /// each of those repaints itself every tick.
    pub fn recompose_world_screen(&mut self) {
        if self.reel.is_some() || !self.shell.draws_engine_status_line() {
            return;
        }
        let mut ctx = self.flow_ctx();
        crate::corridor::redraw_view(&mut ctx);
    }

    /// The per-tick [`FlowCtx`] bundle, borrowed out of this engine's own
    /// fields. `tick` deliberately keeps its own inline copy (it needs
    /// `self.shell` alongside the context, which a whole-`self` borrow would
    /// forbid); every other site that just wants to *draw* through the shared
    /// helpers uses this.
    fn flow_ctx(&mut self) -> FlowCtx<'_> {
        FlowCtx {
            machine: &mut self.machine,
            vm_memory: &mut self.vm_memory,
            data: &self.data,
            input: &mut self.input,
            dt_ticks: 1,
            state: &mut self.state,
            geo: &mut self.geo,
            party: &mut self.party_predicates,
            roster: &mut self.party,
            rules: &self.rules,
            slots: &self.slots,
            io_request: &mut self.io_request,
            char_files: &self.char_files,
            char_io_request: &mut self.char_io_request,
            quit_requested: &mut self.quit_requested,
            rng: &mut self.rng,
            fb: &mut self.fb,
            font: &self.font,
            cursor: &mut self.cursor,
            pacer: &mut self.pacer,
            sounds: &mut self.sounds,
            symbols: &mut self.symbol_sets,
            sky: &self.sky,
            combat_icons: &self.combat_icons,
            pictures: &mut self.pictures,
        }
    }

    /// The ScriptMemory unknown-access log + service-call log + halt
    /// diagnostics (task deliverable 4's discovery-backlog/halt-policy
    /// surface) — read by the demo and, eventually, `tools/inspect`.
    pub fn vm_memory(&self) -> &VmMemoryState {
        &self.vm_memory
    }

    /// Drains the transcript-mode content log (M2 step 8: `restrike walk
    /// --transcript`) accumulated since the last call — every PRINT/
    /// PRINTCLEAR text and VM-request label emitted by `tick` calls in
    /// between. A frontend calls this once per tick (or in a batch) to
    /// stream content out; the engine itself does no I/O (D8).
    pub fn take_transcript(&mut self) -> Vec<crate::vmhost::TranscriptEntry> {
        std::mem::take(&mut self.vm_memory.transcript)
    }

    pub fn state(&self) -> &EngineState {
        &self.state
    }

    /// The live PRNG state (`DS:0x47F0`) — a synchronization point for the
    /// oracle rig (D-OR1), for the shell-path draw-parity test (which forks a
    /// headless fight from exactly this value), and for the H5 state digest.
    pub fn prng_state(&self) -> u32 {
        self.rng.state()
    }

    /// ★ The **H5 checkpoint** (roll-credits D-RC3): a deterministic hash of
    /// the engine state a playthrough cares about — position/area/block/clock/
    /// party/PRNG/`search_flags` **plus the ScriptMemory windows**, where the
    /// quest flags live.
    ///
    /// Read [`crate::digest`]'s module doc before touching anything it hashes:
    /// **the field order is append-only forever**, because a digest is a bare
    /// hash in a trace file and a reshaped one silently invalidates every
    /// recorded run. Deliberately not a frame hash — the M6 arc showed those
    /// die to the renderer's own progress.
    pub fn state_digest(&self) -> String {
        crate::digest::state_digest(self)
    }

    /// The engine-owned framebuffer — a demo/test seam for dumping a frame
    /// outside the tick loop (`Frame` borrows it, so a caller that already
    /// dropped its `Frame` needs this).
    #[cfg(test)]
    pub(crate) fn framebuffer_for_demo(&self) -> &Framebuffer {
        &self.fb
    }

    /// The UI shell state machine's current node (`Boot`/`WorldMenu`/`Look`/
    /// `Step`/`GameOver`) — read by `tools/inspect`'s live engine pane
    /// (D-UI8) to show what flow stage the machine is in. `Shell` is a plain
    /// serde-able enum with no engine-internal borrows, so this is a
    /// read-only seam, not a control surface.
    /// The shell's one-line debug summary ([`Shell::probe`]) — for frontend
    /// debug logs (`RESTRIKE_DEBUG_LOG`).
    pub fn probe(&self) -> String {
        self.shell.probe()
    }

    pub fn shell(&self) -> &Shell {
        &self.shell
    }

    /// The resident GEO block (this session's fixed Tilverton City map) —
    /// read by `tools/inspect`'s resource browser so it can render the
    /// automap using the same block the live engine pane is walking,
    /// without re-parsing `GameData` itself.
    pub fn geo(&self) -> &GeoBlock {
        &self.geo
    }

    /// The boot-loaded mono font — read by `tools/inspect` for glyph-grid
    /// rendering and text-layout debugging.
    pub fn font(&self) -> &Font {
        &self.font
    }

    /// The `GameData` this engine was built from — read by `tools/inspect`'s
    /// resource browser so it can reuse the live engine's already-loaded
    /// archive set instead of loading a second copy from disk.
    pub fn game_data(&self) -> &GameData {
        &self.data
    }

    /// Resident 8×8 symbol sets + wallset slots (step 5's `load_walldef`
    /// deliverable) — read by the demo/tests to confirm `load_walldef`
    /// populated the wall-texture data a walk exercises.
    pub fn symbol_sets(&self) -> &SymbolSets {
        &self.symbol_sets
    }

    /// Sets the game-speed setting text pacing derives from
    /// (`seg041.cs:97-100`: `SysDelay(game_speed_var * 3)` ms per character;
    /// the shipped default is 4). This is the knob the original's camp
    /// Alter ▸ Speed menu adjusts — until that screen lands, frontends may
    /// expose it directly (`RESTRIKE_GAME_SPEED` on the desktop): the same
    /// range, the original's own semantics, D4-clean.
    pub fn set_game_speed(&mut self, speed: u8) {
        self.pacer = TextPacer::new(speed.clamp(1, 9));
    }

    /// D-RP4's verify-on-load report (boot diagnostics, the `restrike
    /// verify` CLI subcommand, and the inspector). Advisory only.
    pub fn verify_report(&self) -> &VerifyReport {
        &self.verify_report
    }

    /// Advances by one tick (D-UI1): dispatches `input`, advances the UI
    /// shell within its step budget, and recomposes the status line before
    /// returning a borrowed view of the framebuffer.
    pub fn tick(&mut self, input: &[InputEvent]) -> Frame<'_> {
        self.tick_count += 1;
        self.input.push_all(input);
        self.sounds.clear();

        if self.reel.is_some() {
            return self.tick_reel();
        }

        {
            let mut ctx = FlowCtx {
                machine: &mut self.machine,
                vm_memory: &mut self.vm_memory,
                data: &self.data,
                input: &mut self.input,
                dt_ticks: 1,
                state: &mut self.state,
                geo: &mut self.geo,
                party: &mut self.party_predicates,
                roster: &mut self.party,
                rules: &self.rules,
                slots: &self.slots,
                io_request: &mut self.io_request,
                char_files: &self.char_files,
                char_io_request: &mut self.char_io_request,
                quit_requested: &mut self.quit_requested,
                rng: &mut self.rng,
                fb: &mut self.fb,
                font: &self.font,
                cursor: &mut self.cursor,
                pacer: &mut self.pacer,
                sounds: &mut self.sounds,
                symbols: &mut self.symbol_sets,
                sky: &self.sky,
                combat_icons: &self.combat_icons,
                pictures: &mut self.pictures,
            };
            self.shell.tick(&mut ctx);
            // The parked widget's prompt-row line (`Widget::display_line`) —
            // menus were engine-state-only until 2026-08-03: every script
            // menu (and the world menu) ran INVISIBLE, playable only blind.
            self.shell.draw_parked_widget(&mut ctx);
        }

        // The position/time status line (§1.9): row 15, cols 17-38,
        // refreshed after every command. Redrawn unconditionally each tick
        // (a documented simplification — the original's per-field redraw
        // discipline is step-5 rendering scope; unconditional redraw is
        // idempotent and keeps hash-goldens simple).
        //
        // Except over a fight, and except over a full-screen `Shell::Screen`
        // — see [`Shell::draws_engine_status_line`] for the per-state coab
        // derivation. The reel host skips it the same way (`tick_reel` never
        // reaches here).
        if self.shell.draws_engine_status_line() {
            // The right-panel roster (`PartySummary`, `ovr025.cs:216-261`):
            // the original repaints it at every walk-loop step
            // (`ovr003.cs:2217-2219`) and every recompose (`LoadPic`'s
            // arms), so outside combat and the full-screen Screens it is
            // simply always there — over the intro's pictures and parked
            // script menus too (Bryan's 2026-08-08 DOSBox side-by-sides).
            // Camp composes its own copy inside its layout; combat replaces
            // the whole screen. An empty roster draws nothing at all: a
            // bare fixture boot has no party, and the original cannot reach
            // the walk loop partyless.
            //
            // ★ NOT in the wilderness: `PartySummary` opens with
            // `if (gbl.game_state == GameState.WildernessMap) return;`
            // (`ovr025.cs:218-221`) — the overland screen has no roster
            // band. The status line's own predicate is separate
            // (`Shell::draws_engine_status_line`) and unchanged.
            // ★ And the status line is not in the wilderness either:
            // `display_map_position_time` opens with
            // `if (gbl.game_state != GameState.WildernessMap)`
            // (`ovr025.cs:1476-1478`) — the overland screen is the BIGPIC and
            // the script's own text, nothing else. (Roll-credits D-S7d; it
            // also keeps the position line off the map cells the
            // `crate::mapcursor` blink owns.)
            let wilderness = self.state.game_state == crate::shell::GameState::WildernessMap;
            if !wilderness && !self.party.members.is_empty() {
                let rows: Vec<_> = self
                    .party
                    .members
                    .iter()
                    .map(crate::charsheet::sheet_view)
                    .collect();
                let selected = Some((self.state.selected_player as usize) % rows.len());
                crate::charsheet::render_party_summary(&mut self.fb, &self.font, &rows, selected);
            }
            if !wilderness {
                let status = Shell::status_line(&self.state);
                draw_string(&mut self.fb, &self.font, &status, 15, 17, 0, 10);
            }
        }

        // The host's save/load verdict, last so it lands on top of whatever
        // owns the screen (the walk loop, camp, the death screen) — but never
        // over a fight, which composes the whole display itself.
        if self.shell.combat_host().is_none() {
            self.draw_host_notice(!input.is_empty());
        }

        let hash = self.fb.hash();
        if self.last_hash != Some(hash) {
            self.serial += 1;
            self.last_hash = Some(hash);
        }

        Frame {
            pixels: self.fb.pixels(),
            palette: self.fb.palette(),
            sounds: &self.sounds,
            serial: self.serial,
        }
    }

    /// Paints the live host notice (if any) into the exploration text
    /// window's first row, counting its budget down; clears the row once, on
    /// the tick it expires, so it never becomes permanent furniture.
    /// `dismissed` (any key this tick) retires it early.
    fn draw_host_notice(&mut self, dismissed: bool) {
        let Some(notice) = &mut self.host_notice else {
            return;
        };
        let row = crate::text::NORMAL_BOTTOM.y_start;
        let (x0, x1) = (0usize, 39usize);
        if dismissed || notice.ticks_left == 0 {
            self.host_notice = None;
            crate::draw::cell_rect_fill(&mut self.fb, 0, row, row, x0, x1);
            return;
        }
        notice.ticks_left -= 1;
        let text = notice.text.clone();
        crate::draw::cell_rect_fill(&mut self.fb, 0, row, row, x0, x1);
        draw_string(&mut self.fb, &self.font, &text, row, 1, 0, 15);
    }

    /// The watch-mode tick: the reel advances the captured fight's playback and
    /// paints the whole combat screen, so none of normal play's tick runs — no
    /// shell, no VM, and **no exploration status line** over the fight.
    ///
    /// Keys are drained rather than dispatched: the reel is a viewer, and the
    /// combat menu's real keys land with M6c's suspensions.
    fn tick_reel(&mut self) -> Frame<'_> {
        self.input.clear();
        let mut reel = self.reel.take().expect("watch mode was just checked");
        let cues = reel.tick(&mut self.fb, &self.symbol_sets, &self.font);
        self.sounds.extend_from_slice(cues);
        self.reel = Some(reel);

        let hash = self.fb.hash();
        if self.last_hash != Some(hash) {
            self.serial += 1;
            self.last_hash = Some(hash);
        }
        Frame {
            pixels: self.fb.pixels(),
            palette: self.fb.palette(),
            sounds: &self.sounds,
            serial: self.serial,
        }
    }

    pub fn title(&self) -> &str {
        "Restrike"
    }
}

/// [`Engine::new_reel`]'s failure modes: booting the data, or building the reel
/// over it.
#[derive(Debug, Clone, PartialEq)]
pub enum ReelHostError {
    Boot(BootError),
    Reel(crate::combat::reel::ReelError),
}

impl std::fmt::Display for ReelHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReelHostError::Boot(e) => write!(f, "the reel's game data did not boot: {e:?}"),
            ReelHostError::Reel(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReelHostError {}

impl From<BootError> for ReelHostError {
    fn from(e: BootError) -> Self {
        ReelHostError::Boot(e)
    }
}
impl From<crate::combat::reel::ReelError> for ReelHostError {
    fn from(e: crate::combat::reel::ReelError) -> Self {
        ReelHostError::Reel(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::image::{DecodedItem, ImageBlock};
    use gbx_vm::test_support::EclBuilder;

    fn synthetic_set4() -> ImageBlock {
        ImageBlock {
            height: 8,
            width_cols: 1,
            x_pos: 0,
            y_pos: 0,
            field_9: [0; 8],
            items: (0..40)
                .map(|i| DecodedItem {
                    pixels: vec![(i % 16) as u8; 64],
                })
                .collect(),
        }
    }

    fn synthetic_font() -> Font {
        let mut data =
            Vec::with_capacity(gbx_formats::font::GLYPH_COUNT * gbx_formats::font::GLYPH_BYTES);
        for j in 0..gbx_formats::font::GLYPH_COUNT {
            data.extend_from_slice(&[j as u8; gbx_formats::font::GLYPH_BYTES]);
        }
        gbx_formats::font::decode(&data)
    }

    fn open_geo() -> GeoBlock {
        GeoBlock::parse(&vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE]).unwrap()
    }

    /// A minimal single-block `ECL{GAME_AREA}.DAX` whose block
    /// [`INITIAL_ECL_BLOCK`] is a real, resolvable-header EXIT-only script
    /// (`shell.rs`'s test module owns the general-purpose builder; this
    /// crate-internal copy keeps `engine.rs`'s own tests self-contained).
    fn exit_only_game_data() -> GameData {
        let mut b = EclBuilder::new();
        for _ in 0..5 {
            b.raw(&[0]);
            b.imm_word_label("entry");
        }
        b.label("entry");
        b.op(0x00); // EXIT
        let bytecode = b.build_bytes();

        let mut raw = vec![0u8, 0u8]; // load_ecl_dax's 2-byte prefix
        raw.extend_from_slice(&bytecode);

        let comp: Vec<u8> = raw
            .chunks(128)
            .flat_map(|chunk| {
                let mut v = vec![(chunk.len() - 1) as u8];
                v.extend_from_slice(chunk);
                v
            })
            .collect();
        let mut file = Vec::new();
        file.extend_from_slice(&9u16.to_le_bytes());
        file.push(INITIAL_ECL_BLOCK);
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        file.extend_from_slice(&(comp.len() as u16).to_le_bytes());
        file.extend_from_slice(&comp);

        GameData::from_files([(format!("ECL{GAME_AREA}.DAX"), file)])
    }

    fn engine() -> Engine {
        let mut sets = SymbolSets::new();
        sets.load(4, synthetic_set4());
        Engine::new_fixture(synthetic_font(), sets, open_geo(), exit_only_game_data(), 1)
    }

    /// Any pixel lit inside the roster panel's cells (rows 2-9, cols 17-38).
    fn roster_band_lit(e: &Engine) -> bool {
        let fb = e.framebuffer_for_demo();
        (17 * 8..39 * 8).any(|x| (2 * 8..10 * 8).any(|y| fb.get_pixel(x, y) != 0))
    }

    /// ★ The walk loop draws the right-panel roster (`PartySummary`,
    /// `ovr025.cs:216-261`; repainted per step at `ovr003.cs:2217-2219`) —
    /// Bryan's 2026-08-08 DOSBox side-by-sides show it over the intro and
    /// the bar menu alike, where ours drew a black panel. Empty roster =
    /// nothing (a bare fixture boot); a full-screen `Shell::Screen`
    /// suppresses it exactly as it does the status line.
    #[test]
    fn the_walk_loop_paints_the_party_roster_panel() {
        let mut e = engine();
        for _ in 0..5 {
            e.tick(&[]);
        }
        assert!(
            !roster_band_lit(&e),
            "an empty roster draws no panel (bare fixture boot)"
        );

        let mut member = crate::test_support::blank_character();
        member.name = "MATHEW".into();
        member.hit_point_current = 49;
        e.party.members.push(member);
        e.tick(&[]);
        assert!(
            roster_band_lit(&e),
            "a real roster paints the NAME/AC/HP panel every walk-loop tick"
        );

        e.shell = Shell::Screen(crate::screens::Screen::SaveLoad(
            crate::screens::SaveLoad::new(crate::screens::ReturnTo::World),
        ));
        e.tick(&[]);
        assert!(
            !roster_band_lit(&e),
            "a full-screen Screen owns the whole framebuffer — no panel over it"
        );
    }

    fn roster_band_pixels(e: &Engine) -> Vec<u8> {
        let fb = e.framebuffer_for_demo();
        (2 * 8..10 * 8)
            .flat_map(|y| (17 * 8..39 * 8).map(move |x| (x, y)))
            .map(|(x, y)| fb.get_pixel(x, y))
            .collect()
    }

    /// ★ `PartySummary` early-returns in the wilderness
    /// (`ovr025.cs:218-221`: `if (gbl.game_state == GameState.WildernessMap)
    /// return;`) — the overland screen carries no roster band, and the
    /// original's own wilderness recompose (`draw_bigpic`) is what covers
    /// the cells, so "returns before drawing" is the whole behaviour: the
    /// band simply stops being repainted. Only the roster's predicate
    /// moves; the status line has its own
    /// (`Shell::draws_engine_status_line`) and keeps drawing.
    #[test]
    fn the_roster_panel_stops_repainting_in_the_wilderness() {
        let mut e = engine();
        let mut member = crate::test_support::blank_character();
        member.name = "MATHEW".into();
        member.hit_point_current = 49;
        member.hit_point_max = 49;
        e.party.members.push(member);
        for _ in 0..5 {
            e.tick(&[]);
        }
        assert!(roster_band_lit(&e), "the dungeon walk loop paints it");
        let painted = roster_band_pixels(&e);

        // In the wilderness, a roster change must not reach the panel.
        e.state.game_state = crate::shell::GameState::WildernessMap;
        e.party.members[0].name = "SOMEONEELSE".into();
        e.party.members[0].hit_point_current = 1;
        e.tick(&[]);
        assert_eq!(
            roster_band_pixels(&e),
            painted,
            "PartySummary returns before drawing anything in the wilderness"
        );
        assert!(
            status_band_lit(&e),
            "the status line's own predicate is unchanged"
        );

        // Indoors again, the same change lands.
        e.state.game_state = crate::shell::GameState::DungeonMap;
        e.tick(&[]);
        assert_ne!(
            roster_band_pixels(&e),
            painted,
            "and the panel comes back indoors"
        );
    }

    /// ★ FD-31 resolved: the sky colour reads the REAL Area cells
    /// (`outdoor_sky_colour`/`indoor_sky_colour`, DataOffset `0x1FA`/`0x1FC`
    /// = addrs `0x4BFD`/`0x4BFE` under the halved mapping). A script `SAVE`
    /// into the outdoor cell must tint the 3D view's sky band — the
    /// un-halved addresses this replaces always read 0, so every sky since
    /// M2 was black no matter what the save or a script put there.
    #[test]
    fn a_script_written_sky_colour_tints_the_view() {
        use crate::test_support::labeled_block;
        let block = labeled_block(["entry"; 5], |b| {
            b.label("entry");
            b.op(0x09).imm_byte(3).imm_word(0x4BFD); // SAVE 3 -> outdoor sky
            b.op(0x00);
        });
        let data = crate::test_support::ecl_game_data(GAME_AREA, vec![(1, block)]);
        let mut sets = SymbolSets::new();
        sets.load(4, synthetic_set4());
        let mut e = Engine::new_fixture(synthetic_font(), sets, open_geo(), data, 1);
        for _ in 0..10 {
            e.tick(&[]);
        }
        // SKY_COLOURS[3] = 0x0B; the sky band sits above the horizon in the
        // viewport interior. An all-zero GEO is outdoor everywhere.
        let fb = e.framebuffer_for_demo();
        let lit = (24..176usize).any(|x| (24..60usize).any(|y| fb.get_pixel(x, y) == 0x0B));
        assert!(lit, "the outdoor sky band must show SKY_COLOURS[3]");
    }

    /// ★ The indoor sibling: `RedrawView` picks between the two Area cells on
    /// the standing square's `indoor` flag (`ovr029.cs:23-32`,
    /// `mapWallRoof > 0x7F`), so an INDOOR square must tint from
    /// `indoor_sky_colour` (DataOffset `0x1FC` = addr `0x4BFE`) and ignore
    /// the outdoor cell entirely. `0x4BFE` is also one of the two addresses
    /// whose writes arm `byte_1EE94`, which is why a sky write forces the
    /// redraw that shows it.
    #[test]
    fn an_indoor_square_tints_from_the_indoor_sky_cell() {
        use crate::test_support::labeled_block;
        let block = labeled_block(["entry"; 5], |b| {
            b.label("entry");
            b.op(0x09).imm_byte(3).imm_word(0x4BFD); // outdoor cell: a decoy
            b.op(0x09).imm_byte(4).imm_word(0x4BFE); // indoor cell -> SKY_COLOURS[4] = 0x0D
            b.op(0x00);
        });
        let data = crate::test_support::ecl_game_data(GAME_AREA, vec![(1, block)]);
        let mut sets = SymbolSets::new();
        sets.load(4, synthetic_set4());

        // The party stands at (0,0); set that square's `x2` high bit
        // (`indoor`) — plane 2, index `x + 16*y`, after the 2-byte header.
        let mut geo_bytes = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        geo_bytes[2 + 2 * 256] = 0x80;
        let geo = GeoBlock::parse(&geo_bytes).unwrap();

        let mut e = Engine::new_fixture(synthetic_font(), sets, geo, data, 1);
        for _ in 0..10 {
            e.tick(&[]);
        }
        let fb = e.framebuffer_for_demo();
        let band: Vec<u8> = (24..176usize)
            .flat_map(|x| (24..60usize).map(move |y| (x, y)))
            .map(|(x, y)| fb.get_pixel(x, y))
            .collect();
        assert!(
            band.contains(&0x0D),
            "an indoor square must show SKY_COLOURS[4] = 0x0D from the indoor cell"
        );
        assert!(
            !band.contains(&0x0B),
            "the outdoor cell's SKY_COLOURS[3] = 0x0B must not appear indoors"
        );
    }

    #[test]
    fn tick_returns_a_frame_and_bumps_serial_on_change() {
        let mut e = engine();
        let f0 = e.tick(&[]);
        assert_eq!(f0.pixels.len(), 320 * 200);
        let serial0 = f0.serial;
        assert!(serial0 >= 1, "the very first tick must bump serial from 0");
    }

    #[test]
    fn repeated_empty_ticks_keep_the_same_serial_once_stable() {
        let mut e = engine();
        e.tick(&[]);
        let f1 = e.tick(&[]);
        let s1 = f1.serial;
        let f2 = e.tick(&[]);
        assert_eq!(f2.serial, s1, "an unchanged frame must not bump serial");
    }

    #[test]
    fn title_is_stable() {
        let e = engine();
        assert_eq!(e.title(), "Restrike");
    }

    #[test]
    fn engine_reaches_world_menu_headlessly() {
        let mut e = engine();
        for _ in 0..5 {
            e.tick(&[]);
        }
        assert!(matches!(e.shell, Shell::WorldMenu { .. }));
    }

    /// Any pixel lit inside the position/time line's cells (row 15, cols
    /// 17-38).
    fn status_band_lit(e: &Engine) -> bool {
        let fb = e.framebuffer_for_demo();
        (17 * 8..39 * 8).any(|x| (15 * 8..16 * 8).any(|y| fb.get_pixel(x, y) != 0))
    }

    /// ★ The status line no longer overdraws a full-screen `Shell::Screen`
    /// (`display_map_position_time` runs only at `LoadPic`'s own recomposition
    /// sites, `ovr025.cs:1398-1455` — never over the character sheet or the
    /// save/load screen).
    #[test]
    fn the_position_time_line_is_suppressed_while_a_screen_is_parked() {
        let mut e = engine();
        for _ in 0..5 {
            e.tick(&[]);
        }
        assert!(matches!(e.shell, Shell::WorldMenu { .. }));
        assert!(status_band_lit(&e), "the walk loop paints the status line");

        e.shell = Shell::Screen(crate::screens::Screen::SaveLoad(
            crate::screens::SaveLoad::new(crate::screens::ReturnTo::World),
        ));
        e.tick(&[]);
        assert!(
            !status_band_lit(&e),
            "the save/load screen's layout has no position/time line — the engine \
             must not paint one over it"
        );
    }
}
