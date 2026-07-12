//! The mono 8×8 font: `8X8D1.DAX` block 201, decoded into 177 fixed-size
//! 1bpp glyphs. Pure over bytes — no filesystem access.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `engine/seg041.cs` `Load8x8Tiles` (`:24-41`, `load_8x8d1_201`) —
//!   the glyph-table layout this module replicates: 177 glyphs of 8 bytes
//!   each, one row per byte, packed sequentially with no header.
//!
//! ## On-disk layout (`docs/design/renderer-ui-shell.md` §1.3)
//!
//! Block 201's decompressed payload is a flat run of glyph rows: glyph `j`
//! occupies bytes `[j*8, j*8+8)`, one byte per row, up to 177 glyphs
//! (`1416` bytes total). Each row byte is 8 monochrome pixels, MSB first
//! (`Display.DisplayMono8x8`'s `MonoBitMask = {0x80, 0x40, .., 0x01}` reads
//! bit 7 as the leftmost pixel — engine-side, noted here for context only;
//! this decoder exposes raw row bytes and takes no position on bit order).
//!
//! **The original never errors here** (`Load8x8Tiles`'s loop guards on
//! `i < block_size` and `j < 177` independently, `seg041.cs:33-39`): a short
//! block leaves the tail of `gbl.dax_8x8d1_201` at its zero-initialized
//! default, and a long block's extra bytes are simply never read. This
//! decoder matches that exactly — [`decode`] is infallible, zero-padding a
//! short input and ignoring bytes past the 177th glyph.
//!
//! The `toupper(ch) % 0x40` glyph-index mapping ([`display_char01`] in the
//! original) is engine-side, not this decoder's concern — [`Font::glyph`]
//! is a plain index accessor.

/// Total glyphs the font table holds (`Load8x8Tiles`'s `j < 177` bound).
pub const GLYPH_COUNT: usize = 177;
/// Bytes per glyph (one per 8×8 row).
pub const GLYPH_BYTES: usize = 8;

/// A decoded mono font: 177 glyphs, each 8 row-bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    glyphs: [[u8; GLYPH_BYTES]; GLYPH_COUNT],
}

impl Font {
    /// The 8 row-bytes of glyph `index`. Panics if `index >= GLYPH_COUNT` —
    /// a caller bug (the table is always exactly 177 glyphs), not a runtime
    /// condition; the `toupper % 0x40` index computation is engine-side.
    pub fn glyph(&self, index: usize) -> &[u8; GLYPH_BYTES] {
        &self.glyphs[index]
    }
}

/// Decodes the mono font block's raw (already DAX-decompressed) bytes.
/// Infallible: matches `Load8x8Tiles`'s original behavior of zero-padding a
/// short block and ignoring bytes beyond the 177th glyph (see this module's
/// doc comment) rather than erroring.
pub fn decode(data: &[u8]) -> Font {
    let mut glyphs = [[0u8; GLYPH_BYTES]; GLYPH_COUNT];
    for (j, glyph) in glyphs.iter_mut().enumerate() {
        let start = j * GLYPH_BYTES;
        if start >= data.len() {
            break;
        }
        let end = (start + GLYPH_BYTES).min(data.len());
        glyph[..end - start].copy_from_slice(&data[start..end]);
    }
    Font { glyphs }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-authored (D10): a full-size 1416-byte font block where glyph
    /// `j`'s bytes are all set to `j as u8` for easy identification.
    fn full_size_block() -> Vec<u8> {
        let mut data = Vec::with_capacity(GLYPH_COUNT * GLYPH_BYTES);
        for j in 0..GLYPH_COUNT {
            data.extend_from_slice(&[j as u8; GLYPH_BYTES]);
        }
        data
    }

    #[test]
    fn decodes_full_size_block() {
        let font = decode(&full_size_block());
        assert_eq!(font.glyph(0), &[0u8; 8]);
        assert_eq!(font.glyph(1), &[1u8; 8]);
        assert_eq!(font.glyph(176), &[176u8; 8]);
    }

    #[test]
    fn short_block_zero_pads_remaining_glyphs() {
        // Only 3 full glyphs' worth of data.
        let data = vec![0xFFu8; 3 * GLYPH_BYTES];
        let font = decode(&data);
        assert_eq!(font.glyph(0), &[0xFF; 8]);
        assert_eq!(font.glyph(2), &[0xFF; 8]);
        assert_eq!(font.glyph(3), &[0u8; 8]);
        assert_eq!(font.glyph(176), &[0u8; 8]);
    }

    #[test]
    fn partial_final_glyph_zero_pads_its_tail() {
        // 3 full glyphs plus 5 stray bytes of a 4th.
        let mut data = vec![0xAAu8; 3 * GLYPH_BYTES];
        data.extend_from_slice(&[1, 2, 3, 4, 5]);
        let font = decode(&data);
        assert_eq!(font.glyph(3), &[1, 2, 3, 4, 5, 0, 0, 0]);
    }

    #[test]
    fn empty_block_decodes_to_all_zero_glyphs() {
        let font = decode(&[]);
        for i in 0..GLYPH_COUNT {
            assert_eq!(font.glyph(i), &[0u8; 8]);
        }
    }

    #[test]
    fn extra_trailing_bytes_beyond_177_glyphs_are_ignored() {
        let mut data = full_size_block();
        data.extend_from_slice(&[0xEE; 100]); // must not panic or affect anything
        let font = decode(&data);
        assert_eq!(font.glyph(176), &[176u8; 8]);
    }

    #[test]
    #[should_panic]
    fn out_of_range_index_panics_as_a_caller_bug() {
        let font = decode(&full_size_block());
        font.glyph(GLYPH_COUNT);
    }

    /// Local-only tier (pattern from `dax.rs`/`geo.rs`): `8X8D1.DAX` block
    /// 201 in the real data set is a full, untruncated `GLYPH_COUNT *
    /// GLYPH_BYTES`-byte table — the task brief's "the font block yields
    /// 177 glyphs" requirement (all 177 populated from real data, not
    /// zero-padded).
    #[test]
    fn real_font_block_is_full_size_and_yields_177_glyphs() {
        use crate::dax::DaxArchive;
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        let path = dir.join("8X8D1.DAX");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("{}: failed to read: {e}", path.display()));
        let archive = DaxArchive::parse(&bytes)
            .unwrap_or_else(|e| panic!("{}: failed to parse DAX: {e:?}", path.display()));
        let raw = archive
            .block_data(201)
            .unwrap_or_else(|e| panic!("{}: block 201 failed to extract: {e:?}", path.display()));

        assert_eq!(
            raw.len(),
            GLYPH_COUNT * GLYPH_BYTES,
            "real font block is not the full {} bytes — some glyphs would be zero-padded",
            GLYPH_COUNT * GLYPH_BYTES
        );
        let font = decode(&raw);
        // Every glyph slot is reachable and, for a full-size block, every
        // glyph is populated from real bytes (not the zero-pad default).
        for i in 0..GLYPH_COUNT {
            let _ = font.glyph(i);
        }
        eprintln!("real font block: {GLYPH_COUNT} glyphs, {} bytes", raw.len());
    }
}
