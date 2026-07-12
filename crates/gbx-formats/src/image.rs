//! The 4bpp DAX image block: the container format behind every static
//! picture asset (`8X8D*` symbol sets, `BIGPIC*`, `HEAD*`/`BODY*` portraits,
//! `SKY`). Pure over bytes — no filesystem access.
//!
//! Derived by reading coab for behavior (D11, never copied) and
//! cross-checked against an independent reimplementation:
//! - coab `Classes/DaxFiles/DaxBlock.cs` (`DaxBlock(byte[], int, int)`'s
//!   header read at `:33-50`, `DaxToPicture`/`SetMaskedColor` at `:124-159`
//!   — the header layout, the packed-pixel unpack order, and the masked-
//!   color-to-transparency-16 rule this module implements).
//! - Gold Box Explorer (C#) `Common/Plugins/Dax/DaxImagePlugin.cs` /
//!   `RenderBlockFactory` — an independently maintained decoder for the same
//!   container, cited by `docs/design/renderer-ui-shell.md` D-UI5 as the
//!   cross-check for this format; GBE's `DaxWallDefFile.cs` additionally
//!   corroborates the "one 8x8 image is one item in a multi-item block"
//!   framing this module assumes.
//!
//! ## On-disk layout (`docs/design/renderer-ui-shell.md` §1.8)
//!
//! ```text
//! offset 0x00..0x02  u16 LE  height        (pixel rows, shared by every item)
//! offset 0x02..0x04  u16 LE  width_cols    (pixel width, in 8-pixel columns)
//! offset 0x04..0x06  u16 LE  x_pos
//! offset 0x06..0x08  u16 LE  y_pos
//! offset 0x08        u8      item_count
//! offset 0x09..0x11  [u8; 8] field_9       (unread by the original's draw
//!                                            path — carried, unused; docket)
//! offset 0x11..      packed pixel data: `item_count` items, each
//!                     `height * width_cols * 4` bytes (2 pixels/byte,
//!                     high-nibble-first, `width_cols * 4` bytes per row)
//! ```
//!
//! A masked decode ([`decode`]'s `mask` parameter) maps one caller-chosen
//! palette code to transparency-16 (`DaxBlock.SetMaskedColor`); the engine
//! picks which code per asset class (13 for 8x8 symbol sets, 0 for pictures,
//! §1.3/§1.8) — this decoder takes it as a parameter and has no opinion.

/// One decoded item's pixels: `width * height` bytes, one byte per pixel,
/// row-major, values `0..=15` (palette index) or `16` (transparent, only
/// when [`decode`] was called with a mask).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedItem {
    pub pixels: Vec<u8>,
}

/// A parsed 4bpp image block: the shared header plus one [`DecodedItem`] per
/// `item_count`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBlock {
    pub height: u16,
    /// Pixel width in 8-pixel columns, as stored on disk. Use
    /// [`ImageBlock::width_px`] for the actual pixel width.
    pub width_cols: u16,
    pub x_pos: u16,
    pub y_pos: u16,
    /// The 8-byte header field the original stores but never reads
    /// (`field_9`, docket item 2 in the design doc). Carried verbatim.
    pub field_9: [u8; 8],
    pub items: Vec<DecodedItem>,
}

impl ImageBlock {
    /// Pixel width (`width_cols * 8`), the unit every [`DecodedItem::pixels`]
    /// row is measured in.
    pub fn width_px(&self) -> usize {
        self.width_cols as usize * 8
    }
}

/// [`decode`]'s failure mode. Malformed/truncated input is expected input
/// (fuzz posture) — never a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// Fewer than 17 bytes total — can't even read the fixed header.
    TooShortForHeader { len: usize },
    /// The header's declared `item_count * height * width_cols * 4` packed
    /// bytes extend past the end of the input.
    TruncatedPixelData { needed: usize, available: usize },
}

const HEADER_LEN: usize = 17;

/// Decodes a 4bpp image block's raw (already DAX-decompressed) bytes.
///
/// `mask`, when `Some(mask_color)`, turns every pixel whose raw nibble value
/// equals `mask_color` into transparency-16 (`DaxBlock.SetMaskedColor`,
/// `masked == 1`); `None` performs an unmasked decode (nibble values pass
/// through as-is, `masked == 0`).
pub fn decode(data: &[u8], mask: Option<u8>) -> Result<ImageBlock, ImageError> {
    if data.len() < HEADER_LEN {
        return Err(ImageError::TooShortForHeader { len: data.len() });
    }

    let height = u16::from_le_bytes([data[0], data[1]]);
    let width_cols = u16::from_le_bytes([data[2], data[3]]);
    let x_pos = u16::from_le_bytes([data[4], data[5]]);
    let y_pos = u16::from_le_bytes([data[6], data[7]]);
    let item_count = data[8];
    let mut field_9 = [0u8; 8];
    field_9.copy_from_slice(&data[9..17]);

    let bytes_per_row = width_cols as usize * 4;
    let bytes_per_item = height as usize * bytes_per_row;
    let needed = item_count as usize * bytes_per_item;
    let pixel_data = data[HEADER_LEN..]
        .get(..needed)
        .ok_or(ImageError::TruncatedPixelData {
            needed,
            available: data.len().saturating_sub(HEADER_LEN),
        })?;

    let mut items = Vec::with_capacity(item_count as usize);
    for i in 0..item_count as usize {
        let item_chunk = &pixel_data[i * bytes_per_item..(i + 1) * bytes_per_item];
        items.push(DecodedItem {
            pixels: unpack_nibbles(item_chunk, mask),
        });
    }

    Ok(ImageBlock {
        height,
        width_cols,
        x_pos,
        y_pos,
        field_9,
        items,
    })
}

/// Unpacks one item's encoded 4bpp bytes into one-byte-per-pixel row-major
/// pixels (`DaxBlock.DaxToPicture`'s innermost loop), applying `mask` per
/// [`decode`]'s contract. Shared with [`crate::anim`], whose per-frame
/// header differs from this module's but whose pixel packing is identical.
pub(crate) fn unpack_nibbles(encoded: &[u8], mask: Option<u8>) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(encoded.len() * 2);
    for &byte in encoded {
        pixels.push(masked_nibble(byte >> 4, mask));
        pixels.push(masked_nibble(byte & 0x0F, mask));
    }
    pixels
}

fn masked_nibble(color: u8, mask: Option<u8>) -> u8 {
    match mask {
        Some(mask_color) if color == mask_color => 16,
        _ => color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-authored (D10): builds a raw image block's bytes from a header
    /// and a list of items, each item a flat list of 4-bit nibble values
    /// (`height * width_cols * 8` nibbles, row-major) packed high-nibble-
    /// first.
    fn build_block(
        height: u16,
        width_cols: u16,
        x_pos: u16,
        y_pos: u16,
        field_9: [u8; 8],
        items: &[&[u8]],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&width_cols.to_le_bytes());
        out.extend_from_slice(&x_pos.to_le_bytes());
        out.extend_from_slice(&y_pos.to_le_bytes());
        out.push(items.len() as u8);
        out.extend_from_slice(&field_9);
        for item in items {
            for pair in item.chunks(2) {
                let hi = pair[0];
                let lo = *pair.get(1).unwrap_or(&0);
                out.push((hi << 4) | lo);
            }
        }
        out
    }

    #[test]
    fn decodes_single_item_unmasked() {
        // 1x1 column (8px wide), 1 row tall, nibbles 0..8 then 8..0.
        let nibbles: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let bytes = build_block(1, 1, 0, 0, [0; 8], &[&nibbles]);
        let block = decode(&bytes, None).unwrap();
        assert_eq!(block.height, 1);
        assert_eq!(block.width_cols, 1);
        assert_eq!(block.width_px(), 8);
        assert_eq!(block.items.len(), 1);
        assert_eq!(block.items[0].pixels, nibbles);
    }

    #[test]
    fn field_9_is_carried_verbatim() {
        let field_9 = [1, 2, 3, 4, 5, 6, 7, 8];
        let bytes = build_block(1, 1, 0, 0, field_9, &[&[0; 8]]);
        let block = decode(&bytes, None).unwrap();
        assert_eq!(block.field_9, field_9);
    }

    #[test]
    fn x_pos_y_pos_pass_through() {
        let bytes = build_block(1, 1, 0xAB, 0xCD, [0; 8], &[&[0; 8]]);
        let block = decode(&bytes, None).unwrap();
        assert_eq!(block.x_pos, 0xAB);
        assert_eq!(block.y_pos, 0xCD);
    }

    #[test]
    fn masked_decode_maps_chosen_color_to_transparency_16() {
        let nibbles: Vec<u8> = vec![13, 0, 13, 1, 2, 13, 3, 4];
        let bytes = build_block(1, 1, 0, 0, [0; 8], &[&nibbles]);
        let block = decode(&bytes, Some(13)).unwrap();
        assert_eq!(block.items[0].pixels, vec![16, 0, 16, 1, 2, 16, 3, 4]);
    }

    #[test]
    fn unmasked_decode_never_produces_16() {
        // Even a raw nibble of 15 (the max 4-bit value) must never become
        // transparency-16 without an explicit mask.
        let nibbles: Vec<u8> = vec![15, 15, 15, 15, 15, 15, 15, 15];
        let bytes = build_block(1, 1, 0, 0, [0; 8], &[&nibbles]);
        let block = decode(&bytes, None).unwrap();
        assert_eq!(block.items[0].pixels, nibbles);
    }

    #[test]
    fn multi_item_block_decodes_every_item() {
        let item_a: Vec<u8> = vec![1; 8];
        let item_b: Vec<u8> = vec![2; 8];
        let item_c: Vec<u8> = vec![3; 8];
        let bytes = build_block(1, 1, 0, 0, [0; 8], &[&item_a, &item_b, &item_c]);
        let block = decode(&bytes, None).unwrap();
        assert_eq!(block.items.len(), 3);
        assert_eq!(block.items[0].pixels, item_a);
        assert_eq!(block.items[1].pixels, item_b);
        assert_eq!(block.items[2].pixels, item_c);
    }

    #[test]
    fn multi_row_multi_column_item_decodes_row_major() {
        // 2 rows, 2 columns (16px wide) -> 4 bytes/row, 8 bytes total,
        // 16 pixels. Verify row-major layout matches DaxToPicture's nested
        // loop order (row outer, column inner).
        let row0: Vec<u8> = (0..16).collect();
        let row1: Vec<u8> = (0..16).rev().collect();
        let mut nibbles = row0.clone();
        nibbles.extend_from_slice(&row1);
        let bytes = build_block(2, 2, 0, 0, [0; 8], &[&nibbles]);
        let block = decode(&bytes, None).unwrap();
        assert_eq!(block.items[0].pixels.len(), 32);
        assert_eq!(&block.items[0].pixels[0..16], &row0[..]);
        assert_eq!(&block.items[0].pixels[16..32], &row1[..]);
    }

    #[test]
    fn too_short_for_header_errors_cleanly() {
        let err = decode(&[0u8; 10], None).unwrap_err();
        assert_eq!(err, ImageError::TooShortForHeader { len: 10 });
    }

    #[test]
    fn truncated_pixel_data_errors_cleanly() {
        // Declares a 1x1, 1-item block (8 bytes needed) but supplies none.
        let mut bytes = build_block(1, 1, 0, 0, [0; 8], &[]);
        bytes[8] = 1; // item_count = 1, but no pixel bytes follow
        let err = decode(&bytes, None).unwrap_err();
        assert_eq!(
            err,
            ImageError::TruncatedPixelData {
                needed: 4,
                available: 0
            }
        );
    }

    #[test]
    fn zero_item_count_decodes_to_empty_items() {
        let bytes = build_block(4, 2, 0, 0, [0; 8], &[]);
        let block = decode(&bytes, None).unwrap();
        assert!(block.items.is_empty());
    }

    /// Local-only tier (pattern from `dax.rs`/`geo.rs`): every 8X8D*/
    /// BIGPIC*/HEAD*/BODY*/SKY block in the real data set decodes without
    /// error and has sane dimensions. `8X8D1.DAX` block 201 is excluded —
    /// it is the mono font's flat glyph table ([`crate::font`]), not this
    /// container format, despite living in a same-prefixed file.
    #[test]
    fn every_real_image_block_decodes_with_sane_dimensions() {
        use crate::dax::DaxArchive;
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);

        let mut blocks_checked = 0usize;
        for entry in std::fs::read_dir(dir).expect("GBX_DATA_DIR must be readable") {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_ascii_uppercase();
            let is_image_file = ["8X8D", "BIGPIC", "HEAD", "BODY", "SKY"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
                && name.ends_with(".DAX");
            if !is_image_file {
                continue;
            }

            let bytes = std::fs::read(&path).unwrap();
            let archive = DaxArchive::parse(&bytes)
                .unwrap_or_else(|e| panic!("{}: failed to parse DAX: {e:?}", path.display()));
            for block_entry in archive.entries() {
                if name == "8X8D1.DAX" && block_entry.id == 201 {
                    continue; // the mono font block — crate::font's format, not this one.
                }
                let raw = archive.block_data(block_entry.id).unwrap_or_else(|e| {
                    panic!(
                        "{}: block {} failed to extract: {e:?}",
                        path.display(),
                        block_entry.id
                    )
                });
                let block = decode(&raw, None).unwrap_or_else(|e| {
                    panic!(
                        "{}: block {} failed to decode as an image: {e:?}",
                        path.display(),
                        block_entry.id
                    )
                });
                assert!(
                    block.height > 0 && block.width_cols > 0,
                    "{}: block {} decoded to a zero dimension ({}x{})",
                    path.display(),
                    block_entry.id,
                    block.width_px(),
                    block.height
                );
                blocks_checked += 1;
            }
        }

        assert!(
            blocks_checked > 0,
            "GBX_DATA_DIR is set but no image blocks were found in it"
        );
        eprintln!("checked {blocks_checked} real image block(s)");
    }
}
