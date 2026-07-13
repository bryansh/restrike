//! Palette expansion: indexed pixels (`0..=15` EGA codes, `16` = the
//! decoder's transparency sentinel — `gbx_formats::image`'s doc comment) to
//! RGBA bytes for an egui texture. Pure — no egui types, so the expansion
//! logic is unit-testable independent of a display.

use gbx_rules::palette::EGA_PALETTE;

/// The decoders' transparency sentinel (`gbx_formats::image::decode`'s
/// masked-pixel value) — rendered here as fully transparent black rather
/// than a 17th palette color.
pub const TRANSPARENT: u8 = 16;

/// Expands one palette-index pixel to RGBA. `TRANSPARENT` (16) maps to
/// alpha 0; any other value `>15` (malformed input) also renders as
/// transparent rather than panicking or wrapping — a decode bug should be
/// visually obvious (a hole), not a crash or a silently wrong color.
pub fn pixel_to_rgba(index: u8) -> [u8; 4] {
    if index == TRANSPARENT || index as usize >= EGA_PALETTE.len() {
        return [0, 0, 0, 0];
    }
    let [r, g, b] = EGA_PALETTE[index as usize];
    [r, g, b, 0xFF]
}

/// Expands a full row-major indexed pixel buffer to RGBA bytes, in the same
/// row-major order — the shape `egui::ColorImage::from_rgba_unmultiplied`
/// wants.
pub fn expand_rgba(pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for &p in pixels {
        out.extend_from_slice(&pixel_to_rgba(p));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_indices_use_the_ega_table_with_full_alpha() {
        assert_eq!(pixel_to_rgba(10), [82, 255, 82, 0xFF]);
        assert_eq!(pixel_to_rgba(0), [0, 0, 0, 0xFF]);
        assert_eq!(pixel_to_rgba(15), [255, 255, 255, 0xFF]);
    }

    #[test]
    fn transparent_sentinel_is_alpha_zero() {
        assert_eq!(pixel_to_rgba(TRANSPARENT), [0, 0, 0, 0]);
    }

    #[test]
    fn out_of_range_index_is_also_transparent_not_a_panic() {
        assert_eq!(pixel_to_rgba(200), [0, 0, 0, 0]);
    }

    #[test]
    fn expand_rgba_preserves_row_major_order_and_length() {
        let pixels = [0u8, 10, 16, 15];
        let rgba = expand_rgba(&pixels);
        assert_eq!(rgba.len(), 16);
        assert_eq!(&rgba[4..8], &[82, 255, 82, 0xFF]);
        assert_eq!(&rgba[8..12], &[0, 0, 0, 0]);
    }
}
