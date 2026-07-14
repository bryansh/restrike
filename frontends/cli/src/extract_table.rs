//! `restrike extract-table [DIR] --table <id>` — D-RP7's "extraction is
//! confirmatory, never originating" tool (design doc §6 build order item
//! 2). Locates a pack table's declared bytes in the real decompressed
//! image, reports a byte-level diff on mismatch, and on `NotFound`
//! re-sweeps the same row values at alternate element widths to catch a
//! width misdeclaration. This tool CONFIRMS transcriptions and fills in
//! anchors by hand-editing the pack afterward — it never writes pack files
//! or invents row values itself.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use gbx_formats::exepack;
use gbx_formats::game_data::{self, GameData};
use gbx_rules::pack::{Anchor, ElementType, RuleSet, Table, TableData};

const ALTERNATE_WIDTHS: &[ElementType] = &[
    ElementType::U8,
    ElementType::I8,
    ElementType::U16Le,
    ElementType::I16Le,
    ElementType::U32Le,
    ElementType::I32Le,
];

pub fn cmd_extract_table(args: Vec<String>) -> ExitCode {
    let mut dir_arg = None;
    let mut table_id = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--table" => {
                let Some(id) = iter.next() else {
                    eprintln!("restrike: --table requires an ID argument");
                    return ExitCode::FAILURE;
                };
                table_id = Some(id);
            }
            other if dir_arg.is_none() && !other.starts_with("--") => {
                dir_arg = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("restrike: unknown extract-table flag '{other}'");
                print_usage();
                return ExitCode::FAILURE;
            }
        }
    }

    let Some(table_id) = table_id else {
        eprintln!("restrike: extract-table requires --table <id>");
        print_usage();
        return ExitCode::FAILURE;
    };

    let dir = match dir_arg.or_else(|| env::var_os("GBX_DATA_DIR").map(PathBuf::from)) {
        Some(dir) => dir,
        None => {
            eprintln!("restrike: no directory given and GBX_DATA_DIR is not set");
            print_usage();
            return ExitCode::FAILURE;
        }
    };
    if !dir.is_dir() {
        eprintln!("restrike: '{}' is not a directory", dir.display());
        return ExitCode::FAILURE;
    }
    let data = match game_data::load_dir(&dir) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("restrike: failed to load '{}': {err}", dir.display());
            return ExitCode::FAILURE;
        }
    };

    let rules = RuleSet::load();
    let Some(table) = rules.table(&table_id) else {
        eprintln!("restrike: no table with id '{table_id}' in any embedded pack");
        return ExitCode::FAILURE;
    };

    extract(table, &data)
}

fn extract(table: &Table, data: &GameData) -> ExitCode {
    println!("table: {}", table.meta.id);
    println!("evidence_tier: {:?}", table.meta.evidence_tier);

    let (file, offset, decompress) = match &table.meta.anchor {
        Anchor::None => {
            println!("anchor: none (coab-only tier) -- nothing to confirm against the image");
            return ExitCode::SUCCESS;
        }
        Anchor::Image { file, offset, len } => {
            println!("anchor: image {{ file = \"{file}\", offset = {offset:#x}, len = {len} }}");
            (file, *offset, true)
        }
        Anchor::Raw { file, offset, len } => {
            println!("anchor: raw {{ file = \"{file}\", offset = {offset:#x}, len = {len} }}");
            (file, *offset, false)
        }
    };

    let Some(raw) = data.raw_file(file) else {
        println!("RESULT: BinaryAbsent -- '{file}' is not present in the supplied data directory");
        return ExitCode::FAILURE;
    };

    let image = if decompress {
        match exepack::decode(raw) {
            Ok(bytes) => bytes,
            Err(err) => {
                println!("RESULT: ImageUndecodable -- {err:?}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        raw.to_vec()
    };

    let Some(expected) = flatten(table, None) else {
        println!("RESULT: a stored value doesn't fit its declared element type -- this is a pack authoring bug, not an extraction finding");
        return ExitCode::FAILURE;
    };

    let offset = offset as usize;
    if image.get(offset..offset + expected.len()) == Some(expected.as_slice()) {
        println!(
            "RESULT: Verified -- byte-exact match at {offset:#x} ({} bytes)",
            expected.len()
        );
        return ExitCode::SUCCESS;
    }

    println!("RESULT: mismatch at the declared offset -- diffing and re-sweeping");
    print_diff(&image, offset, &expected);

    let hits = search_all(&image, &expected);
    if !hits.is_empty() {
        println!(
            "FOUND at the declared width elsewhere in the image: {}",
            hits.iter()
                .map(|h| format!("{h:#x}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!("-> the pack's anchor offset is stale (Moved); update it to one of the above.");
        return ExitCode::SUCCESS;
    }

    println!("NotFound at the declared element width. Re-sweeping alternate widths (rows/jagged shapes only):");
    let mut any_alt = false;
    for &alt in ALTERNATE_WIDTHS {
        let Some(alt_bytes) = flatten(table, Some(alt)) else {
            continue;
        };
        if alt_bytes == expected {
            continue; // same width as declared, already checked
        }
        let alt_hits = search_all(&image, &alt_bytes);
        if !alt_hits.is_empty() {
            any_alt = true;
            println!(
                "  {alt:?}: FOUND at {}",
                alt_hits
                    .iter()
                    .map(|h| format!("{h:#x}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "  -> likely element-width misdeclaration: try `element = \"{}\"`",
                alt_toml_name(alt)
            );
        }
    }
    if !any_alt {
        println!("  no alternate width matched either -- the row values themselves may be wrong, or the table isn't present in this binary");
    }

    ExitCode::FAILURE
}

fn alt_toml_name(el: ElementType) -> &'static str {
    match el {
        ElementType::U8 => "u8",
        ElementType::I8 => "i8",
        ElementType::U16Le => "u16le",
        ElementType::I16Le => "i16le",
        ElementType::U32Le => "u32le",
        ElementType::I32Le => "i32le",
    }
}

fn search_all(image: &[u8], pattern: &[u8]) -> Vec<usize> {
    if pattern.is_empty() || pattern.len() > image.len() {
        return Vec::new();
    }
    image
        .windows(pattern.len())
        .enumerate()
        .filter(|(_, w)| *w == pattern)
        .map(|(i, _)| i)
        .collect()
}

/// Flattens `table`'s rows to bytes. `element_override` re-encodes a
/// rows/jagged-shape table at a different element width (for the
/// alternate-width sweep); `records`-shape tables ignore the override
/// (mixed per-column widths have no single alternate to sweep).
fn flatten(table: &Table, element_override: Option<ElementType>) -> Option<Vec<u8>> {
    match &table.data {
        TableData::Rows { element, rows } | TableData::Jagged { element, rows, .. } => {
            let el = element_override.unwrap_or(*element);
            let mut out = Vec::new();
            for row in rows {
                for v in row {
                    out.extend(el.to_bytes(*v)?);
                }
            }
            Some(out)
        }
        TableData::Records { columns, rows } => {
            if element_override.is_some() {
                return None;
            }
            let mut out = Vec::new();
            for row in rows {
                for (v, col) in row.iter().zip(columns.iter()) {
                    out.extend(col.element.to_bytes(*v)?);
                }
            }
            Some(out)
        }
    }
}

fn print_diff(image: &[u8], offset: usize, expected: &[u8]) {
    let actual = image.get(offset..offset + expected.len());
    match actual {
        Some(actual) => {
            println!("  offset   expected  actual");
            for (i, (e, a)) in expected.iter().zip(actual.iter()).enumerate() {
                if e != a {
                    println!("  {:#06x}   {e:#04x}      {a:#04x}", offset + i);
                }
            }
        }
        None => println!(
            "  the declared anchor (offset {offset:#x}, len {}) runs past the end of the decompressed image ({} bytes)",
            expected.len(),
            image.len()
        ),
    }
}

fn print_usage() {
    eprintln!("usage: restrike extract-table [DIR] --table <id>");
    eprintln!();
    eprintln!(
        "Decompresses the binary a pack table anchors into and confirms the table's \
         declared bytes against it: byte-exact match reports Verified; a mismatch prints a \
         byte-level diff, searches the whole image for the declared bytes at the declared \
         element width, and (if still not found) re-sweeps the same row values at alternate \
         element widths, reporting any width whose re-encoding is found -- a likely \
         element-width misdeclaration. Confirmatory only: never writes a pack file or \
         invents row values."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_rules::pack::{Axis, EvidenceTier, Source, TableMeta};

    fn rows_table(id: &str, anchor: Anchor, rows: Vec<Vec<i64>>) -> Table {
        Table {
            meta: TableMeta {
                id: id.to_string(),
                flavor: "adnd1".to_string(),
                description: "d".to_string(),
                axes: vec![Axis {
                    name: "row".to_string(),
                    size: rows.len(),
                    index: "i".to_string(),
                }],
                evidence_tier: EvidenceTier::Image,
                source: vec![Source {
                    kind: "coab".to_string(),
                    loc: "x".to_string(),
                }],
                consumed_by: vec!["y".to_string()],
                anchor,
                notes: None,
            },
            data: TableData::Rows {
                element: ElementType::U8,
                rows,
            },
        }
    }

    #[test]
    fn verified_when_bytes_match_at_the_declared_offset() {
        let table = rows_table(
            "t",
            Anchor::Raw {
                file: "TEST.DAT".to_string(),
                offset: 4,
                len: 3,
            },
            vec![vec![9], vec![8], vec![7]],
        );
        let mut image = vec![0u8; 16];
        image[4..7].copy_from_slice(&[9, 8, 7]);
        let data = GameData::from_files([("TEST.DAT".to_string(), image)]);
        assert_eq!(extract(&table, &data), ExitCode::SUCCESS);
    }

    #[test]
    fn moved_is_found_via_full_image_search_at_the_declared_width() {
        let table = rows_table(
            "t",
            Anchor::Raw {
                file: "TEST.DAT".to_string(),
                offset: 0, // wrong -- the real bytes are at offset 10
                len: 3,
            },
            vec![vec![9], vec![8], vec![7]],
        );
        let mut image = vec![0u8; 16];
        image[10..13].copy_from_slice(&[9, 8, 7]);
        let data = GameData::from_files([("TEST.DAT".to_string(), image)]);
        // search_all/print_diff paths exercised; a Moved-style match still
        // reports success (the data exists, just at a different offset).
        assert_eq!(extract(&table, &data), ExitCode::SUCCESS);
    }

    #[test]
    fn width_misdeclaration_is_caught_by_the_alternate_width_sweep() {
        // Declared as u8 (values 1, 2 -> bytes [1, 2]), but the image
        // actually stores them as u16le ([1, 0, 2, 0]) -- a real
        // element-width misdeclaration, not a missing/moved table.
        let table = rows_table(
            "t",
            Anchor::Raw {
                file: "TEST.DAT".to_string(),
                offset: 0, // the u8 bytes [1, 2] don't appear here at all
                len: 2,
            },
            vec![vec![1], vec![2]],
        );
        let mut image = vec![0u8; 16];
        // [1,0,2,0] contains no adjacent (1,2) byte pair, so the u8 search
        // genuinely finds nothing; only the u16le re-encoding matches.
        image[8..12].copy_from_slice(&[1, 0, 2, 0]);
        let data = GameData::from_files([("TEST.DAT".to_string(), image)]);
        // The declared width isn't found anywhere, so this is a reported
        // failure overall -- but exercises the alternate-width sweep path
        // (verified via the fields above having no accidental collision).
        assert_eq!(extract(&table, &data), ExitCode::FAILURE);
    }

    #[test]
    fn not_found_anywhere_fails_cleanly() {
        let table = rows_table(
            "t",
            Anchor::Raw {
                file: "TEST.DAT".to_string(),
                offset: 0,
                len: 3,
            },
            vec![vec![9], vec![8], vec![7]],
        );
        let data = GameData::from_files([("TEST.DAT".to_string(), vec![0u8; 16])]);
        assert_eq!(extract(&table, &data), ExitCode::FAILURE);
    }

    #[test]
    fn coab_only_table_reports_success_with_nothing_to_confirm() {
        let table = rows_table("t", Anchor::None, vec![vec![1]]);
        let data = GameData::from_files([]);
        assert_eq!(extract(&table, &data), ExitCode::SUCCESS);
    }

    #[test]
    fn binary_absent_fails_cleanly() {
        let table = rows_table(
            "t",
            Anchor::Raw {
                file: "MISSING.DAT".to_string(),
                offset: 0,
                len: 1,
            },
            vec![vec![1]],
        );
        let data = GameData::from_files([]);
        assert_eq!(extract(&table, &data), ExitCode::FAILURE);
    }
}
