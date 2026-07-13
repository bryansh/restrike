//! 8×8 symbol sets and id routing (D-UI4 item 1/§1.3;
//! `docs/design/renderer-ui-shell.md`).
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `engine/ovr038.cs` `Put8x8Symbol`/`Load8x8D` (`:8-72`) — the
//!   five-set id-range routing table, the per-set base offset subtraction,
//!   and the hard error on id 0 / `0x128..0x7FFF` (the "symbol 0 = draw
//!   nothing" skip belongs to the future wall drawer's call site, not this
//!   primitive — `draw_3D_8x8_titles`'s `symbolId > 0` guard, out of scope
//!   for M2 step 2).
//! - `Classes/Gbl.cs:425` (`symbol_set_fix`).

use crate::draw::{blit_image, Clip};
use crate::framebuffer::Framebuffer;
use gbx_formats::image::ImageBlock;
use gbx_formats::walldef::{STYLES_PER_WALLSET, TILE_IDS_PER_STYLE, WALLSET_SIZE};

/// Resident symbol sets: 5 slots. Sets 1-3 (wallsets) are step-5 scope —
/// the slots exist here, nothing loads them yet.
pub const SYMBOL_SET_COUNT: usize = 5;

/// Per-set base offset subtracted from a routed symbol id to get the index
/// within that set's decoded items (`Gbl.cs:425`).
pub const SYMBOL_SET_FIX: [u32; SYMBOL_SET_COUNT] = [0x01, 0x2E, 0x74, 0xBA, 0x100];

/// Inclusive `(low, high)` id ranges routed to each set (`ovr038.cs:29-48`).
pub const SYMBOL_SET_RANGES: [(u32, u32); SYMBOL_SET_COUNT] = [
    (0x01, 0x2D),
    (0x2E, 0x73),
    (0x74, 0xB9),
    (0xBA, 0xFF),
    (0x100, 0x127),
];

/// [`resolve_symbol`]/[`draw_symbol`]'s failure mode. `Put8x8Symbol` treats
/// id 0 and `0x128..0x7FFF` as a hard error (`ovr038.cs:49-51`); this keeps
/// that loud error rather than silently skipping, so id-arithmetic bugs
/// can't hide (design doc §1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolError {
    /// `symbol_id` falls outside every set's id range (includes id 0).
    BadSymbolId { symbol_id: u32 },
    /// The id resolved to a real set, but that set hasn't been loaded yet.
    SymbolSetNotLoaded { set: usize },
}

/// Routes `symbol_id` to `(set_index, index_within_set)` per the five-range
/// table, subtracting that set's [`SYMBOL_SET_FIX`] base.
pub fn resolve_symbol(symbol_id: u32) -> Result<(usize, usize), SymbolError> {
    for (set, &(lo, hi)) in SYMBOL_SET_RANGES.iter().enumerate() {
        if symbol_id >= lo && symbol_id <= hi {
            return Ok((set, (symbol_id - SYMBOL_SET_FIX[set]) as usize));
        }
    }
    Err(SymbolError::BadSymbolId { symbol_id })
}

/// LOAD PIECES' wallset slot count (sets 1-3, `ovr003.cs`'s `CMD_LoadFiles`
/// `load_pieces` branch — step 5, task deliverable 1).
pub const WALLSET_SLOT_COUNT: usize = 3;

/// One resident wallset slot's tile-id table (`gbl.wallDef.blocks[slot]`,
/// `Classes/GeoBlock.cs`'s `maxBlocks = 3` global array — LOAD PIECES set
/// `slot + 1`'s single 780-byte sub-block): a research pass this session
/// (`ovr031.cs:642-687`, `Classes/GeoBlock.cs:78-90`'s `Offset`) found a
/// `LoadWalldef` call can populate *multiple consecutive* slots from one
/// multi-sub-block walldef load (`idx = symbolSet + block` for each of the
/// loaded blocks, not just `symbolSet` itself) — so this holds exactly one
/// already-rebased sub-block, not the whole loaded [`WalldefBlock`]. The
/// `>=0x2D` rebase (`var_A = symbol_set_fix[symbolSet] - symbol_set_fix[1]`,
/// computed once per `LoadWalldef` call from the *original* `symbolSet`
/// parameter, applied to every touched slot) is baked into `tiles` at load
/// time (`GeoBlock.cs:84`: `if (data[y,x] >= 0x2D) data[y,x] += (byte)off`,
/// wrapping byte arithmetic) — no further adjustment needed at texture
/// lookup time.
#[derive(Debug, Clone)]
pub struct WallsetSlot {
    tiles: [u8; WALLSET_SIZE],
}

impl WallsetSlot {
    /// Builds a slot from `tiles` (`[style][idx]`, flat as
    /// `style * TILE_IDS_PER_STYLE + idx`) — already-rebased tile ids.
    pub fn from_tiles(tiles: [u8; WALLSET_SIZE]) -> Self {
        WallsetSlot { tiles }
    }

    /// The rebased tile id at `(style, idx)` — `style < 5`, `idx < 156`.
    pub fn tile_id(&self, style: usize, idx: usize) -> Option<u8> {
        if style >= STYLES_PER_WALLSET || idx >= TILE_IDS_PER_STYLE {
            return None;
        }
        self.tiles.get(style * TILE_IDS_PER_STYLE + idx).copied()
    }
}

/// The five resident 8×8 symbol sets, each an optional decoded image block
/// (`gbl.symbol_8x8_set`), plus the three LOAD PIECES wallset slots
/// (`gbl.wallDef.blocks[0..2]`) backing sets 1-3's wall-texture id tables.
#[derive(Debug, Clone, Default)]
pub struct SymbolSets {
    sets: [Option<ImageBlock>; SYMBOL_SET_COUNT],
    wallsets: [Option<WallsetSlot>; WALLSET_SLOT_COUNT],
}

impl SymbolSets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads `block` into slot `set` (`0..5`). Panics on an out-of-range
    /// slot — a caller bug, the slot count is fixed.
    pub fn load(&mut self, set: usize, block: ImageBlock) {
        self.sets[set] = Some(block);
    }

    pub fn get(&self, set: usize) -> Option<&ImageBlock> {
        self.sets.get(set).and_then(Option::as_ref)
    }

    /// Loads a wallset slot (`slot` = LOAD PIECES `set - 1`, `0..3`).
    /// Panics on an out-of-range slot, same contract as
    /// [`SymbolSets::load`].
    pub fn load_wallset(&mut self, slot: usize, tiles: WallsetSlot) {
        self.wallsets[slot] = Some(tiles);
    }

    pub fn wallset(&self, slot: usize) -> Option<&WallsetSlot> {
        self.wallsets.get(slot).and_then(Option::as_ref)
    }

    /// `gbl.setBlocks[index].Reset()` (LOAD PIECES' `0xFF`-argument
    /// branch): clears both the wallset's tile-id table and the pixel data
    /// loaded into the paired symbol set (`slot + 1`).
    pub fn reset_wallset(&mut self, slot: usize) {
        self.wallsets[slot] = None;
        self.sets[slot + 1] = None;
    }
}

/// `Put8x8Symbol`'s non-overlay path (`ovr038.cs:54-71` + `seg040.draw_picture`,
/// `seg040.cs:120-123`): routes `symbol_id`, then blits that set's item at
/// `(cell_row, cell_col)` full-canvas-clipped, no no-draw/recolor.
pub fn draw_symbol(
    fb: &mut Framebuffer,
    sets: &SymbolSets,
    symbol_id: u32,
    cell_row: usize,
    cell_col: usize,
) -> Result<(), SymbolError> {
    draw_symbol_inner(fb, sets, symbol_id, cell_row, cell_col, None)
}

/// Like [`draw_symbol`], but skips any pixel equal to `no_draw` — the
/// area-map party arrow's transient "no-draw color 8"
/// (`seg040.draw_clipped_nodraw(8)` around the one `Put8x8Symbol` call,
/// `ovr031.cs:86-88`, restored to 17 immediately after).
pub fn draw_symbol_no_draw(
    fb: &mut Framebuffer,
    sets: &SymbolSets,
    symbol_id: u32,
    cell_row: usize,
    cell_col: usize,
    no_draw: u8,
) -> Result<(), SymbolError> {
    draw_symbol_inner(fb, sets, symbol_id, cell_row, cell_col, Some(no_draw))
}

fn draw_symbol_inner(
    fb: &mut Framebuffer,
    sets: &SymbolSets,
    symbol_id: u32,
    cell_row: usize,
    cell_col: usize,
    no_draw: Option<u8>,
) -> Result<(), SymbolError> {
    let (set, index) = resolve_symbol(symbol_id)?;
    let block = sets
        .get(set)
        .ok_or(SymbolError::SymbolSetNotLoaded { set })?;
    let item = block
        .items
        .get(index)
        .ok_or(SymbolError::BadSymbolId { symbol_id })?;
    blit_image(
        fb,
        &item.pixels,
        block.width_px(),
        block.height as usize,
        cell_row,
        cell_col,
        Clip::FULL,
        no_draw,
        None,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::image::DecodedItem;

    fn tiny_block(item_count: usize) -> ImageBlock {
        ImageBlock {
            height: 1,
            width_cols: 1,
            x_pos: 0,
            y_pos: 0,
            field_9: [0; 8],
            items: (0..item_count)
                .map(|i| DecodedItem {
                    pixels: vec![(i % 16) as u8; 8],
                })
                .collect(),
        }
    }

    #[test]
    fn resolve_symbol_routes_each_set_boundary_id() {
        assert_eq!(resolve_symbol(0x01).unwrap(), (0, 0));
        assert_eq!(resolve_symbol(0x2D).unwrap(), (0, 0x2C));
        assert_eq!(resolve_symbol(0x2E).unwrap(), (1, 0));
        assert_eq!(resolve_symbol(0x73).unwrap(), (1, 0x73 - 0x2E));
        assert_eq!(resolve_symbol(0x74).unwrap(), (2, 0));
        assert_eq!(resolve_symbol(0xB9).unwrap(), (2, 0xB9 - 0x74));
        assert_eq!(resolve_symbol(0xBA).unwrap(), (3, 0));
        assert_eq!(resolve_symbol(0xFF).unwrap(), (3, 0xFF - 0xBA));
        assert_eq!(resolve_symbol(0x100).unwrap(), (4, 0));
        assert_eq!(resolve_symbol(0x127).unwrap(), (4, 0x127 - 0x100));
    }

    #[test]
    fn resolve_symbol_id_zero_is_a_hard_error() {
        assert_eq!(
            resolve_symbol(0),
            Err(SymbolError::BadSymbolId { symbol_id: 0 })
        );
    }

    #[test]
    fn resolve_symbol_out_of_range_is_a_hard_error() {
        assert_eq!(
            resolve_symbol(0x128),
            Err(SymbolError::BadSymbolId { symbol_id: 0x128 })
        );
        assert_eq!(
            resolve_symbol(0x7FFF),
            Err(SymbolError::BadSymbolId { symbol_id: 0x7FFF })
        );
    }

    #[test]
    fn resolve_symbol_between_ranges_is_a_hard_error() {
        // Nothing covers e.g. nothing between the tight ranges here since
        // they're contiguous 1..=0x127, but 0x7FFF+1 and beyond still error.
        assert!(resolve_symbol(0x8000).is_err());
    }

    #[test]
    fn draw_symbol_errors_loudly_when_the_set_is_not_loaded() {
        let fb_err = {
            let mut fb = Framebuffer::new();
            let sets = SymbolSets::new();
            draw_symbol(&mut fb, &sets, 0x100, 0, 0)
        };
        assert_eq!(fb_err, Err(SymbolError::SymbolSetNotLoaded { set: 4 }));
    }

    #[test]
    fn draw_symbol_draws_the_routed_items_pixels_at_the_cell() {
        let mut fb = Framebuffer::new();
        let mut sets = SymbolSets::new();
        sets.load(4, tiny_block(3));
        draw_symbol(&mut fb, &sets, 0x102, 1, 2).unwrap(); // index 2 -> pixel value 2
        assert_eq!(fb.get_pixel(16, 8), 2);
    }

    #[test]
    fn draw_symbol_out_of_range_within_a_loaded_set_is_a_hard_error() {
        let mut fb = Framebuffer::new();
        let mut sets = SymbolSets::new();
        sets.load(4, tiny_block(1)); // only index 0 exists
        let err = draw_symbol(&mut fb, &sets, 0x101, 0, 0).unwrap_err();
        assert_eq!(err, SymbolError::BadSymbolId { symbol_id: 0x101 });
    }
}
