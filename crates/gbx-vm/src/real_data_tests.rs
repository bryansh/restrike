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

    /// ★ **The shipped PARLAY, driven live** (roll-credits slice 3's
    /// acceptance): `ECL3` block 16 `@0x92C6` —
    /// `PARLAY 0, 1, 1, 1, 0, 0x7F79` — with both outcomes exercised.
    ///
    /// The site is a real negotiation: a `RANDOM 0x63` is rolled just above
    /// it (`@0x92AD`) and only a roll of 10 or more reaches the PARLAY at
    /// all; the tone the player picks then writes 0 (HAUGHTY, ABUSIVE) or 1
    /// (SLY, NICE, MEEK) into `0x7F79`, and the very next instruction is
    /// `COMPARE 0x7F79, 0` — so the outcome table really does feed a compare,
    /// exactly as review E6 said and not a dialogue tree.
    ///
    /// Note the address: `@0x92C6` is only reachable past an `IF >=` + `GOTO`
    /// pair, which is precisely the traversal the census used to drop (§7.1).
    /// The old dashboard placed this block's single PARLAY at `@0x8B15`.
    #[test]
    fn the_shipped_parlay_site_writes_its_tone_outcome() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let path = std::path::Path::new(&dir).join("ECL3.DAX");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let archive = DaxArchive::parse(&bytes).unwrap();
        let raw = archive.block_data(16).expect("ECL3 block 16");
        let payload = dax::ecl_block_payload(&raw);
        let block = BlockBytes::from_bytes(payload);

        const PARLAY_SITE: u16 = 0x92C6;
        const RESULT_CELL: u16 = 0x7F79;
        // HAUGHTY, SLY, NICE, MEEK, ABUSIVE -> the operands 0, 1, 1, 1, 0.
        for (tone, expected) in [(0u8, 0u16), (1, 1), (2, 1), (3, 1), (4, 0)] {
            let mut m = crate::EclMachine::load_block(block.clone(), &COTAB).unwrap();
            m.enter(PARLAY_SITE);
            let mut h = crate::test_support::TestHost::new();

            let step = m.step(&mut h).expect("the PARLAY decodes");
            let crate::VmStep::Request(crate::Request::HorizontalMenu { options }) = step else {
                panic!("expected the tone menu at {PARLAY_SITE:#06X}, got {step:?}");
            };
            assert_eq!(options.len(), 5, "five tones");

            m.resume(crate::Reply::Selection(tone), &mut h)
                .expect("the reply resolves");
            assert_eq!(
                h.word(RESULT_CELL),
                Some(expected),
                "tone {tone} must write outcome {expected} to {RESULT_CELL:#06X}"
            );

            // The instruction after the PARLAY is the COMPARE that reads it.
            let pc = m.current_pc().expect("the script continues");
            let instr = crate::decode::decode(&block, pc, &COTAB).expect("decodes");
            assert_eq!(instr.op.0, 0x03, "PARLAY feeds a COMPARE");
        }
    }

    /// ★ The six shipped `ADD NPC` sites (roll-credits §7.1) really are there,
    /// with the operands the doc records — the evidence for G5's verdict,
    /// pinned so a traversal regression cannot quietly un-find them.
    #[test]
    fn every_shipped_add_npc_site_is_reachable_with_the_recorded_operands() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        // (file, block, address, monster id, morale)
        let sites: [(&str, u8, u16, u8, u8); 6] = [
            ("ECL3.DAX", 17, 0x8D0F, 0x16, 0x64), // ALIAS
            ("ECL3.DAX", 17, 0x8D14, 0x17, 0x64), // DRAGONBAIT
            ("ECL3.DAX", 18, 0x9010, 0x16, 0x64),
            ("ECL3.DAX", 18, 0x9015, 0x17, 0x64),
            ("ECL5.DAX", 49, 0x8BCE, 0x3B, 0x64), // AKABAR BEL AKAS
            ("ECL6.DAX", 66, 0x8A04, 0x43, 0x64), // the area-6 RAKSHASA
        ];
        for (file, block_id, addr, monster, morale) in sites {
            let Ok(bytes) = std::fs::read(dir.join(file)) else {
                return;
            };
            let archive = DaxArchive::parse(&bytes).unwrap();
            let raw = archive.block_data(block_id).expect("the block exists");
            let block = BlockBytes::from_bytes(dax::ecl_block_payload(&raw));
            let instr = crate::decode::decode(&block, addr, &COTAB)
                .unwrap_or_else(|e| panic!("{file}#{block_id} @{addr:#06X}: {e:?}"));
            assert_eq!(
                instr.op.0, 0x36,
                "{file}#{block_id} @{addr:#06X} is ADD NPC"
            );
            assert_eq!(
                instr.args.len(),
                2,
                "ADD NPC decodes two operands despite skip_size 1"
            );
            let operand = |i: usize| match instr.args[i] {
                Arg::ImmByte(b) => b,
                ref other => panic!("{file}#{block_id}: operand {i} is {other:?}"),
            };
            assert_eq!(operand(0), monster);
            assert_eq!(operand(1), morale);
        }
    }

    /// The corrected traversal is what finds them: disassembling
    /// `ECL3` block 17 from its header vectors must now REACH the ADD NPC
    /// pair, where before slice 3 it reported those bytes as data (§7.1).
    #[test]
    fn the_add_npc_pair_is_reached_from_the_blocks_own_header_vectors() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let Ok(bytes) = std::fs::read(std::path::Path::new(&dir).join("ECL3.DAX")) else {
            return;
        };
        let archive = DaxArchive::parse(&bytes).unwrap();
        let raw = archive.block_data(17).unwrap();
        let block = BlockBytes::from_bytes(dax::ecl_block_payload(&raw));
        let (vectors, _) = read_header_vectors(&block, COTAB_VECTOR_COUNT);
        let entries: Vec<u16> = vectors.into_iter().flatten().collect();
        let listing = disassemble(&block, &COTAB, &entries);
        for addr in [0x8D0Fu16, 0x8D14] {
            assert!(
                listing.instructions.contains_key(&addr),
                "{addr:#06X} must be reached, not reported as data"
            );
        }
    }

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
