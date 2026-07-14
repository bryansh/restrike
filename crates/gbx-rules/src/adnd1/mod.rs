//! AD&D 1e (CotAB flavor) typed accessors over the loaded [`crate::pack::RuleSet`].
//!
//! The `adnd1` flavor trait itself (D-RP5) is session-3 scope; this module
//! only holds the one accessor D-RP9's `class_alignments` authoring flow
//! asks for this session.

pub mod creation;
