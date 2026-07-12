//! The 320×200 indexed framebuffer and 16-entry palette (D-UI4's
//! "Palette" section; `docs/design/renderer-ui-shell.md` §1.1).
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `Classes/Display.cs` `SetPixel3` (`:163-174`) — the `value < 16`
//!   write guard this module's [`Framebuffer::set_pixel`] replicates:
//!   transparency-16 and no-draw-17 (and anything above) never land in the
//!   buffer, silently.
//! - coab `Classes/Display.cs` `SetEgaPalette` (`:82-103`) — palette slots
//!   are remapped by repointing at a color from the *original* canon table,
//!   never from the currently-remapped table (no cascading remaps).

use sha2::{Digest, Sha256};

/// Pixel canvas width.
pub const WIDTH: usize = 320;
/// Pixel canvas height.
pub const HEIGHT: usize = 200;
/// Composited assets use 16 as the transparency code (`DaxBlock.SetMaskedColor`).
pub const TRANSPARENT: u8 = 16;
/// The clipped blit's default no-draw color (`seg040.cs`'s `color_no_draw`
/// default) — never a real palette index.
pub const NO_DRAW_DEFAULT: u8 = 17;

/// A 320×200 buffer of 4-bit palette indices, plus the 16-entry palette it
/// composites against. Pixel values `0..=15` are real palette indices; `16`
/// (transparency) and `17` (no-draw) and anything else are never stored —
/// [`Framebuffer::set_pixel`] enforces this the same way `SetPixel3`'s
/// `value < 16` guard does in the original.
#[derive(Debug, Clone)]
pub struct Framebuffer {
    pixels: Box<[u8; WIDTH * HEIGHT]>,
    palette: [[u8; 3]; 16],
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framebuffer {
    /// A black-filled canvas with the palette initialized to the EGA canon
    /// (`gbx_rules::palette::EGA_PALETTE`).
    pub fn new() -> Self {
        Framebuffer {
            pixels: Box::new([0u8; WIDTH * HEIGHT]),
            palette: gbx_rules::palette::EGA_PALETTE,
        }
    }

    /// Writes one pixel, replicating `SetPixel3`'s guard: only values
    /// `0..=15` are ever stored; `16`/`17`/anything higher, and any
    /// out-of-canvas coordinate, are silently ignored — never a panic.
    pub fn set_pixel(&mut self, x: usize, y: usize, value: u8) {
        if x >= WIDTH || y >= HEIGHT || value >= 16 {
            return;
        }
        self.pixels[y * WIDTH + x] = value;
    }

    /// Reads one pixel's stored palette index (`0..=15`). Panics on an
    /// out-of-canvas coordinate — a caller bug, not a runtime condition.
    pub fn get_pixel(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * WIDTH + x]
    }

    /// The raw pixel buffer, row-major, `WIDTH * HEIGHT` palette indices.
    pub fn pixels(&self) -> &[u8; WIDTH * HEIGHT] {
        &self.pixels
    }

    /// The current 16-entry palette (RGB triples).
    pub fn palette(&self) -> &[[u8; 3]; 16] {
        &self.palette
    }

    /// `SetEgaPalette(index, colour)` (`Display.cs:82-103`): repoints
    /// palette slot `index` at `canon_color`'s RGB triple from the fixed EGA
    /// canon — never from the currently-remapped palette, so remaps never
    /// cascade. Pixels already drawn with the old color are unaffected
    /// (palette effects are pointer-cheap precisely because pixels stay
    /// indices, D-UI1).
    pub fn set_ega_palette(&mut self, index: usize, canon_color: usize) {
        self.palette[index] = gbx_rules::palette::EGA_PALETTE[canon_color];
    }

    /// The D-UI7 golden surface: `SHA-256(pixels ‖ palette)`.
    pub fn hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&self.pixels[..]);
        for rgb in &self.palette {
            hasher.update(rgb);
        }
        hasher.finalize().into()
    }

    /// Hex-encoded [`Framebuffer::hash`], for golden-test literals.
    pub fn hash_hex(&self) -> String {
        self.hash().iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_canvas_is_black_with_the_ega_canon_palette() {
        let fb = Framebuffer::new();
        assert_eq!(fb.get_pixel(0, 0), 0);
        assert_eq!(fb.get_pixel(319, 199), 0);
        assert_eq!(*fb.palette(), gbx_rules::palette::EGA_PALETTE);
    }

    #[test]
    fn set_pixel_writes_values_0_to_15() {
        let mut fb = Framebuffer::new();
        fb.set_pixel(10, 20, 15);
        assert_eq!(fb.get_pixel(10, 20), 15);
    }

    #[test]
    fn set_pixel_ignores_transparency_16_and_no_draw_17() {
        let mut fb = Framebuffer::new();
        fb.set_pixel(5, 5, 7);
        fb.set_pixel(5, 5, 16);
        assert_eq!(fb.get_pixel(5, 5), 7, "transparency-16 must not overwrite");
        fb.set_pixel(5, 5, 17);
        assert_eq!(fb.get_pixel(5, 5), 7, "no-draw-17 must not overwrite");
        fb.set_pixel(5, 5, 200);
        assert_eq!(
            fb.get_pixel(5, 5),
            7,
            "out-of-range values must not overwrite"
        );
    }

    #[test]
    fn set_pixel_ignores_out_of_canvas_coordinates() {
        let mut fb = Framebuffer::new();
        // Must not panic.
        fb.set_pixel(WIDTH, 0, 5);
        fb.set_pixel(0, HEIGHT, 5);
        fb.set_pixel(usize::MAX, usize::MAX, 5);
    }

    #[test]
    fn set_ega_palette_remaps_from_the_fixed_canon_not_the_current_table() {
        let mut fb = Framebuffer::new();
        fb.set_ega_palette(1, 10); // slot 1 <- canon color 10 (green)
        assert_eq!(fb.palette()[1], gbx_rules::palette::EGA_PALETTE[10]);
        // Remapping slot 10 itself must not chase the already-remapped slot 1.
        fb.set_ega_palette(10, 1);
        assert_eq!(fb.palette()[10], gbx_rules::palette::EGA_PALETTE[1]);
        assert_eq!(fb.palette()[1], gbx_rules::palette::EGA_PALETTE[10]);
    }

    #[test]
    fn hash_changes_on_pixel_mutation_and_on_palette_mutation() {
        let mut fb = Framebuffer::new();
        let h0 = fb.hash();
        fb.set_pixel(0, 0, 3);
        let h1 = fb.hash();
        assert_ne!(h0, h1, "a pixel write must change the hash");
        fb.set_pixel(0, 0, 0);
        assert_eq!(fb.hash(), h0, "reverting the pixel must reproduce the hash");
        fb.set_ega_palette(0, 4);
        let h2 = fb.hash();
        assert_ne!(
            h0, h2,
            "a palette write must change the hash even with identical pixels"
        );
    }

    #[test]
    fn hash_is_deterministic_across_independent_instances() {
        let a = Framebuffer::new();
        let mut b = Framebuffer::new();
        b.set_pixel(1, 1, 1);
        b.set_pixel(1, 1, 0);
        assert_eq!(a.hash(), b.hash());
    }
}
