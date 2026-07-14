//! D-RP4's verify-on-load protocol: decompress the binaries a `RuleSet`'s
//! tables anchor into once, then byte-exact compare each anchored table at
//! its recorded offset, falling back to a full-image search on mismatch.
//!
//! Per-detection-entry decompression strategy (`docs/design/rules-packs.md`
//! §1.2's closing paragraph — "M7's Buck Rogers binaries may pack
//! differently or not at all") isn't a registry yet: this session's only
//! detection entry (CotAB v1.3) always ships `START.EXE` EXEPACK-packed, so
//! `anchor.kind = "image"` is hardcoded to `gbx_formats::exepack::decode`.
//! `anchor.kind = "raw"` compares against the file's bytes directly, no
//! decompression step, per §1.2's "raw ... same semantics [as image],
//! unpacked".

use std::collections::HashMap;

use gbx_formats::detect::Detection;
use gbx_formats::exepack::{self, ExepackError};
use gbx_formats::game_data::GameData;

use super::element::ElementType;
use super::validate::{Anchor, Table, TableData};

/// One anchored table's verification outcome (D-RP4's exact statuses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyStatus {
    /// Byte-exact match at the anchor's recorded offset.
    Verified,
    /// Not found at the recorded offset, but the full table-length pattern
    /// was found elsewhere in the image — every hit, not just the first.
    Moved { found_at: Vec<u64> },
    /// Not found at the recorded offset, and the full-image search found no
    /// match either.
    NotFound,
    /// The detection entry matched, but the anchor's `file` isn't present
    /// in the supplied `GameData` — distinct from version skew.
    BinaryAbsent { file: String },
    /// The anchor's `file` is present but couldn't be decoded (wrong/corrupt
    /// packing, `src != dst`, `skip_len != 1`, truncation, ...).
    ImageUndecodable { file: String, reason: String },
    /// `anchor.kind = "none"` (`coab-only` tier) — nothing to check here;
    /// the oracle rungs (H2/H4) verify these instead.
    Unanchored,
    /// Verification was not attempted at all: the game data is unrecognized
    /// (`Detection::Unknown`), so there's no known decompression strategy to
    /// apply.
    NotAttempted { reason: String },
}

/// The full report from one [`super::RuleSet::verify`] call: one entry per
/// table, in `RuleSet::tables()` order. Advisory only (D-RP4) — never
/// serialized into saves, never blocks or fails boot.
#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub entries: Vec<(String, VerifyStatus)>,
}

impl VerifyReport {
    pub fn status(&self, table_id: &str) -> Option<&VerifyStatus> {
        self.entries
            .iter()
            .find(|(id, _)| id == table_id)
            .map(|(_, s)| s)
    }

    pub fn all_verified(&self) -> bool {
        self.entries
            .iter()
            .all(|(_, s)| matches!(s, VerifyStatus::Verified | VerifyStatus::Unanchored))
    }
}

/// Flattens a table's rows into the little-endian byte sequence its anchor
/// should match, per D-RP3a's flattening rules (rows: row-major within the
/// first axis; records: row-major over records, column order within a
/// row). Panics on a value that doesn't fit its element type — schema
/// validation (`pack::validate`) already guarantees every stored value
/// fits, so this can't happen for a `Table` that made it through
/// `RuleSet::load()`.
fn expected_bytes(table: &Table) -> Vec<u8> {
    match &table.data {
        TableData::Rows { element, rows } => flatten_uniform(rows, *element),
        TableData::Jagged { element, rows, .. } => flatten_uniform(rows, *element),
        TableData::Records { columns, rows } => {
            let mut out = Vec::new();
            for row in rows {
                for (value, column) in row.iter().zip(columns.iter()) {
                    out.extend(column.element.to_bytes(*value).expect(
                        "schema validation guarantees column values fit their element type",
                    ));
                }
            }
            out
        }
    }
}

fn flatten_uniform(rows: &[Vec<i64>], element: ElementType) -> Vec<u8> {
    let mut out = Vec::new();
    for row in rows {
        for value in row {
            out.extend(
                element
                    .to_bytes(*value)
                    .expect("schema validation guarantees stored values fit their element type"),
            );
        }
    }
    out
}

/// One resolved binary's decompressed (or raw) bytes, or why it couldn't be
/// obtained -- memoized per `file` name across a single [`verify`] call so
/// a binary anchoring multiple tables is only decompressed once (D-RP4:
/// "~61 KB, sub-millisecond -- deliberately every boot", not per-table).
enum ResolvedImage<'a> {
    Bytes(Vec<u8>),
    Absent,
    Undecodable(ExepackError),
    /// `raw` kind borrows the file's bytes directly -- no decode, no copy.
    Raw(&'a [u8]),
}

fn resolve_image<'a>(data: &'a GameData, file: &str, decompress: bool) -> ResolvedImage<'a> {
    let Some(raw) = data.raw_file(file) else {
        return ResolvedImage::Absent;
    };
    if !decompress {
        return ResolvedImage::Raw(raw);
    }
    match exepack::decode(raw) {
        Ok(bytes) => ResolvedImage::Bytes(bytes),
        Err(e) => ResolvedImage::Undecodable(e),
    }
}

fn compare_at_anchor(image: &[u8], offset: u64, expected: &[u8]) -> VerifyStatus {
    let offset = offset as usize;
    if let Some(actual) = image.get(offset..offset.saturating_add(expected.len())) {
        if actual == expected {
            return VerifyStatus::Verified;
        }
    }
    if expected.is_empty() || expected.len() > image.len() {
        return VerifyStatus::NotFound;
    }
    let hits: Vec<u64> = image
        .windows(expected.len())
        .enumerate()
        .filter(|(_, w)| *w == expected)
        .map(|(i, _)| i as u64)
        .collect();
    if hits.is_empty() {
        VerifyStatus::NotFound
    } else {
        VerifyStatus::Moved { found_at: hits }
    }
}

/// The public entry point (D-RP4): on `Detection::Unknown`, every anchored
/// table is reported `NotAttempted` without touching any file (there's no
/// known decompression strategy for data we can't even identify).
/// Otherwise defers to [`verify_known`] for the real per-table comparison.
pub fn verify(tables: &[Table], data: &GameData) -> VerifyReport {
    if matches!(data.detect(), Detection::Unknown { .. }) {
        let entries = tables
            .iter()
            .map(|table| {
                let status = match &table.meta.anchor {
                    Anchor::None => VerifyStatus::Unanchored,
                    Anchor::Image { .. } | Anchor::Raw { .. } => VerifyStatus::NotAttempted {
                        reason: "game data unrecognized (Detection::Unknown)".to_string(),
                    },
                };
                (table.meta.id.clone(), status)
            })
            .collect();
        return VerifyReport { entries };
    }
    verify_known(tables, data)
}

/// The per-table comparison loop, assuming `data` is already known to be a
/// recognized game (or, in tests, is being exercised directly regardless of
/// `detect()` — the detection gate is [`verify`]'s concern, not this
/// function's, so unit tests can drive byte comparisons with synthetic
/// `GameData` that no real fingerprint would ever match).
fn verify_known(tables: &[Table], data: &GameData) -> VerifyReport {
    let mut cache: HashMap<String, ResolvedImage> = HashMap::new();
    let mut entries = Vec::with_capacity(tables.len());

    for table in tables {
        let (file, offset, kind_decompresses) = match &table.meta.anchor {
            Anchor::None => {
                entries.push((table.meta.id.clone(), VerifyStatus::Unanchored));
                continue;
            }
            Anchor::Image { file, offset, .. } => (file, *offset, true),
            Anchor::Raw { file, offset, .. } => (file, *offset, false),
        };

        let resolved = cache
            .entry(file.clone())
            .or_insert_with(|| resolve_image(data, file, kind_decompresses));

        let status = match resolved {
            ResolvedImage::Absent => VerifyStatus::BinaryAbsent { file: file.clone() },
            ResolvedImage::Undecodable(e) => VerifyStatus::ImageUndecodable {
                file: file.clone(),
                reason: format!("{e:?}"),
            },
            ResolvedImage::Bytes(image) => compare_at_anchor(image, offset, &expected_bytes(table)),
            ResolvedImage::Raw(image) => compare_at_anchor(image, offset, &expected_bytes(table)),
        };
        entries.push((table.meta.id.clone(), status));
    }

    VerifyReport { entries }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::validate::load_pack_str;

    fn one_table(src: &str) -> Table {
        load_pack_str(src).unwrap().remove(0)
    }

    const TABLE_SRC: &str = r#"
[[table]]
id = "t"
schema_version = 1
flavor = "adnd1"
description = "d"
element = "u8"
axes = [{ name = "row", size = 1, index = "i" }, { name = "col", size = 3, index = "j" }]
evidence_tier = "image"
source = [{ kind = "coab", loc = "x" }]
consumed_by = ["y"]
anchor = { kind = "image", file = "TEST.EXE", offset = 4, len = 3 }
rows = [[9, 8, 7]]
"#;

    fn exepacked_test_exe(image: &[u8]) -> Vec<u8> {
        // A trivial EXEPACK file whose decompressed image is exactly
        // `image`: one final copy opcode, no raw prefix, no padding.
        let mut mz = vec![0u8; 32];
        mz[0..2].copy_from_slice(b"MZ");
        mz[0x08..0x0A].copy_from_slice(&2u16.to_le_bytes()); // e_cparhdr
        let body = 32usize;

        let mut packed = image.to_vec();
        packed.extend_from_slice(&(image.len() as u16).to_le_bytes());
        packed.push(0xB3); // copy, final
        let padded_len = packed.len().div_ceil(16) * 16;
        packed.resize(padded_len, 0xFF);

        let e_cs = (padded_len / 16) as u16;
        mz[0x16..0x18].copy_from_slice(&e_cs.to_le_bytes());
        mz[0x14..0x16].copy_from_slice(&18u16.to_le_bytes());

        let dest_len_paragraphs = (image.len().div_ceil(16)) as u16;
        let mut header = vec![0u8; 12];
        header.extend_from_slice(&dest_len_paragraphs.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes()); // skip_len
        header.extend_from_slice(b"RB");

        let mut file = mz;
        file.resize(body, 0);
        file.extend_from_slice(&packed);
        file.extend_from_slice(&header);
        file
    }

    #[test]
    fn verified_when_bytes_match_at_the_anchor_offset() {
        let table = one_table(TABLE_SRC);
        let mut image = vec![0u8; 16];
        image[4..7].copy_from_slice(&[9, 8, 7]);
        // pad dest_len to a full paragraph (16 bytes) so exepacked_test_exe's
        // dest_len-paragraphs math lines up with the image length exactly.
        let exe = exepacked_test_exe(&image);
        let data = GameData::from_files([("TEST.EXE".to_string(), exe)]);

        // verify_known, not verify: synthetic TEST.EXE never matches the
        // real DETECTION_TABLE, so the public verify() would report
        // NotAttempted (see not_attempted_when_game_data_is_unrecognized
        // below) -- verify_known is the comparison logic under test here.
        let report = verify_known(std::slice::from_ref(&table), &data);
        assert_eq!(report.status("t"), Some(&VerifyStatus::Verified));
        assert!(report.all_verified());
    }

    #[test]
    fn moved_when_bytes_are_found_elsewhere() {
        let table = one_table(TABLE_SRC);
        let mut image = vec![0u8; 16];
        image[10..13].copy_from_slice(&[9, 8, 7]); // not at offset 4
        let exe = exepacked_test_exe(&image);
        let data = GameData::from_files([("TEST.EXE".to_string(), exe)]);

        let report = verify_known(std::slice::from_ref(&table), &data);
        match report.status("t") {
            Some(VerifyStatus::Moved { found_at }) => assert_eq!(found_at, &vec![10]),
            other => panic!("expected Moved, got {other:?}"),
        }
    }

    #[test]
    fn not_found_when_bytes_are_nowhere_in_the_image() {
        let table = one_table(TABLE_SRC);
        let image = vec![0u8; 16];
        let exe = exepacked_test_exe(&image);
        let data = GameData::from_files([("TEST.EXE".to_string(), exe)]);

        let report = verify_known(std::slice::from_ref(&table), &data);
        assert_eq!(report.status("t"), Some(&VerifyStatus::NotFound));
    }

    #[test]
    fn binary_absent_when_the_anchored_file_is_missing() {
        let table = one_table(TABLE_SRC);
        let data = GameData::from_files([]);
        let report = verify_known(std::slice::from_ref(&table), &data);
        assert_eq!(
            report.status("t"),
            Some(&VerifyStatus::BinaryAbsent {
                file: "TEST.EXE".to_string()
            })
        );
    }

    #[test]
    fn image_undecodable_when_the_file_is_not_valid_exepack() {
        let table = one_table(TABLE_SRC);
        let data = GameData::from_files([("TEST.EXE".to_string(), vec![0u8; 4])]);
        let report = verify_known(std::slice::from_ref(&table), &data);
        assert!(matches!(
            report.status("t"),
            Some(VerifyStatus::ImageUndecodable { .. })
        ));
    }

    #[test]
    fn unanchored_for_coab_only_tables() {
        let src = r#"
[[table]]
id = "t"
schema_version = 1
flavor = "adnd1"
description = "d"
element = "u8"
axes = [{ name = "row", size = 1, index = "i" }]
evidence_tier = "coab-only"
source = [{ kind = "coab", loc = "x" }]
consumed_by = ["y"]
anchor = { kind = "none" }
rows = [[1]]
"#;
        let table = one_table(src);
        let data = GameData::from_files([]);
        let report = verify(std::slice::from_ref(&table), &data);
        assert_eq!(report.status("t"), Some(&VerifyStatus::Unanchored));
        assert!(report.all_verified());
    }

    #[test]
    fn not_attempted_when_game_data_is_unrecognized() {
        let table = one_table(TABLE_SRC);
        let image = vec![0u8; 16];
        let exe = exepacked_test_exe(&image);
        // Same file present, but detect() won't recognize this synthetic
        // data set (no real signature match) -- so decode is never even
        // attempted.
        let data = GameData::from_files([("TEST.EXE".to_string(), exe)]);
        let report = verify(std::slice::from_ref(&table), &data);
        assert!(matches!(
            report.status("t"),
            Some(VerifyStatus::NotAttempted { .. })
        ));
    }
}
