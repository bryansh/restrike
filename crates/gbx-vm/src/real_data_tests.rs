//! Local-only content tests (`docs/design/vm-scriptmemory.md` §4, pattern
//! from `gbx-formats`' `detect.rs`/`dax.rs`): exercised only when
//! `GBX_DATA_DIR` is set, silently passing otherwise so public CI never
//! touches game data (D10). This module's one test lives here rather than
//! in `gbx-formats` because reliably finding real `0x80` inline-string
//! operands needs flow-following disassembly (`disasm::disassemble`) —
//! a linear byte scan for `0x80` risks false positives on arbitrary data
//! bytes (in-block strings, `GETTABLE`/`SAVETABLE` tables, self-modified
//! regions are all "data" a census-style scan must not wander into).

#[cfg(test)]
mod tests {
    use crate::decode::{read_header_vectors, Arg, BlockBytes, ECL_BLOCK_SIZE};
    use crate::dialect::{COTAB, COTAB_VECTOR_COUNT};
    use crate::disasm::disassemble;
    use gbx_formats::dax::{self, DaxArchive};

    /// Task 1 (ECL inline-string decompression)'s real-data check: every
    /// `0x80`-mode inline-string operand reached by a flow-following
    /// traversal of every real CotAB block decompresses to plausible
    /// English-like text — mostly alphabetic/space characters, always
    /// within the printable ASCII range `gbx_formats::ecl_text::decompress`
    /// can even produce. No game text is asserted on or printed (D10):
    /// only character-class statistics, matching the task brief's
    /// "assert on character-class statistics, not content" instruction.
    #[test]
    fn real_ecl_inline_strings_decompress_to_plausible_ascii_text() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);

        let mut strings_checked = 0usize;
        let mut alpha_or_space = 0usize;
        let mut total_chars = 0usize;

        for entry in std::fs::read_dir(dir).expect("GBX_DATA_DIR must be readable") {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_ascii_uppercase();
            if !(name.starts_with("ECL") && name.ends_with(".DAX")) {
                continue;
            }

            let bytes = std::fs::read(&path).unwrap();
            let archive = DaxArchive::parse(&bytes).unwrap();
            for block_entry in archive.entries() {
                let raw = archive.block_data(block_entry.id).unwrap();
                let payload = dax::ecl_block_payload(&raw);
                if payload.is_empty() || payload.len() > ECL_BLOCK_SIZE {
                    continue;
                }
                let block = BlockBytes::from_bytes(payload);
                let (vectors, _) = read_header_vectors(&block, COTAB_VECTOR_COUNT);
                let entry_points: Vec<u16> = vectors.into_iter().flatten().collect();
                let listing = disassemble(&block, &COTAB, &entry_points);

                for instr in listing.instructions.values() {
                    for arg in &instr.args {
                        let Arg::InlineStr(packed) = arg else {
                            continue;
                        };
                        if packed.is_empty() {
                            continue;
                        }
                        let text = gbx_formats::ecl_text::decompress(packed);
                        if text.is_empty() {
                            continue;
                        }
                        strings_checked += 1;
                        for &c in &text {
                            total_chars += 1;
                            if c.is_ascii_alphabetic() || c == b' ' {
                                alpha_or_space += 1;
                            }
                            assert!(
                                (0x20..=0x7E).contains(&c),
                                "decompressed byte {c:#04X} outside printable ASCII"
                            );
                        }
                    }
                }
            }
        }

        assert!(
            strings_checked > 0,
            "GBX_DATA_DIR is set but no reached 0x80-mode inline strings were found"
        );
        let ratio = alpha_or_space as f64 / total_chars as f64;
        assert!(
            ratio > 0.5,
            "only {:.1}% of {total_chars} decompressed characters across {strings_checked} \
             string(s) were alphabetic/space — expected mostly plausible English text",
            ratio * 100.0
        );
        eprintln!(
            "checked {strings_checked} inline string(s), {total_chars} char(s), \
             {:.1}% alphabetic/space",
            ratio * 100.0
        );
    }
}
