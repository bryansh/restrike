//! PPM encoding for the "Save .ppm" buttons (task brief deliverable 3),
//! reusing the repo's existing convention (`crates/gbx-engine/src/demo.rs`
//! and friends: a `P6` header, raw RGB triples, no alpha channel — native
//! Preview opens these directly). Pure over already-decoded RGBA bytes; the
//! egui/arboard/filesystem side lives in `crate::widgets`.

/// Encodes `rgba` (row-major, `width*height*4` bytes) as a binary PPM
/// (`P6`). Alpha is dropped, not composited — this app's transparency
/// sentinel already renders as opaque black
/// ([`crate::viewmodel::palette::pixel_to_rgba`]), so a transparent pixel's
/// RGB triple is already the right on-disk color.
pub fn encode_ppm(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    out.reserve(width * height * 3);
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&px[..3]);
    }
    out
}

/// Builds a default `.ppm` filename from `parts` (e.g. block id, item
/// index) joined with `_` — the "default filename with block id" the task
/// brief asks for.
pub fn ppm_filename(parts: &[&str]) -> String {
    format!("{}.ppm", parts.join("_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_ppm_writes_the_p6_header() {
        let rgba = [0u8; 4];
        let out = encode_ppm(1, 1, &rgba);
        assert!(out.starts_with(b"P6\n1 1\n255\n"));
    }

    #[test]
    fn encode_ppm_drops_the_alpha_byte() {
        let rgba = [10u8, 20, 30, 255, 40, 50, 60, 0];
        let out = encode_ppm(2, 1, &rgba);
        let header_len = "P6\n2 1\n255\n".len();
        assert_eq!(&out[header_len..], &[10, 20, 30, 40, 50, 60]);
    }

    #[test]
    fn encode_ppm_output_length_is_header_plus_3_bytes_per_pixel() {
        let rgba = vec![0u8; 4 * 6];
        let out = encode_ppm(3, 2, &rgba);
        let header_len = "P6\n3 2\n255\n".len();
        assert_eq!(out.len(), header_len + 3 * 6);
    }

    #[test]
    fn ppm_filename_joins_parts_with_underscore() {
        assert_eq!(
            ppm_filename(&["block14", "item0"]),
            "block14_item0.ppm".to_string()
        );
    }

    #[test]
    fn ppm_filename_single_part() {
        assert_eq!(
            ppm_filename(&["framebuffer"]),
            "framebuffer.ppm".to_string()
        );
    }
}
