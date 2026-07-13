//! Walldef composite builder: for a chosen (wallset, style), pairs the raw
//! tile-id table with the resident 8×8 pixel data the same way
//! `gbx-engine`'s `EngineVmHost::load_walldef` does (rebase included), then
//! resolves each tile id to a decoded item. Pure over already-decoded
//! `gbx-formats` types — no rendering, no `GameData` access (the caller
//! looks up and decodes the paired `8X8D*.DAX` block; this module only
//! knows the pairing rule).
//!
//! Derived from `crates/gbx-engine/src/vmhost.rs`'s `load_walldef` (task
//! deliverable 1, step 5): this view-model assumes the common real-data
//! case documented there — a walldef loaded at base symbol set 1
//! (`LoadWalldef(1, id)`), so wallset `n` (0-based within the block) lands
//! at symbol-set slot `1 + n` and the rebase is computed from that slot,
//! exactly like the engine's own load path.

use gbx_engine::symbols::SYMBOL_SET_FIX;
use gbx_formats::image::{DecodedItem, ImageBlock};
use gbx_formats::walldef::{WalldefBlock, TILE_IDS_PER_STYLE};

use crate::viewmodel::palette::TRANSPARENT;

/// The `8X8D{game_area}.DAX` sub-block id paired with wallset `wallset`
/// (0-based) of a walldef block holding `wallset_count` sub-blocks total —
/// `LoadWalldef`'s own convention (`vmhost.rs:747-751`): the walldef's own
/// block id when there's exactly one wallset, `id*10 + wallset + 1` (1-based
/// `n`) when there are several.
pub fn paired_image_block_id(walldef_block_id: u8, wallset_count: usize, wallset: usize) -> u8 {
    if wallset_count > 1 {
        walldef_block_id
            .wrapping_mul(10)
            .wrapping_add(wallset as u8 + 1)
    } else {
        walldef_block_id
    }
}

/// The symbol-set slot wallset `wallset` (0-based) lands in when the walldef
/// is loaded at base set 1 — `1 + wallset` (sets 1-3 are the three wallset
/// slots, `symbols.rs`'s `WALLSET_SLOT_COUNT`).
pub fn target_symbol_set(wallset: usize) -> usize {
    wallset + 1
}

/// One rebased style: 156 tile ids, `>=0x2D` ids shifted by
/// `SYMBOL_SET_FIX[target_set] - SYMBOL_SET_FIX[1]` (wrapping byte
/// arithmetic) — `load_walldef`'s rebase, computed once per style here
/// (matching the engine, which computes it once per *call*, i.e. once per
/// wallset, and applies it uniformly across all 5 styles).
pub fn rebase_style(
    walldef: &WalldefBlock,
    wallset: usize,
    style: usize,
    target_set: usize,
) -> [u8; TILE_IDS_PER_STYLE] {
    let rebase = (SYMBOL_SET_FIX[target_set] as i32 - SYMBOL_SET_FIX[1] as i32) as u8;
    let mut out = [0u8; TILE_IDS_PER_STYLE];
    for (i, slot) in out.iter_mut().enumerate() {
        let raw = walldef.tile_id(wallset, style, i).unwrap_or(0);
        *slot = if raw >= 0x2D {
            raw.wrapping_add(rebase)
        } else {
            raw
        };
    }
    out
}

/// Resolves one rebased tile id to its pixel item within the paired,
/// already-decoded image sub-block. `None` for a hole (id 0, "draw
/// nothing" per `symbols.rs`'s `draw_3d_8x8_titles`) or an id that doesn't
/// land inside `pixels`' own item range (an unexpected/malformed tile id —
/// the caller renders a hole here too, never guesses).
pub fn resolve_tile(tile_id: u8, target_set: usize, pixels: &ImageBlock) -> Option<&DecodedItem> {
    if tile_id == 0 {
        return None;
    }
    let base = SYMBOL_SET_FIX[target_set];
    let idx = (tile_id as u32).checked_sub(base)?;
    pixels.items.get(idx as usize)
}

/// One composited style: 156 slots, each the resolved pixel item (or `None`
/// for a hole/unresolved id) — the resource browser's tile-grid pane reads
/// this directly, laying it out in a fixed-column grid.
pub fn compose_style<'a>(
    walldef: &WalldefBlock,
    wallset: usize,
    style: usize,
    pixels: &'a ImageBlock,
) -> Vec<Option<&'a DecodedItem>> {
    let target_set = target_symbol_set(wallset);
    let tiles = rebase_style(walldef, wallset, style, target_set);
    tiles
        .iter()
        .map(|&id| resolve_tile(id, target_set, pixels))
        .collect()
}

/// Stitches one [`compose_style`] result (156 tile slots, `cols`-wide) into
/// a single row-major indexed pixel buffer — the "walldef composites" image
/// copy/save target (task brief deliverable 3): one flat image per
/// (wallset, style) instead of 156 separate tile textures. Holes (a `None`
/// slot) render as [`TRANSPARENT`]. Returns `(width_px, height_px, pixels)`.
pub fn stitch_composite(
    composed: &[Option<&DecodedItem>],
    tile_w: usize,
    tile_h: usize,
    cols: usize,
) -> (usize, usize, Vec<u8>) {
    let cols = cols.max(1);
    let rows = composed.len().div_ceil(cols);
    let width = cols * tile_w;
    let height = rows * tile_h;
    let mut pixels = vec![TRANSPARENT; width * height];
    for (i, tile) in composed.iter().enumerate() {
        let Some(item) = tile else { continue };
        let ox = (i % cols) * tile_w;
        let oy = (i / cols) * tile_h;
        for y in 0..tile_h {
            for x in 0..tile_w {
                let src = y * tile_w + x;
                let Some(&px) = item.pixels.get(src) else {
                    continue;
                };
                pixels[(oy + y) * width + ox + x] = px;
            }
        }
    }
    (width, height, pixels)
}

/// Derives the paired `8X8D{game_area}.DAX` file name from a
/// `WALLDEF{game_area}.DAX` file name (case-insensitive; `game_area` is
/// whatever suffix follows the `WALLDEF` prefix, carried verbatim) —
/// mirrors `load_walldef`'s own `format!("8X8D{game_area}.DAX")` file-naming
/// convention (`vmhost.rs`), so the resource browser can look up a
/// walldef's paired pixel file without the engine's `game_area` constant.
/// `None` if `file_name` doesn't match the `WALLDEF*.DAX` convention.
pub fn sym_file_for_walldef_file(file_name: &str) -> Option<String> {
    let upper = file_name.to_ascii_uppercase();
    let suffix = upper.strip_prefix("WALLDEF")?.strip_suffix(".DAX")?;
    Some(format!("8X8D{suffix}.DAX"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::walldef::WALLSET_SIZE;

    #[test]
    fn paired_image_block_id_single_wallset_uses_the_walldef_id_itself() {
        assert_eq!(paired_image_block_id(14, 1, 0), 14);
    }

    #[test]
    fn paired_image_block_id_multi_wallset_uses_id_times_10_plus_n_plus_1() {
        assert_eq!(paired_image_block_id(14, 3, 0), 141);
        assert_eq!(paired_image_block_id(14, 3, 1), 142);
        assert_eq!(paired_image_block_id(14, 3, 2), 143);
    }

    #[test]
    fn target_symbol_set_maps_wallset_zero_to_set_one() {
        assert_eq!(target_symbol_set(0), 1);
        assert_eq!(target_symbol_set(1), 2);
        assert_eq!(target_symbol_set(2), 3);
    }

    fn walldef_with_tile(wallset: usize, style: usize, idx: usize, raw_id: u8) -> WalldefBlock {
        let mut data = vec![0u8; WALLSET_SIZE * (wallset + 1)];
        data[wallset * WALLSET_SIZE + style * TILE_IDS_PER_STYLE + idx] = raw_id;
        WalldefBlock::parse(&data).unwrap()
    }

    #[test]
    fn rebase_style_leaves_ids_below_0x2d_untouched() {
        let walldef = walldef_with_tile(0, 0, 5, 0x10);
        let out = rebase_style(&walldef, 0, 0, target_symbol_set(0));
        assert_eq!(out[5], 0x10);
    }

    #[test]
    fn rebase_style_shifts_ids_at_or_above_0x2d_for_set_two() {
        // wallset 1 -> target_set 2; rebase = FIX[2]-FIX[1] = 0x74-0x2E = 0x46.
        let walldef = walldef_with_tile(1, 2, 10, 0x2E);
        let out = rebase_style(&walldef, 1, 2, target_symbol_set(1));
        assert_eq!(out[10], 0x2E + (0x74 - 0x2E));
    }

    #[test]
    fn rebase_style_for_set_one_is_a_zero_shift() {
        // The design doc's own citation (§1.3): a walldef loaded at base set
        // 1 is rebased by zero everywhere.
        let walldef = walldef_with_tile(0, 0, 0, 0x50);
        let out = rebase_style(&walldef, 0, 0, target_symbol_set(0));
        assert_eq!(out[0], 0x50);
    }

    fn image_block(items: usize) -> ImageBlock {
        ImageBlock {
            height: 8,
            width_cols: 1,
            x_pos: 0,
            y_pos: 0,
            field_9: [0; 8],
            items: (0..items)
                .map(|i| DecodedItem {
                    pixels: vec![(i % 16) as u8; 64],
                })
                .collect(),
        }
    }

    #[test]
    fn resolve_tile_zero_is_a_hole() {
        let pixels = image_block(5);
        assert_eq!(resolve_tile(0, 1, &pixels), None);
    }

    #[test]
    fn resolve_tile_indexes_relative_to_the_target_sets_base() {
        let pixels = image_block(5);
        // target_set 1's base is SYMBOL_SET_FIX[1] = 0x2E; id 0x2E+2 -> item 2.
        let tile_id = (SYMBOL_SET_FIX[1] + 2) as u8;
        let item = resolve_tile(tile_id, 1, &pixels).unwrap();
        assert_eq!(item.pixels[0], 2);
    }

    #[test]
    fn resolve_tile_out_of_range_is_none_not_a_panic() {
        let pixels = image_block(2);
        let tile_id = (SYMBOL_SET_FIX[1] + 99) as u8;
        assert_eq!(resolve_tile(tile_id, 1, &pixels), None);
    }

    #[test]
    fn compose_style_produces_156_slots() {
        let walldef = walldef_with_tile(0, 0, 0, 0);
        let pixels = image_block(1);
        let composed = compose_style(&walldef, 0, 0, &pixels);
        assert_eq!(composed.len(), TILE_IDS_PER_STYLE);
    }

    #[test]
    fn sym_file_for_walldef_file_derives_the_paired_8x8d_name() {
        assert_eq!(
            sym_file_for_walldef_file("WALLDEF5.DAX"),
            Some("8X8D5.DAX".to_string())
        );
        assert_eq!(
            sym_file_for_walldef_file("walldef2.dax"),
            Some("8X8D2.DAX".to_string())
        );
    }

    #[test]
    fn sym_file_for_walldef_file_rejects_non_matching_names() {
        assert_eq!(sym_file_for_walldef_file("GEO2.DAX"), None);
        assert_eq!(sym_file_for_walldef_file("WALLDEF5.GAM"), None);
    }

    #[test]
    fn stitch_composite_sizes_the_buffer_from_cols_and_tile_dims() {
        let composed: Vec<Option<&DecodedItem>> = vec![None; 5];
        let (w, h, pixels) = stitch_composite(&composed, 4, 8, 3);
        assert_eq!(w, 12); // 3 cols * 4px
        assert_eq!(h, 16); // ceil(5/3)=2 rows * 8px
        assert_eq!(pixels.len(), 12 * 16);
    }

    #[test]
    fn stitch_composite_all_holes_is_all_transparent() {
        let composed: Vec<Option<&DecodedItem>> = vec![None; 4];
        let (_, _, pixels) = stitch_composite(&composed, 2, 2, 2);
        assert!(pixels.iter().all(|&p| p == TRANSPARENT));
    }

    #[test]
    fn stitch_composite_places_a_tile_at_its_grid_offset() {
        let tile = DecodedItem {
            pixels: vec![7, 7, 7, 7], // 2x2, all index 7
        };
        let composed: Vec<Option<&DecodedItem>> = vec![None, Some(&tile), None, None];
        let (w, _h, pixels) = stitch_composite(&composed, 2, 2, 2);
        // slot 1 -> col 1, row 0 -> pixel offset (ox=2, oy=0)
        assert_eq!(pixels[2], 7);
        assert_eq!(pixels[3], 7);
        assert_eq!(pixels[w + 2], 7);
        assert_eq!(pixels[w + 3], 7);
        // slot 0 (a hole) stays transparent
        assert_eq!(pixels[0], TRANSPARENT);
    }

    #[test]
    fn compose_style_resolves_a_real_tile_end_to_end() {
        let raw_id = (SYMBOL_SET_FIX[1] + 3) as u8; // set 1, item 3
        let walldef = walldef_with_tile(0, 4, 7, raw_id);
        let pixels = image_block(10);
        let composed = compose_style(&walldef, 0, 4, &pixels);
        let item = composed[7].expect("tile 7 must resolve");
        assert_eq!(item.pixels[0], 3);
        // Every other slot in this style is tile id 0 -> a hole.
        assert!(composed[0].is_none());
    }
}
