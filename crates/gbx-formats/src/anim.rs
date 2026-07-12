//! Animated picture blocks (`PIC*`, `SPRIT*`, `FINAL*`): a small header
//! (frame count) followed by `numFrames` self-contained frames, each its own
//! `{delay, height, width, x_pos, y_pos, field_9}` header plus 4bpp packed
//! pixels — the same pixel packing [`crate::image`] decodes, reused here via
//! [`crate::image::unpack_nibbles`]. Pure over bytes — no filesystem access.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `engine/ovr030.cs` `load_pic_final` (`:35-149`) — the frame
//!   header layout (`:83-105`), the XOR-delta scheme for `PIC`/`FINAL`
//!   frames ≥ 1 (`:107,119-134`), and the mask-color-0 decode
//!   (`DaxToPicture(0, masked, …)`, `:127`).
//! - Gold Box Explorer independently documents the same container shape for
//!   its `DaxImagePlugin` (`docs/design/renderer-ui-shell.md` D-UI5).
//!
//! ## On-disk layout (`docs/design/renderer-ui-shell.md` §1.8)
//!
//! ```text
//! offset 0x00        u8      numFrames
//! per frame:
//!   u32 LE  delay
//!   u16 LE  height
//!   u16 LE  width_cols   (in 8-pixel columns)
//!   u16 LE  x_pos
//!   u16 LE  y_pos        (stored as 2 bytes; one pad byte follows —
//!                          `ovr030.cs:101-102` advances the cursor by 3)
//!   [u8; 8] field_9      (unread by the original's draw path — carried)
//!   packed 4bpp pixel data: `height * width_cols * 4` bytes
//! ```
//!
//! **The XOR-delta quirk (`PIC`/`FINAL` only, `xor_delta` parameter):**
//! frames ≥ 1 store their *encoded* bytes XORed against frame 0's encoded
//! bytes — but only over indices `0..(encoded_len - 1)`; the final encoded
//! byte (the frame's last two packed pixels) is stored **verbatim, not
//! XORed** (`ega_encoded_size = bpp/2 - 1`, the XOR loop's `i < ega_encoded_size`
//! bound, `ovr030.cs:107,119-134`). `SPRIT` frames are independent —
//! `is_pic_or_final` gates the whole delta scheme off for them. This module
//! undoes the delta (XORs frame N's stored bytes back against frame 0's
//! *stored* bytes over that same index range) before unpacking pixels, so
//! callers always see fully-decoded frames, never raw deltas.
//!
//! Out of scope (engine behavior, not this decoder, per the design doc):
//! `AnimationsOn == false` collapsing `PIC`/`FINAL` to one frame, and the
//! masked-sprite 13→0 recolor pass.

use crate::image::unpack_nibbles;

/// One decoded animation frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimFrame {
    /// Original units: `delay * 100` ms per the original's animation timer
    /// (`docs/design/renderer-ui-shell.md` §1.8); this module leaves the
    /// raw on-disk value untouched.
    pub delay: u32,
    pub height: u16,
    /// Pixel width in 8-pixel columns, as stored on disk. Use
    /// [`AnimFrame::width_px`] for the actual pixel width.
    pub width_cols: u16,
    pub x_pos: u16,
    pub y_pos: u16,
    /// The 8-byte header field the original stores but never reads
    /// (`field_9`, docket item 2 in the design doc). Carried verbatim.
    pub field_9: [u8; 8],
    /// One byte per pixel, row-major, `height * width_px` bytes; values
    /// `0..=15` or `16` (transparent) when `masked` was set.
    pub pixels: Vec<u8>,
}

impl AnimFrame {
    pub fn width_px(&self) -> usize {
        self.width_cols as usize * 8
    }
}

/// A parsed animated picture block: its frames in on-disk order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimatedPicture {
    pub frames: Vec<AnimFrame>,
}

/// [`decode`]'s failure mode. Malformed/truncated input is expected input
/// (fuzz posture) — never a panic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimError {
    /// Zero bytes total — can't even read the frame count.
    Empty,
    /// A frame's fixed 21-byte header extends past the end of the input.
    TruncatedFrameHeader { frame: usize },
    /// A frame's declared `height * width_cols * 4` packed pixel bytes
    /// extend past the end of the input.
    TruncatedPixelData {
        frame: usize,
        needed: usize,
        available: usize,
    },
}

const FRAME_HEADER_LEN: usize = 21;

/// Decodes an animated picture block's raw (already DAX-decompressed)
/// bytes.
///
/// `masked`: when true, pixels equal to mask color 0 decode to
/// transparency-16 (`DaxToPicture(0, masked, …)` — the mask color is fixed
/// at 0 for this container, unlike [`crate::image::decode`]'s caller-chosen
/// mask). `xor_delta`: true for `PIC`/`FINAL` (frames ≥ 1 are deltas against
/// frame 0), false for `SPRIT` (every frame independent).
pub fn decode(data: &[u8], masked: bool, xor_delta: bool) -> Result<AnimatedPicture, AnimError> {
    let Some((&num_frames, mut rest)) = data.split_first() else {
        return Err(AnimError::Empty);
    };

    let mask = masked.then_some(0u8);
    let mut frames = Vec::with_capacity(num_frames as usize);
    let mut frame0_encoded: Option<Vec<u8>> = None;

    for frame_index in 0..num_frames as usize {
        if rest.len() < FRAME_HEADER_LEN {
            return Err(AnimError::TruncatedFrameHeader { frame: frame_index });
        }
        let delay = u32::from_le_bytes(rest[0..4].try_into().unwrap());
        let height = u16::from_le_bytes([rest[4], rest[5]]);
        let width_cols = u16::from_le_bytes([rest[6], rest[7]]);
        let x_pos = u16::from_le_bytes([rest[8], rest[9]]);
        let y_pos = u16::from_le_bytes([rest[10], rest[11]]);
        // byte 12 is a pad byte the original skips without reading.
        let mut field_9 = [0u8; 8];
        field_9.copy_from_slice(&rest[13..21]);
        rest = &rest[FRAME_HEADER_LEN..];

        let encoded_len = height as usize * width_cols as usize * 4;
        if rest.len() < encoded_len {
            return Err(AnimError::TruncatedPixelData {
                frame: frame_index,
                needed: encoded_len,
                available: rest.len(),
            });
        }
        let mut encoded = rest[..encoded_len].to_vec();
        rest = &rest[encoded_len..];

        if xor_delta && frame_index > 0 && encoded_len > 0 {
            let base = frame0_encoded
                .as_ref()
                .expect("frame 0 always sets frame0_encoded before later frames are reached");
            // The last encoded byte is copied verbatim, never XORed
            // (`ega_encoded_size = bpp/2 - 1`, ovr030.cs:107,119-134).
            for i in 0..encoded_len - 1 {
                encoded[i] ^= base[i];
            }
        }
        if frame_index == 0 {
            frame0_encoded = Some(encoded.clone());
        }

        frames.push(AnimFrame {
            delay,
            height,
            width_cols,
            x_pos,
            y_pos,
            field_9,
            pixels: unpack_nibbles(&encoded, mask),
        });
    }

    Ok(AnimatedPicture { frames })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-authored (D10): builds one frame's header + encoded pixel bytes.
    /// `encoded` must already be the delta-applied bytes as they'd appear
    /// on disk for `xor_delta` containers (i.e. the caller XORs against
    /// frame 0 before calling this for frame >= 1, mirroring how the
    /// original's compressor would have written the file).
    fn build_frame(delay: u32, height: u16, width_cols: u16, encoded: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&delay.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&width_cols.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // x_pos
        out.extend_from_slice(&0u16.to_le_bytes()); // y_pos
        out.push(0xEE); // pad byte, never read
        out.extend_from_slice(&[0u8; 8]); // field_9
        out.extend_from_slice(encoded);
        out
    }

    fn build_block(frames: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![frames.len() as u8];
        for frame in frames {
            out.extend_from_slice(frame);
        }
        out
    }

    #[test]
    fn empty_input_errors_cleanly() {
        assert_eq!(decode(&[], false, false).unwrap_err(), AnimError::Empty);
    }

    #[test]
    fn zero_frames_decodes_to_empty() {
        let anim = decode(&[0], false, false).unwrap();
        assert!(anim.frames.is_empty());
    }

    #[test]
    fn single_frame_decodes_unmasked() {
        // 1x1 column, 1 row -> 4 encoded bytes, 8 pixels.
        let encoded = vec![0x12, 0x34, 0x56, 0x78];
        let bytes = build_block(&[build_frame(600, 1, 1, &encoded)]);
        let anim = decode(&bytes, false, true).unwrap();
        assert_eq!(anim.frames.len(), 1);
        let f = &anim.frames[0];
        assert_eq!(f.delay, 600);
        assert_eq!(f.width_px(), 8);
        assert_eq!(f.pixels, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn masked_decode_uses_fixed_mask_color_zero() {
        // 1x1 column, 1 row -> 4 encoded bytes needed; first byte's nibbles
        // are 0 and 15.
        let encoded = vec![0x0F, 0x00, 0x00, 0x00];
        let bytes = build_block(&[build_frame(1, 1, 1, &encoded)]);
        let anim = decode(&bytes, true, true).unwrap();
        // First pixel (nibble 0) must be masked to transparency-16; the
        // second (nibble 15) must not.
        assert_eq!(anim.frames[0].pixels[0], 16);
        assert_eq!(anim.frames[0].pixels[1], 15);
    }

    #[test]
    fn xor_delta_second_frame_reconstructs_against_frame_zero() {
        // frame 0: raw encoded bytes (2x1, 1 row -> width_cols=2 -> 8 bytes).
        let frame0_encoded: Vec<u8> = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22];
        // Desired decoded frame 1 encoded bytes (what frame 1 "really" is).
        let frame1_true: Vec<u8> = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x99];
        // On disk, frame 1 stores: bytes 0..len-1 XORed against frame 0,
        // last byte verbatim.
        let mut frame1_on_disk = frame1_true.clone();
        for i in 0..frame1_true.len() - 1 {
            frame1_on_disk[i] ^= frame0_encoded[i];
        }

        let bytes = build_block(&[
            build_frame(1, 1, 2, &frame0_encoded),
            build_frame(1, 1, 2, &frame1_on_disk),
        ]);
        let anim = decode(&bytes, false, true).unwrap();
        assert_eq!(anim.frames.len(), 2);
        assert_eq!(anim.frames[1].pixels, unpack_nibbles(&frame1_true, None));
    }

    /// The XOR-scope edge (design doc D-UI7): a delta animation whose LAST
    /// packed byte differs between frame 0 and frame 1, proving the decoder
    /// copies it verbatim rather than XORing it.
    #[test]
    fn xor_delta_last_byte_is_copied_verbatim_not_xored() {
        let frame0_encoded: Vec<u8> = vec![0x11, 0x22, 0x33, 0x44];
        // Only the last byte differs; bytes 0..2 are identical to frame 0
        // (so an all-zero XOR there reconstructs them unchanged), and the
        // last byte is a totally different value stored as-is.
        let frame1_on_disk: Vec<u8> = vec![0x00, 0x00, 0x00, 0x99];

        let bytes = build_block(&[
            build_frame(1, 1, 1, &frame0_encoded),
            build_frame(1, 1, 1, &frame1_on_disk),
        ]);
        let anim = decode(&bytes, false, true).unwrap();

        // If the decoder incorrectly XORed the last byte against frame 0's
        // last byte (0x44), it would decode to 0x99 ^ 0x44 = 0xDD, not 0x99.
        let expected_last_byte_pixels = unpack_nibbles(&[0x99], None);
        assert_eq!(&anim.frames[1].pixels[6..8], &expected_last_byte_pixels[..]);
        // The first three bytes reconstruct frame0's own bytes (0 XOR frame0).
        let expected_head = unpack_nibbles(&frame0_encoded[..3], None);
        assert_eq!(&anim.frames[1].pixels[0..6], &expected_head[..]);
    }

    #[test]
    fn sprit_frames_are_independent_no_xor_delta() {
        // xor_delta = false: frame 1's bytes are used exactly as stored,
        // even though they'd look like garbage if XORed against frame 0.
        // 1x1 column, 1 row -> 4 encoded bytes/frame.
        let frame0_encoded: Vec<u8> = vec![0xAA, 0xBB, 0xCC, 0xDD];
        let frame1_encoded: Vec<u8> = vec![0x12, 0x34, 0x56, 0x78];
        let bytes = build_block(&[
            build_frame(6, 1, 1, &frame0_encoded),
            build_frame(6, 1, 1, &frame1_encoded),
        ]);
        let anim = decode(&bytes, false, false).unwrap();
        assert_eq!(anim.frames[1].pixels, unpack_nibbles(&frame1_encoded, None));
    }

    #[test]
    fn truncated_frame_header_errors_cleanly() {
        let bytes = vec![1, 0, 0, 0, 0]; // numFrames=1, only 4 header bytes follow
        let err = decode(&bytes, false, false).unwrap_err();
        assert_eq!(err, AnimError::TruncatedFrameHeader { frame: 0 });
    }

    #[test]
    fn truncated_pixel_data_errors_cleanly() {
        // Declares a 1x1 frame (4 encoded bytes needed) but supplies none.
        let bytes = build_block(&[build_frame(1, 1, 1, &[])]);
        let err = decode(&bytes, false, false).unwrap_err();
        assert_eq!(
            err,
            AnimError::TruncatedPixelData {
                frame: 0,
                needed: 4,
                available: 0
            }
        );
    }

    #[test]
    fn zero_size_frame_decodes_to_empty_pixels_without_panicking() {
        let bytes = build_block(&[build_frame(1, 0, 0, &[])]);
        let anim = decode(&bytes, false, true).unwrap();
        assert!(anim.frames[0].pixels.is_empty());
    }

    /// Local-only tier (pattern from `dax.rs`/`geo.rs`): every PIC*/SPRIT*/
    /// FINAL* block in the real data set decodes without error. For
    /// `PIC`/`FINAL` (the XOR-delta containers), also checks the design
    /// doc's "frame dimensions are effectively required equal" note
    /// (`docs/design/renderer-ui-shell.md` §1.8) — the XOR delta indexes
    /// frame 0's encoded bytes by the current frame's size, so unequal
    /// dimensions across frames would corrupt every frame after the first.
    #[test]
    fn every_real_anim_block_decodes_and_pic_final_frames_share_dimensions() {
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
            let is_pic_or_final = name.starts_with("PIC") || name.starts_with("FINAL");
            let is_anim_file =
                (is_pic_or_final || name.starts_with("SPRIT")) && name.ends_with(".DAX");
            if !is_anim_file {
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
                let anim = decode(&raw, true, is_pic_or_final).unwrap_or_else(|e| {
                    panic!(
                        "{}: block {} failed to decode as an animation: {e:?}",
                        path.display(),
                        block_entry.id
                    )
                });
                assert!(
                    !anim.frames.is_empty(),
                    "{}: block {} decoded to zero frames",
                    path.display(),
                    block_entry.id
                );
                if is_pic_or_final {
                    let (w0, h0) = (anim.frames[0].width_px(), anim.frames[0].height);
                    for (i, frame) in anim.frames.iter().enumerate() {
                        assert_eq!(
                            (frame.width_px(), frame.height),
                            (w0, h0),
                            "{}: block {} frame {} has different dimensions than frame 0",
                            path.display(),
                            block_entry.id,
                            i
                        );
                    }
                }
                blocks_checked += 1;
            }
        }

        assert!(
            blocks_checked > 0,
            "GBX_DATA_DIR is set but no PIC/SPRIT/FINAL blocks were found in it"
        );
        eprintln!("checked {blocks_checked} real animated picture block(s)");
    }
}
