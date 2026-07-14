//! D-RP7's in-repo CI suite, as a library function: strict-parsed
//! [`super::raw::RawTable`]s in, either a fully typed [`Table`] or a
//! [`SchemaError`] describing exactly which contract clause failed. Both
//! [`super::RuleSet::load`] (panics on error — a malformed embedded pack is
//! a shipped bug) and the CI tests in this module (assert specific `Err`
//! variants on deliberately-broken synthetic packs, proving the checks
//! actually catch what they claim to) call through here.
//!
//! Checks implemented, mapped to D-RP7's list:
//! - strict parse: [`super::raw::parse_pack`] (`deny_unknown_fields`
//!   throughout).
//! - shape inference + dims/columns vs axes: [`infer_shape`] +
//!   `check_row_shape`.
//! - domain validation: `records` columns' `min..=max`/`set(...)`/
//!   `bitmask(n)` grammar, plus every stored value fitting its element
//!   type's native range regardless of shape.
//! - anchor well-formedness (`len > 0` and `len == product(axes) x element
//!   width`, records: `row count x summed column widths`):
//!   [`check_anchor_wellformed`].
//! - tier <-> anchor-kind consistency: [`check_tier_anchor_consistency`].
//! - index-domain coverage (consumed_by cite present): `consumed_by`
//!   emptiness check below. (The *coverage* half of that rule — declared
//!   axis sizes actually bounding the cited code's index domain — is an
//!   authoring-time judgment call against the coab citation, not something
//!   generic schema code can check; the turn-undead lesson this rule exists
//!   for is a per-table authoring discipline, not a generic invariant.)

use std::collections::HashSet;

use super::element::{Domain, ElementType};
use super::raw::{EvidenceTier, LengthRule, RawAnchor, RawPack, RawTable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    Toml(String),
    /// The table declared a field combination that isn't one of the three
    /// D-RP3a shapes (`element` alone = rows, `columns` alone = records,
    /// `element` + `length_rule` = jagged).
    AmbiguousShape {
        table: String,
    },
    InvalidElementType {
        table: String,
        detail: String,
    },
    InvalidColumnElementType {
        table: String,
        column: String,
        detail: String,
    },
    InvalidDomain {
        table: String,
        column: String,
        detail: String,
    },
    EmptyAxes {
        table: String,
    },
    EmptySource {
        table: String,
    },
    EmptyConsumedBy {
        table: String,
    },
    /// Jagged tables are only defined (D-RP3a's one worked example,
    /// `RaceClasses`) with a single outer axis; multi-axis jagged has no
    /// specified flattening and is refused rather than guessed at.
    JaggedRequiresSingleAxis {
        table: String,
    },
    RowCountMismatch {
        table: String,
        expected: usize,
        actual: usize,
    },
    RowWidthMismatch {
        table: String,
        row: usize,
        expected: usize,
        actual: usize,
    },
    /// A `count-prefix` jagged row's first element (the declared count)
    /// doesn't match the number of values that actually follow it.
    JaggedCountPrefixMismatch {
        table: String,
        row: usize,
        declared: i64,
        actual: usize,
    },
    ValueOutOfElementRange {
        table: String,
        row: usize,
        column: usize,
        value: i64,
    },
    DomainViolation {
        table: String,
        row: usize,
        column: String,
        value: i64,
    },
    AnchorZeroLen {
        table: String,
    },
    AnchorLenMismatch {
        table: String,
        expected: usize,
        actual: usize,
    },
    TierAnchorMismatch {
        table: String,
        tier: &'static str,
        anchor_kind: &'static str,
    },
    DuplicateTableId {
        id: String,
    },
}

#[derive(Debug, Clone)]
pub struct Axis {
    pub name: String,
    pub size: usize,
    pub index: String,
}

#[derive(Debug, Clone)]
pub struct Source {
    pub kind: String,
    pub loc: String,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub element: ElementType,
    pub meaning: Option<String>,
    pub domain: Option<Domain>,
}

#[derive(Debug, Clone)]
pub enum Anchor {
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

#[derive(Debug, Clone)]
pub struct TableMeta {
    pub id: String,
    pub flavor: String,
    pub description: String,
    pub axes: Vec<Axis>,
    pub evidence_tier: EvidenceTier,
    pub source: Vec<Source>,
    pub consumed_by: Vec<String>,
    pub anchor: Anchor,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TableData {
    Rows {
        element: ElementType,
        rows: Vec<Vec<i64>>,
    },
    Records {
        columns: Vec<Column>,
        rows: Vec<Vec<i64>>,
    },
    Jagged {
        element: ElementType,
        length_rule: LengthRule,
        rows: Vec<Vec<i64>>,
    },
}

#[derive(Debug, Clone)]
pub struct Table {
    pub meta: TableMeta,
    pub data: TableData,
}

enum Shape {
    Rows,
    Records,
    Jagged,
}

fn infer_shape(raw: &RawTable) -> Result<Shape, SchemaError> {
    match (&raw.element, &raw.columns, &raw.length_rule) {
        (Some(_), None, None) => Ok(Shape::Rows),
        (None, Some(_), None) => Ok(Shape::Records),
        (Some(_), None, Some(_)) => Ok(Shape::Jagged),
        _ => Err(SchemaError::AmbiguousShape {
            table: raw.id.clone(),
        }),
    }
}

fn check_value_range(
    table: &str,
    row: usize,
    column: usize,
    value: i64,
    element: ElementType,
) -> Result<(), SchemaError> {
    if element.to_bytes(value).is_none() {
        return Err(SchemaError::ValueOutOfElementRange {
            table: table.to_string(),
            row,
            column,
            value,
        });
    }
    Ok(())
}

fn build_axes(raw: &RawTable) -> Result<Vec<Axis>, SchemaError> {
    if raw.axes.is_empty() {
        return Err(SchemaError::EmptyAxes {
            table: raw.id.clone(),
        });
    }
    Ok(raw
        .axes
        .iter()
        .map(|a| Axis {
            name: a.name.clone(),
            size: a.size,
            index: a.index.clone(),
        })
        .collect())
}

fn axes_product(axes: &[Axis]) -> usize {
    axes.iter().map(|a| a.size).product()
}

fn build_meta(raw: &RawTable, anchor: Anchor, axes: Vec<Axis>) -> Result<TableMeta, SchemaError> {
    if raw.source.is_empty() {
        return Err(SchemaError::EmptySource {
            table: raw.id.clone(),
        });
    }
    if raw.consumed_by.is_empty() {
        return Err(SchemaError::EmptyConsumedBy {
            table: raw.id.clone(),
        });
    }
    Ok(TableMeta {
        id: raw.id.clone(),
        flavor: raw.flavor.clone(),
        description: raw.description.clone(),
        axes,
        evidence_tier: raw.evidence_tier,
        source: raw
            .source
            .iter()
            .map(|s| Source {
                kind: s.kind.clone(),
                loc: s.loc.clone(),
            })
            .collect(),
        consumed_by: raw.consumed_by.clone(),
        anchor,
        notes: raw.notes.clone(),
    })
}

fn to_anchor(raw: &RawAnchor) -> Anchor {
    match raw {
        RawAnchor::Image { file, offset, len } => Anchor::Image {
            file: file.clone(),
            offset: *offset,
            len: *len,
        },
        RawAnchor::Raw { file, offset, len } => Anchor::Raw {
            file: file.clone(),
            offset: *offset,
            len: *len,
        },
        RawAnchor::None => Anchor::None,
    }
}

/// D-RP3: `evidence_tier` must correspond to the anchor kind (`image`/`raw`
/// <-> `image` tier; `none` <-> `coab-only` tier).
fn check_tier_anchor_consistency(
    table: &str,
    tier: EvidenceTier,
    anchor: &Anchor,
) -> Result<(), SchemaError> {
    let anchor_kind = match anchor {
        Anchor::Image { .. } => "image",
        Anchor::Raw { .. } => "raw",
        Anchor::None => "none",
    };
    let ok = match tier {
        EvidenceTier::Image => matches!(anchor, Anchor::Image { .. } | Anchor::Raw { .. }),
        EvidenceTier::CoabOnly => matches!(anchor, Anchor::None),
    };
    if !ok {
        return Err(SchemaError::TierAnchorMismatch {
            table: table.to_string(),
            tier: match tier {
                EvidenceTier::Image => "image",
                EvidenceTier::CoabOnly => "coab-only",
            },
            anchor_kind,
        });
    }
    Ok(())
}

/// D-RP7: `image`/`raw` anchors require `len > 0` and (when `expected_len`
/// is known — rows and records shapes) `len == expected_len` exactly.
/// Jagged tables anchor per-row or `none` (D-RP4) — that mechanism isn't
/// implemented yet (no jagged pack is authored this session), so a
/// jagged table's whole-table `image`/`raw` anchor is only checked for
/// `len > 0` here.
fn check_anchor_wellformed(
    table: &str,
    anchor: &Anchor,
    expected_len: Option<usize>,
) -> Result<(), SchemaError> {
    let len = match anchor {
        Anchor::Image { len, .. } | Anchor::Raw { len, .. } => *len,
        Anchor::None => return Ok(()),
    };
    if len == 0 {
        return Err(SchemaError::AnchorZeroLen {
            table: table.to_string(),
        });
    }
    if let Some(expected) = expected_len {
        if len != expected {
            return Err(SchemaError::AnchorLenMismatch {
                table: table.to_string(),
                expected,
                actual: len,
            });
        }
    }
    Ok(())
}

fn validate_rows_shape(
    raw: &RawTable,
    axes: &[Axis],
) -> Result<(TableData, ElementType), SchemaError> {
    let element = ElementType::parse(raw.element.as_deref().unwrap()).map_err(|e| {
        SchemaError::InvalidElementType {
            table: raw.id.clone(),
            detail: e.0,
        }
    })?;

    let expected_rows = axes[0].size;
    let expected_width: usize = axes[1..].iter().map(|a| a.size).product::<usize>().max(1);

    if raw.rows.len() != expected_rows {
        return Err(SchemaError::RowCountMismatch {
            table: raw.id.clone(),
            expected: expected_rows,
            actual: raw.rows.len(),
        });
    }
    for (row_idx, row) in raw.rows.iter().enumerate() {
        if row.len() != expected_width {
            return Err(SchemaError::RowWidthMismatch {
                table: raw.id.clone(),
                row: row_idx,
                expected: expected_width,
                actual: row.len(),
            });
        }
        for (col_idx, value) in row.iter().enumerate() {
            check_value_range(&raw.id, row_idx, col_idx, *value, element)?;
        }
    }

    Ok((
        TableData::Rows {
            element,
            rows: raw.rows.clone(),
        },
        element,
    ))
}

fn validate_records_shape(
    raw: &RawTable,
    axes: &[Axis],
) -> Result<(TableData, usize), SchemaError> {
    let raw_columns = raw.columns.as_ref().unwrap();
    let mut columns = Vec::with_capacity(raw_columns.len());
    let mut summed_width = 0usize;
    for c in raw_columns {
        let element =
            ElementType::parse(&c.element).map_err(|e| SchemaError::InvalidColumnElementType {
                table: raw.id.clone(),
                column: c.name.clone(),
                detail: e.0,
            })?;
        let domain = c
            .domain
            .as_deref()
            .map(Domain::parse)
            .transpose()
            .map_err(|e| SchemaError::InvalidDomain {
                table: raw.id.clone(),
                column: c.name.clone(),
                detail: e.0,
            })?;
        summed_width += element.width();
        columns.push(Column {
            name: c.name.clone(),
            element,
            meaning: c.meaning.clone(),
            domain,
        });
    }

    let expected_rows = axes_product(axes);
    if raw.rows.len() != expected_rows {
        return Err(SchemaError::RowCountMismatch {
            table: raw.id.clone(),
            expected: expected_rows,
            actual: raw.rows.len(),
        });
    }
    for (row_idx, row) in raw.rows.iter().enumerate() {
        if row.len() != columns.len() {
            return Err(SchemaError::RowWidthMismatch {
                table: raw.id.clone(),
                row: row_idx,
                expected: columns.len(),
                actual: row.len(),
            });
        }
        for (col_idx, (value, column)) in row.iter().zip(columns.iter()).enumerate() {
            check_value_range(&raw.id, row_idx, col_idx, *value, column.element)?;
            if let Some(domain) = &column.domain {
                if !domain.contains(*value) {
                    return Err(SchemaError::DomainViolation {
                        table: raw.id.clone(),
                        row: row_idx,
                        column: column.name.clone(),
                        value: *value,
                    });
                }
            }
        }
    }

    Ok((
        TableData::Records {
            columns,
            rows: raw.rows.clone(),
        },
        summed_width,
    ))
}

fn validate_jagged_shape(raw: &RawTable, axes: &[Axis]) -> Result<TableData, SchemaError> {
    if axes.len() != 1 {
        return Err(SchemaError::JaggedRequiresSingleAxis {
            table: raw.id.clone(),
        });
    }
    let element = ElementType::parse(raw.element.as_deref().unwrap()).map_err(|e| {
        SchemaError::InvalidElementType {
            table: raw.id.clone(),
            detail: e.0,
        }
    })?;
    let length_rule = raw.length_rule.unwrap();

    let expected_rows = axes[0].size;
    if raw.rows.len() != expected_rows {
        return Err(SchemaError::RowCountMismatch {
            table: raw.id.clone(),
            expected: expected_rows,
            actual: raw.rows.len(),
        });
    }

    for (row_idx, row) in raw.rows.iter().enumerate() {
        for (col_idx, value) in row.iter().enumerate() {
            check_value_range(&raw.id, row_idx, col_idx, *value, element)?;
        }
        if length_rule == LengthRule::CountPrefix {
            let declared = *row.first().ok_or_else(|| SchemaError::RowWidthMismatch {
                table: raw.id.clone(),
                row: row_idx,
                expected: 1,
                actual: 0,
            })?;
            let actual = row.len() - 1;
            if declared < 0 || declared as usize != actual {
                return Err(SchemaError::JaggedCountPrefixMismatch {
                    table: raw.id.clone(),
                    row: row_idx,
                    declared,
                    actual,
                });
            }
        }
    }

    Ok(TableData::Jagged {
        element,
        length_rule,
        rows: raw.rows.clone(),
    })
}

pub fn validate_table(raw: &RawTable) -> Result<Table, SchemaError> {
    let axes = build_axes(raw)?;
    let anchor = to_anchor(&raw.anchor);

    let (data, expected_anchor_len) = match infer_shape(raw)? {
        Shape::Rows => {
            let (data, element) = validate_rows_shape(raw, &axes)?;
            (data, Some(axes_product(&axes) * element.width()))
        }
        Shape::Records => {
            let (data, summed_width) = validate_records_shape(raw, &axes)?;
            (data, Some(axes_product(&axes) * summed_width))
        }
        Shape::Jagged => {
            let data = validate_jagged_shape(raw, &axes)?;
            (data, None)
        }
    };

    check_anchor_wellformed(&raw.id, &anchor, expected_anchor_len)?;
    check_tier_anchor_consistency(&raw.id, raw.evidence_tier, &anchor)?;

    let meta = build_meta(raw, anchor, axes)?;
    Ok(Table { meta, data })
}

pub fn validate_pack(raw: RawPack) -> Result<Vec<Table>, SchemaError> {
    let mut seen = HashSet::new();
    let mut tables = Vec::with_capacity(raw.tables.len());
    for raw_table in &raw.tables {
        if !seen.insert(raw_table.id.clone()) {
            return Err(SchemaError::DuplicateTableId {
                id: raw_table.id.clone(),
            });
        }
        tables.push(validate_table(raw_table)?);
    }
    Ok(tables)
}

/// Parses and validates a pack's TOML source in one call — the function
/// [`super::RuleSet::load`] panics through and the negative-path CI tests
/// below exercise directly.
pub fn load_pack_str(src: &str) -> Result<Vec<Table>, SchemaError> {
    let raw = super::raw::parse_pack(src).map_err(|e| SchemaError::Toml(e.to_string()))?;
    validate_pack(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASS_ALIGNMENTS_OK: &str = r#"
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
source = [{ kind = "coab", loc = "Classes/Gbl.cs:801" }]
consumed_by = ["ovr018.cs:590-618"]
anchor = { kind = "image", file = "START.EXE", offset = 0xedba, len = 6 }
rows = [
  [2, 0, 1],
  [1, 3, 0],
]
"#;

    #[test]
    fn valid_rows_table_loads() {
        let tables = load_pack_str(CLASS_ALIGNMENTS_OK).unwrap();
        assert_eq!(tables.len(), 1);
        match &tables[0].data {
            TableData::Rows { element, rows } => {
                assert_eq!(*element, ElementType::U8);
                assert_eq!(rows.len(), 2);
            }
            other => panic!("expected Rows, got {other:?}"),
        }
    }

    #[test]
    fn valid_records_table_loads() {
        let src = r#"
[[table]]
id = "starting_age"
schema_version = 1
flavor = "adnd1"
description = "Starting-age dice by race x class"
axes = [
  { name = "race", size = 2, index = "RaceId" },
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
anchor = { kind = "image", file = "START.EXE", offset = 0xBEEF, len = 8 }
rows = [
  [6, 2, 6],
  [0x0c08, 0xe, 0],
]
"#;
        let tables = load_pack_str(src).unwrap();
        match &tables[0].data {
            TableData::Records { columns, rows } => {
                assert_eq!(columns.len(), 3);
                assert_eq!(rows[1][0], 0x0c08);
            }
            other => panic!("expected Records, got {other:?}"),
        }
    }

    #[test]
    fn valid_jagged_table_loads() {
        let src = r#"
[[table]]
id = "race_classes"
schema_version = 1
flavor = "adnd1"
description = "Legal classes per race"
element = "u8"
length_rule = "count-prefix"
axes = [{ name = "race", size = 2, index = "RaceId" }]
evidence_tier = "coab-only"
source = [{ kind = "coab", loc = "Classes/Gbl.cs:276" }]
consumed_by = ["ovr018.cs:1-1"]
anchor = { kind = "none" }
rows = [
  [3, 0, 2, 5],
  [1, 6],
]
"#;
        let tables = load_pack_str(src).unwrap();
        match &tables[0].data {
            TableData::Jagged { rows, .. } => {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0], vec![3, 0, 2, 5]);
            }
            other => panic!("expected Jagged, got {other:?}"),
        }
    }

    #[test]
    fn jagged_count_prefix_mismatch_is_rejected() {
        let src = r#"
[[table]]
id = "race_classes"
schema_version = 1
flavor = "adnd1"
description = "d"
element = "u8"
length_rule = "count-prefix"
axes = [{ name = "race", size = 1, index = "RaceId" }]
evidence_tier = "coab-only"
source = [{ kind = "coab", loc = "x" }]
consumed_by = ["y"]
anchor = { kind = "none" }
rows = [
  [3, 0, 2],
]
"#;
        let err = load_pack_str(src).unwrap_err();
        assert!(matches!(
            err,
            SchemaError::JaggedCountPrefixMismatch {
                declared: 3,
                actual: 2,
                ..
            }
        ));
    }

    #[test]
    fn vacuous_zero_len_anchor_is_rejected() {
        let src = CLASS_ALIGNMENTS_OK.replace("len = 6", "len = 0");
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(err, SchemaError::AnchorZeroLen { .. }));
    }

    #[test]
    fn mismatched_anchor_len_is_rejected() {
        let src = CLASS_ALIGNMENTS_OK.replace("len = 6", "len = 7");
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(
            err,
            SchemaError::AnchorLenMismatch {
                expected: 6,
                actual: 7,
                ..
            }
        ));
    }

    #[test]
    fn tier_anchor_mismatch_is_rejected() {
        let src = CLASS_ALIGNMENTS_OK.replace(
            r#"evidence_tier = "image""#,
            r#"evidence_tier = "coab-only""#,
        );
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(err, SchemaError::TierAnchorMismatch { .. }));
    }

    #[test]
    fn missing_consumed_by_is_rejected() {
        let src = CLASS_ALIGNMENTS_OK
            .replace(r#"consumed_by = ["ovr018.cs:590-618"]"#, "consumed_by = []");
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(err, SchemaError::EmptyConsumedBy { .. }));
    }

    #[test]
    fn missing_source_is_rejected() {
        let src = CLASS_ALIGNMENTS_OK.replace(
            r#"source = [{ kind = "coab", loc = "Classes/Gbl.cs:801" }]"#,
            "source = []",
        );
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(err, SchemaError::EmptySource { .. }));
    }

    #[test]
    fn domain_violation_is_rejected() {
        let src = r#"
[[table]]
id = "x"
schema_version = 1
flavor = "adnd1"
description = "d"
axes = [{ name = "row", size = 1, index = "i" }]
columns = [{ name = "v", element = "u8", meaning = "m", domain = "0..=5" }]
evidence_tier = "coab-only"
source = [{ kind = "coab", loc = "x" }]
consumed_by = ["y"]
anchor = { kind = "none" }
rows = [[9]]
"#;
        let err = load_pack_str(src).unwrap_err();
        assert!(matches!(err, SchemaError::DomainViolation { value: 9, .. }));
    }

    #[test]
    fn row_count_mismatch_is_rejected() {
        let src = CLASS_ALIGNMENTS_OK.replace("[1, 3, 0],\n", "");
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(
            err,
            SchemaError::RowCountMismatch {
                expected: 2,
                actual: 1,
                ..
            }
        ));
    }

    #[test]
    fn row_width_mismatch_is_rejected() {
        let src = CLASS_ALIGNMENTS_OK.replace("[1, 3, 0],", "[1, 3],");
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(err, SchemaError::RowWidthMismatch { .. }));
    }

    #[test]
    fn value_out_of_element_range_is_rejected() {
        let src = CLASS_ALIGNMENTS_OK.replace("[2, 0, 1],", "[2, 0, 999],");
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(
            err,
            SchemaError::ValueOutOfElementRange { value: 999, .. }
        ));
    }

    #[test]
    fn ambiguous_shape_neither_element_nor_columns_is_rejected() {
        let src = r#"
[[table]]
id = "x"
schema_version = 1
flavor = "adnd1"
description = "d"
axes = [{ name = "row", size = 1, index = "i" }]
evidence_tier = "coab-only"
source = [{ kind = "coab", loc = "x" }]
consumed_by = ["y"]
anchor = { kind = "none" }
rows = [[1]]
"#;
        let err = load_pack_str(src).unwrap_err();
        assert!(matches!(err, SchemaError::AmbiguousShape { .. }));
    }

    #[test]
    fn duplicate_table_id_is_rejected() {
        let mut src = CLASS_ALIGNMENTS_OK.to_string();
        src.push_str(CLASS_ALIGNMENTS_OK);
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(err, SchemaError::DuplicateTableId { .. }));
    }

    #[test]
    fn malformed_toml_is_reported_as_toml_error() {
        let err = load_pack_str("not valid toml [[[").unwrap_err();
        assert!(matches!(err, SchemaError::Toml(_)));
    }

    #[test]
    fn unknown_field_is_reported_as_toml_error() {
        let src = CLASS_ALIGNMENTS_OK.replace(
            "schema_version = 1",
            "schema_version = 1\nnonexistent_field = true",
        );
        let err = load_pack_str(&src).unwrap_err();
        assert!(matches!(err, SchemaError::Toml(_)));
    }
}
