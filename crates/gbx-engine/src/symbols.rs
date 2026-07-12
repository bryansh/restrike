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

/// The five resident 8×8 symbol sets, each an optional decoded image block
/// (`gbl.symbol_8x8_set`).
#[derive(Debug, Clone, Default)]
pub struct SymbolSets {
    sets: [Option<ImageBlock>; SYMBOL_SET_COUNT],
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
        None,
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
