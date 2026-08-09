//! End-to-end tests for the picture layer (`crate::picture`), driven through
//! `Engine::tick` against **hand-authored** picture assets (D10 — no game data
//! ever enters this repo; the real-asset checks are the `GBX_DATA_DIR`-gated
//! local tier in `picture.rs`'s own test module).
//!
//! Every fixture block is a flat fill of one palette value, so "did the right
//! picture land at the right cell" is a pixel equality, not an eyeball.

#![cfg(test)]

use crate::engine::{Engine, GAME_AREA, INITIAL_ECL_BLOCK};
use crate::framebuffer::Framebuffer;
use crate::picture::{compose_into, PictureCache, PictureLayer, Shown};
use crate::test_support::{build_dax_file, ecl_dax_block, labeled_block};
use gbx_formats::font::{self, Font};
use gbx_formats::game_data::GameData;
use gbx_formats::geo::{GeoBlock, GEO_BLOCK_SIZE};
use gbx_formats::image::{DecodedItem, ImageBlock};
use gbx_vm::test_support::EclBuilder;

// --- synthetic asset authoring (D10) ---------------------------------------

/// Packs `width_px * height` pixel values into the 4bpp, high-nibble-first
/// stream both container formats share (`gbx_formats::image::unpack_nibbles`'s
/// inverse).
fn pack_nibbles(pixels: &[u8]) -> Vec<u8> {
    pixels
        .chunks(2)
        .map(|pair| (pair[0] << 4) | pair.get(1).copied().unwrap_or(0))
        .collect()
}

/// A one-item [`gbx_formats::image::decode`] block, every pixel `fill`.
/// `width_cols * 8` wide, `height` tall.
fn image_block(width_cols: u16, height: u16, fill: u8) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&width_cols.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // x_pos
    out.extend_from_slice(&0u16.to_le_bytes()); // y_pos
    out.push(1); // item_count
    out.extend_from_slice(&[0u8; 8]); // field_9
    let px = vec![fill; width_cols as usize * 8 * height as usize];
    out.extend_from_slice(&pack_nibbles(&px));
    out
}

/// An animated (`PIC`) block whose frame `n` is a flat fill of `fills[n]`.
/// Frames ≥ 1 are stored XOR-delta'd against frame 0 with the last encoded
/// byte verbatim, exactly as `load_pic_final` reads them
/// (`ovr030.cs:107,119-134`).
fn anim_block(width_cols: u16, height: u16, fills: &[u8]) -> Vec<u8> {
    let encoded_len = height as usize * width_cols as usize * 4;
    let true_bytes: Vec<Vec<u8>> = fills
        .iter()
        .map(|&f| pack_nibbles(&vec![f; width_cols as usize * 8 * height as usize]))
        .collect();

    let mut out = vec![fills.len() as u8];
    for (i, truth) in true_bytes.iter().enumerate() {
        let mut stored = truth.clone();
        if i > 0 {
            for j in 0..encoded_len - 1 {
                stored[j] ^= true_bytes[0][j];
            }
        }
        out.extend_from_slice(&2u32.to_le_bytes()); // delay
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&width_cols.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // x_pos
        out.extend_from_slice(&0u16.to_le_bytes()); // y_pos
        out.push(0); // pad byte, never read
        out.extend_from_slice(&[0u8; 8]); // field_9
        out.extend_from_slice(&stored);
    }
    out
}

const PIC_FILL: u8 = 9;
const PIC_FRAME2_FILL: u8 = 11;
const HEAD_FILL: u8 = 4;
const BODY_FILL: u8 = 5;
const BIGPIC_FILL: u8 = 6;

/// The full synthetic asset set the tests below draw from: a 3-frame `PIC`
/// block 1, a `HEAD`/`BODY` pair at block 3, and a `BIGPIC` block `0x78` —
/// all at the real set's own geometry (88×88 / 88×40 + 88×48 / 304×120), so
/// the pixel bounds asserted here are the bounds real content produces.
fn picture_game_data(blocks: Vec<(u8, EclBuilder)>) -> GameData {
    let ecl: Vec<(u8, Vec<u8>)> = blocks
        .iter()
        .map(|(id, b)| (*id, ecl_dax_block(&b.build_bytes())))
        .collect();
    GameData::from_files([
        (format!("ECL{GAME_AREA}.DAX"), build_dax_file(&ecl)),
        (
            format!("PIC{GAME_AREA}.DAX"),
            build_dax_file(&[(1, anim_block(11, 88, &[PIC_FILL, PIC_FRAME2_FILL, 12]))]),
        ),
        (
            format!("HEAD{GAME_AREA}.DAX"),
            build_dax_file(&[(3, image_block(11, 40, HEAD_FILL))]),
        ),
        (
            format!("BODY{GAME_AREA}.DAX"),
            build_dax_file(&[(3, image_block(11, 48, BODY_FILL))]),
        ),
        (
            format!("BIGPIC{GAME_AREA}.DAX"),
            build_dax_file(&[(0x78, image_block(38, 120, BIGPIC_FILL))]),
        ),
    ])
}

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
    let mut data = Vec::with_capacity(font::GLYPH_COUNT * font::GLYPH_BYTES);
    for j in 0..font::GLYPH_COUNT {
        data.extend_from_slice(&[j as u8; font::GLYPH_BYTES]);
    }
    font::decode(&data)
}

/// `area2_ptr.HeadBlockId`'s ECL address (`vmhost.rs`'s `HEAD_BLOCK_ID_ADDR`).
const HEAD_BLOCK_ID_ADDR: u16 = 0x7EE1;

/// Builds an engine whose resident block runs `body` from every vector.
fn engine_running(body: impl FnOnce(&mut EclBuilder)) -> Engine {
    let block = labeled_block(["entry"; 5], |b| {
        b.label("entry");
        body(b);
    });
    let data = picture_game_data(vec![(INITIAL_ECL_BLOCK, block)]);
    let mut sets = crate::symbols::SymbolSets::new();
    sets.load(4, synthetic_set4());
    let geo = GeoBlock::parse(&vec![0u8; GEO_BLOCK_SIZE]).unwrap();
    Engine::new_fixture(synthetic_font(), sets, geo, data, 1)
}

/// One tick's pixels, copied out — `Frame` is the only public view of the
/// engine's framebuffer and it borrows the engine, so tests snapshot rather
/// than re-tick (a re-tick would advance the boot flow into the walk loop,
/// whose viewport recompose is exactly what destroys a picture).
fn tick_pixels(engine: &mut Engine, input: &[crate::input::InputEvent]) -> Vec<u8> {
    engine.tick(input).pixels.to_vec()
}

fn at(px: &[u8], x: usize, y: usize) -> u8 {
    px[y * crate::framebuffer::WIDTH + x]
}

// --- the four draw arms ----------------------------------------------------

/// Deliverable 1: `blockId < 0x78` with no encounter head draws the `PIC`
/// animation's first frame at cell (3,3) — pixels (24,24)..(112,112).
#[test]
fn a_plain_picture_draws_the_pic_animations_first_frame_at_cell_3_3() {
    let mut engine = engine_running(|b| {
        b.op(0x0E).imm_byte(1); // PICTURE 1
        b.op(0x00); // EXIT
    });
    // One tick: the effect drains inside the boot flow's first pump/present,
    // before the flow finishes and the walk loop recomposes the viewport.
    let px = tick_pixels(&mut engine, &[]);

    assert_eq!(at(&px, 24, 24), PIC_FILL, "top-left of the picture");
    assert_eq!(
        at(&px, 111, 111),
        PIC_FILL,
        "bottom-right of the 88x88 picture"
    );
    assert_ne!(at(&px, 23, 24), PIC_FILL, "one pixel left of it");
    assert_ne!(at(&px, 112, 111), PIC_FILL, "one pixel right of it");
    assert_eq!(engine.state.picture.shown, Shown::Pic);
    assert_eq!(engine.state.picture.anim_block, Some(1));
    assert_eq!(engine.state.picture.anim_frame, 1, "curFrame is 1-based");
}

/// Deliverable 2: `blockId >= 0x78` draws `DrawFrame_WildernessMap`'s frame
/// plus the BIGPIC at cell (1,1) — pixels (8,8)..(312,128), full-canvas clip
/// (so it reaches well past the PIC arm's 176-pixel overlay bound).
#[test]
fn a_bigpic_block_draws_the_wilderness_frame_and_the_picture_at_cell_1_1() {
    let mut engine = engine_running(|b| {
        b.op(0x0E).imm_byte(0x78); // PICTURE 0x78
        b.op(0x00);
    });
    let px = tick_pixels(&mut engine, &[]);

    assert_eq!(at(&px, 8, 8), BIGPIC_FILL, "top-left at cell (1,1)");
    assert_eq!(
        at(&px, 311, 127),
        BIGPIC_FILL,
        "bottom-right of the 304x120 bigpic"
    );
    assert_eq!(
        at(&px, 200, 100),
        BIGPIC_FILL,
        "past the overlay clip's 176px bound -- BIGPIC uses the full clip"
    );
    assert_eq!(engine.state.picture.shown, Shown::BigPic);
    assert_eq!(engine.state.picture.bigpic_block, Some(0x78));
    assert_eq!(
        engine.state.picture.anim_block, None,
        "`load_bigpic` frees the running animation first (ovr030.cs:230)"
    );
}

/// Deliverable 1 (head/body arm): with `HeadBlockId != 0xFF` the operand is a
/// BODY id and the pair tiles the viewport — head 40px at row 3, body 48px at
/// row 8 (`ovr008.cs:208-217`, `ovr030.cs:193-204`).
#[test]
fn an_encounter_head_draws_head_at_row_3_and_body_five_cells_lower() {
    let mut engine = engine_running(|b| {
        b.op(0x09).imm_byte(3).mem(HEAD_BLOCK_ID_ADDR); // SAVE 3 -> HeadBlockId
        b.op(0x0E).imm_byte(3); // PICTURE 3 (a BODY id here)
        b.op(0x09).imm_byte(0xFF).mem(HEAD_BLOCK_ID_ADDR); // SAVE 0xFF -> HeadBlockId
        b.op(0x00);
    });
    let px = tick_pixels(&mut engine, &[]);

    assert_eq!(at(&px, 24, 24), HEAD_FILL, "head top-left");
    assert_eq!(at(&px, 111, 63), HEAD_FILL, "head bottom-right");
    assert_eq!(at(&px, 24, 64), BODY_FILL, "body top-left, row 8");
    assert_eq!(at(&px, 111, 111), BODY_FILL, "body bottom-right");
    assert_eq!(engine.state.picture.shown, Shown::HeadBody);
    assert_eq!(engine.state.picture.head_block, 3);
    assert_eq!(engine.state.picture.body_block, 3);
}

/// **The buffered-effect trap** (`shell.rs`'s `QueuedEffect`): the script
/// clears `HeadBlockId` on the very next instruction, so a drain-time read
/// would see `0xFF` and take the wrong arm. Real CotAB content is written
/// exactly this way. Proven by the negative: the same script *without* the
/// head write takes the plain-PIC arm instead.
#[test]
fn the_head_block_id_is_read_at_execution_time_not_at_drain_time() {
    let mut with_head = engine_running(|b| {
        b.op(0x09).imm_byte(3).mem(HEAD_BLOCK_ID_ADDR);
        b.op(0x0E).imm_byte(3);
        b.op(0x09).imm_byte(0xFF).mem(HEAD_BLOCK_ID_ADDR);
        b.op(0x00);
    });
    with_head.tick(&[]);
    assert_eq!(with_head.state.picture.shown, Shown::HeadBody);
    assert_eq!(
        with_head.state.head_block_id, 0xFF,
        "the script's own reset still landed"
    );

    let mut without_head = engine_running(|b| {
        b.op(0x0E).imm_byte(1);
        b.op(0x00);
    });
    without_head.tick(&[]);
    assert_eq!(without_head.state.picture.shown, Shown::Pic);
}

/// Deliverable 3: `PICTURE 0xFF` runs `RedrawView` and leaves nothing shown
/// (`ovr003.cs:343-356`).
#[test]
fn picture_0xff_clears_the_layer_and_redraws_the_view() {
    let mut engine = engine_running(|b| {
        b.op(0x0E).imm_byte(1); // PICTURE 1
        b.op(0x0E).imm_byte(0xFF); // PICTURE 0xFF
        b.op(0x00);
    });
    let px = tick_pixels(&mut engine, &[]);

    assert_eq!(engine.state.picture.shown, Shown::Nothing);
    assert_ne!(at(&px, 24, 24), PIC_FILL, "the picture is gone");
    assert_eq!(
        engine.state.picture.anim_block,
        Some(1),
        "the clear repaints, it does not free the loaded animation"
    );
}

/// Deliverable 4: ANIMATION draws the **current** frame then advances, so the
/// first one re-draws frame 0 and the second shows frame 1
/// (`ovr003.cs:1902-1905`, `DaxArray.cs:34-47`).
#[test]
fn animation_draws_the_current_frame_then_advances_the_cursor() {
    const ANIMATION: u16 = 0x7FFFu16.wrapping_add(0xE804);
    let mut engine = engine_running(|b| {
        b.op(0x0E).imm_byte(1); // PICTURE 1 -> curFrame 1
        b.op(0x2D).imm_word(ANIMATION);
        b.op(0x2D).imm_word(ANIMATION);
        b.op(0x00);
    });

    // Tick 1 drains PICTURE and the first ANIMATION: the frame drawn is still
    // frame 0 (`CurrentPicture()` before `NextFrame()`), and the cursor has
    // moved to 2.
    let px = tick_pixels(&mut engine, &[]);
    assert_eq!(
        at(&px, 24, 24),
        PIC_FILL,
        "the first ANIMATION re-draws frame 0 before advancing"
    );
    assert_eq!(engine.state.picture.anim_frame, 2);

    // Each ANIMATION parks a `Request::Delay` behind it; drive it until the
    // second one lands.
    let mut frame_two_pixels = None;
    for _ in 0..120 {
        let px = tick_pixels(&mut engine, &[crate::input::InputEvent::Enter]);
        if engine.state.picture.anim_frame == 3 {
            frame_two_pixels = Some(px);
            break;
        }
    }
    let px = frame_two_pixels.expect("the second ANIMATION must advance to frame 3");
    assert_eq!(
        at(&px, 24, 24),
        PIC_FRAME2_FILL,
        "the second ANIMATION draws frame 1, the XOR-delta frame"
    );
}

// --- the redraw rule -------------------------------------------------------

/// The design decision, pinned: a viewport recompose destroys the picture,
/// because `Draw3dWorld` paints the very cells it occupies. Every real
/// picture-bearing vector relies on this (their tails are `PICTURE 0xFF` or
/// `CALL 0xAE11`), and it is what stops a portrait from surviving into the
/// walk loop.
#[test]
fn a_viewport_recompose_destroys_the_picture() {
    let mut engine = engine_running(|b| {
        b.op(0x0E).imm_byte(1);
        b.op(0x00);
    });
    let px = tick_pixels(&mut engine, &[]);
    assert_eq!(engine.state.picture.shown, Shown::Pic);
    assert_eq!(at(&px, 24, 24), PIC_FILL);

    // The boot flow reaches the world menu, whose entry recomposes.
    let mut px = Vec::new();
    for _ in 0..8 {
        px = tick_pixels(&mut engine, &[]);
    }
    assert_eq!(engine.state.picture.shown, Shown::Nothing);
    assert_ne!(at(&px, 24, 24), PIC_FILL);
}

// --- persistence -----------------------------------------------------------

/// Deliverable 5: the layer round-trips through the `.rsav` payload, and a
/// restored state recomposes to the same pixels the live engine had
/// (`Engine::assemble`'s own `compose_into` call, exercised directly here —
/// `Engine::restore` needs the full boot asset set, which the picture fixture
/// deliberately does not carry).
#[test]
fn the_picture_layer_survives_save_and_restore_and_recomposes() {
    let mut engine = engine_running(|b| {
        b.op(0x0E).imm_byte(1);
        b.op(0x00);
    });
    engine.tick(&[]);
    assert_eq!(engine.state.picture.shown, Shown::Pic);
    let live = engine.state.picture;

    let bytes = engine.save();
    let data = picture_game_data(vec![(
        INITIAL_ECL_BLOCK,
        labeled_block(["entry"; 5], |b| {
            b.label("entry");
            b.op(0x0E).imm_byte(1);
            b.op(0x00);
        }),
    )]);
    let (_header, state) = crate::save::load(&bytes, &data).expect("the payload must decode");
    assert_eq!(state.state.picture, live, "the layer round-trips verbatim");

    let mut sets = crate::symbols::SymbolSets::new();
    sets.load(4, synthetic_set4());
    let mut fb = Framebuffer::new();
    let mut cache = PictureCache::new();
    compose_into(
        &mut fb,
        &sets,
        &data,
        GAME_AREA,
        &state.state.picture,
        &mut cache,
        0,
    )
    .expect("the restored picture must compose");
    assert_eq!(
        fb.get_pixel(24, 24),
        PIC_FILL,
        "a restored save puts the picture back before any tick runs"
    );
}

// --- composition properties ------------------------------------------------

/// D-UI4: composing twice must produce the same pixels as composing once —
/// the whole design rests on the picture being re-derivable state.
#[test]
fn composing_twice_is_identical_to_composing_once() {
    let data = picture_game_data(vec![(INITIAL_ECL_BLOCK, {
        let mut b = labeled_block(["entry"; 5], |b| {
            b.label("entry");
            b.op(0x00);
        });
        b.raw(&[]);
        b
    })]);
    let mut sets = crate::symbols::SymbolSets::new();
    sets.load(4, synthetic_set4());
    let layer = PictureLayer {
        anim_block: Some(1),
        anim_frame: 1,
        shown: Shown::Pic,
        ..PictureLayer::default()
    };
    let mut cache = PictureCache::new();

    let mut once = Framebuffer::new();
    compose_into(&mut once, &sets, &data, GAME_AREA, &layer, &mut cache, 0).unwrap();
    let mut twice = Framebuffer::new();
    compose_into(&mut twice, &sets, &data, GAME_AREA, &layer, &mut cache, 0).unwrap();
    compose_into(&mut twice, &sets, &data, GAME_AREA, &layer, &mut cache, 0).unwrap();

    assert_eq!(once.hash(), twice.hash());
}

/// A missing asset is refused loudly (a halt-log entry) and stops showing,
/// rather than panicking or silently blanking the viewport forever.
#[test]
fn a_missing_picture_block_is_reported_and_stops_showing() {
    let mut engine = engine_running(|b| {
        b.op(0x0E).imm_byte(2); // PIC block 2 is not in the fixture set
        b.op(0x00);
    });
    engine.tick(&[]);

    assert_eq!(engine.state.picture.shown, Shown::Nothing);
    let halts = &engine.vm_memory().halts;
    assert!(
        halts.iter().any(|h| h.description.contains("PICTURE")),
        "the failure must be in the halt log, got {halts:?}"
    );
}

/// ★ FD-33's menu-wait half (`ovr027.cs:184-198` via `tick_gate`): a
/// HORIZONTAL MENU parked over a fresh PICTURE keeps re-blitting the running
/// animation and advances it once per frame's own `delay * 100` ms — the
/// synthetic block's delay is 2, i.e. 12 ticks at 60 Hz. `CMD_Picture`
/// itself armed `useOverlay`'s two flags (`spriteChanged` `ovr003.cs:320`,
/// `byte_1EE8D` `:324`), which is what makes the intro's sword-arm sigils
/// cycle while "PRESS BUTTON OR RETURN" waits (Bryan's 2026-08-08 DOSBox
/// side-by-side).
#[test]
fn a_parked_menu_animates_the_picture_at_the_frames_own_delay() {
    let mut engine = engine_running(|b| {
        b.op(0x0E).imm_byte(1); // PICTURE 1 -> curFrame 1, flags armed
        b.op(0x2B) // HORIZONTAL MENU parks a Hotbar gate over it
            .mem(0x5000)
            .imm_byte(1)
            .inline_str(b"WAIT");
        b.op(0x00);
    });
    let px = tick_pixels(&mut engine, &[]);
    assert_eq!(at(&px, 24, 24), PIC_FILL, "frame 1 up at the parked menu");
    assert_eq!(engine.state.picture.anim_frame, 1);

    let mut advanced_at = None;
    for waited in 1..=30u32 {
        tick_pixels(&mut engine, &[]);
        if engine.state.picture.anim_frame == 2 {
            advanced_at = Some(waited);
            break;
        }
    }
    let at_tick = advanced_at.expect("the parked menu must advance the animation");
    assert!(
        (11..=13).contains(&at_tick),
        "the dwell is the frame's own delay*6 = 12 ticks, got {at_tick}"
    );
    // The advancing tick drew the *current* frame before moving the cursor
    // (the original's draw-then-NextFrame order); frame 2's pixels land on
    // the next re-blit.
    let px = tick_pixels(&mut engine, &[]);
    assert_eq!(
        at(&px, 24, 24),
        PIC_FRAME2_FILL,
        "frame 2 is on screen while the menu still waits"
    );
}
