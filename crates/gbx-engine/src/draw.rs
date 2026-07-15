//! Draw primitives: pure functions over `&mut Framebuffer` + assets
//! (D-UI4 item 1; `docs/design/renderer-ui-shell.md` §1.1/§1.8).
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `Classes/Display.cs` `DisplayMono8x8` (`:65-80`) — the mono glyph
//!   blit.
//! - coab `engine/seg041.cs` `DrawRectangle` (`:7-21`) — the cell-rect fill
//!   (cell coordinates × 8, inclusive end cell).
//! - coab `engine/seg040.cs` `draw_clipped_picture`/`draw_clipped_nodraw`/
//!   `draw_clipped_recolor` (`:58-113`) — the 4bpp image blit: a pixel clip
//!   window plus *mutable* no-draw/recolor state in the original, taken here
//!   as per-call parameters instead (§1.1 — blit params, not global state).
//! - coab `engine/seg040.cs` `DrawColorBlock` (`:143-161`) — the raw pixel
//!   fill anchored at a cell but sized/offset in pixels (the `+8` origin
//!   offset is transcribed as-is; its meaning is unexplained in the
//!   original, docketed alongside the other under-explained constants).
//! - coab `engine/ovr030.cs` (`:8-11,17-24,129-132`) and `Classes/DaxFiles/
//!   DaxBlock.cs` `Recolor` (`:71-94`) — the fade/transparent recolor
//!   tables and the fade path's 1-in-4 random dither.

use crate::framebuffer::{Framebuffer, HEIGHT, WIDTH};

/// A pixel clip window: `[x0, x1) x [y0, y1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clip {
    pub x0: usize,
    pub x1: usize,
    pub y0: usize,
    pub y1: usize,
}

impl Clip {
    /// No clipping beyond the canvas itself (`draw_picture`'s `0,320,0,200`).
    pub const FULL: Clip = Clip {
        x0: 0,
        x1: WIDTH,
        y0: 0,
        y1: HEIGHT,
    };
    /// The overlay draw path's clip (`draw_combat_picture`, `seg040.cs:115-118`).
    pub const OVERLAY: Clip = Clip {
        x0: 8,
        x1: 176,
        y0: 8,
        y1: 176,
    };
}

/// Fills cells `[y_start, y_end] x [x_start, x_end]` (inclusive, the 40×25
/// cell grid) with `color`. Transcribed from `seg041.DrawRectangle`
/// (`seg041.cs:7-21`): cell coordinates are multiplied by 8, and the end
/// cell is inclusive (`(end + 1) * 8`).
pub fn cell_rect_fill(
    fb: &mut Framebuffer,
    color: u8,
    y_start: usize,
    y_end: usize,
    x_start: usize,
    x_end: usize,
) {
    let px0 = x_start * 8;
    let px1 = (x_end + 1) * 8;
    let py0 = y_start * 8;
    let py1 = (y_end + 1) * 8;
    for y in py0..py1 {
        for x in px0..px1 {
            fb.set_pixel(x, y, color);
        }
    }
}

/// Mono 8×8 glyph blit (`Display.DisplayMono8x8`, `Display.cs:65-80`):
/// `glyph`'s bytes are row-major, MSB (bit 7) is the leftmost pixel.
pub fn draw_glyph(
    fb: &mut Framebuffer,
    glyph: &[u8; 8],
    cell_row: usize,
    cell_col: usize,
    bg: u8,
    fg: u8,
) {
    const BIT_MASK: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];
    let px0 = cell_col * 8;
    let py0 = cell_row * 8;
    for (y_step, row_byte) in glyph.iter().enumerate() {
        for (i, mask) in BIT_MASK.iter().enumerate() {
            let color = if row_byte & mask != 0 { fg } else { bg };
            fb.set_pixel(px0 + i, py0 + y_step, color);
        }
    }
}

/// The 4bpp image blit (`seg040.draw_clipped_picture`, `seg040.cs:73-113`),
/// anchored at cell `(cell_row, cell_col)`, clipped to `clip`.
///
/// `no_draw`, when `Some`, skips every pixel equal to that color — this
/// catches no-draw colors that are otherwise valid palette indices (e.g.
/// the area-map arrow's no-draw-8, §1.7), which the universal
/// transparency-16 skip below cannot. `recolor`, when `Some((from, to))`,
/// remaps `from` to `to` *before* the universal skip — so, faithfully, a
/// recolor whose `from` is `TRANSPARENT` (16) does draw `to`, overriding
/// the default transparency behavior for that one pixel (this mirrors the
/// original's exact check order: no-draw, then recolor, then the plain
/// write, `seg040.cs:93-104`). Every write ultimately passes through
/// [`Framebuffer::set_pixel`], whose own `< 16` guard is the universal
/// transparency-16 skip for any pixel not caught by `no_draw`/`recolor`.
#[allow(clippy::too_many_arguments)]
pub fn blit_image(
    fb: &mut Framebuffer,
    pixels: &[u8],
    width: usize,
    height: usize,
    cell_row: usize,
    cell_col: usize,
    clip: Clip,
    no_draw: Option<u8>,
    recolor: Option<(u8, u8)>,
) {
    let min_x = cell_col * 8;
    let min_y = cell_row * 8;
    for row in 0..height {
        for col in 0..width {
            let px = min_x + col;
            let py = min_y + row;
            if px < clip.x0 || px >= clip.x1 || py < clip.y0 || py >= clip.y1 {
                continue;
            }
            let color = pixels[row * width + col];
            if no_draw == Some(color) {
                continue;
            }
            if let Some((from, to)) = recolor {
                if color == from {
                    fb.set_pixel(px, py, to);
                    continue;
                }
            }
            fb.set_pixel(px, py, color);
        }
    }
}

/// `seg040.DrawColorBlock` (`:143-161`): a raw pixel fill anchored at cell
/// column `cell_col`, `col_width` 8-px columns wide and `line_count` pixel
/// rows tall starting `line_y + 8` pixels down. Clips to the canvas
/// (the original's explicit `pixX/pixY` bounds check); negative pixel
/// coordinates (possible with the original's signed `int` params) clip out
/// entirely rather than wrapping, matching `usize` semantics here via a
/// signed intermediate.
pub fn draw_color_block(
    fb: &mut Framebuffer,
    color: u8,
    line_count: i32,
    col_width: i32,
    line_y: i32,
    cell_col: i32,
) {
    let min_y = line_y + 8;
    let max_y = min_y + line_count;
    let min_x = (cell_col * 8) + 8;
    let max_x = min_x + col_width * 8;
    for py in min_y..max_y {
        for px in min_x..max_x {
            if (0..WIDTH as i32).contains(&px) && (0..HEIGHT as i32).contains(&py) {
                fb.set_pixel(px as usize, py as usize, color);
            }
        }
    }
}

/// The fade recolor table (`ovr030.fadeNewColors`, `ovr030.cs:9`): pixel
/// value `v` fades toward `FADE_RECOLOR[v]` (identity where unchanged).
pub const FADE_RECOLOR: [u8; 16] = [12, 12, 12, 12, 4, 5, 6, 7, 12, 12, 10, 12, 12, 12, 14, 12];

/// The transparent-sprite recolor table (`ovr030.transparentNewColors`,
/// `ovr030.cs:11`): masked `SPRIT` loads recolor 13 → 0 (black), identity
/// otherwise.
pub const TRANSPARENT_RECOLOR: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0, 14, 15];

/// A fixed (non-random) 16-entry recolor pass over decoded pixel data —
/// e.g. [`TRANSPARENT_RECOLOR`]'s masked-sprite 13→0 recolor
/// (`DaxBlock.Recolor(useRandom=false, ...)`). Values `>= 16` (already
/// transparent) are left untouched.
pub fn apply_recolor(pixels: &mut [u8], table: &[u8; 16]) {
    for p in pixels.iter_mut() {
        let v = *p as usize;
        if v < 16 {
            let new = table[v];
            if new != *p {
                *p = new;
            }
        }
    }
}

/// A deterministic 1-in-4 dither key over a pixel's position (its index in the
/// flat framebuffer slice). No PRNG: this is a pure position hash.
fn dither_hit(index: usize) -> bool {
    // Knuth multiplicative hash of the index; take two spread-out bits so
    // adjacent pixels don't fall on an obvious 4-stride comb. ~1-in-4 by
    // construction. Exact pattern is unspecified (dither pixels are declared
    // non-comparable by the renderer doc) — only determinism matters.
    ((index as u32).wrapping_mul(2_654_435_761) >> 13) & 3 == 0
}

/// The fade recolor pass, with `DaxBlock.Recolor`'s 1-in-4 dither per matching
/// pixel (`useRandom == true`, `DaxBlock.cs:84`).
///
/// D-OR1(c)/FD-28: this draw has **no original counterpart in the game PRNG
/// stream** (coab uses a *separate* time-seeded `random_number` for the dither,
/// `DaxBlock.cs:84` — and whether the binary's dither touches `DS:0x47F0` at
/// all is FD-28, still open). A framebuffer-content-dependent draw count would
/// desync any traced window, so the dither draws from `gbx-prng` *not at all*:
/// it is a deterministic position hash ([`dither_hit`]). No `VmRng` parameter.
pub fn apply_recolor_dithered(pixels: &mut [u8], table: &[u8; 16]) {
    for (i, p) in pixels.iter_mut().enumerate() {
        let v = *p as usize;
        if v < 16 {
            let new = table[v];
            if new != *p && dither_hit(i) {
                *p = new;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_rect_fill_covers_exact_pixel_bounds_inclusive_end_cell() {
        let mut fb = Framebuffer::new();
        cell_rect_fill(&mut fb, 5, 1, 2, 1, 2);
        // Cells (1,1)-(2,2) inclusive -> pixels [8,24) x [8,24).
        assert_eq!(fb.get_pixel(8, 8), 5);
        assert_eq!(fb.get_pixel(23, 23), 5);
        assert_eq!(
            fb.get_pixel(7, 7),
            0,
            "must not spill into the cell above/left"
        );
        assert_eq!(
            fb.get_pixel(24, 24),
            0,
            "must not spill past the inclusive end cell"
        );
    }

    fn solid_glyph(bit: u8) -> [u8; 8] {
        [bit; 8]
    }

    #[test]
    fn draw_glyph_msb_is_leftmost_pixel() {
        let mut fb = Framebuffer::new();
        draw_glyph(&mut fb, &solid_glyph(0x80), 0, 0, 1, 2);
        assert_eq!(fb.get_pixel(0, 0), 2, "MSB set -> fg at the leftmost pixel");
        assert_eq!(
            fb.get_pixel(7, 0),
            1,
            "LSB clear -> bg at the rightmost pixel"
        );
    }

    #[test]
    fn draw_glyph_places_at_the_correct_cell() {
        let mut fb = Framebuffer::new();
        draw_glyph(&mut fb, &solid_glyph(0xFF), 2, 3, 0, 9);
        assert_eq!(fb.get_pixel(24, 16), 9);
        assert_eq!(fb.get_pixel(0, 0), 0, "must not draw outside its cell");
    }

    fn checkerboard(w: usize, h: usize) -> Vec<u8> {
        (0..w * h).map(|i| (i % 16) as u8).collect()
    }

    #[test]
    fn blit_image_skips_transparency_16_via_the_framebuffer_guard() {
        let mut fb = Framebuffer::new();
        let pixels = vec![16u8, 5, 16, 3];
        blit_image(&mut fb, &pixels, 2, 2, 0, 0, Clip::FULL, None, None);
        assert_eq!(fb.get_pixel(0, 0), 0, "transparent pixel must not draw");
        assert_eq!(fb.get_pixel(1, 0), 5);
        assert_eq!(fb.get_pixel(0, 1), 0, "transparent pixel must not draw");
        assert_eq!(fb.get_pixel(1, 1), 3);
    }

    #[test]
    fn blit_image_respects_the_clip_window() {
        let mut fb = Framebuffer::new();
        let pixels = vec![7u8; 4 * 4];
        blit_image(
            &mut fb,
            &pixels,
            4,
            4,
            0,
            0,
            Clip {
                x0: 1,
                x1: 3,
                y0: 1,
                y1: 3,
            },
            None,
            None,
        );
        assert_eq!(fb.get_pixel(0, 0), 0, "clipped out");
        assert_eq!(fb.get_pixel(1, 1), 7, "inside the clip window");
        assert_eq!(fb.get_pixel(2, 2), 7, "inside the clip window");
        assert_eq!(fb.get_pixel(3, 3), 0, "clip x1/y1 are exclusive");
    }

    #[test]
    fn blit_image_no_draw_skips_a_valid_palette_color() {
        let mut fb = Framebuffer::new();
        fb.set_pixel(0, 0, 4);
        let pixels = vec![8u8]; // 8 is a normal palette color, not transparency
        blit_image(&mut fb, &pixels, 1, 1, 0, 0, Clip::FULL, Some(8), None);
        assert_eq!(
            fb.get_pixel(0, 0),
            4,
            "no_draw color must be skipped, not drawn"
        );
    }

    #[test]
    fn blit_image_recolor_remaps_before_the_transparency_check() {
        let mut fb = Framebuffer::new();
        let pixels = vec![16u8]; // transparency-16
        blit_image(
            &mut fb,
            &pixels,
            1,
            1,
            0,
            0,
            Clip::FULL,
            None,
            Some((16, 9)),
        );
        assert_eq!(
            fb.get_pixel(0, 0),
            9,
            "recolor(from=16) overrides the default transparency skip, matching the original's check order"
        );
    }

    #[test]
    fn blit_image_recolor_only_affects_the_matched_color() {
        let mut fb = Framebuffer::new();
        let pixels = vec![3u8, 4u8];
        blit_image(
            &mut fb,
            &pixels,
            2,
            1,
            0,
            0,
            Clip::FULL,
            None,
            Some((3, 12)),
        );
        assert_eq!(fb.get_pixel(0, 0), 12);
        assert_eq!(
            fb.get_pixel(1, 0),
            4,
            "unmatched color passes through unchanged"
        );
    }

    #[test]
    fn draw_color_block_fills_the_offset_pixel_region() {
        let mut fb = Framebuffer::new();
        draw_color_block(&mut fb, 6, 4, 1, 0, 0);
        // min_y=8,max_y=12; min_x=8,max_x=16.
        assert_eq!(fb.get_pixel(8, 8), 6);
        assert_eq!(fb.get_pixel(15, 11), 6);
        assert_eq!(fb.get_pixel(7, 8), 0);
        assert_eq!(fb.get_pixel(16, 8), 0);
        assert_eq!(fb.get_pixel(8, 12), 0);
    }

    #[test]
    fn draw_color_block_clips_negative_coordinates_out() {
        let mut fb = Framebuffer::new();
        // line_y = -8 -> min_y = 0; still fine. cell_col very negative clips fully.
        draw_color_block(&mut fb, 6, 4, 1, -8, -100);
        assert_eq!(
            fb.get_pixel(0, 0),
            0,
            "must not panic or wrap into the canvas"
        );
    }

    #[test]
    fn apply_recolor_transparent_table_maps_13_to_0_identity_elsewhere() {
        let mut pixels = checkerboard(4, 4);
        apply_recolor(&mut pixels, &TRANSPARENT_RECOLOR);
        for (i, &p) in pixels.iter().enumerate() {
            let orig = (i % 16) as u8;
            if orig == 13 {
                assert_eq!(p, 0);
            } else {
                assert_eq!(p, orig);
            }
        }
    }

    #[test]
    fn apply_recolor_never_touches_already_transparent_pixels() {
        let mut pixels = vec![16u8, 20u8];
        apply_recolor(&mut pixels, &TRANSPARENT_RECOLOR);
        assert_eq!(pixels, vec![16, 20]);
    }

    #[test]
    fn apply_recolor_dithered_is_deterministic_and_touches_no_rng() {
        // No RNG parameter exists (D-OR1(c)): determinism is structural. The
        // same input recolors the same subset every time.
        let base: Vec<u8> = (0..256).map(|i| (i % 16) as u8).collect();
        let mut a = base.clone();
        let mut b = base.clone();
        apply_recolor_dithered(&mut a, &FADE_RECOLOR);
        apply_recolor_dithered(&mut b, &FADE_RECOLOR);
        assert_eq!(a, b, "dither must be deterministic across calls");
    }

    #[test]
    fn apply_recolor_dithered_recolors_a_proper_subset_of_eligible_pixels() {
        // A run of eligible pixels (0 -> 12): the dither recolors some but not
        // all of them, keeping the 1-in-4 character. Identity/out-of-range
        // pixels are never touched.
        let mut pixels = vec![0u8; 64];
        apply_recolor_dithered(&mut pixels, &FADE_RECOLOR);
        let recolored = pixels.iter().filter(|&&p| p == 12).count();
        assert!(recolored > 0, "some eligible pixels must recolor");
        assert!(
            recolored < 64,
            "not all eligible pixels may recolor (dither)"
        );
    }

    #[test]
    fn apply_recolor_dithered_never_touches_identity_or_out_of_range_pixels() {
        // 4 -> 4 is identity in FADE_RECOLOR; 200 is out of the 0..16 range.
        let mut pixels = vec![4u8, 200u8, 4u8, 200u8];
        apply_recolor_dithered(&mut pixels, &FADE_RECOLOR);
        assert_eq!(pixels, vec![4, 200, 4, 200]);
    }
}
