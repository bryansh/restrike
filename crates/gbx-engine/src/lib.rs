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

pub mod award;
pub mod boot;
/// ★ The camp Magic submenu's leaves (roll-credits §8): Memorize, Scribe,
/// Display and Rest.
pub mod camp_magic;
pub mod charsheet;
pub mod combat;
pub mod combat_art;
pub mod combat_host;
pub mod corridor;
/// The `RESTRIKE_DEBUG_LOG` replay pipeline (host-side: filesystem + save
/// slots), shared by `examples/replay_debug_log` and `restrike replay`.
#[cfg(not(target_arch = "wasm32"))]
pub mod debug_log;
/// H5 state digests (roll-credits D-RC3) — the checkpoint definition, whose
/// field order is append-only forever.
pub mod digest;
pub mod draw;
pub mod engine;
pub mod framebuffer;
pub mod frames;
pub mod import;
pub mod input;
/// ★ Vancian camp magic (roll-credits §8): the `SpellList` record model, the
/// spell table's class/level/name columns, and the capacity formula.
pub mod magic;
pub mod money;
pub mod monster;
pub mod movement;
pub mod party;
pub mod picture;
pub mod rest;
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

/// Roll-credits slice 1's acceptance suite (`roll-credits.md` §5): the
/// `SAVE → 0x7F12` + cross-file-NEWECL transition, synthetic and against real
/// CotAB data.
#[cfg(test)]
mod area_transition_tests;
#[cfg(test)]
mod combat_wiring;
#[cfg(test)]
mod demo;
#[cfg(test)]
mod h2_conformance;
#[cfg(test)]
mod hash_goldens;
#[cfg(test)]
mod picture_tests;
#[cfg(test)]
mod save_roundtrip_tests;

/// The M6 slice-6 state-chart tests (`combat-visualizer.md` §8.5).
#[cfg(test)]
mod shell_combat_tests;
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
