//! EXEPACK decompression: `docs/design/rules-packs.md` §1.2's normative
//! decode contract. `START.EXE` (and any other EXEPACK-packed original
//! binary a rules pack anchors into) ships LZ-free run-length-compressed —
//! `LINK /EXEPACK`'s format, not this project's invention — and the rules
//! packs' `anchor = { kind = "image", ... }` offsets are recorded against
//! the *decompressed* image, so verification needs a decoder before a
//! single byte comparison is possible.
//!
//! This module is an **untrusted-input parser**: `START.EXE` is
//! user-supplied game data read at every boot (PLAN.md M1's fuzz-roster
//! convention, restated by the design doc's §1.2 closing paragraph), so
//! every arithmetic step here is bounds-checked and every malformed-input
//! path returns a typed [`ExepackError`] — never a panic.
//!
//! ## The format, concretely
//!
//! The packed EXE is a normal MZ executable whose header fields are
//! repurposed to locate an 18-byte EXEPACK header glued onto the end of the
//! compressed data:
//!
//! ```text
//! MZ header (>= 24 bytes read here):
//!   0x00  2s   e_magic       "MZ"
//!   0x08  u16  e_cparhdr     header size, in 16-byte paragraphs
//!   0x14  u16  e_ip          EXEPACK header length (16 or 18)
//!   0x16  u16  e_cs          EXEPACK header offset, in paragraphs from body
//!
//! body = e_cparhdr * 16                    (the load module's start)
//! header_offset = body + e_cs * 16
//!
//! EXEPACK header at header_offset (18-byte form; this codebase treats the
//! 16-byte form, which omits skip_len, as an equally normative variant with
//! skip_len implicitly 1):
//!   +0x00  u16  real_ip
//!   +0x02  u16  real_cs
//!   +0x04  u16  (unused)
//!   +0x06  u16  exepack_size    (unused by decode)
//!   +0x08  u16  real_sp
//!   +0x0A  u16  real_ss
//!   +0x0C  u16  dest_len        decompressed size, in paragraphs
//!   +0x0E  u16  skip_len        (18-byte form only; hard-errors unless 1)
//!   +0x10  2s   signature       "RB"
//! ```
//!
//! The packed stream is `data[body..header_offset]`. It ends in a run of
//! `0xFF` pad bytes (skipped before the first backwards read) and decodes
//! **back to front**: each opcode is a command byte (`0xB0`/`0xB1` = fill,
//! `0xB2`/`0xB3` = copy; the low bit marks the final opcode) followed
//! (moving backward) by a 16-bit little-endian count, then — for fill only
//! — a single fill byte. `fill` writes `count` copies of that byte; `copy`
//! copies `count` bytes verbatim from the packed stream itself (which,
//! having been produced by a backwards LZ-free encoder, holds them as
//! literal bytes immediately before the count field). Both opcodes advance
//! the read cursor (`src`, into the packed stream) and write cursor (`dst`,
//! into the `dest_len`-paragraph output image) backward by `count`.
//!
//! When the final-bit opcode completes, `src` must equal `dst` — the
//! stream's only cheap integrity check (a decoder that ignores this and
//! keeps going passes naive length/spot-check tests while silently
//! producing a corrupt low image, per the design doc's round-2 review
//! finding). The output image is then `packed[..src]` (the **raw prefix**,
//! kept in place unmodified — 877 bytes on the real v1.3 `START.EXE`)
//! followed by the backwards-decoded tail already written into `out`.

/// [`decode`]'s failure mode. Every variant corresponds to one of the
/// decode contract's hard-error cases (§1.2); none of them are recoverable
/// by the caller beyond reporting (typically as
/// `VerifyStatus::ImageUndecodable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExepackError {
    /// Fewer than 24 bytes, so even the MZ header fields this decoder reads
    /// don't fit.
    TooShortForMzHeader,
    /// The first two bytes aren't `"MZ"`.
    NotAnMzExe,
    /// The EXEPACK header (16 or 18 bytes starting at `body + e_cs * 16`)
    /// doesn't fit inside the file, or `e_ip` isn't 16 or 18.
    TruncatedHeader,
    /// The 18-byte header's trailing 2 bytes (or the 16-byte header's) are
    /// not the `"RB"` signature — this file isn't EXEPACK-packed at all.
    NotExepacked,
    /// `skip_len != 1` — normative per §1.2 until a binary that uses it
    /// appears (design doc §5 item 7).
    UnsupportedSkipLen { skip_len: u16 },
    /// The backwards opcode stream ran off the start of the packed data
    /// before reaching a final-bit opcode, or an opcode's `count` reaches
    /// past the start of the output buffer.
    Truncated,
    /// A command byte outside `{0xB0, 0xB1, 0xB2, 0xB3}`.
    UnknownOpcode { opcode: u8 },
    /// The final-bit opcode completed with `src != dst` — the decode
    /// contract's cheap integrity check failed, meaning either this isn't a
    /// valid EXEPACK stream or the decoder disagrees with the encoder about
    /// something structural.
    SrcDstMismatch { src: usize, dst: usize },
}

fn read_u16_le(data: &[u8], offset: usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(offset..offset + 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

/// Decompresses an EXEPACK-packed DOS executable, per this module's
/// documented contract. Returns the decompressed image
/// (`dest_len * 16` bytes) on success.
pub fn decode(data: &[u8]) -> Result<Vec<u8>, ExepackError> {
    if data.len() < 24 {
        return Err(ExepackError::TooShortForMzHeader);
    }
    if &data[0..2] != b"MZ" {
        return Err(ExepackError::NotAnMzExe);
    }

    let e_cparhdr = read_u16_le(data, 0x08).ok_or(ExepackError::TooShortForMzHeader)? as usize;
    let e_ip = read_u16_le(data, 0x14).ok_or(ExepackError::TooShortForMzHeader)? as usize;
    let e_cs = read_u16_le(data, 0x16).ok_or(ExepackError::TooShortForMzHeader)? as usize;

    let body = e_cparhdr * 16;
    let header_offset = body
        .checked_add(e_cs * 16)
        .ok_or(ExepackError::TruncatedHeader)?;

    if e_ip != 16 && e_ip != 18 {
        return Err(ExepackError::TruncatedHeader);
    }
    let header_end = header_offset
        .checked_add(e_ip)
        .ok_or(ExepackError::TruncatedHeader)?;
    // `body > header_offset` would mean the "header" sits before the load
    // module even starts -- can't happen with sane e_cs, but the check
    // keeps the packed-region slice below from underflowing on hostile
    // input.
    if header_end > data.len() || body > header_offset {
        return Err(ExepackError::TruncatedHeader);
    }

    let dest_len = read_u16_le(data, header_offset + 0x0C).ok_or(ExepackError::TruncatedHeader)?;
    let (skip_len, sig_offset) = if e_ip == 18 {
        let skip_len =
            read_u16_le(data, header_offset + 0x0E).ok_or(ExepackError::TruncatedHeader)?;
        (skip_len, header_offset + 0x10)
    } else {
        (1u16, header_offset + 0x0E)
    };

    let sig = data
        .get(sig_offset..sig_offset + 2)
        .ok_or(ExepackError::TruncatedHeader)?;
    if sig != b"RB" {
        return Err(ExepackError::NotExepacked);
    }
    if skip_len != 1 {
        return Err(ExepackError::UnsupportedSkipLen { skip_len });
    }

    // skip_len == 1 (enforced above), so the packed region runs all the way
    // to header_offset with no trailing skip.
    let packed = &data[body..header_offset];

    let mut pad = 0usize;
    while pad < packed.len() && packed[packed.len() - 1 - pad] == 0xFF {
        pad += 1;
    }
    if pad == packed.len() {
        return Err(ExepackError::Truncated);
    }

    let dest_len_bytes = dest_len as usize * 16;
    let mut out = vec![0u8; dest_len_bytes];

    let mut src = packed.len() - pad;
    let mut dst = dest_len_bytes;

    loop {
        src = src.checked_sub(1).ok_or(ExepackError::Truncated)?;
        let opcode = packed[src];

        src = src.checked_sub(2).ok_or(ExepackError::Truncated)?;
        let count = packed[src] as usize | ((packed[src + 1] as usize) << 8);

        match opcode & 0xFE {
            0xB0 => {
                src = src.checked_sub(1).ok_or(ExepackError::Truncated)?;
                let fill_byte = packed[src];
                dst = dst.checked_sub(count).ok_or(ExepackError::Truncated)?;
                out[dst..dst + count].fill(fill_byte);
            }
            0xB2 => {
                dst = dst.checked_sub(count).ok_or(ExepackError::Truncated)?;
                src = src.checked_sub(count).ok_or(ExepackError::Truncated)?;
                out[dst..dst + count].copy_from_slice(&packed[src..src + count]);
            }
            _ => return Err(ExepackError::UnknownOpcode { opcode }),
        }

        if opcode & 1 != 0 {
            break;
        }
    }

    if src != dst {
        return Err(ExepackError::SrcDstMismatch { src, dst });
    }

    out[..src].copy_from_slice(&packed[..src]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal MZ+EXEPACK file: `body` bytes of packed data,
    /// followed by `pad` trailing `0xFF` bytes, followed by an 18-byte
    /// EXEPACK header declaring `dest_len` paragraphs and `skip_len`.
    /// `packed_payload` is everything in the packed region *before* the
    /// padding (i.e. the encoded opcode stream plus any raw prefix).
    fn build_exe(
        packed_payload: &[u8],
        pad: usize,
        dest_len_paragraphs: u16,
        skip_len: u16,
    ) -> Vec<u8> {
        let mut mz = vec![0u8; 32];
        mz[0..2].copy_from_slice(b"MZ");
        // e_cparhdr = 2 paragraphs (32 bytes) -- just enough for this test
        // builder's synthetic MZ header (up through e_cs at 0x16) to fit
        // entirely before body. Real files have a much bigger header; only
        // the fields this decoder reads matter here.
        let e_cparhdr = 2u16;
        mz[0x08..0x0A].copy_from_slice(&e_cparhdr.to_le_bytes());

        let body = e_cparhdr as usize * 16;
        let mut packed = packed_payload.to_vec();
        packed.extend(std::iter::repeat_n(0xFFu8, pad));

        // EXEPACK header directly follows the packed region; e_cs is in
        // paragraphs from body, so pad packed's length to a paragraph
        // boundary with extra 0xFF (harmless -- still inside the padding
        // that gets skipped) to keep the math exact for the test builder.
        let padded_len = packed.len().div_ceil(16) * 16;
        packed.extend(std::iter::repeat_n(0xFFu8, padded_len - packed.len()));

        let e_cs = (padded_len / 16) as u16;
        mz[0x16..0x18].copy_from_slice(&e_cs.to_le_bytes());
        let e_ip = 18u16;
        mz[0x14..0x16].copy_from_slice(&e_ip.to_le_bytes());

        let mut header = Vec::new();
        header.extend_from_slice(&0u16.to_le_bytes()); // real_ip
        header.extend_from_slice(&0u16.to_le_bytes()); // real_cs
        header.extend_from_slice(&0u16.to_le_bytes()); // unused
        header.extend_from_slice(&0u16.to_le_bytes()); // exepack_size
        header.extend_from_slice(&0u16.to_le_bytes()); // real_sp
        header.extend_from_slice(&0u16.to_le_bytes()); // real_ss
        header.extend_from_slice(&dest_len_paragraphs.to_le_bytes());
        header.extend_from_slice(&skip_len.to_le_bytes());
        header.extend_from_slice(b"RB");
        assert_eq!(header.len(), 18);

        let mut file = mz;
        file.resize(body, 0);
        file.extend_from_slice(&packed);
        file.extend_from_slice(&header);
        file
    }

    /// Encodes a fill opcode's backward-read bytes in forward storage
    /// order: `[fill_byte, count_lo, count_hi, opcode]`.
    fn fill_op(fill_byte: u8, count: u16, last: bool) -> Vec<u8> {
        let mut v = vec![fill_byte];
        v.extend_from_slice(&count.to_le_bytes());
        v.push(0xB0 | if last { 1 } else { 0 });
        v
    }

    /// Encodes a copy opcode: `[<count literal bytes>, count_lo, count_hi, opcode]`.
    fn copy_op(literal: &[u8], last: bool) -> Vec<u8> {
        let mut v = literal.to_vec();
        v.extend_from_slice(&(literal.len() as u16).to_le_bytes());
        v.push(0xB2 | if last { 1 } else { 0 });
        v
    }

    #[test]
    fn fill_opcode_writes_a_repeated_byte() {
        // dest_len = 1 paragraph = 16 bytes, entirely filled by one op.
        let payload = fill_op(0x42, 16, true);
        let exe = build_exe(&payload, 0, 1, 1);
        let out = decode(&exe).unwrap();
        assert_eq!(out, vec![0x42u8; 16]);
    }

    #[test]
    fn copy_opcode_copies_literal_bytes() {
        let literal: Vec<u8> = (0..16u8).collect();
        let payload = copy_op(&literal, true);
        let exe = build_exe(&payload, 0, 1, 1);
        let out = decode(&exe).unwrap();
        assert_eq!(out, literal);
    }

    #[test]
    fn low_bit_marks_the_final_opcode_and_stops_the_loop() {
        // Two fill ops covering the whole 16-byte image. Decode reads
        // backward, so the op stored *last* (closest to the header) is
        // processed *first* -- it must NOT carry the final bit, or the loop
        // would stop after only writing the last 8 bytes. The op stored
        // first (final=true) is processed last and stops the loop.
        let mut payload = fill_op(0xAA, 8, true); // processed last (final), writes bytes 0..8
        payload.extend_from_slice(&fill_op(0xBB, 8, false)); // processed first, writes bytes 8..16
        let exe = build_exe(&payload, 0, 1, 1);
        let out = decode(&exe).unwrap();
        let mut expected = vec![0xAAu8; 8];
        expected.extend(vec![0xBBu8; 8]);
        assert_eq!(out, expected);
    }

    #[test]
    fn nonzero_raw_prefix_is_kept_in_place() {
        // dest_len = 2 paragraphs = 32 bytes. The opcode stream only
        // accounts for the last 16; the first 16 bytes of the packed
        // region must survive untouched as the raw prefix.
        let raw_prefix: Vec<u8> = (100..116u8).collect();
        let mut packed_payload = raw_prefix.clone();
        packed_payload.extend_from_slice(&fill_op(0x7, 16, true));
        let exe = build_exe(&packed_payload, 0, 2, 1);
        let out = decode(&exe).unwrap();
        assert_eq!(&out[..16], raw_prefix.as_slice());
        assert_eq!(&out[16..], [0x7u8; 16].as_slice());
    }

    #[test]
    fn src_ne_dst_at_final_opcode_is_a_hard_error() {
        // dest_len = 2 paragraphs = 32 bytes, but the final opcode only
        // fills 16 -- src (pointing at the unclaimed raw-prefix boundary)
        // won't equal dst (still 16 bytes short of the image start).
        let payload = fill_op(0x9, 16, true);
        let exe = build_exe(&payload, 0, 2, 1);
        let err = decode(&exe).unwrap_err();
        assert!(matches!(err, ExepackError::SrcDstMismatch { .. }));
    }

    #[test]
    fn trailing_pad_is_skipped_before_the_first_backward_read() {
        let payload = fill_op(0x5, 16, true);
        let exe = build_exe(&payload, 8, 1, 1);
        let out = decode(&exe).unwrap();
        assert_eq!(out, vec![0x5u8; 16]);
    }

    #[test]
    fn skip_len_other_than_one_is_a_hard_error() {
        let payload = fill_op(0x1, 16, true);
        let exe = build_exe(&payload, 0, 1, 2);
        let err = decode(&exe).unwrap_err();
        assert_eq!(err, ExepackError::UnsupportedSkipLen { skip_len: 2 });
    }

    #[test]
    fn unknown_opcode_is_a_typed_error_not_a_panic() {
        let mut payload = vec![0x2Au8]; // fill byte, never reached
        payload.extend_from_slice(&16u16.to_le_bytes());
        payload.push(0xC5); // not a recognized opcode
        let exe = build_exe(&payload, 0, 1, 1);
        let err = decode(&exe).unwrap_err();
        assert_eq!(err, ExepackError::UnknownOpcode { opcode: 0xC5 });
    }

    #[test]
    fn truncated_opcode_stream_errors_instead_of_panicking() {
        // A count that claims far more bytes than exist in the packed
        // region -- must not panic on subtraction underflow.
        let mut payload = vec![0x2Au8];
        payload.extend_from_slice(&0xFFFFu16.to_le_bytes());
        payload.push(0xB1);
        let exe = build_exe(&payload, 0, 1, 1);
        let err = decode(&exe).unwrap_err();
        assert_eq!(err, ExepackError::Truncated);
    }

    #[test]
    fn not_an_mz_exe_is_rejected_cleanly() {
        let err = decode(&[0u8; 64]).unwrap_err();
        assert_eq!(err, ExepackError::NotAnMzExe);
    }

    #[test]
    fn too_short_input_never_panics() {
        for len in 0..24 {
            let err = decode(&vec![0u8; len]).unwrap_err();
            assert_eq!(err, ExepackError::TooShortForMzHeader);
        }
    }

    #[test]
    fn missing_rb_signature_is_reported_as_not_exepacked() {
        let payload = fill_op(0x1, 16, true);
        let mut exe = build_exe(&payload, 0, 1, 1);
        let len = exe.len();
        exe[len - 2..].copy_from_slice(b"XX");
        let err = decode(&exe).unwrap_err();
        assert_eq!(err, ExepackError::NotExepacked);
    }

    /// Local-only tier: decode the real `START.EXE`, per §1.2's own
    /// anchoring experiment -- `class_alignments`' clean paladin row at
    /// `0xedba` and the cleric XP run (i32le) at `0xee7b`.
    #[test]
    fn decodes_real_start_exe_and_matches_known_anchors() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (exepack::decodes_real_start_exe_and_matches_known_anchors)");
            return;
        };
        let path = std::path::Path::new(&dir).join("START.EXE");
        let data = std::fs::read(&path).expect("GBX_DATA_DIR/START.EXE must be readable");

        let out = decode(&data).expect("real START.EXE must decode cleanly");
        assert_eq!(out.len(), 0xf3e0, "dest_len paragraphs must match exactly");

        let class_alignments_row0 = &out[0xedba..0xedba + 10];
        assert_eq!(
            class_alignments_row0,
            &[9, 0, 1, 2, 3, 4, 5, 6, 7, 8],
            "class_alignments row 0 (cleric) must match coab Gbl.cs:801"
        );

        let cleric_xp: [i32; 4] = std::array::from_fn(|i| {
            i32::from_le_bytes(out[0xee7b + i * 4..0xee7b + i * 4 + 4].try_into().unwrap())
        });
        assert_eq!(cleric_xp, [1501, 3001, 6001, 13001]);
    }
}
