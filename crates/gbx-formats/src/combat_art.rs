//! Combat art: the 24×24 ground-tile sets (`DUNGCOM`/`WILDCOM`/`RANDCOM`)
//! and the combatant icon sprites (`CHEAD`/`CBODY` party halves, `CPIC*`
//! monster pictures, `COMSPR` missiles/effects/focus-box), plus the four
//! pixel transforms the original applies to them: horizontal mirror, the
//! head-over-body merge, and the 16-entry recolor table.
//!
//! Pure over bytes — no filesystem access. The container underneath every
//! one of these is the same 4bpp DAX image block [`crate::image`] already
//! decodes ([verified against the real CotAB data set](#verified-shapes));
//! this module is the validating, transform-carrying layer above it that
//! the combat visualizer needs, and it holds no opinion about which DAX
//! file or block id a sprite came from (that's the engine's loader).
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `Classes/DaxFiles/DaxBlock.cs` — `FlipIconLeftToRight` (`:52-69`),
//!   `Recolor` (`:71-94`), `MergeIcons` (`:96-121`), and the
//!   `SetMaskedColor` transparency-16 rule (`:149-159`) this module's
//!   transforms transliterate.
//! - `engine/ovr034.cs` — `Load24x24Set` (`:9-27`, the atlas copy whose
//!   `bpp = height * width * 8` cell stride the [`TileSet`] shape check
//!   pins) and `chead_cbody_comspr_icon` (`:50-88`, the mask-color-0/
//!   masked-1 icon decode and the `+0x80` attack-frame convention).
//! - `engine/ovr017.cs` `LoadPlayerCombatIcon` (`:86-122`) — the party
//!   recolor tables [`party_recolor_tables`] builds.
//! - `Classes/Gbl.cs:274` — `default_icon_colours`.
//!
//! # Verified shapes
//!
//! Every claim below was read off the real CotAB data set (`~/goldbox-data/
//! cotab`, 2026-08-01) before this module was written, and is re-checked by
//! the `GBX_DATA_DIR` local-tier tests at the bottom of this file:
//!
//! | File | Blocks | Shape |
//! |---|---|---|
//! | `DUNGCOM.DAX` | 1 | one block, 25 (`0x19`) items, 24×24 |
//! | `WILDCOM.DAX` | 1 | one block, **34** items, 24×24 (coab loads only the first `0x21` = 33) |
//! | `RANDCOM.DAX` | 1 | one block, 6 items, 24×24 |
//! | `COMSPR.DAX` | 26 | ids 0–11 + 25, each with a `+0x80` twin; 1 item, 24×24 |
//! | `CHEAD.DAX` | 56 | ids 0–13 (h=10) and 64–77 (h=8), each with a `+0x80` twin; 1 item, 24 px wide |
//! | `CBODY.DAX` | 128 | ids 0–31 and 64–95, each with a `+0x80` twin; 1 item, 24×24 |
//! | `CPIC{1..6}.DAX` | 11–36 | 1 item each, 24×24 / 48×24 / 24×48 / 48×48 |
//!
//! So: the design doc's `image.rs`-shaped 4bpp multi-item assumption holds
//! for all seven containers, with two refinements worth stating out loud —
//! (1) icon blocks always carry exactly **one** item (multi-frame-ness is
//! expressed as separate block ids, not as items), and (2) a **head half is
//! not a whole cell**: `CHEAD` blocks are 10 rows (size-1) or 8 rows
//! (size-2) of a 24-row body, which is exactly what makes
//! [`Sprite::merge_from`]'s "src may be shorter than dst" rule load-bearing.
//!
//! Monster footprints follow the original's `Steps` table
//! (`ovr033.cs:10-16`: size 1 = 1×1, size 2 = 1 wide × 2 tall, size 3 =
//! 2 wide × 1 tall, size 4 = 2×2) and are read straight off the DAX header
//! — no separate footprint table is needed to decode them.

use crate::image::{self, ImageError};

/// One combat map cell's edge in pixels (`IconColumnSize=3` 8-pixel columns,
/// `ovr033.cs:316-334`).
pub const CELL_PX: usize = 24;

/// The palette code every icon block is masked against
/// (`chead_cbody_comspr_icon`'s `LoadIcons(0, 1, ...)` — mask color 0,
/// masked on). Ground tiles are loaded *unmasked* (`Load24x24Set`'s
/// `LoadDax(0, 0, ...)`), which is why [`decode_tile_set`] takes no mask.
pub const ICON_MASK: u8 = 0;

/// The transparency sentinel [`crate::image::decode`] writes for masked
/// pixels (`SetMaskedColor`'s `data[offset] = 16`).
pub const TRANSPARENT: u8 = 16;

/// The block-id offset from an icon's Normal frame to its Attack frame
/// (`LoadIcons(..., block_id, block_id + 0x80)`).
pub const ATTACK_BLOCK_OFFSET: u8 = 0x80;

/// The block-id offset applied to a size-2 (`'T'`) party icon half
/// (`chead_cbody_comspr_icon:59-62`).
pub const TALL_BLOCK_OFFSET: u8 = 0x40;

/// The six template palette codes a party icon's recolor rewrites
/// (`Gbl.cs:274`'s `default_icon_colours`). Each is rewritten together with
/// its `+8` bright twin — see [`party_recolor_tables`].
pub const DEFAULT_ICON_COLOURS: [u8; 6] = [1, 2, 3, 4, 6, 7];

/// Everything that can go wrong decoding or transforming combat art.
/// Malformed input is expected input (fuzz posture, PLAN M1) — every
/// variant is a clean `Err`, never a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatArtError {
    /// The underlying 4bpp image block failed to decode.
    Image(ImageError),
    /// A tile set's items are not 24×24 (`Load24x24Set` copies fixed-stride
    /// `bpp`-sized cells into the atlas — a differently-shaped block would
    /// silently shear every tile after the first).
    TileNotCellSized { width: usize, height: usize },
    /// A tile set declared zero items.
    EmptyTileSet,
    /// An icon block holds other than exactly one item. Every real combat
    /// icon block carries one; multi-frame-ness lives in separate block ids
    /// (`+0x80` Attack, `+0x40` size-2), never in `item_count`.
    IconItemCount { count: usize },
    /// An icon decoded to a zero-sized sprite.
    EmptyIcon,
    /// [`Sprite::merge_from`]'s source is wider than, or has more pixels
    /// than, the destination. `MergeIcons` walks a *flat* index over the
    /// source's pixel count, so a width mismatch would shear the overlay
    /// and an oversized source would run off the end of the destination —
    /// both are reported instead of reproduced.
    MergeShapeMismatch {
        dst_width: usize,
        dst_pixels: usize,
        src_width: usize,
        src_pixels: usize,
    },
}

impl From<ImageError> for CombatArtError {
    fn from(e: ImageError) -> Self {
        CombatArtError::Image(e)
    }
}

/// One decoded combat-art sprite: `width * height` pixels, row-major, each
/// `0..=15` (palette code) or [`TRANSPARENT`].
///
/// This is a single *frame* — a `CHEAD` head half, a `CBODY` body, a `CPIC`
/// monster picture, or a `COMSPR` effect cell. Poses (Normal/Attack) and
/// facings (base/mirrored) are separate `Sprite`s, matching the original's
/// four-`DaxBlock` `CombatIcon` (`Classes/Combat/CombatIcon.cs:16-20`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sprite {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Sprite {
    /// The sprite's footprint in 24×24 combat cells, when its dimensions are
    /// a whole number of them (`Some((cols, rows))`); `None` otherwise — a
    /// `CHEAD` half (24×10, 24×8) is the deliberate `None` case.
    pub fn cell_footprint(&self) -> Option<(usize, usize)> {
        if self.width.is_multiple_of(CELL_PX) && self.height.is_multiple_of(CELL_PX) {
            Some((self.width / CELL_PX, self.height / CELL_PX))
        } else {
            None
        }
    }

    /// A horizontally mirrored copy — the cached flip every icon carries for
    /// facings 4-7 (`FlipIconLeftToRight`, `DaxBlock.cs:52-69`).
    pub fn mirrored(&self) -> Sprite {
        let mut pixels = Vec::with_capacity(self.pixels.len());
        for row in self.pixels.chunks(self.width) {
            pixels.extend(row.iter().rev().copied());
        }
        Sprite {
            width: self.width,
            height: self.height,
            pixels,
        }
    }

    /// Overlays `src` onto `self` from the top-left corner — the head-over-
    /// body blend (`MergeIcons`, `DaxBlock.cs:96-121`).
    ///
    /// Per-pixel: transparent-over-transparent stays transparent, either
    /// side transparent takes the other, and **two opaque pixels are
    /// bitwise-OR'd** (coab's own comment flags that OR as a guess about the
    /// original; it is transliterated as read, not "fixed" — the real head
    /// and body art overlap only on transparent pixels in practice, so the
    /// OR arm is unreachable for real data and any change to it would be
    /// invisible anyway).
    ///
    /// `src` may be *shorter* than `self` — that is the normal case (a 24×10
    /// head over a 24×24 body): the merge covers exactly `src`'s pixel count
    /// and leaves the rest of `self` untouched.
    pub fn merge_from(&mut self, src: &Sprite) -> Result<(), CombatArtError> {
        if src.width != self.width || src.pixels.len() > self.pixels.len() {
            return Err(CombatArtError::MergeShapeMismatch {
                dst_width: self.width,
                dst_pixels: self.pixels.len(),
                src_width: src.width,
                src_pixels: src.pixels.len(),
            });
        }
        for (dst, &b) in self.pixels.iter_mut().zip(src.pixels.iter()) {
            let a = *dst;
            *dst = match (a == TRANSPARENT, b == TRANSPARENT) {
                (true, true) => TRANSPARENT,
                (true, false) => b,
                (false, true) => a,
                (false, false) => a | b,
            };
        }
        Ok(())
    }

    /// Rewrites palette codes in place (`Recolor`, `DaxBlock.cs:71-94`).
    ///
    /// For each table slot `i` in ascending order, every pixel equal to
    /// `old_colors[i]` becomes `new_colors[i]`; slots where old == new are
    /// skipped. **The passes are sequential and can chain** — a pixel
    /// rewritten by slot `i` is visible to slot `j > i` — so the iteration
    /// order is part of the behavior, not an implementation detail.
    /// [`TRANSPARENT`] pixels are never touched (the tables hold only
    /// `0..=15`).
    pub fn recolor(&mut self, new_colors: &[u8; 16], old_colors: &[u8; 16]) {
        for i in 0..16 {
            if old_colors[i] == new_colors[i] {
                continue;
            }
            for px in self.pixels.iter_mut() {
                if *px == old_colors[i] {
                    *px = new_colors[i];
                }
            }
        }
    }
}

/// A decoded 24×24 tile set — one `DUNGCOM`/`WILDCOM`/`RANDCOM` block's
/// items, in file order, ready to be copied into the engine's 48-slot atlas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileSet {
    /// Each entry is exactly `CELL_PX * CELL_PX` pixels, row-major.
    pub tiles: Vec<Vec<u8>>,
}

impl TileSet {
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

/// Decodes a ground-tile block (`DUNGCOM`/`WILDCOM`/`RANDCOM` block 1).
///
/// **Unmasked** — `Load24x24Set` loads via `LoadDax(0, 0, ...)`
/// (`masked == 0`), so a tile's palette-0 pixels are opaque black floor, not
/// transparency. Every item must be exactly 24×24: the original copies
/// `cellCount * bpp` bytes at a fixed cell stride into the shared atlas
/// buffer, so an off-shape block would shear silently.
pub fn decode_tile_set(data: &[u8]) -> Result<TileSet, CombatArtError> {
    let block = image::decode(data, None)?;
    let width = block.width_px();
    let height = block.height as usize;
    if width != CELL_PX || height != CELL_PX {
        return Err(CombatArtError::TileNotCellSized { width, height });
    }
    if block.items.is_empty() {
        return Err(CombatArtError::EmptyTileSet);
    }
    Ok(TileSet {
        tiles: block.items.into_iter().map(|item| item.pixels).collect(),
    })
}

/// Decodes one combat icon frame (`CHEAD`/`CBODY`/`CPIC*`/`COMSPR` block),
/// masked on palette code 0 per `LoadIcons(0, 1, ...)`.
///
/// Accepts any non-empty dimensions: `CBODY`/`CPIC`/`COMSPR` frames are
/// whole 24×24 cell grids (see [`Sprite::cell_footprint`]), but `CHEAD`
/// halves are deliberately shorter than a cell.
pub fn decode_icon(data: &[u8]) -> Result<Sprite, CombatArtError> {
    let mut block = image::decode(data, Some(ICON_MASK))?;
    if block.items.len() != 1 {
        return Err(CombatArtError::IconItemCount {
            count: block.items.len(),
        });
    }
    let width = block.width_px();
    let height = block.height as usize;
    if width == 0 || height == 0 {
        return Err(CombatArtError::EmptyIcon);
    }
    Ok(Sprite {
        width,
        height,
        pixels: block.items.pop().expect("item_count checked == 1").pixels,
    })
}

/// Builds the `(new_colors, old_colors)` pair `LoadPlayerCombatIcon`
/// (`ovr017.cs:99-116`) hands to [`Sprite::recolor`] for a party member's
/// stored `icon_colours[6]`.
///
/// `old_colors` is the identity table; `new_colors` is the identity table
/// with twelve slots rewritten — for each of the six [`DEFAULT_ICON_COLOURS`]
/// template codes `d`, slot `d` takes the low nibble of the character's
/// colour byte and slot `d + 8` (the bright twin) takes the high nibble.
///
/// Note the asymmetry, transliterated as read: the *source* index is
/// `d + 8`, but the value written is the raw high nibble — no `+8` is added
/// back, so a character can map a bright template code onto a dim palette
/// entry. A default-coloured character (`icon_colours[i] =
/// ((d + 8) << 4) + d`, `ovr018.cs:341`) round-trips to the identity table,
/// which is why an unmodified party recolors to a no-op.
pub fn party_recolor_tables(icon_colours: &[u8; 6]) -> ([u8; 16], [u8; 16]) {
    let old_colors: [u8; 16] = std::array::from_fn(|i| i as u8);
    let mut new_colors = old_colors;
    for (i, &template) in DEFAULT_ICON_COLOURS.iter().enumerate() {
        new_colors[template as usize] = icon_colours[i] & 0x0F;
        new_colors[template as usize + 8] = (icon_colours[i] & 0xF0) >> 4;
    }
    (new_colors, old_colors)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-authored (D10): packs a 4bpp image block from a header and a
    /// list of items, each item a flat row-major list of nibble values.
    /// Mirrors `image.rs`'s own `build_block` test helper.
    fn build_block(height: u16, width_cols: u16, items: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&width_cols.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // x_pos
        out.extend_from_slice(&0u16.to_le_bytes()); // y_pos
        out.push(items.len() as u8);
        out.extend_from_slice(&[0u8; 8]); // field_9
        for item in items {
            for pair in item.chunks(2) {
                out.push((pair[0] << 4) | *pair.get(1).unwrap_or(&0));
            }
        }
        out
    }

    /// A 24×24 item whose every pixel is `fill`.
    fn cell(fill: u8) -> Vec<u8> {
        vec![fill; CELL_PX * CELL_PX]
    }

    fn sprite(width: usize, height: usize, pixels: Vec<u8>) -> Sprite {
        assert_eq!(pixels.len(), width * height);
        Sprite {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn tile_set_decodes_every_item_unmasked() {
        // Palette code 0 must survive as 0 (opaque floor), never become 16.
        let bytes = build_block(24, 3, &[&cell(0), &cell(5)]);
        let set = decode_tile_set(&bytes).unwrap();
        assert_eq!(set.len(), 2);
        assert!(!set.is_empty());
        assert_eq!(set.tiles[0], cell(0));
        assert_eq!(set.tiles[1], cell(5));
    }

    #[test]
    fn tile_set_rejects_off_cell_dimensions() {
        // 24 px wide but only 10 rows — a CHEAD-shaped block, not a tile.
        let bytes = build_block(10, 3, &[&vec![1u8; 24 * 10]]);
        assert_eq!(
            decode_tile_set(&bytes).unwrap_err(),
            CombatArtError::TileNotCellSized {
                width: 24,
                height: 10
            }
        );
        // 48 px wide, 24 tall — a size-3 monster picture, not a tile.
        let bytes = build_block(24, 6, &[&vec![1u8; 48 * 24]]);
        assert_eq!(
            decode_tile_set(&bytes).unwrap_err(),
            CombatArtError::TileNotCellSized {
                width: 48,
                height: 24
            }
        );
    }

    #[test]
    fn tile_set_rejects_empty_item_list() {
        let bytes = build_block(24, 3, &[]);
        assert_eq!(
            decode_tile_set(&bytes).unwrap_err(),
            CombatArtError::EmptyTileSet
        );
    }

    #[test]
    fn tile_set_propagates_image_errors() {
        assert_eq!(
            decode_tile_set(&[0u8; 4]).unwrap_err(),
            CombatArtError::Image(ImageError::TooShortForHeader { len: 4 })
        );
        let mut bytes = build_block(24, 3, &[]);
        bytes[8] = 1; // claims an item, supplies no pixels
        assert!(matches!(
            decode_tile_set(&bytes).unwrap_err(),
            CombatArtError::Image(ImageError::TruncatedPixelData { .. })
        ));
    }

    #[test]
    fn icon_decodes_masked_on_color_zero() {
        let mut pixels = cell(7);
        pixels[0] = 0;
        pixels[CELL_PX] = 0;
        let bytes = build_block(24, 3, &[&pixels]);
        let icon = decode_icon(&bytes).unwrap();
        assert_eq!((icon.width, icon.height), (24, 24));
        assert_eq!(icon.pixels[0], TRANSPARENT);
        assert_eq!(icon.pixels[CELL_PX], TRANSPARENT);
        assert_eq!(icon.pixels[1], 7);
    }

    #[test]
    fn icon_accepts_a_short_head_half() {
        // CHEAD size-1 shape: 24 px wide, 10 rows — not a whole cell.
        let bytes = build_block(10, 3, &[&vec![3u8; 24 * 10]]);
        let head = decode_icon(&bytes).unwrap();
        assert_eq!((head.width, head.height), (24, 10));
        assert_eq!(head.cell_footprint(), None);
    }

    #[test]
    fn cell_footprint_reports_multi_cell_monster_sizes() {
        // The four real CPIC shapes, matching ovr033.cs:10-16's Steps table.
        let cases = [
            (24, 24, (1, 1)),
            (24, 48, (1, 2)),
            (48, 24, (2, 1)),
            (48, 48, (2, 2)),
        ];
        for (w, h, expected) in cases {
            let s = sprite(w, h, vec![0; w * h]);
            assert_eq!(s.cell_footprint(), Some(expected), "{w}x{h}");
        }
    }

    #[test]
    fn icon_rejects_multi_item_blocks() {
        let bytes = build_block(24, 3, &[&cell(1), &cell(2)]);
        assert_eq!(
            decode_icon(&bytes).unwrap_err(),
            CombatArtError::IconItemCount { count: 2 }
        );
        let bytes = build_block(24, 3, &[]);
        assert_eq!(
            decode_icon(&bytes).unwrap_err(),
            CombatArtError::IconItemCount { count: 0 }
        );
    }

    #[test]
    fn icon_rejects_zero_dimensions() {
        let bytes = build_block(0, 3, &[&[]]);
        assert_eq!(decode_icon(&bytes).unwrap_err(), CombatArtError::EmptyIcon);
        let bytes = build_block(24, 0, &[&[]]);
        assert_eq!(decode_icon(&bytes).unwrap_err(), CombatArtError::EmptyIcon);
    }

    #[test]
    fn icon_propagates_image_errors() {
        assert_eq!(
            decode_icon(&[]).unwrap_err(),
            CombatArtError::Image(ImageError::TooShortForHeader { len: 0 })
        );
    }

    #[test]
    fn mirror_reverses_each_row_independently() {
        let s = sprite(4, 2, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        let m = s.mirrored();
        assert_eq!((m.width, m.height), (4, 2));
        assert_eq!(m.pixels, vec![4, 3, 2, 1, 8, 7, 6, 5]);
    }

    #[test]
    fn mirror_is_an_involution() {
        let s = sprite(3, 3, (0..9).collect());
        assert_eq!(s.mirrored().mirrored(), s);
    }

    #[test]
    fn merge_overlays_the_shorter_head_and_leaves_the_rest() {
        // 2-wide body, 3 rows; head is 2-wide, 1 row.
        let mut body = sprite(2, 3, vec![5, 5, 6, 6, 7, 7]);
        let head = sprite(2, 1, vec![TRANSPARENT, 9]);
        body.merge_from(&head).unwrap();
        // row 0: transparent head pixel keeps the body's 5; opaque 9 ORs
        // with the body's 5 -> 13. Rows 1-2 untouched.
        assert_eq!(body.pixels, vec![5, 5 | 9, 6, 6, 7, 7]);
    }

    #[test]
    fn merge_transparency_rules() {
        let mut dst = sprite(4, 1, vec![TRANSPARENT, TRANSPARENT, 4, 4]);
        let src = sprite(4, 1, vec![TRANSPARENT, 3, TRANSPARENT, 2]);
        dst.merge_from(&src).unwrap();
        assert_eq!(dst.pixels, vec![TRANSPARENT, 3, 4, 4 | 2]);
    }

    #[test]
    fn merge_rejects_a_width_mismatch() {
        let mut dst = sprite(4, 2, vec![0; 8]);
        let src = sprite(2, 2, vec![0; 4]);
        assert_eq!(
            dst.merge_from(&src).unwrap_err(),
            CombatArtError::MergeShapeMismatch {
                dst_width: 4,
                dst_pixels: 8,
                src_width: 2,
                src_pixels: 4,
            }
        );
    }

    #[test]
    fn merge_rejects_an_oversized_source() {
        let mut dst = sprite(2, 1, vec![0; 2]);
        let src = sprite(2, 3, vec![0; 6]);
        assert_eq!(
            dst.merge_from(&src).unwrap_err(),
            CombatArtError::MergeShapeMismatch {
                dst_width: 2,
                dst_pixels: 2,
                src_width: 2,
                src_pixels: 6,
            }
        );
    }

    #[test]
    fn recolor_rewrites_only_listed_codes_and_never_transparency() {
        let mut s = sprite(4, 1, vec![1, 2, TRANSPARENT, 15]);
        let old: [u8; 16] = std::array::from_fn(|i| i as u8);
        let mut new = old;
        new[1] = 10;
        new[15] = 0;
        s.recolor(&new, &old);
        assert_eq!(s.pixels, vec![10, 2, TRANSPARENT, 0]);
    }

    #[test]
    fn recolor_passes_chain_in_table_order() {
        // Slot 1 maps 1 -> 2; slot 2 then maps 2 -> 3. The original's
        // sequential loop lets the first pass's output feed the second, so
        // a pixel that started as 1 ends as 3, not 2.
        let mut s = sprite(2, 1, vec![1, 2]);
        let old: [u8; 16] = std::array::from_fn(|i| i as u8);
        let mut new = old;
        new[1] = 2;
        new[2] = 3;
        s.recolor(&new, &old);
        assert_eq!(s.pixels, vec![3, 3]);
    }

    #[test]
    fn identity_recolor_is_a_no_op() {
        // The CPIC path's `Recolor(false, unk_16E40, unk_16E30)`
        // (`ovr034.cs:80-81`) passes two identity tables — monsters render
        // unrecolored.
        let before = sprite(4, 2, vec![0, 1, 15, TRANSPARENT, 7, 7, 3, 9]);
        let mut after = before.clone();
        let identity: [u8; 16] = std::array::from_fn(|i| i as u8);
        after.recolor(&identity, &identity);
        assert_eq!(after, before);
    }

    #[test]
    fn party_recolor_tables_rewrite_the_six_templates_and_their_bright_twins() {
        // Character colours: low nibble -> template slot, high nibble ->
        // template + 8.
        let colours = [0x91, 0xA2, 0xB3, 0xC4, 0xE6, 0xF7];
        let (new, old) = party_recolor_tables(&colours);
        assert_eq!(old, std::array::from_fn::<u8, 16, _>(|i| i as u8));
        for (i, &d) in DEFAULT_ICON_COLOURS.iter().enumerate() {
            assert_eq!(new[d as usize], colours[i] & 0x0F, "slot {d}");
            assert_eq!(new[d as usize + 8], colours[i] >> 4, "slot {}", d + 8);
        }
        // Codes 0, 5, 13 are outside the template set and stay identity.
        for untouched in [0usize, 5, 13] {
            assert_eq!(new[untouched], untouched as u8);
        }
    }

    #[test]
    fn default_party_colours_round_trip_to_the_identity_table() {
        // `ovr018.cs:341`: icon_colours[i] = ((d + 8) << 4) + d.
        let colours: [u8; 6] =
            std::array::from_fn(|i| ((DEFAULT_ICON_COLOURS[i] + 8) << 4) + DEFAULT_ICON_COLOURS[i]);
        let (new, old) = party_recolor_tables(&colours);
        assert_eq!(new, old, "an unmodified party must recolor to a no-op");
    }

    /// Local-only tier (pattern from `image.rs`/`dax.rs`): the real combat
    /// art decodes, and its shapes are the ones this module's doc comment
    /// pins. Loud-skips when `GBX_DATA_DIR` is unset.
    #[test]
    fn real_combat_art_matches_the_documented_shapes() {
        use crate::dax::DaxArchive;
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);

        let read = |name: &str| -> Option<Vec<u8>> { std::fs::read(dir.join(name)).ok() };

        // --- ground tiles: one block each, all items 24x24 ---
        let mut tiles_checked = 0usize;
        for (file, expected_items) in [
            ("DUNGCOM.DAX", 25usize),
            ("WILDCOM.DAX", 34),
            ("RANDCOM.DAX", 6),
        ] {
            let Some(bytes) = read(file) else {
                panic!("{file} missing from GBX_DATA_DIR");
            };
            let archive = DaxArchive::parse(&bytes).unwrap_or_else(|e| panic!("{file}: {e:?}"));
            assert_eq!(archive.entries().len(), 1, "{file} block count");
            let raw = archive
                .block_data(1)
                .unwrap_or_else(|e| panic!("{file} block 1: {e:?}"));
            let set = decode_tile_set(&raw).unwrap_or_else(|e| panic!("{file} block 1: {e:?}"));
            assert_eq!(set.len(), expected_items, "{file} item count");
            tiles_checked += set.len();
        }

        // --- icon containers: one item each, masked, cell-aligned except
        //     CHEAD's short head halves ---
        let mut icons_checked = 0usize;
        let icon_files: Vec<String> = ["COMSPR.DAX", "CHEAD.DAX", "CBODY.DAX"]
            .iter()
            .map(|s| s.to_string())
            .chain((1..=6).map(|n| format!("CPIC{n}.DAX")))
            .collect();
        for file in &icon_files {
            let Some(bytes) = read(file) else {
                panic!("{file} missing from GBX_DATA_DIR");
            };
            let archive = DaxArchive::parse(&bytes).unwrap_or_else(|e| panic!("{file}: {e:?}"));
            for entry in archive.entries() {
                let raw = archive
                    .block_data(entry.id)
                    .unwrap_or_else(|e| panic!("{file} block {}: {e:?}", entry.id));
                let icon = decode_icon(&raw)
                    .unwrap_or_else(|e| panic!("{file} block {}: {e:?}", entry.id));
                assert!(
                    icon.width == CELL_PX || icon.width == 2 * CELL_PX,
                    "{file} block {}: width {} is neither one nor two cells",
                    entry.id,
                    icon.width
                );
                if file == "CHEAD.DAX" {
                    assert!(
                        icon.height == 10 || icon.height == 8,
                        "{file} block {}: head half height {}",
                        entry.id,
                        icon.height
                    );
                } else {
                    assert!(
                        icon.cell_footprint().is_some(),
                        "{file} block {}: {}x{} is not a whole cell grid",
                        entry.id,
                        icon.width,
                        icon.height
                    );
                }
                icons_checked += 1;
            }
            // Every Normal frame must have its +0x80 Attack twin.
            let ids: std::collections::BTreeSet<u8> =
                archive.entries().iter().map(|e| e.id).collect();
            for &id in &ids {
                if id < ATTACK_BLOCK_OFFSET {
                    assert!(
                        ids.contains(&(id + ATTACK_BLOCK_OFFSET)),
                        "{file}: block {id} has no +0x80 attack frame"
                    );
                }
            }
        }

        eprintln!("checked {tiles_checked} real tile(s), {icons_checked} real icon frame(s)");
    }
}
