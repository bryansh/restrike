//! D-UI6 scaling: the default is aspect-correct per-axis integer scaling at
//! the 5:6 pixel ratio (320x200 -> 1600x1200 at x5,x6; 3200x2400 at x10,x12
//! on a Retina-class display) -- the largest `(5k, 6k)` that fits the
//! surface in physical pixels, letterboxed on black. `--square-pixels`
//! (default off, D4) instead picks the largest single integer `k`, crisper
//! but 17% squashed. If even the minimum `(5,6)` step doesn't fit the
//! surface, fall back to the largest square-pixel integer that does (the
//! doc's stated minimum-viable presentation).

use gbx_engine::framebuffer::{HEIGHT, WIDTH};

pub struct Scale {
    pub scale_x: u32,
    pub scale_y: u32,
    pub offset_x: u32,
    pub offset_y: u32,
}

pub fn compute(surface_w: u32, surface_h: u32, square_pixels: bool) -> Scale {
    let (scale_x, scale_y) = if square_pixels {
        largest_square(surface_w, surface_h)
    } else {
        let k = (surface_w / (WIDTH as u32 * 5)).min(surface_h / (HEIGHT as u32 * 6));
        if k >= 1 {
            (k * 5, k * 6)
        } else {
            largest_square(surface_w, surface_h)
        }
    };
    let offset_x = surface_w.saturating_sub(WIDTH as u32 * scale_x) / 2;
    let offset_y = surface_h.saturating_sub(HEIGHT as u32 * scale_y) / 2;
    Scale {
        scale_x,
        scale_y,
        offset_x,
        offset_y,
    }
}

fn largest_square(surface_w: u32, surface_h: u32) -> (u32, u32) {
    let k = (surface_w / WIDTH as u32)
        .min(surface_h / HEIGHT as u32)
        .max(1);
    (k, k)
}
