//! Rules packs: strict-TOML-parsed, D-RP7-validated tables embedded into
//! the binary at compile time (D-RP1 — `include_str!`, `RuleSet::load()`
//! infallible-or-panic).

pub mod element;
pub mod raw;
pub mod validate;
pub mod verify;

pub use element::{Domain, ElementType};
pub use raw::{EvidenceTier, LengthRule};
pub use validate::{Anchor, Axis, Column, SchemaError, Source, Table, TableData, TableMeta};
pub use verify::{VerifyReport, VerifyStatus};

/// The embedded `adnd1` packs (D-RP9's authoring order — `creation.toml` is
/// the first, containing `class_alignments`).
const ADND1_CREATION: &str = include_str!("../../packs/adnd1/creation.toml");
/// D-RP9 authoring order item 1: THAC0, saving throws, turn undead, XP thresholds.
const ADND1_PROGRESSION: &str = include_str!("../../packs/adnd1/progression.toml");
/// D-RP9 authoring order item 2: hit points and hit dice.
const ADND1_HP_HD: &str = include_str!("../../packs/adnd1/hp_hd.toml");

/// Every loaded, validated rules table (D-RP1). Construct via [`RuleSet::load`].
pub struct RuleSet {
    tables: Vec<Table>,
}

impl RuleSet {
    /// Parses and validates every embedded pack. **Infallible-or-panic**
    /// (D-RP1): a malformed embedded pack is a shipped bug the D-RP7 CI
    /// suite must catch before this ever reaches a user, so there is no
    /// error path here to plumb through callers.
    pub fn load() -> Self {
        // D-RP9's remaining clusters (thief skills, spell slots, constants)
        // join this list in later sessions.
        let packs = [
            ("adnd1/creation.toml", ADND1_CREATION),
            ("adnd1/progression.toml", ADND1_PROGRESSION),
            ("adnd1/hp_hd.toml", ADND1_HP_HD),
        ];

        let mut tables = Vec::new();
        for (pack_name, src) in packs {
            let pack_tables = validate::load_pack_str(src).unwrap_or_else(|e| {
                panic!("gbx-rules: embedded pack '{pack_name}' failed D-RP7 validation: {e:?}")
            });
            tables.extend(pack_tables);
        }
        RuleSet { tables }
    }

    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    pub fn table(&self, id: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.meta.id == id)
    }

    /// D-RP4's verify-on-load protocol. Advisory only — never blocks or
    /// fails boot, never serialized into saves.
    pub fn verify(&self, data: &gbx_formats::game_data::GameData) -> VerifyReport {
        verify::verify(&self.tables, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_does_not_panic_on_the_real_embedded_packs() {
        let rs = RuleSet::load();
        assert!(!rs.tables().is_empty());
    }

    #[test]
    fn every_embedded_table_has_a_unique_id() {
        let rs = RuleSet::load();
        let mut ids = std::collections::HashSet::new();
        for t in rs.tables() {
            assert!(
                ids.insert(t.meta.id.clone()),
                "duplicate table id {}",
                t.meta.id
            );
        }
    }

    /// Local-only tier (D-RP7): every `image`/`raw`-anchored table must
    /// report `Verified` against the reference detection entry's real data
    /// (CotAB v1.3, `~/goldbox-data/cotab`). Loud skip when `GBX_DATA_DIR`
    /// is unset -- never a silent pass (one of D-RP7's three closed
    /// green-paths).
    #[test]
    fn all_anchored_tables_verify_against_real_game_data() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!(
                "SKIPPED: local tier needs GBX_DATA_DIR (pack::all_anchored_tables_verify_against_real_game_data)"
            );
            return;
        };
        let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir))
            .expect("GBX_DATA_DIR must be readable");

        let rules = RuleSet::load();
        let report = rules.verify(&data);
        for (id, status) in &report.entries {
            eprintln!("verify: {id}: {status:?}");
        }
        assert!(
            report.all_verified(),
            "every image/raw-anchored table must report Verified against real data: {:?}",
            report.entries
        );
    }
}
