//! The DAX archive container: a flat, indexed, RLE-compressed block store
//! used for every Gold Box resource file (`ECL*.DAX`, `TITLE.DAX`, tile/pic
//! sets, ...). This module is pure over bytes (`parse(&[u8])`) — no
//! filesystem access, so it stays usable from `wasm32` and from tests
//! without a real data directory.
//!
//! Derived by reading two independent implementations for behavior (D11,
//! never copied) and cross-checking them against each other:
//! - coab `Classes/DaxFiles/DaxFileCache.cs` (`LoadFile`/`Decode`) and
//!   `DaxHeaderEntry.cs` — the header/index layout and the RLE algorithm.
//! - ssi-engine (Java, GPL-3) `data/DAXFile.java` (`createFrom`/`uncompress`)
//!   — an independent transliteration of the same binary format. It agrees
//!   with coab byte-for-byte on header layout and RLE semantics, which is
//!   the cross-check this format needed (task instructions, D11): two
//!   unrelated reference implementations converging on the same behavior is
//!   strong evidence the format is understood correctly, not just that one
//!   source was transcribed faithfully.
//!
//! DaxDump.exe/EclDump.exe goldens (H1) are DEFERRED to the oracle-rig
//! milestone (Windows binaries, not runnable yet) — not used here.
//!
//! ## Layout
//!
//! ```text
//! offset 0..2   u16 LE  header_bytes  (total size of the entry table, in bytes)
//! offset 2..    entry table: header_bytes/9 entries (any remainder bytes are
//!               ignored — the original computes entry count via floor
//!               division, `DaxFileCache.cs:46`/`DAXFile.java:24`)
//!   each entry (9 bytes):
//!     u8      id
//!     u32 LE  offset   (relative to the start of the data area)
//!     u16 LE  raw_size   (decompressed size)
//!     u16 LE  comp_size  (compressed size)
//! data area starts at `2 + header_bytes`; block `offset` is relative to it.
//! ```
//!
//! Each block's compressed bytes are a byte-oriented RLE stream: each run
//! starts with a signed control byte —
//! - `>= 0`: a **literal run** of `control + 1` bytes follow verbatim.
//! - `< 0`: a **repeat run**: the next single byte is repeated `-control`
//!   times.
//!
//! `control == -128` (`0x80`) is a **degenerate encoding** we treat as
//! malformed rather than replicate: negating `i8::MIN` overflows a signed
//! byte, and the original's unchecked C# cast (`(sbyte)(-run_length)`)
//! leaves `run_length` at `-128` — the repeat loop then never executes
//! (`i < -128` is false at `i == 0`) *and* `output_index += run_length`
//! walks the output cursor backward by 128, corrupting every run after it.
//! No legitimate encoder emits this (a repeat run's natural encoding range
//! is `1..=127`, well inside a signed byte without wrapping), so real DAX
//! files are not expected to contain it; on synthetic malformed input we
//! report an error instead of reproducing a buffer-corrupting bug.

use std::collections::HashSet;

/// One entry in a DAX file's block index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockEntry {
    pub id: u8,
    pub offset: u32,
    pub raw_size: u16,
    pub comp_size: u16,
}

/// Everything that can go wrong parsing a DAX container or extracting one of
/// its blocks. Every variant is a clean `Err`, never a panic (fuzz posture,
/// PLAN M1) — malformed/truncated/adversarial input is expected input, not a
/// bug to crash on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaxError {
    /// Fewer than 2 bytes total — can't even read the header-length word.
    TooShortForHeaderLength { len: usize },
    /// The declared header table extends past the end of the file.
    HeaderTruncated { header_bytes: u16, available: usize },
    /// Two entries in the same file declared the same block id. The
    /// original's `Dictionary.Add` throws in this case (`DaxFileCache.cs:68`)
    /// — real files are never expected to hit this; we report it instead of
    /// silently keeping the first (or last) entry.
    DuplicateBlockId { id: u8 },
    /// `block_data(id)` was called for an id not present in this archive.
    UnknownBlockId { id: u8 },
    /// A block's `offset + comp_size` overflowed `usize` address arithmetic.
    BlockOffsetOverflow { id: u8 },
    /// A block's compressed span falls outside the file.
    BlockOffsetOutOfBounds {
        id: u8,
        start: usize,
        end: usize,
        file_len: usize,
    },
    /// A literal run's control byte declared more bytes than remain in the
    /// compressed stream.
    TruncatedLiteralRun { id: u8, comp_offset: usize },
    /// A repeat run's control byte has no following byte to repeat.
    TruncatedRepeatRun { id: u8, comp_offset: usize },
    /// Control byte `0x80` (`i8::MIN`) — see the module doc's "degenerate
    /// encoding" note.
    DegenerateRunLength { id: u8, comp_offset: usize },
    /// Decompression produced more bytes than the block's declared
    /// `raw_size` — the compressed stream disagrees with its own header.
    DecompressedOutputOverflow { id: u8, comp_offset: usize },
}

const HEADER_ENTRY_SIZE: usize = 9;

/// A parsed DAX container: the block index, borrowed over the source bytes.
/// Block payloads are decompressed on demand via [`DaxArchive::block_data`],
/// not eagerly — callers that only want the index (e.g. a directory census
/// counting blocks per file) never pay for decompression.
#[derive(Debug, Clone)]
pub struct DaxArchive<'a> {
    data: &'a [u8],
    data_base: usize,
    entries: Vec<BlockEntry>,
}

impl<'a> DaxArchive<'a> {
    /// Parses a DAX container's index (header + entry table) from `data`.
    /// Does not decompress any block payload — see [`block_data`](Self::block_data).
    pub fn parse(data: &'a [u8]) -> Result<Self, DaxError> {
        if data.len() < 2 {
            return Err(DaxError::TooShortForHeaderLength { len: data.len() });
        }
        let header_bytes = u16::from_le_bytes([data[0], data[1]]);
        let data_base = 2 + header_bytes as usize;
        if data.len() < data_base {
            return Err(DaxError::HeaderTruncated {
                header_bytes,
                available: data.len() - 2,
            });
        }

        let entry_count = header_bytes as usize / HEADER_ENTRY_SIZE;
        let mut entries = Vec::with_capacity(entry_count);
        let mut seen = HashSet::with_capacity(entry_count);
        for i in 0..entry_count {
            let start = 2 + i * HEADER_ENTRY_SIZE;
            let id = data[start];
            let offset = u32::from_le_bytes(data[start + 1..start + 5].try_into().unwrap());
            let raw_size = u16::from_le_bytes(data[start + 5..start + 7].try_into().unwrap());
            let comp_size = u16::from_le_bytes(data[start + 7..start + 9].try_into().unwrap());
            if !seen.insert(id) {
                return Err(DaxError::DuplicateBlockId { id });
            }
            entries.push(BlockEntry {
                id,
                offset,
                raw_size,
                comp_size,
            });
        }

        Ok(Self {
            data,
            data_base,
            entries,
        })
    }

    /// The archive's block index, in on-disk order.
    pub fn entries(&self) -> &[BlockEntry] {
        &self.entries
    }

    /// Decompresses and returns the payload of block `id`.
    pub fn block_data(&self, id: u8) -> Result<Vec<u8>, DaxError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or(DaxError::UnknownBlockId { id })?;

        let start = self
            .data_base
            .checked_add(entry.offset as usize)
            .ok_or(DaxError::BlockOffsetOverflow { id })?;
        let end = start
            .checked_add(entry.comp_size as usize)
            .ok_or(DaxError::BlockOffsetOverflow { id })?;
        let comp = self
            .data
            .get(start..end)
            .ok_or(DaxError::BlockOffsetOutOfBounds {
                id,
                start,
                end,
                file_len: self.data.len(),
            })?;

        decompress(id, comp, entry.raw_size as usize)
    }
}

/// The RLE decompressor. `raw_size` is the block's declared decompressed
/// size (its output buffer is exactly that big, zero-filled, matching the
/// original's fixed-size `new byte[rawSize]` — an under-running compressed
/// stream leaves the remainder zeroed rather than erroring, exactly as the
/// original does).
fn decompress(id: u8, comp: &[u8], raw_size: usize) -> Result<Vec<u8>, DaxError> {
    let mut out = vec![0u8; raw_size];
    let mut out_pos = 0usize;
    let mut i = 0usize;

    while i < comp.len() {
        let comp_offset = i;
        let ctrl = comp[i] as i8;
        i += 1;

        if ctrl == i8::MIN {
            return Err(DaxError::DegenerateRunLength { id, comp_offset });
        }

        if ctrl >= 0 {
            let count = ctrl as usize + 1;
            let end = i
                .checked_add(count)
                .filter(|&e| e <= comp.len())
                .ok_or(DaxError::TruncatedLiteralRun { id, comp_offset })?;
            let dst_end = out_pos
                .checked_add(count)
                .filter(|&e| e <= raw_size)
                .ok_or(DaxError::DecompressedOutputOverflow { id, comp_offset })?;
            out[out_pos..dst_end].copy_from_slice(&comp[i..end]);
            out_pos = dst_end;
            i = end;
        } else {
            let count = (-ctrl) as usize;
            let repeat = *comp
                .get(i)
                .ok_or(DaxError::TruncatedRepeatRun { id, comp_offset })?;
            i += 1;
            let dst_end = out_pos
                .checked_add(count)
                .filter(|&e| e <= raw_size)
                .ok_or(DaxError::DecompressedOutputOverflow { id, comp_offset })?;
            out[out_pos..dst_end].fill(repeat);
            out_pos = dst_end;
        }
    }

    Ok(out)
}

/// `ECL*.DAX` block payloads carry two leading bytes that `load_ecl_dax`
/// strips before copying into the resident `0x1E00`-byte block buffer
/// (coab `engine/ovr008.cs:151`: `gbl.ecl_ptr.SetData(block_mem, 2,
/// block_size - 2)`). The exact meaning of these two bytes is undetermined
/// (docket candidate) — they are skipped uniformly here, matching the
/// original exactly. Returns an empty slice if `decompressed` is shorter
/// than 2 bytes rather than panicking; real data is never this short.
pub fn ecl_block_payload(decompressed: &[u8]) -> &[u8] {
    decompressed.get(2..).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-authored (D10): builds a synthetic DAX file's bytes from a list
    /// of `(id, raw_bytes)` pairs, RLE-encoding each block as a single
    /// literal run (or several, if longer than 128 bytes).
    fn build_dax(blocks: &[(u8, &[u8])]) -> Vec<u8> {
        let header_bytes = blocks.len() * HEADER_ENTRY_SIZE;
        let mut data_area = Vec::new();
        let mut entries = Vec::new();
        for &(id, raw) in blocks {
            let offset = data_area.len() as u32;
            let comp = rle_encode_literal(raw);
            let comp_size = comp.len() as u16;
            data_area.extend_from_slice(&comp);
            entries.push((id, offset, raw.len() as u16, comp_size));
        }

        let mut out = Vec::new();
        out.extend_from_slice(&(header_bytes as u16).to_le_bytes());
        for (id, offset, raw_size, comp_size) in entries {
            out.push(id);
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&raw_size.to_le_bytes());
            out.extend_from_slice(&comp_size.to_le_bytes());
        }
        out.extend_from_slice(&data_area);
        out
    }

    /// Encodes `raw` as one or more literal runs (control byte `len-1`),
    /// splitting at the 128-byte-per-run limit (control byte range `0..=127`
    /// means a literal run covers at most 128 bytes).
    fn rle_encode_literal(raw: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in raw.chunks(128) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
        out
    }

    #[test]
    fn parses_header_and_lists_entries() {
        let bytes = build_dax(&[(1, b"hello"), (2, b"world!")]);
        let archive = DaxArchive::parse(&bytes).unwrap();
        assert_eq!(archive.entries().len(), 2);
        assert_eq!(archive.entries()[0].id, 1);
        assert_eq!(archive.entries()[1].id, 2);
    }

    #[test]
    fn extracts_block_data_by_id() {
        let bytes = build_dax(&[(5, b"CURSE"), (9, b"BONDS")]);
        let archive = DaxArchive::parse(&bytes).unwrap();
        assert_eq!(archive.block_data(5).unwrap(), b"CURSE");
        assert_eq!(archive.block_data(9).unwrap(), b"BONDS");
    }

    #[test]
    fn empty_block_round_trips() {
        let bytes = build_dax(&[(1, b"")]);
        let archive = DaxArchive::parse(&bytes).unwrap();
        assert_eq!(archive.block_data(1).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn max_positive_literal_run_length() {
        // control byte 0x7F -> a 128-byte literal run, the largest single
        // literal run the format can encode.
        let raw: Vec<u8> = (0..128u16).map(|n| (n % 256) as u8).collect();
        let bytes = build_dax(&[(1, &raw)]);
        let archive = DaxArchive::parse(&bytes).unwrap();
        assert_eq!(archive.block_data(1).unwrap(), raw);
    }

    #[test]
    fn max_well_formed_repeat_run_length() {
        // control byte 0x81 as i8 is -127 -> repeat 127 times; the largest
        // repeat run that avoids the i8::MIN degenerate case.
        let comp = vec![0x81u8, 0xAB];
        let raw_size = 127usize;
        let mut file = Vec::new();
        file.extend_from_slice(&9u16.to_le_bytes()); // one header entry
        file.push(7); // id
        file.extend_from_slice(&0u32.to_le_bytes()); // offset
        file.extend_from_slice(&(raw_size as u16).to_le_bytes());
        file.extend_from_slice(&(comp.len() as u16).to_le_bytes());
        file.extend_from_slice(&comp);

        let archive = DaxArchive::parse(&file).unwrap();
        let data = archive.block_data(7).unwrap();
        assert_eq!(data, vec![0xABu8; 127]);
    }

    #[test]
    fn degenerate_run_length_0x80_errors_cleanly() {
        let comp = vec![0x80u8, 0xFF];
        let mut file = Vec::new();
        file.extend_from_slice(&9u16.to_le_bytes());
        file.push(3);
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&10u16.to_le_bytes()); // raw_size
        file.extend_from_slice(&(comp.len() as u16).to_le_bytes());
        file.extend_from_slice(&comp);

        let archive = DaxArchive::parse(&file).unwrap();
        let err = archive.block_data(3).unwrap_err();
        assert_eq!(
            err,
            DaxError::DegenerateRunLength {
                id: 3,
                comp_offset: 0
            }
        );
    }

    #[test]
    fn truncated_file_header_length_errors_cleanly() {
        // Declares a 9-byte header table but the file has no bytes after
        // the header-length word.
        let bytes = 9u16.to_le_bytes().to_vec();
        let err = DaxArchive::parse(&bytes).unwrap_err();
        assert_eq!(
            err,
            DaxError::HeaderTruncated {
                header_bytes: 9,
                available: 0
            }
        );
    }

    #[test]
    fn file_shorter_than_two_bytes_errors_cleanly() {
        let err = DaxArchive::parse(&[0x01]).unwrap_err();
        assert_eq!(err, DaxError::TooShortForHeaderLength { len: 1 });
    }

    #[test]
    fn index_pointing_past_eof_errors_cleanly() {
        let mut file = Vec::new();
        file.extend_from_slice(&9u16.to_le_bytes());
        file.push(1); // id
        file.extend_from_slice(&1000u32.to_le_bytes()); // offset way past EOF
        file.extend_from_slice(&4u16.to_le_bytes()); // raw_size
        file.extend_from_slice(&4u16.to_le_bytes()); // comp_size
                                                     // no data area at all

        let archive = DaxArchive::parse(&file).unwrap();
        let err = archive.block_data(1).unwrap_err();
        assert!(matches!(
            err,
            DaxError::BlockOffsetOutOfBounds { id: 1, .. }
        ));
    }

    #[test]
    fn truncated_compressed_stream_errors_cleanly() {
        // control byte declares a 5-byte literal run but only 2 bytes follow.
        let comp = vec![0x04u8, 0xAA, 0xBB];
        let mut file = Vec::new();
        file.extend_from_slice(&9u16.to_le_bytes());
        file.push(1);
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&5u16.to_le_bytes());
        file.extend_from_slice(&(comp.len() as u16).to_le_bytes());
        file.extend_from_slice(&comp);

        let archive = DaxArchive::parse(&file).unwrap();
        let err = archive.block_data(1).unwrap_err();
        assert!(matches!(
            err,
            DaxError::TruncatedLiteralRun {
                id: 1,
                comp_offset: 0
            }
        ));
    }

    #[test]
    fn truncated_repeat_run_errors_cleanly() {
        let comp = vec![0xFFu8]; // -1 -> repeat 1 time, but no byte follows
        let mut file = Vec::new();
        file.extend_from_slice(&9u16.to_le_bytes());
        file.push(1);
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&(comp.len() as u16).to_le_bytes());
        file.extend_from_slice(&comp);

        let archive = DaxArchive::parse(&file).unwrap();
        let err = archive.block_data(1).unwrap_err();
        assert!(matches!(
            err,
            DaxError::TruncatedRepeatRun {
                id: 1,
                comp_offset: 0
            }
        ));
    }

    #[test]
    fn duplicate_block_id_errors_cleanly() {
        let mut file = Vec::new();
        file.extend_from_slice(&18u16.to_le_bytes()); // two entries
        for _ in 0..2 {
            file.push(1); // same id twice
            file.extend_from_slice(&0u32.to_le_bytes());
            file.extend_from_slice(&0u16.to_le_bytes());
            file.extend_from_slice(&0u16.to_le_bytes());
        }
        let err = DaxArchive::parse(&file).unwrap_err();
        assert_eq!(err, DaxError::DuplicateBlockId { id: 1 });
    }

    #[test]
    fn unknown_block_id_errors_cleanly() {
        let bytes = build_dax(&[(1, b"x")]);
        let archive = DaxArchive::parse(&bytes).unwrap();
        assert_eq!(
            archive.block_data(99).unwrap_err(),
            DaxError::UnknownBlockId { id: 99 }
        );
    }

    #[test]
    fn header_byte_count_not_a_multiple_of_nine_ignores_remainder() {
        // 13 declared header bytes -> floor(13/9) = 1 entry; the trailing 4
        // bytes are skipped, matching the original's integer division
        // (`DaxFileCache.cs:46`, `DAXFile.java:24`) rather than erroring.
        let mut file = Vec::new();
        file.extend_from_slice(&13u16.to_le_bytes());
        file.push(1);
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes()); // raw_size
        file.extend_from_slice(&2u16.to_le_bytes()); // comp_size
        file.extend_from_slice(&[0, 0, 0, 0]); // 4 ignored trailing header bytes
        file.extend_from_slice(&[0x00, 0xAB]); // one-byte literal run

        let archive = DaxArchive::parse(&file).unwrap();
        assert_eq!(archive.entries().len(), 1);
        assert_eq!(archive.block_data(1).unwrap(), vec![0xAB]);
    }

    #[test]
    fn ecl_block_payload_skips_two_leading_bytes() {
        assert_eq!(ecl_block_payload(&[0xAA, 0xBB, 1, 2, 3]), &[1, 2, 3]);
        assert_eq!(ecl_block_payload(&[0xAA, 0xBB]), &[] as &[u8]);
        assert_eq!(ecl_block_payload(&[0xAA]), &[] as &[u8]);
        assert_eq!(ecl_block_payload(&[]), &[] as &[u8]);
    }

    /// Local-only tier (pattern from `detect.rs`): exercises the parser and
    /// decompressor against every real `*.DAX` file in `GBX_DATA_DIR`, and
    /// checks the D-VM2/task-brief invariant that every `ECL*.DAX` block's
    /// payload (after stripping the two leading bytes, see
    /// [`ecl_block_payload`]) fits the `0x1E00`-byte resident block size.
    /// Silently passes when `GBX_DATA_DIR` is unset — public CI never sees
    /// game data (D10).
    #[test]
    fn every_real_dax_file_parses_and_every_block_extracts() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        let mut files_checked = 0usize;
        let mut blocks_checked = 0usize;

        for entry in std::fs::read_dir(dir).expect("GBX_DATA_DIR must be readable") {
            let entry = entry.unwrap();
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let is_dax = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("dax"));
            if !is_dax {
                continue;
            }

            let bytes = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("{}: failed to read: {e}", path.display()));
            let archive = DaxArchive::parse(&bytes)
                .unwrap_or_else(|e| panic!("{}: failed to parse: {e:?}", path.display()));

            let is_ecl = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.to_ascii_uppercase().starts_with("ECL"));

            for block in archive.entries() {
                let data = archive.block_data(block.id).unwrap_or_else(|e| {
                    panic!(
                        "{}: block {} failed to extract: {e:?}",
                        path.display(),
                        block.id
                    )
                });
                if is_ecl {
                    let payload = ecl_block_payload(&data);
                    assert!(
                        payload.len() <= 0x1E00,
                        "{}: block {} ECL payload is {} bytes, exceeding the 0x1E00-byte \
                         resident block size",
                        path.display(),
                        block.id,
                        payload.len()
                    );
                }
                blocks_checked += 1;
            }
            files_checked += 1;
        }

        assert!(
            files_checked > 0,
            "GBX_DATA_DIR is set but no *.DAX files were found in it"
        );
        eprintln!("checked {files_checked} DAX file(s), {blocks_checked} block(s)");
    }
}
