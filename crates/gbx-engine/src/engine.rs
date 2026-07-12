//! `Engine::new` + `tick` (D-UI1, task deliverable 1): the tick core that
//! ties the framebuffer, text system, input queue, [`Shell`] state machine,
//! and [`StubVm`] together into one `tick(input) -> Frame` loop.
//!
//! The M2 session's hardcoded resident block ([`DEFAULT_GEO_FILE`]/
//! [`DEFAULT_GEO_BLOCK`]) is Tilverton City (`GEO2.DAX` block 1, per
//! `SOURCES.md`'s GEO row and `restrike map`'s prior verification against
//! *Cluebook.pdf*) — real block/ECL selection is step 4/5 scope; this
//! session always walks the one map.

use crate::boot::{self, BootError};
use crate::framebuffer::Framebuffer;
use crate::input::InputEvent;
use crate::movement::DefaultPartyPredicates;
use crate::rng::EngineRng;
use crate::shell::{EngineState, FlowCtx, Shell, SoundEvent};
use crate::symbols::SymbolSets;
use crate::text::{draw_string, TextCursor, TextPacer};
use crate::vm_stub::StubVm;
use gbx_formats::font::Font;
use gbx_formats::game_data::GameData;
use gbx_formats::geo::GeoBlock;
use gbx_vm::{Exit, VmStep};

/// `GEO2.DAX` block 1 — Tilverton City (this session's fixed resident map).
pub const DEFAULT_GEO_FILE: &str = "GEO2.DAX";
pub const DEFAULT_GEO_BLOCK: u8 = 1;

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
/// derived assets, input queue, and UI shell state.
pub struct Engine {
    fb: Framebuffer,
    font: Font,
    geo: GeoBlock,
    pub(crate) shell: Shell,
    pub(crate) state: EngineState,
    vm: StubVm,
    party: DefaultPartyPredicates,
    rng: EngineRng,
    input: crate::input::InputQueue,
    cursor: TextCursor,
    pacer: TextPacer,
    sounds: Vec<SoundEvent>,
    serial: u64,
    last_hash: Option<[u8; 32]>,
}

impl Engine {
    /// Boots from real `GameData`: the D-UI1/`boot.rs` asset slice plus this
    /// session's hardcoded resident GEO block.
    pub fn new(data: GameData, seed: u64) -> Result<Self, BootError> {
        let assets = boot::boot(&data)?;
        let geo_bytes = data.block(DEFAULT_GEO_FILE, DEFAULT_GEO_BLOCK)?;
        let geo = GeoBlock::parse(&geo_bytes)?;
        Ok(Self::build(assets.font, assets.symbol_sets, geo, seed))
    }

    /// The synthetic-fixture seam (task deliverable: hash goldens driven
    /// from a hand-authored `GeoBlock` + step-2 fixture assets, D10) —
    /// skips `GameData`/`boot()` entirely, mirroring `frames.rs`/
    /// `hash_goldens.rs`'s existing `synthetic_set4`/`synthetic_font`
    /// pattern rather than hand-encoding synthetic DAX bytes. Panics if
    /// `symbol_sets` lacks set 4 (a fixture-authoring bug, not a runtime
    /// condition `Engine::new`'s real-data path needs to handle gracefully).
    pub fn new_fixture(font: Font, symbol_sets: SymbolSets, geo: GeoBlock, seed: u64) -> Self {
        Self::build(font, symbol_sets, geo, seed)
    }

    fn build(font: Font, symbol_sets: SymbolSets, geo: GeoBlock, seed: u64) -> Self {
        let mut fb = Framebuffer::new();
        crate::frames::draw8x8_03(&mut fb, &symbol_sets)
            .expect("Engine::build: symbol set 4 must be loaded for the exploration frame");

        let mut vm = StubVm::new();
        // A sensible no-op default for the entry vector, so construction
        // never panics on an unscripted `StubVm`; callers wanting specific
        // vector behavior call `script_vm_call` before ticking further.
        vm.script_call(vec![VmStep::Done(Exit::Ended)]);
        let mut state = EngineState::new();
        let shell = Shell::boot(&mut vm, &mut state);

        Engine {
            fb,
            font,
            geo,
            shell,
            state,
            vm,
            party: DefaultPartyPredicates::default(),
            rng: EngineRng::new(seed),
            input: crate::input::InputQueue::new(),
            cursor: TextCursor::new(),
            pacer: TextPacer::new(4),
            sounds: Vec::new(),
            serial: 0,
            last_hash: None,
        }
    }

    /// Test/demo seam (step 3's stub-VM stand-in, task deliverable 3):
    /// scripts the next vector-run's full outcome. Step 4 removes this in
    /// favor of a real `EclMachine` + `VmHost`.
    pub fn script_vm_call(&mut self, steps: Vec<VmStep>) {
        self.vm.script_call(steps);
    }

    /// Test/demo seam (the M3 party-predicate seam, task deliverable 4):
    /// mutable access to the party-predicate stand-in (bash/pick/knock
    /// availability and rolls) so a scripted trace can exercise every door
    /// path ahead of M3's real party model.
    pub fn party_predicates_mut(&mut self) -> &mut DefaultPartyPredicates {
        &mut self.party
    }

    pub fn state(&self) -> &EngineState {
        &self.state
    }

    /// Advances by one tick (D-UI1): dispatches `input`, advances the UI
    /// shell within its step budget, and recomposes the status line before
    /// returning a borrowed view of the framebuffer.
    pub fn tick(&mut self, input: &[InputEvent]) -> Frame<'_> {
        self.input.push_all(input);
        self.sounds.clear();

        {
            let mut ctx = FlowCtx {
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

    fn engine() -> Engine {
        let mut sets = SymbolSets::new();
        sets.load(4, synthetic_set4());
        Engine::new_fixture(synthetic_font(), sets, open_geo(), 1)
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
