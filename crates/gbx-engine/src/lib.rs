//! Game state, world simulation, combat, magic, UI shell, core framebuffer,
//! `tick(input) -> frame` API, and save/load (ours plus original-format import).
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.
//! Frontends are thin presenters: input events in, framebuffer + audio + window
//! title out.
//!
//! M2 step 3 (`docs/design/renderer-ui-shell.md` §5 build order item 3):
//! step 2's framebuffer/text system now drives `Engine::new`/`tick`
//! (`engine.rs`), the five prompt-line widgets (`widgets.rs`), the `Shell`
//! state machine and walk-loop flows over a stub VM (`shell.rs`/
//! `vm_stub.rs`), and movement/door interaction (`movement.rs`) — a
//! synthetic map can be walked headlessly end-to-end. Real `EclMachine`
//! binding is step 4, 3D corridor/wallset rendering is step 5 (the viewport
//! stays black), frontends are step 6.

pub mod boot;
pub mod draw;
pub mod engine;
pub mod framebuffer;
pub mod frames;
pub mod input;
pub mod movement;
pub mod rng;
pub mod shell;
pub mod symbols;
pub mod text;
pub mod vm_stub;
pub mod widgets;

#[cfg(test)]
mod demo;
#[cfg(test)]
mod hash_goldens;
#[cfg(test)]
mod walk_goldens;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
