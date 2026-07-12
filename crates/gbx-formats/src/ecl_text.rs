//! ECL inline-string compression: the 6-bit-per-character packing scripts
//! use for `0x80`-mode string operands (`docs/design/vm-scriptmemory.md` §5
//! docket item 5; `gbx-vm`'s `Arg::InlineStr` captures the still-packed raw
//! bytes this module decodes).
//!
//! Derived by reading coab for behavior (D11, never copied) and
//! cross-checked against an independent GPL-3 reimplementation:
//! - coab `engine/ovr008.cs`: `DecompressString` (`:1026-1062`, the bit
//!   unpacker), `inflateChar`/`deflateChar` (`:279-287`/`:289-298`, the
//!   6-bit-code <-> ASCII table), `LoadCompressedEclString` (`:1064-1076`,
//!   confirms the `0x80` operand's length byte counts *packed input bytes*,
//!   not decoded characters — decoding simply stops when the packed bytes
//!   are exhausted, no in-band terminator), and `vm_CopyStringFromMemory`/
//!   `vm_WriteStringToMemory` (`:1079-1127`/`:913-983`, confirming the
//!   *separate* `0x81` memory-string path is plain NUL-terminated ASCII —
//!   no bit-unpacking at all, so [`decompress`] must never be applied to a
//!   `ScriptMemory`-sourced string).
//! - ssi-engine (Java, GPL-3) `src/main/java/engine/script/EclArgument.java`
//!   (`:41-95`, the same 3-state bit-unpacking machine, independently
//!   derived from binary/format analysis rather than decompilation) and
//!   `src/main/java/shared/GoldboxString.java` (`:13-18`, the same
//!   inflate-style table) — byte-for-byte agreement with coab.
//! - `~/src/goldbox-refs/tools/hackdocs/GEODATA.TXT` (`:107-118`) and
//!   `STRGFORM.TXT` (`:33-38`) describe the identical 6-bit scheme for the
//!   sibling "Unlimited Adventures" Gold Box tool, including the exact
//!   `'@'`-collides-with-the-terminator-code quirk this module replicates
//!   (see [`deflate_char`]) — corroboration that this is an SSI-wide
//!   convention, not something CotAB-specific.
//!
//! ## The algorithm
//!
//! 4 characters pack into 3 bytes, 6 bits per character, **MSB-first**
//! (the earliest character occupies the highest-order bits of the byte
//! stream) — equivalent to reading the packed bytes as one continuous bit
//! stream and pulling 6-bit codes off the front. Symbol code `0` is never a
//! real character: the original's `DecompressString` gates every emission
//! on `curr != 0` and silently drops it. This is simultaneously the
//! trailing-padding mechanism (a packed-byte count that isn't a multiple of
//! 3 always leaves the final code's low bits zero-padded) and a genuine
//! original-engine quirk (`'@'` deflates to code `0` — see
//! [`deflate_char`] — so a literal `'@'` in source text is silently dropped
//! by the original compressor; [`compress`] replicates this exactly rather
//! than "fixing" it).

/// 6-bit code -> ASCII byte (coab's `inflateChar`, `ovr008.cs:279-287`).
/// Codes `0x20..=0x3F` pass through unchanged; codes `0x00..=0x1F` shift up
/// by `0x40`. Total over its domain (every `u8 & 0x3F` value); callers that
/// must skip code `0` (the terminator/quirk value) do so before calling
/// this, matching the original's `curr != 0` gate — this function itself
/// has no opinion on that skip.
fn inflate_char(code: u8) -> u8 {
    debug_assert!(code <= 0x3F, "6-bit code out of range: {code:#04X}");
    if code <= 0x1F {
        code + 0x40
    } else {
        code
    }
}

/// ASCII byte -> 6-bit code (coab's `deflateChar`, `ovr008.cs:289-298`),
/// the inverse of [`inflate_char`] over the representable domain
/// `0x20..=0x5F` (space, punctuation, digits, and uppercase letters — no
/// lowercase, no control characters; Gold Box script text is all-caps).
/// `None` for any byte outside that domain — [`compress`] surfaces this as
/// [`EclTextError::CharacterOutOfDomain`].
///
/// `'@'` (`0x40`) is a genuine quirk, not a bug: it deflates to code `0`,
/// the same value [`decompress`] treats as "no character" — so a literal
/// `'@'` compressed by the original tooling is silently dropped on
/// decompression. Documented in the sibling "Unlimited Adventures" hackdocs
/// (`STRGFORM.TXT:33-38`) as a known behavior of this SSI-wide scheme;
/// replicated here exactly, matching this codebase's convention of
/// preserving original quirks rather than correcting them.
fn deflate_char(b: u8) -> Option<u8> {
    match b {
        0x20..=0x3F => Some(b),
        0x40..=0x5F => Some(b - 0x40),
        _ => None,
    }
}

/// [`compress`]'s failure mode: a byte outside the 6-bit scheme's
/// representable domain (`0x20..=0x5F`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EclTextError {
    pub byte: u8,
    pub index: usize,
}

/// Decompresses a `0x80`-mode inline string operand's raw packed bytes into
/// decoded ASCII text. Never fails: every packed-byte count decodes to
/// *some* string (`docs/design/vm-scriptmemory.md`'s docket item 5) — a
/// byte count that leaves a partial trailing 6-bit code just yields a
/// smaller (possibly zero, via the code-`0` skip) number of trailing
/// characters, exactly as the original's fixed-iteration decoder does.
///
/// `packed` is *only* ever the `0x80` inline-string bytes — never apply
/// this to a `0x81` `ScriptMemory` string, which is already plain ASCII
/// (see this module's doc comment).
pub fn decompress(packed: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(packed.len() * 4 / 3);
    let mut acc: u32 = 0;
    let mut nbits: u32 = 0;

    for &byte in packed {
        acc = (acc << 8) | u32::from(byte);
        nbits += 8;
        while nbits >= 6 {
            let shift = nbits - 6;
            let code = ((acc >> shift) & 0x3F) as u8;
            nbits -= 6;
            acc &= (1u32 << nbits) - 1;
            if code != 0 {
                out.push(inflate_char(code));
            }
        }
    }

    out
}

/// Packs plain ASCII text into `0x80`-mode inline-string bytes — the
/// inverse of [`decompress`], used by `gbx-vm`'s `EclBuilder` to author
/// conformance-test fixtures in the real on-wire format rather than a raw
/// escape hatch (`docs/design/vm-scriptmemory.md` §4). Errors on any byte
/// outside the representable domain (`0x20..=0x5F`) rather than silently
/// mis-encoding it — test-authoring feedback, not a runtime condition (the
/// original tooling's behavior for out-of-domain input, if any existed, was
/// never traced; erroring is the safe default for a test helper).
pub fn compress(text: &[u8]) -> Result<Vec<u8>, EclTextError> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4 + 1);
    let mut acc: u32 = 0;
    let mut nbits: u32 = 0;

    for (index, &b) in text.iter().enumerate() {
        let code = deflate_char(b).ok_or(EclTextError { byte: b, index })?;
        acc = (acc << 6) | u32::from(code);
        nbits += 6;
        while nbits >= 8 {
            let shift = nbits - 8;
            out.push(((acc >> shift) & 0xFF) as u8);
            nbits -= 8;
            acc &= (1u32 << nbits) - 1;
        }
    }
    if nbits > 0 {
        out.push(((acc << (8 - nbits)) & 0xFF) as u8);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-traced in the research pass that pinned this algorithm: "HI"
    /// (2 chars, doesn't fill a 3-byte group) packs to exactly these bytes.
    #[test]
    fn hand_verified_hi_packs_to_two_bytes() {
        assert_eq!(compress(b"HI").unwrap(), vec![0x20, 0x90]);
        assert_eq!(decompress(&[0x20, 0x90]), b"HI");
    }

    /// "CAT" (3 chars) exercises the trailing zero-code skip: the third
    /// byte's low 6 bits are all zero-padding, which must be silently
    /// dropped rather than decoded as a spurious 4th character.
    #[test]
    fn hand_verified_cat_packs_to_three_bytes_with_a_skipped_pad_code() {
        assert_eq!(compress(b"CAT").unwrap(), vec![0x0C, 0x15, 0x00]);
        assert_eq!(decompress(&[0x0C, 0x15, 0x00]), b"CAT");
    }

    #[test]
    fn round_trips_every_representable_byte() {
        let text: Vec<u8> = (0x20u8..=0x5Fu8).filter(|&b| b != b'@').collect();
        let packed = compress(&text).unwrap();
        assert_eq!(decompress(&packed), text);
    }

    #[test]
    fn round_trips_across_group_boundary_lengths() {
        for len in 0..16 {
            let text: Vec<u8> = (0..len).map(|i| b'A' + (i % 26) as u8).collect();
            let packed = compress(&text).unwrap();
            assert_eq!(decompress(&packed), text, "len={len}");
        }
    }

    #[test]
    fn empty_input_round_trips_to_empty() {
        assert_eq!(compress(b"").unwrap(), Vec::<u8>::new());
        assert_eq!(decompress(&[]), Vec::<u8>::new());
    }

    /// The original quirk, replicated exactly: `'@'` deflates to code `0`,
    /// the same value the decompressor treats as "no character" — so a
    /// literal `'@'` vanishes on the round trip rather than erroring.
    #[test]
    fn at_sign_collides_with_the_terminator_code_and_is_dropped_on_decompress() {
        let packed = compress(b"A@B").unwrap();
        assert_eq!(decompress(&packed), b"AB");
    }

    #[test]
    fn compress_rejects_a_byte_outside_the_representable_domain() {
        let err = compress(b"AB\x01CD").unwrap_err();
        assert_eq!(
            err,
            EclTextError {
                byte: 0x01,
                index: 2
            }
        );
    }

    #[test]
    fn compress_rejects_lowercase() {
        let err = compress(b"lowercase").unwrap_err();
        assert_eq!(
            err,
            EclTextError {
                byte: b'l',
                index: 0
            }
        );
    }

    #[test]
    fn decompress_never_panics_on_arbitrary_bytes() {
        // Fuzz-smoke coverage (PLAN.md M1: "10 minutes of cargo-fuzz on
        // every parser") in miniature: every possible byte count/content up
        // to a few bytes must decode without panicking.
        for len in 0..=3 {
            let mut bytes = vec![0u8; len];
            loop {
                let _ = decompress(&bytes);
                if !increment(&mut bytes) {
                    break;
                }
            }
        }
    }

    /// Treats `bytes` as a little-endian counter and increments it,
    /// returning `false` on overflow (exhausted every combination).
    fn increment(bytes: &mut [u8]) -> bool {
        for b in bytes.iter_mut() {
            if *b == u8::MAX {
                *b = 0;
            } else {
                *b += 1;
                return true;
            }
        }
        false
    }

    // The local-only "real CotAB data decompresses to plausible English
    // text" test lives in `gbx-vm` (`machine.rs`), not here: reliably
    // finding real `0x80` operands needs flow-following disassembly (a
    // linear byte scan for `0x80` risks false positives on arbitrary data
    // bytes), which is `gbx-vm`'s responsibility, not this crate's.
}
