//! The strict-serde TOML shape (`docs/design/rules-packs.md` §3): one
//! `[[table]]` per cluster entry, deserialized with `deny_unknown_fields`
//! everywhere so a typo'd key or leftover draft field fails CI instead of
//! silently vanishing (D-RP7's "every pack parses (strict serde)").
//!
//! This module only describes the TOML shape. Shape inference (rows vs
//! records vs jagged), axis/anchor/domain validation, and the typed
//! `Table`/`TableData` this crate actually consumes live in
//! `super::validate` and `super::mod` — a raw-parsed pack is not yet a
//! trusted one.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPack {
    #[serde(rename = "table")]
    pub tables: Vec<RawTable>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTable {
    pub id: String,
    pub schema_version: u32,
    pub flavor: String,
    pub description: String,
    pub axes: Vec<RawAxis>,
    /// Present for `rows` and `jagged` shapes (one element type for every
    /// stored value); absent for `records`, which types per column instead.
    #[serde(default)]
    pub element: Option<String>,
    /// Present only for the `records` shape.
    #[serde(default)]
    pub columns: Option<Vec<RawColumn>>,
    /// Present only for the `jagged` shape.
    #[serde(default)]
    pub length_rule: Option<LengthRule>,
    pub evidence_tier: EvidenceTier,
    pub source: Vec<RawSource>,
    pub consumed_by: Vec<String>,
    pub anchor: RawAnchor,
    #[serde(default)]
    pub notes: Option<String>,
    pub rows: Vec<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawAxis {
    pub name: String,
    pub size: usize,
    pub index: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawColumn {
    pub name: String,
    pub element: String,
    /// Optional per §3's own `starting_age` example (only `base_age` sets
    /// it there) — a human-readable unit/semantic note, not load-bearing
    /// for validation.
    #[serde(default)]
    pub meaning: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSource {
    pub kind: String,
    pub loc: String,
}

/// `evidence_tier` (D-RP3): must correspond to the anchor kind — enforced
/// by [`super::validate`], not by this type alone.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceTier {
    Image,
    CoabOnly,
}

/// The `jagged` shape's row-length convention (D-RP3a).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LengthRule {
    CountPrefix,
    Explicit,
}

/// `anchor = { kind = "image" | "raw" | "none", ... }` (D-RP4). Internally
/// tagged on `kind` so `image`/`raw` variants carry `file`/`offset`/`len`
/// alongside the tag, exactly as §3's examples show.
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RawAnchor {
    Image {
        file: String,
        offset: u64,
        len: usize,
    },
    Raw {
        file: String,
        offset: u64,
        len: usize,
    },
    None,
}

pub fn parse_pack(src: &str) -> Result<RawPack, toml::de::Error> {
    toml::from_str(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_class_alignments_style_example() {
        let src = r#"
[[table]]
id = "class_alignments"
schema_version = 1
flavor = "adnd1"
description = "Allowed alignments per class"
element = "u8"
axes = [
  { name = "class", size = 2, index = "ClassId order" },
  { name = "slot", size = 3, index = "[0]=count, [1..=count]=ids" },
]
evidence_tier = "image"
source = [
  { kind = "coab", loc = "Classes/Gbl.cs:801" },
]
consumed_by = ["ovr018.cs:590-618"]
anchor = { kind = "image", file = "START.EXE", offset = 0xedba, len = 6 }
rows = [
  [2, 0, 1],
  [1, 3, 0],
]
"#;
        let pack = parse_pack(src).unwrap();
        assert_eq!(pack.tables.len(), 1);
        let t = &pack.tables[0];
        assert_eq!(t.id, "class_alignments");
        assert_eq!(t.element.as_deref(), Some("u8"));
        assert!(t.columns.is_none());
        assert_eq!(t.evidence_tier, EvidenceTier::Image);
        match &t.anchor {
            RawAnchor::Image { file, offset, len } => {
                assert_eq!(file, "START.EXE");
                assert_eq!(*offset, 0xedba);
                assert_eq!(*len, 6);
            }
            other => panic!("expected Image anchor, got {other:?}"),
        }
        assert_eq!(t.rows, vec![vec![2, 0, 1], vec![1, 3, 0]]);
    }

    #[test]
    fn parses_a_records_shape_table() {
        let src = r#"
[[table]]
id = "starting_age"
schema_version = 1
flavor = "adnd1"
description = "Starting-age dice by race x class"
axes = [
  { name = "race", size = 1, index = "RaceId" },
  { name = "class", size = 1, index = "ClassId" },
]
columns = [
  { name = "base_age", element = "u16le", meaning = "years", domain = "0..=5000" },
  { name = "dice_count", element = "u8", domain = "0..=20" },
  { name = "dice_size", element = "u8", domain = "0..=20" },
]
evidence_tier = "image"
source = [{ kind = "coab", loc = "Classes/Gbl.cs:724" }]
consumed_by = ["ovr018.cs:624-650"]
anchor = { kind = "image", file = "START.EXE", offset = 0xBEEF, len = 4 }
rows = [
  [6, 2, 6],
]
"#;
        let pack = parse_pack(src).unwrap();
        let t = &pack.tables[0];
        assert!(t.element.is_none());
        let cols = t.columns.as_ref().unwrap();
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[0].domain.as_deref(), Some("0..=5000"));
    }

    #[test]
    fn parses_a_jagged_shape_table() {
        let src = r#"
[[table]]
id = "race_classes"
schema_version = 1
flavor = "adnd1"
description = "Legal classes per race"
element = "u8"
length_rule = "count-prefix"
axes = [{ name = "race", size = 1, index = "RaceId" }]
evidence_tier = "coab-only"
source = [{ kind = "coab", loc = "Classes/Gbl.cs:276" }]
consumed_by = ["ovr018.cs:1-1"]
anchor = { kind = "none" }
rows = [
  [3, 0, 2, 5],
]
"#;
        let pack = parse_pack(src).unwrap();
        let t = &pack.tables[0];
        assert_eq!(t.length_rule, Some(LengthRule::CountPrefix));
        assert!(matches!(t.anchor, RawAnchor::None));
    }

    #[test]
    fn unknown_field_is_rejected() {
        let src = r#"
[[table]]
id = "x"
schema_version = 1
flavor = "adnd1"
description = "d"
axes = []
element = "u8"
evidence_tier = "coab-only"
source = []
consumed_by = []
anchor = { kind = "none" }
rows = []
this_field_does_not_exist = true
"#;
        assert!(parse_pack(src).is_err());
    }

    /// PLAN.md §6.2 / the task's ground rules: packs hold uncopyrightable
    /// mechanics numbers, never game text. `rows: Vec<Vec<i64>>` makes
    /// string cell values a parse error by construction, not a runtime
    /// content check -- this test is the executable proof of that claim.
    #[test]
    fn string_row_values_are_rejected_by_construction() {
        let src = r#"
[[table]]
id = "x"
schema_version = 1
flavor = "adnd1"
description = "d"
axes = [{ name = "row", size = 1, index = "i" }]
element = "u8"
evidence_tier = "coab-only"
source = []
consumed_by = []
anchor = { kind = "none" }
rows = [["not a number"]]
"#;
        assert!(parse_pack(src).is_err());
    }
}
