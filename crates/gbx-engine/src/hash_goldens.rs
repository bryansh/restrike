//! Framebuffer-hash goldens (D-UI7): synthetic fixture assets driven
//! through draw sequences, SHA-256 of `pixels ‖ palette` pinned per
//! `(scenario, tick_index)`. Every fixture here is hand-authored (D10) —
//! nothing decoded from real game data. Regenerate pinned hashes by running
//! with `RESTRIKE_REGEN_GOLDENS=1` set (prints the actual hash instead of
//! asserting); on a mismatch (or during regen) a `.ppm` is dumped to the
//! system temp dir for eyeballing.

#![cfg(test)]

use crate::draw::{blit_image, Clip};
use crate::framebuffer::Framebuffer;
use crate::frames::draw8x8_03;
use crate::symbols::SymbolSets;
use crate::text::{JobStatus, TextCursor, TextJob, NORMAL_BOTTOM};
use gbx_formats::font::{self, Font};
use gbx_formats::image::{DecodedItem, ImageBlock};

fn write_ppm(name: &str, fb: &Framebuffer) {
    let path = std::env::temp_dir().join(format!("restrike-golden-{name}.ppm"));
    let mut out = format!(
        "P6\n{} {}\n255\n",
        crate::framebuffer::WIDTH,
        crate::framebuffer::HEIGHT
    )
    .into_bytes();
    for y in 0..crate::framebuffer::HEIGHT {
        for x in 0..crate::framebuffer::WIDTH {
            let idx = fb.get_pixel(x, y);
            out.extend_from_slice(&fb.palette()[idx as usize]);
        }
    }
    if std::fs::write(&path, &out).is_ok() {
        eprintln!("golden '{name}': dumped {}", path.display());
    }
}

fn check_golden(name: &str, fb: &Framebuffer, expected_hex: &str) {
    let actual = fb.hash_hex();
    let regen = std::env::var_os("RESTRIKE_REGEN_GOLDENS").is_some();
    if regen {
        eprintln!("golden '{name}': {actual}");
        write_ppm(name, fb);
        return;
    }
    if actual != expected_hex {
        write_ppm(name, fb);
    }
    assert_eq!(
        actual, expected_hex,
        "golden '{name}' mismatched — see dumped .ppm (or rerun with RESTRIKE_REGEN_GOLDENS=1)"
    );
}

/// A synthetic set-4 image block covering every id `draw8x8_03`'s tables
/// reference (`0x100..=0x127`, i.e. 40 items), each item a solid 8×8 block
/// of `item_index % 16` — deterministic and visually distinguishable.
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
    // Glyph `j`'s 8 row bytes each equal `j as u8` — deterministic, and
    // distinct glyphs produce visually distinct patterns.
    let mut data = Vec::with_capacity(font::GLYPH_COUNT * font::GLYPH_BYTES);
    for j in 0..font::GLYPH_COUNT {
        data.extend_from_slice(&[j as u8; font::GLYPH_BYTES]);
    }
    font::decode(&data)
}

#[test]
fn golden_empty_frame() {
    let mut fb = Framebuffer::new();
    let mut sets = SymbolSets::new();
    sets.load(4, synthetic_set4());
    draw8x8_03(&mut fb, &sets).unwrap();
    check_golden(
        "empty-frame",
        &fb,
        "679c4933fad4f6744b2eb6c81db894b65855c669e5854a6221f36c0b04ceb7ec",
    );
}

#[test]
fn golden_frame_with_wrapped_text() {
    let mut fb = Framebuffer::new();
    let mut sets = SymbolSets::new();
    sets.load(4, synthetic_set4());
    draw8x8_03(&mut fb, &sets).unwrap();

    let font = synthetic_font();
    let mut cursor = TextCursor {
        col: NORMAL_BOTTOM.x_start,
        row: NORMAL_BOTTOM.y_start,
    };
    let mut job = TextJob::new(
        "The party enters a quiet stone corridor, torchlight flickering.",
        10,
        NORMAL_BOTTOM,
        true,
        &mut cursor,
        &mut fb,
    );
    loop {
        match job.advance(1_000_000, &mut fb, &font, &mut cursor) {
            JobStatus::Done => break,
            JobStatus::NeedsKey => job.release(&mut fb),
            JobStatus::Continuing => unreachable!("budget was effectively unlimited"),
        }
    }

    check_golden(
        "frame-wrapped-text",
        &fb,
        "5f25ab37c55a4a3ecb0f3b9eb65aa5c668fa6a823a9987cd7767eb0a1735e9c5",
    );
}

#[test]
fn golden_paginated_text_mid_pause() {
    let mut fb = Framebuffer::new();
    let mut sets = SymbolSets::new();
    sets.load(4, synthetic_set4());
    draw8x8_03(&mut fb, &sets).unwrap();

    let font = synthetic_font();
    // A narrow, short region so a modest sentence is guaranteed to paginate.
    let region = crate::text::TextRegion {
        y_start: 17,
        y_end: 17,
        x_start: 1,
        x_end: 10,
    };
    let mut cursor = TextCursor {
        col: region.x_start,
        row: region.y_start,
    };
    let mut job = TextJob::new(
        "one two three four five six seven",
        10,
        region,
        true,
        &mut cursor,
        &mut fb,
    );

    let mut status = JobStatus::Continuing;
    for _ in 0..10_000 {
        status = job.advance(1, &mut fb, &font, &mut cursor);
        if status == JobStatus::NeedsKey {
            break;
        }
    }
    assert_eq!(
        status,
        JobStatus::NeedsKey,
        "fixture must actually reach pagination"
    );

    check_golden(
        "paginated-text-mid-pause",
        &fb,
        "4563e995fc157d1208229be36012e3aba3c1b5116c6f3c6d6d48ada0b2f3b26f",
    );
}

#[test]
fn golden_recolor_blit() {
    let mut fb = Framebuffer::new();
    let pixels = vec![5u8, 3, 16, 8, 5, 3, 16, 8];
    blit_image(
        &mut fb,
        &pixels,
        8,
        1,
        5,
        5,
        Clip::FULL,
        Some(8),
        Some((3, 12)),
    );

    check_golden(
        "recolor-blit",
        &fb,
        "c5ba546d43aec2fd0f258543e1a2035838349d961df1347688db5bf3e8fbf514",
    );
}
