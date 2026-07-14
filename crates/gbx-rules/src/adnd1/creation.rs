//! `class_alignments`' typed accessor (D-RP9). Derived by reading coab for
//! behavior (D11, never copied): `ovr018.cs:590-604` reads
//! `class_alignments[class, 0]` as a count, then walks
//! `class_alignments[class, 1..=count]` to build the alignment picker menu
//! — the count-prefix convention this accessor exists to hide from callers.

use crate::pack::{RuleSet, TableData};

/// The legal alignment ids for `class` (`ClassId` order, coab
/// `Classes/Enums.cs:69` — `cleric = 0` .. `mc_mu_t = 16`), honoring the
/// table's count-prefix slot layout: each stored row is `[count, id_0, ...,
/// id_9]`, zero-padded past `count`, and only the first `count` ids are
/// meaningful (`ovr018.cs:590-604`).
///
/// Panics if `class` is out of range, or if the embedded `class_alignments`
/// table isn't shaped as expected — both pack-authoring bugs the D-RP7 CI
/// suite (`pack::validate`) is expected to have already caught, not runtime
/// conditions a caller needs to handle.
pub fn allowed_alignments(rules: &RuleSet, class: usize) -> Vec<u8> {
    let table = rules
        .table("class_alignments")
        .expect("class_alignments must be embedded (packs/adnd1/creation.toml)");
    let TableData::Rows { rows, .. } = &table.data else {
        panic!("class_alignments must be a rows-shape table");
    };
    let row = rows.get(class).unwrap_or_else(|| {
        panic!(
            "class {class} out of range for class_alignments ({} rows)",
            rows.len()
        )
    });
    let count = row[0] as usize;
    row[1..=count].iter().map(|&v| v as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Conformance test reproducing coab's own consumption loop
    /// (`ovr018.cs:590-604`): count-prefix honored, not the raw
    /// fixed-10-wide row.
    #[test]
    fn honors_the_count_prefix_not_the_raw_row_width() {
        let rules = RuleSet::load();

        // cleric (class 0): "any alignment" -- coab Gbl.cs:801 row 0 is
        // {9,0,1,2,3,4,5,6,7,8}, so all 9 alignment ids 0..=8.
        assert_eq!(
            allowed_alignments(&rules, 0),
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8]
        );

        // paladin (class 3): Lawful Good only -- Gbl.cs:801 row 3 is
        // {1,0,0,0,0,0,0,0,0,0}; the trailing zero-padding must NOT leak
        // into the result (id 0 legitimately appears once, not nine times).
        assert_eq!(allowed_alignments(&rules, 3), vec![0]);

        // druid (class 1): the five neutral-touching ids -- Gbl.cs:801 row
        // 1 is {5,1,3,4,5,7,0,0,0,0}.
        assert_eq!(allowed_alignments(&rules, 1), vec![1, 3, 4, 5, 7]);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn out_of_range_class_panics() {
        let rules = RuleSet::load();
        allowed_alignments(&rules, 17);
    }

    /// D-RP7's cluster-invariant example for this table: "alignment ids <
    /// 9" (coab `ovr020.cs:23`'s `alignmentString` has exactly 9 entries,
    /// LG..CE). This is a hand-authored per-cluster check, not generic
    /// schema validation (D-RP7: "cluster invariants ... as applicable to
    /// what exists") — it runs over every class in the real embedded pack.
    #[test]
    fn every_alignment_id_is_a_valid_alignment_index() {
        let rules = RuleSet::load();
        for class in 0..17 {
            for id in allowed_alignments(&rules, class) {
                assert!(
                    id < 9,
                    "class {class} lists alignment id {id}, outside the 9-entry alignmentString domain"
                );
            }
        }
    }
}
