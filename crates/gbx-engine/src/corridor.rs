//! The 3D dungeon-corridor renderer and its area-map alternative (§1.7,
//! task deliverables 2-3), plus `RedrawView`'s sky-color selection (part of
//! task deliverable 4's redraw consolidation).
//!
//! Derived by reading coab for behavior (D11, never copied) — two dedicated
//! research passes this session read `engine/ovr029.cs`'s `RedrawView` in
//! full and `engine/ovr031.cs`'s `Draw3dWorld`/`Draw3dWorldBackground`/
//! `Draw3dWorldFar`/`Draw3dWorldMid`/`Draw3dWorldNear`/`draw_3D_8x8_titles`/
//! `DrawAreaMap`/`LoadWalldef`/the coordinate-wrap helpers in full,
//! cross-checked against `Classes/GeoBlock.cs`, `Classes/Gbl.cs`, and
//! `Classes/Sys.cs`; every formula/table below cites that pass. Every table
//! and formula here is either quoted verbatim from that research (Far's two
//! front sweeps, Mid, Near, the background, the area map, `LoadWalldef`) or,
//! where noted, a documented reconstruction filling a gap the research
//! summarized rather than transcribed line-by-line (Far's two *side* sweeps,
//! §1.11-style flagged, not silently absorbed).

use crate::draw::draw_color_block;
use crate::framebuffer::Framebuffer;
use crate::movement::Facing;
use crate::shell::FlowCtx;
use crate::symbols::{draw_symbol, draw_symbol_no_draw, SymbolSets};
use gbx_formats::geo::GeoBlock;
use gbx_formats::image::ImageBlock;

// --- Draw-cell classes A-J (`ovr031.cs:8-27,140-142`) ---

/// `idxOffset` (`ovr031.cs:140`) — the 11th literal element (`1`) is dead:
/// unreachable (no caller ever passes class index 10) and would panic
/// `colCount`/`rowCount` (both 10-element) if it were — a decompiler
/// artifact, omitted here.
const IDX_OFFSET: [usize; 10] = [0, 2, 6, 10, 22, 38, 54, 110, 132, 154];
const COL_COUNT: [i32; 10] = [1, 1, 1, 3, 2, 2, 7, 2, 2, 1];
const ROW_COUNT: [i32; 10] = [2, 4, 4, 4, 8, 8, 8, 11, 11, 2];

const CLASS_A: usize = 0; // far front
const CLASS_B: usize = 1; // far side, left sweep
const CLASS_C: usize = 2; // far side, right sweep
const CLASS_D: usize = 3; // mid front
const CLASS_E: usize = 4; // mid side, left sweep
const CLASS_F: usize = 5; // mid side, right sweep
const CLASS_G: usize = 6; // near front
const CLASS_H: usize = 7; // near side, left sweep
const CLASS_I: usize = 8; // near side, right sweep
const CLASS_J: usize = 9; // far filler

const ROW_A: i32 = 4;
const COL_A: i32 = 5;
const ROW_B: i32 = 3;
const COL_B: i32 = 4;
const ROW_C: i32 = 3;
const COL_C: i32 = 6;
const ROW_D: i32 = 3;
const COL_D: i32 = 4;
const ROW_E: i32 = 1;
const COL_E: i32 = 2;
const ROW_F: i32 = 1;
const COL_F: i32 = 7;
const ROW_G: i32 = 1;
const COL_G: i32 = 2;
const ROW_H: i32 = 0;
const COL_H: i32 = 0;
const ROW_I: i32 = 0;
const COL_I: i32 = 9;
const ROW_J: i32 = 4;
const COL_J: i32 = 5;

/// `sky_colours` (`ovr029.cs:7-8`) — 16 entries, two identical 8-entry
/// halves, confirmed byte-for-byte this session.
const SKY_COLOURS: [u8; 16] = [
    0x00, 0x0F, 0x04, 0x0B, 0x0D, 0x02, 0x09, 0x0E, 0x00, 0x0F, 0x04, 0x0B, 0x0D, 0x02, 0x09, 0x0E,
];

/// `area_ptr.outdoor_sky_colour`/`indoor_sky_colour` (`Area1.cs` DataOffset
/// `0x1FA`/`0x1FC`) — FD-31 RESOLVED: the Area window's confirmed mapping is
/// `addr = 0x4B00 + DataOffset/2` (`vm_SetMemoryValue` passes
/// `0x6A00 + location*2` to the setter and the constant vanishes in the
/// `ushort` index, `ovr008.cs:715`; full three-way derivation in
/// `crate::picture::PICTURE_FADE_ADDR`'s doc), so the sky cells are
/// `0x4BFD`/`0x4BFE` — exactly the pair whose writes dirty `byte_1EE94`
/// (`vmhost.rs`'s `FORCE_REDRAW_ADDRS`), which is *why* writing a sky
/// colour forces a redraw. The un-halved `0x4CFA`/`0x4CFC` this replaced
/// always read raw-store `0`: every sky since M2 was `SKY_COLOURS[0]` black
/// regardless of what the imported save or a script put in the real cells.
const OUTDOOR_SKY_COLOUR_ADDR: u16 = 0x4B00 + 0x1FA / 2;
const INDOOR_SKY_COLOUR_ADDR: u16 = 0x4B00 + 0x1FC / 2;

/// `MapDirectionXDelta`/`MapDirectionYDelta`, restricted to the cardinal
/// (even-code) entries this engine's [`Facing`] models — `dir_left`/
/// `dir_right`/`dir_behind` (`±2`/`±4`/`±6` mod 8 from an even `partyDir`)
/// always land on another even code, so the odd (diagonal) table entries
/// the original also carries are never read for a cardinal party facing and
/// are omitted here.
fn offset_dir(facing: Facing, offset: i32) -> Facing {
    let raw = (facing.raw_code() as i32 + offset).rem_euclid(8) as u8;
    Facing::from_raw(raw)
}

/// `MapCoordIsValid` (`ovr031.cs:175-178`), replicated bug and all: checks
/// `x >= 0` twice, never `y >= 0`. Written out longhand (not via
/// `Range::contains`) so the duplicated `x >= 0` clause stays visible as
/// the deliberate replication it is, not a range-refactor away from it.
#[allow(clippy::manual_range_contains)]
fn map_coord_is_valid(x: i32, y: i32) -> bool {
    x < 16 && x >= 0 && y < 16 && x >= 0
}

/// The snap-to-opposite-bound wrap every `getMap_XXX`/`get_wall_x2` call
/// site reimplements inline (`Sys.WrapMinMax`, confirmed *not* a true
/// modulo wrap — `v=17` snaps straight to `0`, not `1`).
fn wrap_coord(v: i32) -> usize {
    if v > 15 {
        0
    } else if v < 0 {
        15
    } else {
        v as usize
    }
}

/// `getMap_wall_type(direction, y, x)` (`ovr031.cs:222-251`): the wall-type
/// nibble at `(x, y)`'s `direction` edge, with the "blocks 0/10" special
/// case (`ecl_block_id` `0`/`10` + an out-of-range coordinate returns `0`
/// rather than wrapping, `:258-261`) and the snap-to-bound wrap otherwise.
fn get_wall_type(geo: &GeoBlock, ecl_block_id: u8, direction: Facing, x: i32, y: i32) -> u8 {
    if !map_coord_is_valid(x, y) && (ecl_block_id == 0 || ecl_block_id == 10) {
        return 0;
    }
    let sq = geo.square(wrap_coord(x), wrap_coord(y));
    match direction {
        Facing::North => sq.wall_north,
        Facing::East => sq.wall_east,
        Facing::South => sq.wall_south,
        Facing::West => sq.wall_west,
    }
}

/// `draw_3D_8x8_titles` (`ovr031.cs:145-171`): `wallset = (t-1)/5`,
/// `slice = (t-1)%5`; `idx` starts at the class's `IDX_OFFSET` and
/// increments once per `(row, col)` cell, row-major. The `0..=10` clip
/// guard and the `symbol_id > 0` "hole" skip are both replicated exactly —
/// `Put8x8Symbol` itself throws on id `0` in the original, so this
/// pre-filter is load-bearing, not redundant. The final framebuffer cell is
/// `local + 3` (`Put8x8Symbol`'s own `+2` plus `OverlayUnbounded`'s `+1`,
/// landing in the confirmed 11×11 viewport at cells `(3,3)`-`(13,13)`).
/// Missing wallset/tile data (an unloaded slot, or a wall type the resident
/// walldef simply doesn't cover) is a silent no-op — a documented
/// simplification of the original's own unguarded array index, which would
/// throw; real CotAB data never hits this path in this session's testing.
#[allow(clippy::too_many_arguments)]
fn draw_3d_8x8_titles(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    class: usize,
    wall_type: u8,
    row_start: i32,
    col_start: i32,
) {
    if wall_type == 0 {
        return;
    }
    let wallset = ((wall_type - 1) / 5) as usize;
    let slice = ((wall_type - 1) % 5) as usize;
    let Some(slot) = symbols.wallset(wallset) else {
        return;
    };
    let mut idx = IDX_OFFSET[class];
    for row in 0..ROW_COUNT[class] {
        for col in 0..COL_COUNT[class] {
            let row_local = row_start + row;
            let col_local = col_start + col;
            if (0..=10).contains(&row_local) && (0..=10).contains(&col_local) {
                if let Some(symbol_id) = slot.tile_id(slice, idx) {
                    if symbol_id > 0 {
                        let _ = draw_symbol(
                            fb,
                            symbols,
                            symbol_id as u32,
                            (row_local + 3) as usize,
                            (col_local + 3) as usize,
                        );
                    }
                }
            }
            idx += 1;
        }
    }
}

/// `Draw3dWorldBackground` (`ovr031.cs:93-137`): sky/black-band/gray-ground
/// fills (unconditional), then sun/moon overlays gated on outdoor + daytime
/// sky (`sky_colour == 11`) — including the confirmed **`get_wall_x2(mapY,
/// mapY)` bug** (`:99`, Y passed twice, never `mapX` — replicated exactly,
/// not "fixed", per D11), the confirmed narrowed South windows (`hour > 2`
/// morning / `hour >= 16` evening) and the North branch's own **unconditional
/// on hour** sibling `if` (not nested under the hour checks) — then the
/// horizon backdrop, confirmed **unconditional** on every call (outside the
/// sun/moon `if` block entirely, contrary to an "outdoors+daytime only"
/// assumption this session's research corrected).
#[allow(clippy::too_many_arguments)]
fn draw_3d_world_background(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    sky: &[ImageBlock; 3],
    geo: &GeoBlock,
    pos: (u8, u8),
    facing: Facing,
    hour: u8,
    sky_colour: u8,
) {
    draw_color_block(fb, sky_colour, 0x2c, 11, 16, 2); // sky, px rows 24-67
    draw_color_block(fb, 0, 2, 11, 0x3c, 2); // black band, rows 68-69
    draw_color_block(fb, 8, 0x2a, 11, 0x3e, 2); // gray ground, rows 70-111

    // The confirmed (Y,Y) bug: queries square(y,y), not square(x,y).
    let indoor_yy = geo.square(pos.1 as usize, pos.1 as usize).indoor;
    if !indoor_yy && sky_colour == 11 {
        const ROW_Y: i32 = 2;
        const COL_X: i32 = 2;
        let hour = hour as i32;
        if (1..=5).contains(&hour) {
            if facing == Facing::East {
                overlay_sky(fb, &sky[1], ROW_Y + 5 - hour, 9);
            } else if facing == Facing::South && hour > 2 {
                overlay_sky(fb, &sky[1], ROW_Y + 5 - hour, COL_X + hour - 3);
            }
        } else if (13..=18).contains(&hour) {
            if facing == Facing::West {
                overlay_sky(fb, &sky[1], ROW_Y + hour - 13, COL_X);
            } else if facing == Facing::South && hour >= 16 {
                overlay_sky(fb, &sky[1], ROW_Y + hour - 13, COL_X + hour - 8);
            }
        }
        if facing == Facing::North {
            overlay_sky(fb, &sky[0], ROW_Y, COL_X);
        }
    }

    overlay_sky(fb, &sky[2], 7, 2); // horizon backdrop, unconditional
    let _ = symbols; // reserved: wall pieces overlay the same region next
}

/// `seg040.OverlayBounded` (`seg040.cs:29-32`): a direct blit of `block`'s
/// first item (SKY blocks are single-item), landing at cell `(row+1, col+1)`
/// — confirmed by the horizon backdrop's literal call args `(7, 2)` landing
/// at the design doc's stated cell row 8.
///
/// It forwards to `draw_combat_picture` (`seg040.cs:115-118`), whose clip is
/// the 8..176 px box, NOT the whole canvas — [`crate::draw::Clip::OVERLAY`].
/// (`color_no_draw` is 17, an impossible EGA colour, everywhere outside
/// `DrawAreaMap`'s own save/restore pair at `ovr031.cs:74-88`, so these
/// blits are opaque: no transparent index.)
fn overlay_sky(fb: &mut Framebuffer, block: &ImageBlock, row: i32, col: i32) {
    let Some(item) = block.items.first() else {
        return;
    };
    if row < -1 || col < -1 {
        return; // would land at a negative cell — nothing to clip against
    }
    crate::draw::blit_image(
        fb,
        &item.pixels,
        block.width_px(),
        block.height as usize,
        (row + 1) as usize,
        (col + 1) as usize,
        crate::draw::Clip::OVERLAY,
        None,
        None,
    );
}

/// `Draw3dWorldFar` (`ovr031.cs:373-520`): two center-outward front sweeps
/// (class A, with the J-filler run tracking) plus two 3-cell side sweeps
/// (classes B/C). The front sweeps are transcribed from this session's
/// full line-level research; **the side sweeps' exact per-iteration column
/// formula was summarized, not quoted verbatim, by that research** (flagged
/// there as needing closer verification) — this reconstructs them from the
/// design doc's shape description (3 cells per half, starting one cell out
/// from the reference cell) rather than an independently confirmed literal,
/// docketed for a closer `ovr031.cs:463-520` re-read.
#[allow(clippy::too_many_arguments)]
fn draw_far(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    geo: &GeoBlock,
    ecl_block_id: u8,
    party_dir: Facing,
    dir_left: Facing,
    dir_right: Facing,
    ref_x: i32,
    ref_y: i32,
) {
    far_front_sweep(
        fb,
        symbols,
        geo,
        ecl_block_id,
        party_dir,
        dir_left,
        dir_right,
        ref_x,
        ref_y,
        -2,
        1,
    );
    far_front_sweep(
        fb,
        symbols,
        geo,
        ecl_block_id,
        party_dir,
        dir_right,
        dir_left,
        ref_x,
        ref_y,
        2,
        -1,
    );
    far_side_sweep(
        fb,
        symbols,
        geo,
        ecl_block_id,
        dir_left,
        ref_x,
        ref_y,
        CLASS_B,
        ROW_B,
        COL_B,
        -1,
    );
    far_side_sweep(
        fb,
        symbols,
        geo,
        ecl_block_id,
        dir_right,
        ref_x,
        ref_y,
        CLASS_C,
        ROW_C,
        COL_C,
        1,
    );
}

/// One of `Draw3dWorldFar`'s two front (class A) sweeps, tracking the
/// previous front's type (`var_17`) for the class-J filler
/// (`ovr031.cs:381-462`, both directions confirmed line-by-line this
/// session). `col_step`/`j_col_offset` distinguish the two mirrored
/// directions (`-2`/`+1` sweeping `dir_left`, `+2`/`-1` sweeping
/// `dir_right`).
#[allow(clippy::too_many_arguments)]
fn far_front_sweep(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    geo: &GeoBlock,
    ecl_block_id: u8,
    party_dir: Facing,
    sweep_dir: Facing,
    opposite_dir: Facing,
    ref_x: i32,
    ref_y: i32,
    col_step: i32,
    j_col_offset: i32,
) {
    let (sdx, sdy) = sweep_dir.delta();
    let mut tmp_x = ref_x;
    let mut tmp_y = ref_y;
    let mut prev_type: u8 = 0;
    let mut col = 0;
    for _ in 0..4 {
        if !map_coord_is_valid(tmp_x, tmp_y)
            && get_wall_type(geo, ecl_block_id, opposite_dir, tmp_x, tmp_y) == 0
        {
            prev_type = 0;
        }
        let front_type = get_wall_type(geo, ecl_block_id, party_dir, tmp_x, tmp_y);
        if front_type != 0 {
            if prev_type > 0 {
                draw_3d_8x8_titles(
                    fb,
                    symbols,
                    CLASS_J,
                    prev_type,
                    ROW_J,
                    COL_J + col + j_col_offset,
                );
            }
            prev_type = front_type;
            draw_3d_8x8_titles(fb, symbols, CLASS_A, front_type, ROW_A, COL_A + col);
        } else {
            if prev_type > 0 {
                let px = tmp_x - sdx;
                let py = tmp_y - sdy;
                if get_wall_type(geo, ecl_block_id, sweep_dir, px, py) != 0 {
                    draw_3d_8x8_titles(
                        fb,
                        symbols,
                        CLASS_J,
                        prev_type,
                        ROW_J,
                        COL_J + col + j_col_offset,
                    );
                }
            }
            prev_type = 0;
        }
        col += col_step;
        tmp_x += sdx;
        tmp_y += sdy;
    }
}

/// One of `Draw3dWorldFar`'s two side (class B/C) sweeps: no run tracking,
/// no J filler (`ovr031.cs:463-520`, reconstructed shape — see
/// [`draw_far`]'s doc comment).
#[allow(clippy::too_many_arguments)]
fn far_side_sweep(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    geo: &GeoBlock,
    ecl_block_id: u8,
    sweep_dir: Facing,
    ref_x: i32,
    ref_y: i32,
    class: usize,
    row: i32,
    col_anchor: i32,
    col_step: i32,
) {
    let (dx, dy) = sweep_dir.delta();
    let mut tmp_x = ref_x + dx;
    let mut tmp_y = ref_y + dy;
    let mut col = 0;
    for _ in 0..3 {
        let wall_type = get_wall_type(geo, ecl_block_id, sweep_dir, tmp_x, tmp_y);
        if wall_type != 0 {
            draw_3d_8x8_titles(fb, symbols, class, wall_type, row, col_anchor + col);
        }
        col += col_step;
        tmp_x += dx;
        tmp_y += dy;
    }
}

/// `Draw3dWorldMid` (`ovr031.cs:523-577`): two 3-iteration sweeps, each
/// drawing a front (class D, shared by both) and one side (E/F). Starts 2
/// cells out from the reference cell, steps inward.
#[allow(clippy::too_many_arguments)]
fn draw_mid(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    geo: &GeoBlock,
    ecl_block_id: u8,
    party_dir: Facing,
    dir_left: Facing,
    dir_right: Facing,
    ref_x: i32,
    ref_y: i32,
) {
    mid_sweep(
        fb,
        symbols,
        geo,
        ecl_block_id,
        party_dir,
        dir_left,
        dir_right,
        ref_x,
        ref_y,
        CLASS_E,
        ROW_E,
        COL_E,
        -6,
        3,
    );
    mid_sweep(
        fb,
        symbols,
        geo,
        ecl_block_id,
        party_dir,
        dir_right,
        dir_left,
        ref_x,
        ref_y,
        CLASS_F,
        ROW_F,
        COL_F,
        6,
        -3,
    );
}

#[allow(clippy::too_many_arguments)]
fn mid_sweep(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    geo: &GeoBlock,
    ecl_block_id: u8,
    party_dir: Facing,
    start_dir: Facing,
    step_dir: Facing,
    ref_x: i32,
    ref_y: i32,
    side_class: usize,
    side_row: i32,
    side_col: i32,
    var_12_start: i32,
    var_12_step: i32,
) {
    let (sx, sy) = start_dir.delta();
    let (dx, dy) = step_dir.delta();
    let mut tmp_x = ref_x + 2 * sx;
    let mut tmp_y = ref_y + 2 * sy;
    let mut var_12 = var_12_start;
    for _ in 0..3 {
        let front_type = get_wall_type(geo, ecl_block_id, party_dir, tmp_x, tmp_y);
        if front_type != 0 {
            draw_3d_8x8_titles(fb, symbols, CLASS_D, front_type, ROW_D, COL_D + var_12);
        }
        let side_type = get_wall_type(geo, ecl_block_id, start_dir, tmp_x, tmp_y);
        if side_type != 0 {
            draw_3d_8x8_titles(
                fb,
                symbols,
                side_class,
                side_type,
                side_row,
                side_col + var_12,
            );
        }
        var_12 += var_12_step;
        tmp_x += dx;
        tmp_y += dy;
    }
}

/// `Draw3dWorldNear` (`ovr031.cs:580-640`): two 2-iteration sweeps, each
/// drawing a front (class G, shared) and one side (H/I). Starts 1 cell out,
/// steps inward.
#[allow(clippy::too_many_arguments)]
fn draw_near(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    geo: &GeoBlock,
    ecl_block_id: u8,
    party_dir: Facing,
    dir_left: Facing,
    dir_right: Facing,
    ref_x: i32,
    ref_y: i32,
) {
    near_sweep(
        fb,
        symbols,
        geo,
        ecl_block_id,
        party_dir,
        dir_left,
        dir_right,
        ref_x,
        ref_y,
        CLASS_H,
        ROW_H,
        COL_H,
        -7,
        7,
    );
    near_sweep(
        fb,
        symbols,
        geo,
        ecl_block_id,
        party_dir,
        dir_right,
        dir_left,
        ref_x,
        ref_y,
        CLASS_I,
        ROW_I,
        COL_I,
        7,
        -7,
    );
}

#[allow(clippy::too_many_arguments)]
fn near_sweep(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    geo: &GeoBlock,
    ecl_block_id: u8,
    party_dir: Facing,
    start_dir: Facing,
    step_dir: Facing,
    ref_x: i32,
    ref_y: i32,
    side_class: usize,
    side_row: i32,
    side_col: i32,
    var_12_start: i32,
    var_12_step: i32,
) {
    let (sx, sy) = start_dir.delta();
    let (dx, dy) = step_dir.delta();
    let mut tmp_x = ref_x + sx;
    let mut tmp_y = ref_y + sy;
    let mut var_12 = var_12_start;
    for _ in 0..2 {
        let front_type = get_wall_type(geo, ecl_block_id, party_dir, tmp_x, tmp_y);
        if front_type != 0 {
            draw_3d_8x8_titles(fb, symbols, CLASS_G, front_type, ROW_G, COL_G + var_12);
        }
        let side_type = get_wall_type(geo, ecl_block_id, start_dir, tmp_x, tmp_y);
        if side_type != 0 {
            draw_3d_8x8_titles(
                fb,
                symbols,
                side_class,
                side_type,
                side_row,
                side_col + var_12,
            );
        }
        var_12 += var_12_step;
        tmp_x += dx;
        tmp_y += dy;
    }
}

/// `DrawAreaMap` (`ovr031.cs:29-90`): an 11×11 window over the 16×16 grid,
/// offset clamped `0..=5` on each axis independently; per-cell symbol
/// `0x104` + N/E/S/W wall-presence bits (`+1/+2/+4/+8`); the party arrow
/// `0x100 + facing/2` drawn with no-draw color 8 so the underlying cell
/// symbol shows through the arrow's background pixels.
fn draw_area_map(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    geo: &GeoBlock,
    pos: (u8, u8),
    facing: Facing,
) {
    let offset_x = (pos.0 as i32 - 5).clamp(0, 5);
    let offset_y = (pos.1 as i32 - 5).clamp(0, 5);

    for y in 0..11 {
        for x in 0..11 {
            let gx = (offset_x + x) as usize;
            let gy = (offset_y + y) as usize;
            let sq = geo.square(gx, gy);
            let mut symbol_id: u32 = 0x104;
            if sq.wall_north > 0 {
                symbol_id += 1;
            }
            if sq.wall_east > 0 {
                symbol_id += 2;
            }
            if sq.wall_south > 0 {
                symbol_id += 4;
            }
            if sq.wall_west > 0 {
                symbol_id += 8;
            }
            let _ = draw_symbol(fb, symbols, symbol_id, (y + 3) as usize, (x + 3) as usize);
        }
    }

    let arrow_row = (pos.1 as i32 - offset_y) + 3;
    let arrow_col = (pos.0 as i32 - offset_x) + 3;
    let arrow_symbol = 0x100 + (facing.raw_code() as u32 / 2);
    if arrow_row >= 0 && arrow_col >= 0 {
        let _ = draw_symbol_no_draw(
            fb,
            symbols,
            arrow_symbol,
            arrow_row as usize,
            arrow_col as usize,
            8,
        );
    }
}

/// `Draw3dWorld` (`ovr031.cs:321-370`): the area map, or the corridor —
/// background then far→mid→near, the reference cell starting 2 cells ahead
/// of the party and stepping toward the party (`dir_behind`) once per
/// depth slot.
#[allow(clippy::too_many_arguments)]
fn draw_3d_world(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    sky: &[ImageBlock; 3],
    geo: &GeoBlock,
    pos: (u8, u8),
    facing: Facing,
    ecl_block_id: u8,
    hour: u8,
    area_map_shown: bool,
    sky_colour: u8,
) {
    if area_map_shown {
        draw_area_map(fb, symbols, geo, pos, facing);
        return;
    }

    draw_3d_world_background(fb, symbols, sky, geo, pos, facing, hour, sky_colour);

    let dir_left = offset_dir(facing, 6);
    let dir_right = offset_dir(facing, 2);
    let dir_behind = offset_dir(facing, 4);
    let (fx, fy) = facing.delta();
    let (bx, by) = dir_behind.delta();
    let mut ref_x = pos.0 as i32 + 2 * fx;
    let mut ref_y = pos.1 as i32 + 2 * fy;

    for step in (0..=2).rev() {
        match step {
            2 => draw_far(
                fb,
                symbols,
                geo,
                ecl_block_id,
                facing,
                dir_left,
                dir_right,
                ref_x,
                ref_y,
            ),
            1 => draw_mid(
                fb,
                symbols,
                geo,
                ecl_block_id,
                facing,
                dir_left,
                dir_right,
                ref_x,
                ref_y,
            ),
            0 => draw_near(
                fb,
                symbols,
                geo,
                ecl_block_id,
                facing,
                dir_left,
                dir_right,
                ref_x,
                ref_y,
            ),
            _ => unreachable!(),
        }
        ref_x += bx;
        ref_y += by;
    }
}

/// ★ **`RedrawView`** (`ovr029.cs:10-49`) — whole, both arms
/// (roll-credits D-S7a; M2 shipped the `inDungeon != 0` half only).
///
/// The shape, statement for statement:
///
/// ```text
/// :12  if lastDaxBlockId == 0x50   -> can_draw_bigpic = false
/// :17  if party_killed == false
/// :19    if area_ptr.inDungeon != 0
/// :21      mapWallRoof = get_wall_x2(partyY, partyX)
/// :23-31   sky_colour = sky_colours[indoor ? indoor_sky_colour : outdoor_sky_colour]
/// :34-38   if block_area_view != 0 && !Cheats.always_show_areamap -> mapAreaDisplay = false
/// :40      Draw3dWorld(...)
/// :42    else if can_draw_bigpic  -> draw_bigpic()
/// :47    can_draw_bigpic = false
/// ```
///
/// Three details the M2 slice could not carry, and one that is a correction:
///
/// - The fork is on the **RAW `area_ptr.inDungeon` cell**, not `game_state`
///   (FD-37's asymmetry — `vm_init_ecl` writes the struct field directly,
///   `ovr008.cs:126`, bypassing the hook that maintains `game_state`).
/// - `can_draw_bigpic = false` at `:47` is INSIDE the `party_killed` gate;
///   the `lastDaxBlockId` force-clear at `:12-15` is outside it. Both
///   transcribed where they sit.
/// - **`lastDaxBlockId == 0x50` is not a demo guard** (the door's working
///   name for it): `0x50` is the block `load_pic_final` last loaded, and the
///   only script that loads it is `ECL1#80`'s own city preamble
///   (`@0x85E5 PICTURE 0x50`, the "YOU ARE IN <city>. WHAT PLACE WILL YOU
///   VISIT?" scene). The guard means "a city scene owns the viewport" — while
///   it does, the overland map must not be repainted under it. The `Shop`
///   arm of `LoadPic` reads the same cell the same way (`ovr025.cs:1414`).
/// - FD-41's unread cell is read here: `block_area_view` (`0x4BFB`) is a live
///   ScriptMemory cell a block can `SAVE` into, so it is sampled at redraw
///   time rather than copied at boot.
///
/// Called at every point the design doc's walk loop calls the original's own
/// `RedrawView` (`shell.rs`'s `enter_world_menu` — see that function's doc
/// comment for the deliberate call-site simplification).
pub fn redraw_view(ctx: &mut FlowCtx) {
    // `:12-15` — outside the `party_killed` gate, and the first thing the
    // function does.
    if ctx.state.picture.last_dax_block == crate::picture::CITY_SCENE_PIC_BLOCK {
        ctx.vm_memory.can_draw_bigpic = false;
    }

    if ctx.state.party_killed {
        // `:17` — a wiped party gets no view at all, and `:47`'s consume is
        // skipped with it.
        ctx.vm_memory.clear_redraw_flags();
        return;
    }

    if ctx.vm_memory.in_dungeon() {
        draw_dungeon_view(ctx);
    } else if ctx.vm_memory.can_draw_bigpic {
        // `:44 ovr030.draw_bigpic()` — the full-screen overland backdrop,
        // whichever BIGPIC block a `PICTURE >= 0x78` last selected.
        draw_bigpic(ctx);
    }

    // `RedrawView`'s own tail (`ovr029.cs:47`): the bigpic permission is
    // one-shot, consumed by every redraw whether or not the non-dungeon
    // branch used it.
    ctx.vm_memory.can_draw_bigpic = false;

    // The consolidated gate's own clear (`ovr003.cs:1855-1859`) — see
    // `VmMemoryState::clear_redraw_flags`'s doc comment for why this
    // session's redraw isn't itself gated on these flags.
    ctx.vm_memory.clear_redraw_flags();
}

/// `RedrawView`'s `inDungeon != 0` arm (`ovr029.cs:21-40`): the sky colour
/// from the party cell's `indoor` flag (`mapWallRoof > 0x7F`, exactly
/// equivalent to `x2 & 0x80` for a `u8`), FD-41's area-map force-clear, then
/// [`draw_3d_world`].
fn draw_dungeon_view(ctx: &mut FlowCtx) {
    // The 3D world is painted over the very cells a picture occupies (a PIC
    // is 88x88 at cell (3,3), exactly the viewport), so a redraw destroys it
    // — in the original too, which is why every picture-bearing vector in the
    // real scripts ends with `PICTURE 0xFF` and/or the `CALL 0xAE11` redraw
    // gate. `crate::picture`'s module doc carries the evidence; the layer is
    // what makes the destruction *state* rather than just pixels, so a save
    // taken mid-scene restores the picture (D-SAVE3).
    ctx.state.picture.hide();

    let square = ctx
        .geo
        .square(ctx.state.pos.0 as usize, ctx.state.pos.1 as usize);
    let sky_idx_addr = if square.indoor {
        INDOOR_SKY_COLOUR_ADDR
    } else {
        OUTDOOR_SKY_COLOUR_ADDR
    };
    let sky_idx = ctx.vm_memory.raw_word(sky_idx_addr).unwrap_or(0) as usize % 16;
    let sky_colour = SKY_COLOURS[sky_idx];
    let hour = ctx.state.clock.hh_mm().0;

    // `:34-38` — FD-41. `Cheats.always_show_areamap` is coab's own debug
    // switch, permanently false here.
    if ctx.vm_memory.block_area_view() != 0 {
        ctx.state.area_map_shown = false;
    }

    draw_3d_world(
        ctx.fb,
        ctx.symbols,
        ctx.sky,
        ctx.geo,
        ctx.state.pos,
        ctx.state.facing,
        ctx.state.ecl_block_id,
        hour,
        ctx.state.area_map_shown,
        sky_colour,
    );
}

/// `draw_bigpic` (`ovr030.cs:243-247`) — `DrawFrame_WildernessMap()` then the
/// resident `bigpic_dax` at cell (1,1). Both live in
/// [`crate::picture::compose`]'s [`Shown::BigPic`] arm already (the
/// `PICTURE >= 0x78` path draws through exactly the same code), so this is
/// the same paint reached from the redraw side.
///
/// The block id is the one `load_bigpic` last resolved. With none resident
/// the original would blit a null `bigpic_dax`; ours simply draws the frame,
/// which is the honest empty overland screen rather than a crash.
///
/// [`Shown::BigPic`]: crate::picture::Shown::BigPic
fn draw_bigpic(ctx: &mut FlowCtx) {
    ctx.state.picture.shown = crate::picture::Shown::BigPic;
    crate::picture::compose(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framebuffer::Framebuffer;
    use crate::symbols::WallsetSlot;
    use gbx_formats::geo::GEO_BLOCK_SIZE;
    use gbx_formats::image::DecodedItem;
    use gbx_formats::walldef::{STYLES_PER_WALLSET, TILE_IDS_PER_STYLE, WALLSET_SIZE};

    fn open_geo() -> GeoBlock {
        GeoBlock::parse(&vec![0u8; GEO_BLOCK_SIZE]).unwrap()
    }

    #[test]
    fn offset_dir_matches_facing_turn_helpers() {
        assert_eq!(offset_dir(Facing::North, 6), Facing::North.turn_left());
        assert_eq!(offset_dir(Facing::North, 2), Facing::North.turn_right());
        assert_eq!(offset_dir(Facing::North, 4), Facing::North.turn_around());
        assert_eq!(offset_dir(Facing::East, 6), Facing::East.turn_left());
    }

    #[test]
    fn map_coord_is_valid_never_checks_y_geq_zero() {
        // The confirmed bug: a negative Y with X in range is still "valid".
        assert!(map_coord_is_valid(5, -1));
        // But a negative X is correctly caught (checked twice).
        assert!(!map_coord_is_valid(-1, 5));
        assert!(!map_coord_is_valid(16, 5));
        assert!(!map_coord_is_valid(5, 16));
    }

    #[test]
    fn wrap_coord_snaps_to_opposite_bound_not_modulo() {
        assert_eq!(wrap_coord(16), 0);
        assert_eq!(wrap_coord(17), 0, "snap, not modulo (17 % 16 == 1)");
        assert_eq!(wrap_coord(-1), 15);
        assert_eq!(wrap_coord(5), 5);
    }

    #[test]
    fn get_wall_type_blocks_0_and_10_return_nothing_on_invalid_coords() {
        let geo = open_geo();
        assert_eq!(get_wall_type(&geo, 0, Facing::North, 5, -1), 0);
        assert_eq!(get_wall_type(&geo, 10, Facing::North, 5, -1), 0);
    }

    #[test]
    fn get_wall_type_wraps_for_other_blocks() {
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        // Square (5,15): North wall type 3.
        data[2 + 5 + 16 * 15] = 3 << 4;
        let geo = GeoBlock::parse(&data).unwrap();
        // y = -1 wraps to 15 for any ecl_block_id other than 0/10.
        assert_eq!(get_wall_type(&geo, 1, Facing::North, 5, -1), 3);
    }

    #[test]
    fn draw_3d_8x8_titles_skips_symbol_id_zero_without_erroring() {
        let mut fb = Framebuffer::new();
        let symbols = SymbolSets::new(); // no wallset loaded: every lookup is a no-op
        draw_3d_8x8_titles(&mut fb, &symbols, CLASS_A, 1, ROW_A, COL_A);
        // Nothing loaded => nothing drawn => framebuffer stays untouched.
        assert_eq!(
            fb.pixels(),
            &[0u8; crate::framebuffer::WIDTH * crate::framebuffer::HEIGHT]
        );
    }

    #[test]
    fn draw_area_map_offset_clamps_to_0_5() {
        // Party at the grid's far corner: offset must clamp to 5, not 10.
        let geo = open_geo();
        let symbols = SymbolSets::new();
        let mut fb = Framebuffer::new();
        // Should not panic even at an extreme position.
        draw_area_map(&mut fb, &symbols, &geo, (15, 15), Facing::North);
        draw_area_map(&mut fb, &symbols, &geo, (0, 0), Facing::North);
    }

    // --- Rendering fixtures (D10: hand-authored, never real game data) ---

    /// A wallset (LOAD PIECES set 1) where wall type `1..=5` (slice `0..4`)
    /// draws as a solid block of `colors[slice]` at every 8×8 position in
    /// its class window — every tile id in the table is
    /// `SYMBOL_SET_FIX[1] + slice`, landing in set 1 at index `slice`.
    fn colored_wallset(colors: [u8; 5]) -> SymbolSets {
        let mut symbols = SymbolSets::new();
        let block = ImageBlock {
            height: 8,
            width_cols: 1,
            x_pos: 0,
            y_pos: 0,
            field_9: [0; 8],
            items: colors
                .iter()
                .map(|&c| DecodedItem {
                    pixels: vec![c; 64],
                })
                .collect(),
        };
        symbols.load(1, block);
        let mut tiles = [0u8; WALLSET_SIZE];
        for s in 0..STYLES_PER_WALLSET {
            let tile_id = (crate::symbols::SYMBOL_SET_FIX[1] as usize + s) as u8;
            for i in 0..TILE_IDS_PER_STYLE {
                tiles[s * TILE_IDS_PER_STYLE + i] = tile_id;
            }
        }
        symbols.load_wallset(0, WallsetSlot::from_tiles(tiles));
        symbols
    }

    /// The pixel at a viewport cell's top-left corner (`local_row`/`col` in
    /// the `0..=10` pre-offset space `draw_3D_8x8_titles` guards, matching
    /// this module's class row/col anchor constants).
    fn cell_pixel(fb: &Framebuffer, local_row: i32, local_col: i32) -> u8 {
        fb.get_pixel(
            ((local_col + 3) * 8) as usize,
            ((local_row + 3) * 8) as usize,
        )
    }

    const NORTH: Facing = Facing::North;

    /// Party at `(8,8)` facing North: far ref `(8,6)`, mid ref `(8,7)`,
    /// near ref = the party's own cell `(8,8)` — every depth slot's front
    /// sweep converges on this single column for its center/last iteration
    /// (this module's own doc comments on `draw_far`/`draw_mid`/
    /// `draw_near`), the simplest cell to target for a front-wall fixture.
    const PARTY: (u8, u8) = (8, 8);

    fn geo_with_wall(x: usize, y: usize, wall_type: u8, edge: Facing) -> GeoBlock {
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        let idx = 2 + x + 16 * y;
        match edge {
            Facing::North => data[idx] |= wall_type << 4,
            Facing::East => data[idx] |= wall_type,
            Facing::South => data[idx + 256] |= wall_type << 4,
            Facing::West => data[idx + 256] |= wall_type,
        }
        GeoBlock::parse(&data).unwrap()
    }

    #[test]
    fn empty_corridor_draws_only_background_no_wall_pieces() {
        let geo = open_geo();
        let symbols = colored_wallset([1, 2, 3, 4, 5]);
        let mut fb = Framebuffer::new();
        draw_3d_world(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            1,
            12,
            false,
            0,
        );
        // No colors 1-5 anywhere: nothing from the wallset drew.
        for c in 1u8..=5 {
            assert!(
                !fb.pixels().contains(&c),
                "color {c} must not appear with an all-open GEO block"
            );
        }
    }

    #[test]
    fn far_front_wall_draws_class_a_at_the_center_column() {
        let geo = geo_with_wall(8, 6, 1, NORTH); // far ref (8,6), North edge
        let symbols = colored_wallset([9, 0, 0, 0, 0]);
        let mut fb = Framebuffer::new();
        draw_3d_world(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            1,
            12,
            false,
            0,
        );
        assert_eq!(cell_pixel(&fb, ROW_A, COL_A), 9);
    }

    #[test]
    fn mid_front_wall_draws_class_d_at_the_center_column() {
        let geo = geo_with_wall(8, 7, 2, NORTH); // mid ref (8,7)
        let symbols = colored_wallset([0, 9, 0, 0, 0]);
        let mut fb = Framebuffer::new();
        draw_3d_world(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            1,
            12,
            false,
            0,
        );
        assert_eq!(cell_pixel(&fb, ROW_D, COL_D), 9);
    }

    #[test]
    fn near_front_wall_draws_class_g_at_the_center_column() {
        let geo = geo_with_wall(8, 8, 3, NORTH); // near ref = party's own cell
        let symbols = colored_wallset([0, 0, 9, 0, 0]);
        let mut fb = Framebuffer::new();
        draw_3d_world(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            1,
            12,
            false,
            0,
        );
        assert_eq!(cell_pixel(&fb, ROW_G, COL_G), 9);
    }

    #[test]
    fn far_side_walls_draw_classes_b_and_c() {
        // far_side_sweep starts one cell out from the far ref (8,6) along
        // dir_left (West) / dir_right (East) and probes that direction's
        // *own* edge — West edge of (7,6) for B, East edge of (9,6) for C.
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        data[2 + 7 + 16 * 6 + 256] |= 1; // (7,6) West wall, type 1
        data[2 + 9 + 16 * 6] |= 2; // (9,6) East wall, type 2
        let geo = GeoBlock::parse(&data).unwrap();
        let symbols = colored_wallset([9, 8, 0, 0, 0]);
        let mut fb = Framebuffer::new();
        draw_3d_world(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            1,
            12,
            false,
            0,
        );
        assert_eq!(cell_pixel(&fb, ROW_B, COL_B), 9);
        assert_eq!(cell_pixel(&fb, ROW_C, COL_C), 8);
    }

    #[test]
    fn j_filler_gap_textures_from_the_previous_fronts_type_not_the_current() {
        // Two far fronts of *different* types, one column apart on the
        // dir_left sweep (iteration 0 = center (8,6) type 1, iteration 1 =
        // (7,6) type 2) — case (a), "new front while var_17 > 0": J must
        // texture from type 1 (the previous front), not type 2.
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        data[2 + 8 + 16 * 6] |= 1 << 4; // center: North wall type 1
        data[2 + 7 + 16 * 6] |= 2 << 4; // one step West: North wall type 2
        let geo = GeoBlock::parse(&data).unwrap();
        let symbols = colored_wallset([9, 8, 0, 0, 0]);
        let mut fb = Framebuffer::new();
        draw_3d_world(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            1,
            12,
            false,
            0,
        );
        // The gap filler sits at Column_J + col + j_col_offset, where `col`
        // is *this* (2nd) iteration's value: `col` starts 0 and steps by
        // `col_step` (-2) *after* iteration 0's draw, so iteration 1 (where
        // the new front is found) sees `col == -2`; `j_col_offset` is +1.
        assert_eq!(
            cell_pixel(&fb, ROW_J, COL_J - 1),
            9,
            "the far filler must use the previous front's color (9), not the new one (8)"
        );
    }

    #[test]
    fn j_filler_end_cap_textures_from_the_previous_fronts_type() {
        // A far front at the center only, with the dir_left side wall of
        // the *previous* (center) cell present at the next probe — case
        // (b), the end-cap: front ends but the sweep-side wall continues.
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        data[2 + 8 + 16 * 6] |= 1 << 4; // center: North wall type 1 (the front)
                                        // (8,6)'s own West edge continues past the front's end.
        data[2 + 8 + 16 * 6 + 256] |= 3; // West wall type 3 at the center cell
        let geo = GeoBlock::parse(&data).unwrap();
        let symbols = colored_wallset([9, 0, 8, 0, 0]);
        let mut fb = Framebuffer::new();
        draw_3d_world(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            1,
            12,
            false,
            0,
        );
        assert_eq!(
            cell_pixel(&fb, ROW_J, COL_J - 1),
            9,
            "the end-cap filler must use the ended front's color (9), never the side wall's (8)"
        );
    }

    #[test]
    fn symbol_id_zero_is_a_hole_leaves_the_cell_undrawn() {
        // A single (style, idx) cell set to tile id 0 inside an otherwise
        // solid class G window: that one 8x8 position must stay
        // background while its neighbors draw.
        let geo = geo_with_wall(8, 8, 3, NORTH); // near, type 3 => slice 2
        let mut symbols = colored_wallset([0, 0, 9, 0, 0]);
        let mut tiles = [0u8; WALLSET_SIZE];
        for i in 0..TILE_IDS_PER_STYLE {
            tiles[2 * TILE_IDS_PER_STYLE + i] =
                (crate::symbols::SYMBOL_SET_FIX[1] as usize + 2) as u8;
        }
        tiles[2 * TILE_IDS_PER_STYLE + IDX_OFFSET[CLASS_G]] = 0; // the class G window's first cell
        symbols.load_wallset(0, WallsetSlot::from_tiles(tiles));
        let mut fb = Framebuffer::new();
        draw_3d_world(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            1,
            12,
            false,
            0,
        );
        assert_ne!(
            cell_pixel(&fb, ROW_G, COL_G),
            9,
            "symbol id 0 must be a hole, not the wall color"
        );
        // A neighboring cell in the same class window must still draw.
        assert_eq!(cell_pixel(&fb, ROW_G, COL_G + 1), 9);
    }

    #[test]
    fn area_map_draws_wall_bits_and_the_party_arrow_over_them() {
        let geo = geo_with_wall(8, 8, 1, NORTH);
        let mut symbols = SymbolSets::new();
        // Area-map symbols (`0x100..=0x127`) route to set 4 (`resolve_symbol`),
        // indexed by `symbol_id - SYMBOL_SET_FIX[4]` (`0x100`). item[4] =
        // "north wall present" (`0x104+1=0x105`) color 7; item[0] = the
        // North-facing party arrow (`0x100+facing/2=0x100` for North),
        // color 6 except one no-draw(8)-colored pixel that should let the
        // cell color show through.
        let mut items = vec![
            DecodedItem {
                pixels: vec![3; 64]
            };
            0x28
        ];
        items[0x105 - 0x100] = DecodedItem {
            pixels: vec![7; 64],
        };
        let mut arrow_pixels = vec![6u8; 64];
        arrow_pixels[0] = 8; // no-draw color: must reveal the cell underneath
        items[0x100 - 0x100] = DecodedItem {
            pixels: arrow_pixels,
        };
        symbols.load(
            4,
            ImageBlock {
                height: 8,
                width_cols: 1,
                x_pos: 0,
                y_pos: 0,
                field_9: [0; 8],
                items,
            },
        );
        let mut fb = Framebuffer::new();
        draw_3d_world(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            1,
            12,
            true,
            0,
        );
        // The party's own cell (North wall present) draws color 7 first...
        let offset_x = (PARTY.0 as i32 - 5).clamp(0, 5);
        let offset_y = (PARTY.1 as i32 - 5).clamp(0, 5);
        let arrow_row = (PARTY.1 as i32 - offset_y) + 3;
        let arrow_col = (PARTY.0 as i32 - offset_x) + 3;
        // ...then the arrow overlays it, except its no-draw(8) pixel which
        // must show the underlying color-7 cell through.
        assert_eq!(
            fb.get_pixel((arrow_col * 8) as usize, (arrow_row * 8) as usize),
            7,
            "the arrow's no-draw-8 pixel must reveal the cell symbol underneath"
        );
        assert_eq!(
            fb.get_pixel((arrow_col * 8 + 1) as usize, (arrow_row * 8) as usize),
            6,
            "the arrow's other pixels must draw normally"
        );
    }

    #[test]
    fn indoor_vs_outdoor_sky_colour_uses_the_hypothesized_area_window_cells() {
        // A direct check of draw_3d_world_background's sky fill color,
        // independent of the ScriptMemory cell lookup (that seam is
        // exercised at the `redraw_view` level, not unit-tested here since
        // it's a raw_word passthrough already covered by vmhost.rs's own
        // tests).
        let geo = open_geo();
        let symbols = SymbolSets::new();
        let mut fb = Framebuffer::new();
        draw_3d_world_background(
            &mut fb,
            &symbols,
            &dummy_sky(),
            &geo,
            PARTY,
            NORTH,
            12,
            0x0B,
        );
        // Sky fill pixel (well inside the 24-67 band, away from the
        // horizon backdrop's 8x8 footprint at cell row 8).
        assert_eq!(fb.get_pixel(80, 30), 0x0B);
    }

    // --- FD-31 follow-through: the sun/moon overlay, against REAL art ---

    /// Writes a `.ppm` beside the other demo dumps so the overlay can be
    /// eyeballed, not just asserted.
    fn dump(fb: &Framebuffer, name: &str) {
        let path = std::env::var_os("RESTRIKE_DEMO_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("restrike-sky-{name}.ppm"));
        let mut out = format!(
            "P6\n{} {}\n255\n",
            crate::framebuffer::WIDTH,
            crate::framebuffer::HEIGHT
        )
        .into_bytes();
        for y in 0..crate::framebuffer::HEIGHT {
            for x in 0..crate::framebuffer::WIDTH {
                out.extend_from_slice(&fb.palette()[fb.get_pixel(x, y) as usize]);
            }
        }
        let _ = std::fs::write(&path, &out);
        eprintln!("sky overlay frame -> {}", path.display());
    }

    /// The horizon backdrop's own footprint: `OverlayBounded(sky_dax_252, 1,
    /// 0, 7, 2)` (`ovr031.cs:136`) is UNCONDITIONAL and runs LAST, so it
    /// paints over any sun/moon that landed in cells `(8,3)`..`(13,13)` —
    /// which is exactly how the sun sets: as the hour advances its row walks
    /// down until the backdrop swallows it whole.
    const HORIZON: (usize, usize, usize, usize) = (3 * 8, 8 * 8, 3 * 8 + 88, 8 * 8 + 48);

    /// Asserts the two renders differ ONLY inside the 8×8-cell rect
    /// `(cell_row, cell_col)` + `(w, h)` pixels — and that they differ *at
    /// all* unless that rect lies entirely under [`HORIZON`]. (Containment,
    /// not an exact bbox match: art whose edge pixels happen to equal the
    /// sky fill leaves the box a pixel or two narrow.)
    fn assert_blit_at(
        base: &Framebuffer,
        drawn: &Framebuffer,
        cell_row: usize,
        cell_col: usize,
        size: (usize, usize),
        what: &str,
    ) {
        let (w, h) = size;
        let (x0, y0) = (cell_col * 8, cell_row * 8);
        let behind_horizon =
            x0 >= HORIZON.0 && y0 >= HORIZON.1 && x0 + w <= HORIZON.2 && y0 + h <= HORIZON.3;
        match diff_box(base, drawn) {
            None => assert!(
                behind_horizon,
                "{what}: nothing drew, and it was not hidden behind the horizon backdrop"
            ),
            Some(bb) => {
                assert!(
                    !behind_horizon,
                    "{what}: drew at {bb:?} even though the horizon backdrop covers it"
                );
                assert!(
                    bb.0 >= x0 && bb.1 >= y0 && bb.2 <= x0 + w && bb.3 <= y0 + h,
                    "{what}: drew at {bb:?}, outside the expected cell \
                     ({cell_row},{cell_col}) rect ({x0},{y0})..({},{})",
                    x0 + w,
                    y0 + h
                );
            }
        }
    }

    /// The pixel bounding box where two renders differ, as
    /// `(x0, y0, x1_exclusive, y1_exclusive)`.
    fn diff_box(a: &Framebuffer, b: &Framebuffer) -> Option<(usize, usize, usize, usize)> {
        let mut bb: Option<(usize, usize, usize, usize)> = None;
        for y in 0..crate::framebuffer::HEIGHT {
            for x in 0..crate::framebuffer::WIDTH {
                if a.get_pixel(x, y) != b.get_pixel(x, y) {
                    bb = Some(match bb {
                        None => (x, y, x + 1, y + 1),
                        Some((x0, y0, x1, y1)) => {
                            (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1))
                        }
                    });
                }
            }
        }
        bb
    }

    /// ★ FD-31 follow-through (local tier, needs `GBX_DATA_DIR`): the
    /// `Draw3dWorldBackground` sun/moon overlay (`ovr031.cs:99-135`) has been
    /// dead since M2 — it only runs when `sky_colour == 11`, which nothing
    /// could produce until FD-31 corrected the Area window's halved address
    /// mapping. This is its first exercise against the real `SKY.DAX` art.
    ///
    /// Every landing cell is checked against `ovr031`'s own arithmetic:
    /// `OverlayBounded(block, 1, 0, rowY, colX)` blits at cell
    /// `(rowY + 1, colX + 1)` (`seg040.cs:29-32`), i.e. pixel
    /// `(8*(colX+1), 8*(rowY+1))`.
    #[test]
    fn the_sun_and_moon_overlay_lands_where_ovr031_says() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!(
                "SKIPPED: local tier needs GBX_DATA_DIR \
                 (corridor::the_sun_and_moon_overlay_lands_where_ovr031_says)"
            );
            return;
        };
        let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir))
            .expect("GBX_DATA_DIR must be readable");
        let assets = crate::boot::boot(&data).expect("boot must succeed against real CotAB data");
        let sky = assets.sky;
        // Moon (SKY 250), sun (SKY 251), horizon backdrop (SKY 252).
        let (moon_w, moon_h) = (sky[0].width_px(), sky[0].height as usize);
        let (sun_w, sun_h) = (sky[1].width_px(), sky[1].height as usize);
        eprintln!(
            "SKY art: moon {moon_w}x{moon_h}, sun {sun_w}x{sun_h}, \
             horizon {}x{}",
            sky[2].width_px(),
            sky[2].height
        );

        // An all-zero GEO is outdoor everywhere, so the `get_wall_x2(Y, Y)`
        // guard passes; no wallset is loaded, so nothing but the background
        // and the overlays ever draws.
        let geo = open_geo();
        let symbols = SymbolSets::new();
        let render = |facing: Facing, hour: u8| {
            let mut fb = Framebuffer::new();
            draw_3d_world_background(&mut fb, &symbols, &sky, &geo, PARTY, facing, hour, 0x0B);
            fb
        };

        // The neutral reference: facing East at an hour in neither window,
        // so only the unconditional horizon backdrop draws.
        let none = render(Facing::East, 8);
        dump(&none, "none-east-h8");
        assert!(
            diff_box(&none, &render(Facing::West, 8)).is_none(),
            "hour 8 is outside both windows, and West is not the moon's facing"
        );

        // Moon: facing North, unconditional on hour (`ovr031.cs:130-133` is
        // a SIBLING `if`, not nested under the hour checks) — row 2, col 2.
        let moon = render(Facing::North, 8);
        dump(&moon, "moon-north-h8");
        assert_blit_at(
            &none,
            &moon,
            3,
            3,
            (moon_w, moon_h),
            "the moon at cell (3,3)",
        );

        // Sun, morning window (`:105-116`): hours 1-5 facing East land at
        // row `2 + 5 - hour`, col `12 - 3` = 9 -> cell (8 - hour, 10).
        for hour in 1..=5u8 {
            let sun = render(Facing::East, hour);
            dump(&sun, &format!("sun-east-h{hour}"));
            let row = 2 + 5 - hour as i32;
            assert_blit_at(
                &none,
                &sun,
                (row + 1) as usize,
                10,
                (sun_w, sun_h),
                &format!("morning sun at hour {hour}"),
            );
        }

        // The morning South window is NARROWER: `hour > 2` (`:113`), and its
        // column tracks the hour (`col_x + hour - 3`).
        assert!(
            diff_box(&none, &render(Facing::South, 2)).is_none(),
            "facing South at hour 2 draws no sun (the > 2 guard)"
        );
        let south_dawn = render(Facing::South, 5);
        dump(&south_dawn, "sun-south-h5");
        assert_blit_at(
            &none,
            &south_dawn,
            3,
            5,
            (sun_w, sun_h),
            "hour 5 South: row 2+5-5 = 2, col 2+5-3 = 4 -> cell (3, 5)",
        );

        // Sun, evening window (`:118-127`): hours 13-18 facing West land at
        // row `2 + hour - 13`, col 2; the South half needs `hour >= 16`.
        for hour in 13..=18u8 {
            let sun = render(Facing::West, hour);
            dump(&sun, &format!("sun-west-h{hour}"));
            let row = 2 + hour as i32 - 13;
            assert_blit_at(
                &none,
                &sun,
                (row + 1) as usize,
                3,
                (sun_w, sun_h),
                &format!("evening sun at hour {hour}"),
            );
        }
        assert!(
            diff_box(&none, &render(Facing::South, 15)).is_none(),
            "facing South at hour 15 draws no sun (the >= 16 guard)"
        );
        let south_dusk = render(Facing::South, 18);
        dump(&south_dusk, "sun-south-h18");
        assert_blit_at(
            &none,
            &south_dusk,
            8,
            13,
            (sun_w, sun_h),
            "hour 18 South: row 2+18-13 = 7, col 2+18-8 = 12 -> cell (8, 13)",
        );

        // An indoor square kills the whole block (`get_wall_x2(Y, Y) < 0x80`,
        // `:99` — the confirmed Y-passed-twice bug, replicated).
        let mut indoor_data = vec![0u8; GEO_BLOCK_SIZE];
        // x2's high bit at (PARTY.1, PARTY.1) — the cell the bug queries.
        indoor_data[2 + 2 * 256 + PARTY.1 as usize + 16 * PARTY.1 as usize] = 0x80;
        let indoor = GeoBlock::parse(&indoor_data).unwrap();
        let mut fb = Framebuffer::new();
        draw_3d_world_background(
            &mut fb,
            &symbols,
            &sky,
            &indoor,
            PARTY,
            Facing::North,
            8,
            0x0B,
        );
        assert!(
            diff_box(&none, &fb).is_none(),
            "indoor: no moon, no sun -- only the unconditional backdrop"
        );
    }

    fn dummy_sky() -> [ImageBlock; 3] {
        let block = ImageBlock {
            height: 8,
            width_cols: 1,
            x_pos: 0,
            y_pos: 0,
            field_9: [0; 8],
            items: vec![DecodedItem {
                pixels: vec![0; 64],
            }],
        };
        [block.clone(), block.clone(), block]
    }
}
