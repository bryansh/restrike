//! View-model layer: pure data transforms the resource browser/disasm/
//! engine panes render, kept free of `egui` so they're unit-testable
//! without a display (task brief: "unit-test the view-model layer").

pub mod block_kind;
pub mod geo_map;
pub mod hex;
pub mod log_table;
pub mod palette;
pub mod walldef;
