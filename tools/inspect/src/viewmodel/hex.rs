//! Hex-view builder: the fallback decoded view for any block `block_kind`
//! doesn't recognize (and a plain "show me the bytes" for every other kind).
//! Pure over bytes, no rendering.

/// One row of a hex dump: `offset` (into the source slice), the `hex`
/// column already formatted as space-separated uppercase pairs, and the
/// `ascii` column with unprintable bytes rendered as `.`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexRow {
    pub offset: usize,
    pub hex: String,
    pub ascii: String,
}

/// Builds a hex dump of `data`, `bytes_per_row` bytes per row (the last row
/// may be short). Panics if `bytes_per_row` is zero — a caller bug, not a
/// runtime condition.
pub fn hex_dump(data: &[u8], bytes_per_row: usize) -> Vec<HexRow> {
    assert!(bytes_per_row > 0, "bytes_per_row must be nonzero");
    data.chunks(bytes_per_row)
        .enumerate()
        .map(|(i, chunk)| HexRow {
            offset: i * bytes_per_row,
            hex: chunk
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" "),
            ascii: chunk
                .iter()
                .map(|&b| {
                    if (0x20..=0x7E).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_rows() {
        assert_eq!(hex_dump(&[], 16), Vec::new());
    }

    #[test]
    fn full_row_formats_hex_and_ascii() {
        let rows = hex_dump(b"HELLO", 5);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].offset, 0);
        assert_eq!(rows[0].hex, "48 45 4C 4C 4F");
        assert_eq!(rows[0].ascii, "HELLO");
    }

    #[test]
    fn short_final_row_is_not_padded() {
        let rows = hex_dump(&[1, 2, 3], 16);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hex, "01 02 03");
        assert_eq!(rows[0].ascii, "...");
    }

    #[test]
    fn multiple_rows_have_increasing_offsets() {
        let data: Vec<u8> = (0..20).collect();
        let rows = hex_dump(&data, 8);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].offset, 0);
        assert_eq!(rows[1].offset, 8);
        assert_eq!(rows[2].offset, 16);
        assert_eq!(rows[2].hex, "10 11 12 13");
    }

    #[test]
    fn unprintable_bytes_render_as_dot() {
        let rows = hex_dump(&[0x00, 0x1F, b'A', 0x7F, 0xFF], 5);
        assert_eq!(rows[0].ascii, "..A..");
    }

    #[test]
    #[should_panic]
    fn zero_bytes_per_row_panics() {
        hex_dump(&[1, 2, 3], 0);
    }
}
