//! Game state, world simulation, combat, magic, UI shell, core framebuffer,
//! `tick(input) -> frame` API, and save/load (ours plus original-format import).
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.
//! Frontends are thin presenters: input events in, framebuffer + audio + window
//! title out.
//!
//! M2 step 2 (`docs/design/renderer-ui-shell.md` §5 build order item 2):
//! framebuffer + palette, draw primitives, resident symbol sets + boot
//! slice, screen frames, and the text system. No Shell/widgets/flows (step
//! 3), no VM wiring (step 4), no 3D corridor/wallset loading (step 5), no
//! frontends (step 6).

pub mod boot;
pub mod draw;
pub mod framebuffer;
pub mod frames;
pub mod symbols;
pub mod text;

#[cfg(test)]
mod demo;
#[cfg(test)]
mod hash_goldens;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
