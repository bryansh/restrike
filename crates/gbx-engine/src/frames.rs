//! Screen frames (D-UI4 item 2/§1.2): `DrawFrame_Outer` + `draw8x8_03`, the
//! standard exploration screen layout. Transcribed as engine-constant data
//! — id/coordinate tables, not art (design doc §4 item 3; the art is the
//! user's 8×8 DAX symbols).
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `engine/seg037.cs` `DrawFrame_Outer` (`:31-54`) and `draw8x8_03`
//!   (`:73-102`), plus the border/frame tables at `:7-27`.
//!
//! `Display.UpdateStop()`/`UpdateStart()` batching (`seg037.cs:33,53`) is
//! omitted: it exists so partial composites never present mid-draw, which a
//! tick model gets for free (a frame presents only at tick end, D-UI1).

use crate::draw::cell_rect_fill;
use crate::framebuffer::Framebuffer;
use crate::symbols::{draw_symbol, SymbolError, SymbolSets};

/// Set-4 base for the outer border (`0x11E`, ids `0x11E..=0x127`).
const OUTER_BASE: u32 = 0x11E;
/// Set-4 base for the inner viewport frame (`0x114`, ids `0x114..=0x11D`).
const INNER_BASE: u32 = 0x114;

/// `seg037.outer_frame_top` (`byte_16E60`, `seg037.cs:13`) — 40 entries, col 0..=0x27.
const OUTER_FRAME_TOP: [u8; 40] = [
    0, 6, 1, 1, 1, 1, 1, 1, 6, 1, 1, 1, 1, 4, 1, 1, 1, 6, 1, 1, 1, 1, 1, 1, 1, 8, 1, 1, 1, 1, 1, 1,
    1, 4, 1, 1, 1, 6, 1, 2,
];

/// `seg037.outer_frame_bottom` (`unk_16EB0`, `seg037.cs:7`) — 40 entries, col 0..=0x27.
const OUTER_FRAME_BOTTOM: [u8; 40] = [
    1, 8, 6, 1, 1, 1, 1, 1, 1, 1, 1, 4, 1, 1, 1, 1, 1, 6, 8, 1, 1, 1, 4, 1, 1, 1, 1, 1, 1, 6, 1, 1,
    1, 1, 1, 1, 1, 1, 4, 3,
];

/// `seg037.outer_frame_left` (`unk_16EF2`, `seg037.cs:20`) — row 0..0x17 (23 rows).
const OUTER_FRAME_LEFT: [u8; 24] = [
    0, 2, 9, 5, 2, 2, 2, 2, 2, 2, 5, 7, 2, 2, 2, 2, 2, 9, 7, 2, 2, 2, 7, 1,
];

/// `seg037.outer_frame_right` (`unk_16F1B`, `seg037.cs:21`) — row 0..0x17 (23 rows).
const OUTER_FRAME_RIGHT: [u8; 24] = [
    2, 2, 9, 7, 2, 2, 2, 5, 2, 2, 2, 2, 2, 2, 2, 2, 2, 7, 2, 2, 2, 2, 5, 2,
];

/// `seg037.x8x8_07` (`seg037.cs:15-17`) — the row-16 horizontal divider, 40 entries.
const X8X8_07: [u8; 40] = [
    0, 8, 1, 1, 1, 1, 1, 1, 1, 1, 1, 6, 1, 1, 1, 8, 4, 1, 1, 1, 6, 1, 1, 1, 1, 1, 1, 1, 1, 1, 4, 1,
    6, 1, 1, 1, 1, 1, 8, 2,
];

/// `seg037.unk_16F0A` (`seg037.cs:11`) — the col-16 vertical divider, row 0..=0x10.
const COL16_DIVIDER: [u8; 17] = [0, 7, 5, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 5, 2, 9, 4];

/// `seg037.unk_16ED6` (`seg037.cs:9`) — inner viewport top frame, indexed by col 2..=14.
const INNER_TOP: [u8; 17] = [4, 3, 0, 6, 1, 1, 1, 1, 8, 1, 1, 4, 1, 1, 2, 1, 4];

/// `seg037.unk_16EE3` (`seg037.cs:19`) — inner viewport bottom frame, indexed by col 2..=14.
const INNER_BOTTOM: [u8; 15] = [1, 2, 1, 4, 1, 1, 1, 1, 1, 1, 8, 4, 1, 1, 3];

/// `seg037.unk_16F31` (`seg037.cs:23`) — inner viewport left frame, indexed by row 2..=14.
const INNER_LEFT: [u8; 15] = [5, 2, 0, 2, 7, 2, 2, 2, 2, 5, 2, 2, 2, 2, 1];

/// `seg037.unk_16F3E` (`seg037.cs:24`) — inner viewport right frame, indexed by row 2..=14.
const INNER_RIGHT: [u8; 15] = [2, 1, 2, 5, 9, 2, 2, 2, 7, 5, 2, 2, 2, 2, 3];

/// `DrawFrame_Outer` / `draw8x8_01` (`seg037.cs:31-54`): clears the inner
/// area, then draws the outer border (row 0, row 0x17, col 0, col 0x27)
/// from set 4.
pub fn draw_frame_outer(fb: &mut Framebuffer, sets: &SymbolSets) -> Result<(), SymbolError> {
    cell_rect_fill(fb, 0, 1, 0x16, 1, 0x26);

    for (col_x, &v) in OUTER_FRAME_TOP.iter().enumerate() {
        draw_symbol(fb, sets, v as u32 + OUTER_BASE, 0, col_x)?;
    }
    for (row_y, (&l, &r)) in OUTER_FRAME_LEFT
        .iter()
        .zip(OUTER_FRAME_RIGHT.iter())
        .take(0x17)
        .enumerate()
    {
        draw_symbol(fb, sets, l as u32 + OUTER_BASE, row_y, 0)?;
        draw_symbol(fb, sets, r as u32 + OUTER_BASE, row_y, 0x27)?;
    }
    for (col_x, &v) in OUTER_FRAME_BOTTOM.iter().enumerate() {
        draw_symbol(fb, sets, v as u32 + OUTER_BASE, 0x17, col_x)?;
    }
    Ok(())
}

/// `draw8x8_03` (`seg037.cs:73-102`): the standard exploration screen —
/// `DrawFrame_Outer` plus the row-16 horizontal divider, col-16 vertical
/// divider, and the inner 3D-viewport frame at cells 2-14.
pub fn draw8x8_03(fb: &mut Framebuffer, sets: &SymbolSets) -> Result<(), SymbolError> {
    draw_frame_outer(fb, sets)?;

    for (col_x, &v) in X8X8_07.iter().enumerate() {
        draw_symbol(fb, sets, v as u32 + OUTER_BASE, 0x10, col_x)?;
    }
    for (row_y, &v) in COL16_DIVIDER.iter().enumerate() {
        draw_symbol(fb, sets, v as u32 + OUTER_BASE, row_y, 0x10)?;
    }
    for col_x in 2..=14usize {
        draw_symbol(fb, sets, INNER_TOP[col_x] as u32 + INNER_BASE, 2, col_x)?;
        draw_symbol(fb, sets, INNER_BOTTOM[col_x] as u32 + INNER_BASE, 14, col_x)?;
    }
    for row_y in 2..=14usize {
        draw_symbol(fb, sets, INNER_LEFT[row_y] as u32 + INNER_BASE, row_y, 2)?;
        draw_symbol(fb, sets, INNER_RIGHT[row_y] as u32 + INNER_BASE, row_y, 14)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::SymbolError;
    use gbx_formats::image::DecodedItem;
    use gbx_formats::image::ImageBlock;

    /// Set 4 needs items covering every id these tables reference:
    /// `0x100..=0x127` (indices `0..=0x27`, i.e. 40 items).
    fn full_set4() -> ImageBlock {
        ImageBlock {
            height: 1,
            width_cols: 1,
            x_pos: 0,
            y_pos: 0,
            field_9: [0; 8],
            items: (0..40)
                .map(|i| DecodedItem {
                    pixels: vec![(i % 16) as u8; 8],
                })
                .collect(),
        }
    }

    #[test]
    fn draw8x8_03_succeeds_with_a_fully_populated_set4() {
        let mut fb = Framebuffer::new();
        let mut sets = SymbolSets::new();
        sets.load(4, full_set4());
        draw8x8_03(&mut fb, &sets).unwrap();
        // col 25's top-border symbol is OUTER_FRAME_TOP[25]=8 -> id 0x126 ->
        // set-4 index 0x26=38 -> fixture pixel value 38 % 16 = 6.
        assert_eq!(fb.get_pixel(25 * 8, 0), 6);
    }

    #[test]
    fn draw_frame_outer_errors_loudly_when_set4_is_missing() {
        let mut fb = Framebuffer::new();
        let sets = SymbolSets::new();
        let err = draw_frame_outer(&mut fb, &sets).unwrap_err();
        assert_eq!(err, SymbolError::SymbolSetNotLoaded { set: 4 });
    }

    #[test]
    fn draw_frame_outer_clears_the_inner_area_before_bordering() {
        let mut fb = Framebuffer::new();
        // Pre-seed a nonzero pixel inside the area draw8x8_clear_area(0x16,0x26,1,1) covers.
        fb.set_pixel(100, 100, 9);
        let mut sets = SymbolSets::new();
        sets.load(4, full_set4());
        draw_frame_outer(&mut fb, &sets).unwrap();
        assert_eq!(
            fb.get_pixel(100, 100),
            0,
            "the pre-border clear must have run"
        );
    }
}
