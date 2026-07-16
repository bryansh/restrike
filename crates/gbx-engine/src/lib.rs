//! Game state, world simulation, combat, magic, UI shell, core framebuffer,
//! `tick(input) -> frame` API, and save/load (ours plus original-format import).
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.
//! Frontends are thin presenters: input events in, framebuffer + audio + window
//! title out.
//!
//! M2 step 4 (`docs/design/renderer-ui-shell.md` §5 build order item 4):
//! the real `EclMachine` is bound in — `vmhost.rs`'s `ScriptMemory`/
//! `EngineServices` implementation, `shell.rs`'s walk-loop flows pump real
//! vectors (step 3's `StubVm` stand-in is gone from production). Real
//! CotAB scripts run inside `Engine::tick` end to end. 3D corridor/wallset
//! rendering is step 5 (the viewport stays black), frontends are step 6.

pub mod boot;
pub mod charsheet;
pub mod combat;
pub mod corridor;
pub mod draw;
pub mod engine;
pub mod framebuffer;
pub mod frames;
pub mod import;
pub mod input;
pub mod money;
pub mod monster;
pub mod movement;
pub mod party;
pub mod rng;
pub mod save;
pub mod saveload;
/// Host-side (filesystem) save/load glue — kept off the wasm target and out
/// of the tick core (D8).
#[cfg(not(target_arch = "wasm32"))]
pub mod saveload_fs;
pub mod screens;
pub mod shell;
pub mod shop;
pub mod symbols;
pub mod text;
pub mod training;
pub mod vmhost;
pub mod widgets;

#[cfg(test)]
mod demo;
#[cfg(test)]
mod h2_conformance;
#[cfg(test)]
mod hash_goldens;
#[cfg(test)]
mod save_roundtrip_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod walk_goldens;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
