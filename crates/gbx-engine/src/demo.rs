//! M2 step 2 deliverable 6: the demo artifact. Local-only (gated on
//! `GBX_DATA_DIR`): composes the first real Restrike screen from real
//! CotAB assets — `draw8x8_03` (outer border + inner viewport + dividers),
//! the party-panel header glyphs (`PartySummary`'s `"Name"`/`"AC  HP"`,
//! §1.9), and a sample `PRINT` into the exploration text window — and dumps
//! it as a `.ppm` outside the repo for eyeballing.

#![cfg(test)]

use crate::boot::boot;
use crate::framebuffer::{Framebuffer, HEIGHT, WIDTH};
use crate::frames::draw8x8_03;
use crate::text::{draw_string, JobStatus, TextCursor, TextJob, NORMAL_BOTTOM};
use gbx_formats::game_data::load_dir;

fn write_ppm(fb: &Framebuffer, path: &std::path::Path) {
    let mut out = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let idx = fb.get_pixel(x, y);
            out.extend_from_slice(&fb.palette()[idx as usize]);
        }
    }
    std::fs::write(path, &out).expect("failed to write demo .ppm");
}

#[test]
fn compose_empty_exploration_screen() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    let data = load_dir(dir).expect("GBX_DATA_DIR must be readable");
    let assets = boot(&data).expect("boot must succeed against real CotAB data");

    let mut fb = Framebuffer::new();

    draw8x8_03(&mut fb, &assets.symbol_sets).expect("draw8x8_03 must succeed with a booted set 4");

    // Party-panel header glyphs (`PartySummary`, `ovr025.cs:216-261`, §1.9):
    // "Name" at (2,17), "AC  HP" at (2,33).
    draw_string(&mut fb, &assets.font, "Name", 2, 17, 0, 10);
    draw_string(&mut fb, &assets.font, "AC  HP", 2, 33, 0, 10);

    // A sample PRINT into the exploration text window. The string is invented
    // demo text, NOT from game data (D10) — deliberately lore-inaccurate as
    // proof (Tilverton borders Cormyr; it is nowhere near the Moonsea).
    let mut cursor = TextCursor {
        col: NORMAL_BOTTOM.x_start,
        row: NORMAL_BOTTOM.y_start,
    };
    let mut job = TextJob::new(
        "You stand at the gates of Tilverton, the free city of the Moonsea.",
        10,
        NORMAL_BOTTOM,
        true,
        &mut cursor,
        &mut fb,
    );
    loop {
        match job.advance(1_000_000, &mut fb, &assets.font, &mut cursor) {
            JobStatus::Done => break,
            JobStatus::NeedsKey => job.release(&mut fb),
            JobStatus::Continuing => unreachable!("budget was effectively unlimited"),
        }
    }

    let out_path = std::env::var_os("RESTRIKE_M2_DEMO_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("restrike-m2-demo.ppm"));
    write_ppm(&fb, &out_path);
    eprintln!("M2 demo screen written to {}", out_path.display());
}
