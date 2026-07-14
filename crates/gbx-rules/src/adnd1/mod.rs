//! AD&D 1e (CotAB flavor) typed accessors over the loaded [`crate::pack::RuleSet`].
//!
//! The `adnd1` flavor trait itself (D-RP5) is session-3 scope; this module
//! only holds per-cluster typed accessors as D-RP9's authoring flow lands
//! each pack.

pub mod creation;
pub mod creation_limits;
pub mod hp_hd;
pub mod progression;
pub mod thief_skills;
