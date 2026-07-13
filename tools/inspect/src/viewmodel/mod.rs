//! View-model layer: pure data transforms the resource browser/disasm/
//! engine panes render, kept free of `egui` so they're unit-testable
//! without a display (task brief: "unit-test the view-model layer").

pub mod block_kind;
pub mod call_table;
pub mod copy;
pub mod geo_map;
pub mod goto;
pub mod halt_table;
pub mod hex;
pub mod log_table;
pub mod palette;
pub mod ppm;
pub mod walldef;
