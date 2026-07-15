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

/// `gbl.game_area` (this session's research pass, `vmhost.rs`'s citation):
/// fixed at the value already validated against real Tilverton data
/// (`ECL2.DAX`/`GEO2.DAX`, M1's `run-script` + this session's demo).
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
    machine: EclMachine,
    vm_memory: VmMemoryState,
    party_predicates: DefaultPartyPredicates,
    /// The party roster (D-SAVE11/task deliverable 2) — empty until
    /// original-save import (D-SAVE5) or (a later session's) new-game
    /// character creation populates it; carried verbatim by `.rsav`
    /// save/restore (D-SAVE3).
    pub(crate) party: crate::party::Party,
    rng: EngineRng,
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
}

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
            geo,
            data,
            seed,
        ))
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
        Self::build(font, symbol_sets, dummy_sky(), geo, data, seed)
    }

    fn build(
        font: Font,
        symbol_sets: SymbolSets,
        sky: [ImageBlock; 3],
        geo: GeoBlock,
        data: GameData,
        seed: u32,
    ) -> Self {
        let mut fb = Framebuffer::new();
        crate::frames::draw8x8_03(&mut fb, &symbol_sets)
            .expect("Engine::build: symbol set 4 must be loaded for the exploration frame");

        let initial = load_ecl_block(&data, GAME_AREA, INITIAL_ECL_BLOCK)
            .expect("Engine::build: the initial resident ECL block must load");
        let mut machine =
            EclMachine::load_block(initial, &COTAB).unwrap_or_else(|never| match never {});
        let mut state = EngineState::new();
        state.ecl_block_id = INITIAL_ECL_BLOCK;
        let shell = Shell::boot(&mut machine, &mut state);

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
            vm_memory: VmMemoryState::new(),
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
            verify_report,
            rules,
            slots: crate::saveload::SlotDirectory::new(),
            io_request: None,
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
            verify_report: a.verify_report,
            // Reloaded from the embedded packs (not carried in the save/import).
            rules: RuleSet::load(),
            slots: crate::saveload::SlotDirectory::new(),
            io_request: None,
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

    /// Opens a shop screen over the given stock (M3 step 6 deliverable 5) —
    /// the entry point for stepping onto a town shop tile (the ECL
    /// `EnterShop`-flag + `TREASURE`-stock flow that populates it is M6).
    /// Returns to the walk-loop world menu on exit.
    pub fn enter_shop(&mut self, shop: crate::shop::Shop) {
        self.shell = Shell::Screen(crate::screens::Screen::Shop(crate::screens::Shop::new(
            shop,
        )));
    }

    /// Takes the save/load screen's pending action, if any — the host calls
    /// this after each tick and fulfills it (write/restore/import) via
    /// `saveload_fs`. Clears the request so it fires exactly once.
    pub fn take_io_request(&mut self) -> Option<crate::saveload::SaveLoadRequest> {
        self.io_request.take()
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

    /// The UI shell state machine's current node (`Boot`/`WorldMenu`/`Look`/
    /// `Step`/`GameOver`) — read by `tools/inspect`'s live engine pane
    /// (D-UI8) to show what flow stage the machine is in. `Shell` is a plain
    /// serde-able enum with no engine-internal borrows, so this is a
    /// read-only seam, not a control surface.
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

        {
            let mut ctx = FlowCtx {
                machine: &mut self.machine,
                vm_memory: &mut self.vm_memory,
                data: &self.data,
                game_area: GAME_AREA,
                input: &mut self.input,
                dt_ticks: 1,
                state: &mut self.state,
                geo: &self.geo,
                party: &mut self.party_predicates,
                roster: &mut self.party,
                rules: &self.rules,
                slots: &self.slots,
                io_request: &mut self.io_request,
                rng: &mut self.rng,
                fb: &mut self.fb,
                font: &self.font,
                cursor: &mut self.cursor,
                pacer: &mut self.pacer,
                sounds: &mut self.sounds,
                symbols: &mut self.symbol_sets,
                sky: &self.sky,
            };
            self.shell.tick(&mut ctx);
        }

        // The position/time status line (§1.9): row 15, cols 17-38,
        // refreshed after every command. Redrawn unconditionally each tick
        // (a documented simplification — the original's per-field redraw
        // discipline is step-5 rendering scope; unconditional redraw is
        // idempotent and keeps hash-goldens simple).
        let status = Shell::status_line(&self.state);
        draw_string(&mut self.fb, &self.font, &status, 15, 17, 0, 10);

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
}
