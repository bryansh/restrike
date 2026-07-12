//! Walldef blocks (`WALLDEF*`): the wall-texture-id tables the 3D corridor
//! renderer selects 8×8 symbols from. Pure over bytes — no filesystem
//! access.
//!
//! Derived by reading coab for behavior (D11, never copied) and
//! cross-checked against an independent reimplementation:
//! - coab `Classes/GeoBlock.cs` (`WallDefs`/`WallDefBlock`, `:29-91`) — the
//!   780-bytes-per-wallset framing (`WallDefs.LoadData`'s `_data.Length /
//!   780` block count) and the flat `[5, 156]` (style, tile-id) layout
//!   (`WallDefBlock.LoadData`/`Id`).
//! - Gold Box Explorer (C#) `Common/Plugins/Dax/DaxWallDefFile.cs`
//!   (`loadWallDefs`, `:204-239`) — an independently maintained decoder that
//!   iterates the identical `156`-byte style slices (its `wallSliceSize`)
//!   out of the same raw block, corroborating the byte layout
//!   (`docs/design/renderer-ui-shell.md` D-UI5's cited cross-check).
//!
//! ## On-disk layout (`docs/design/renderer-ui-shell.md` §1.7)
//!
//! A walldef block is `780 * n` bytes for some `n >= 0` ("wallset" count);
//! each 780-byte wallset is `5` styles (distance/facing slices) of `156`
//! raw tile-id bytes each, flat, no header:
//!
//! ```text
//! wallset 0: style 0 (156 bytes) | style 1 (156) | ... | style 4 (156)
//! wallset 1: style 0 (156 bytes) | ...
//! ...
//! ```
//!
//! **Explicitly out of scope** (renderer/engine concerns per the design
//! doc, not baked into this decoder):
//! - The ten draw-cell class windows (`idxOffset`/`colCount`/`rowCount`)
//!   that carve each 156-byte style slice into the far/mid/near wall
//!   pieces a given screen cell draws from — §1.7's `Column_A..Row_J`
//!   tables.
//! - The `>= 0x2D` id rebase (`WallDefBlock.Offset`) computed once per
//!   `LoadWalldef` call from the *base* symbol set (§1.3) — this decoder
//!   returns raw on-disk tile ids, unrebased.
//! - Which wallset(s) a given `WALLDEF*.DAX` block id pairs with which
//!   `8X8D*` block id(s) (the `blockId`/`blockId*10+n` convention, §1.3) —
//!   that's `LoadWalldef`'s job, not this module's.

/// Styles (distance/facing slices) per wallset.
pub const STYLES_PER_WALLSET: usize = 5;
/// Raw tile-id bytes per style.
pub const TILE_IDS_PER_STYLE: usize = 156;
/// Bytes per wallset (`STYLES_PER_WALLSET * TILE_IDS_PER_STYLE`).
pub const WALLSET_SIZE: usize = STYLES_PER_WALLSET * TILE_IDS_PER_STYLE;

/// A parsed walldef block: zero or more wallsets, each a fixed
/// `[style][idx]` grid of raw tile ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalldefBlock {
    // Flat storage: wallset `w`, style `s`, idx `i` lives at
    // `w * WALLSET_SIZE + s * TILE_IDS_PER_STYLE + i`.
    data: Vec<u8>,
}

/// [`WalldefBlock::parse`]'s failure mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalldefError {
    /// The input length isn't a multiple of [`WALLSET_SIZE`] (780 bytes) —
    /// a truncated or malformed block.
    NotAWholeNumberOfWallsets { len: usize },
}

impl WalldefBlock {
    /// Parses a walldef block's raw (already DAX-decompressed) bytes. A
    /// zero-length input is valid (zero wallsets); any other length must be
    /// an exact multiple of [`WALLSET_SIZE`].
    pub fn parse(data: &[u8]) -> Result<Self, WalldefError> {
        if !data.len().is_multiple_of(WALLSET_SIZE) {
            return Err(WalldefError::NotAWholeNumberOfWallsets { len: data.len() });
        }
        Ok(WalldefBlock {
            data: data.to_vec(),
        })
    }

    /// How many wallsets this block holds (`data.len() / 780`).
    pub fn wallset_count(&self) -> usize {
        self.data.len() / WALLSET_SIZE
    }

    /// The raw tile id at `(wallset, style, idx)` — `style < 5`, `idx <
    /// 156`. `None` if any index is out of range (unlike the on-disk
    /// bounds which are a decode-time contract, out-of-range accessor
    /// calls are a caller error we report rather than panic on, since
    /// `wallset_count` is data-dependent and easy to get wrong from a
    /// renderer call site).
    pub fn tile_id(&self, wallset: usize, style: usize, idx: usize) -> Option<u8> {
        if style >= STYLES_PER_WALLSET
            || idx >= TILE_IDS_PER_STYLE
            || wallset >= self.wallset_count()
        {
            return None;
        }
        let offset = wallset * WALLSET_SIZE + style * TILE_IDS_PER_STYLE + idx;
        self.data.get(offset).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-authored (D10): builds one wallset's 780 bytes where byte
    /// `(style, idx)` equals `(style * 156 + idx) % 256` for easy
    /// identification.
    fn synthetic_wallset() -> Vec<u8> {
        (0..WALLSET_SIZE as u32).map(|i| (i % 256) as u8).collect()
    }

    #[test]
    fn single_wallset_parses_and_reports_count_one() {
        let block = WalldefBlock::parse(&synthetic_wallset()).unwrap();
        assert_eq!(block.wallset_count(), 1);
    }

    #[test]
    fn tile_id_reads_expected_style_idx_layout() {
        let block = WalldefBlock::parse(&synthetic_wallset()).unwrap();
        assert_eq!(block.tile_id(0, 0, 0), Some(0));
        assert_eq!(block.tile_id(0, 0, 1), Some(1));
        assert_eq!(block.tile_id(0, 1, 0), Some(156));
        assert_eq!(
            block.tile_id(0, 4, 155),
            Some(((4 * 156 + 155) % 256) as u8)
        );
    }

    #[test]
    fn multi_wallset_block_indexes_each_wallset_independently() {
        let mut data = synthetic_wallset();
        // A second wallset, all bytes 0xFF, so it's trivially distinguishable.
        data.extend_from_slice(&[0xFFu8; WALLSET_SIZE]);
        let block = WalldefBlock::parse(&data).unwrap();
        assert_eq!(block.wallset_count(), 2);
        assert_eq!(block.tile_id(0, 0, 0), Some(0));
        assert_eq!(block.tile_id(1, 0, 0), Some(0xFF));
        assert_eq!(block.tile_id(1, 4, 155), Some(0xFF));
    }

    #[test]
    fn three_wallset_block_parses() {
        let mut data = synthetic_wallset();
        data.extend_from_slice(&[1u8; WALLSET_SIZE]);
        data.extend_from_slice(&[2u8; WALLSET_SIZE]);
        let block = WalldefBlock::parse(&data).unwrap();
        assert_eq!(block.wallset_count(), 3);
        assert_eq!(block.tile_id(2, 0, 0), Some(2));
    }

    #[test]
    fn empty_block_is_valid_zero_wallsets() {
        let block = WalldefBlock::parse(&[]).unwrap();
        assert_eq!(block.wallset_count(), 0);
        assert_eq!(block.tile_id(0, 0, 0), None);
    }

    #[test]
    fn truncated_block_errors_cleanly() {
        let mut data = synthetic_wallset();
        data.truncate(WALLSET_SIZE - 1);
        let err = WalldefBlock::parse(&data).unwrap_err();
        assert_eq!(
            err,
            WalldefError::NotAWholeNumberOfWallsets {
                len: WALLSET_SIZE - 1
            }
        );
    }

    #[test]
    fn out_of_range_lookup_returns_none_not_panic() {
        let block = WalldefBlock::parse(&synthetic_wallset()).unwrap();
        assert_eq!(block.tile_id(1, 0, 0), None); // wallset out of range
        assert_eq!(block.tile_id(0, 5, 0), None); // style out of range
        assert_eq!(block.tile_id(0, 0, 156), None); // idx out of range
    }

    /// Local-only tier (pattern from `dax.rs`/`geo.rs`): every `WALLDEF*`
    /// block in the real data set parses cleanly, plus the design doc's
    /// docket item 11 (§1.11 item 8/§4 item 11): whether block id 0 is ever
    /// multi-wallset in real CotAB data, deciding between GBE's `block_id
    /// == 0 -> base 100` special case and coab's unconditional `*10`
    /// pairing formula. Finding recorded in this test and in
    /// `docs/design/renderer-ui-shell.md` §4 item 11: across all six
    /// `WALLDEF{2..6}.DAX` files in the real CotAB data set, block id `0`
    /// never appears at all (observed ids: 1-4, 8-14, 16-17) — the
    /// contradiction is moot for this data set; LOAD FILES' `0x7F` ->
    /// `LoadWalldef(1, 0)` path is a live *code* path but not exercised by
    /// any block actually shipped for CotAB.
    #[test]
    fn every_real_walldef_block_parses_and_block_zero_is_absent() {
        use crate::dax::DaxArchive;
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);

        let mut blocks_checked = 0usize;
        let mut block_zero_found = false;
        for entry in std::fs::read_dir(dir).expect("GBX_DATA_DIR must be readable") {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_ascii_uppercase();
            if !(name.starts_with("WALLDEF") && name.ends_with(".DAX")) {
                continue;
            }

            let bytes = std::fs::read(&path).unwrap();
            let archive = DaxArchive::parse(&bytes)
                .unwrap_or_else(|e| panic!("{}: failed to parse DAX: {e:?}", path.display()));
            for block_entry in archive.entries() {
                let raw = archive.block_data(block_entry.id).unwrap_or_else(|e| {
                    panic!(
                        "{}: block {} failed to extract: {e:?}",
                        path.display(),
                        block_entry.id
                    )
                });
                let block = WalldefBlock::parse(&raw).unwrap_or_else(|e| {
                    panic!(
                        "{}: block {} failed to parse as a walldef: {e:?}",
                        path.display(),
                        block_entry.id
                    )
                });
                if block_entry.id == 0 {
                    block_zero_found = true;
                    eprintln!(
                        "{}: WALLDEF block 0 found — {} wallset(s)",
                        path.display(),
                        block.wallset_count()
                    );
                }
                blocks_checked += 1;
            }
        }

        assert!(
            blocks_checked > 0,
            "GBX_DATA_DIR is set but no WALLDEF*.DAX blocks were found in it"
        );
        eprintln!(
            "checked {blocks_checked} real walldef block(s); WALLDEF block 0 present: {block_zero_found}"
        );
        assert!(
            !block_zero_found,
            "WALLDEF block 0 appeared in real data — the design doc's docket item 11 finding \
             (block 0 is absent from CotAB's real data) needs updating, not this assertion"
        );
    }
}
