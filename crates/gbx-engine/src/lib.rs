//! Game state, world simulation, combat, magic, UI shell, core framebuffer,
//! `tick(input) -> frame` API, and save/load (ours plus original-format import).
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.
//! Frontends are thin presenters: input events in, framebuffer + audio + window
//! title out.

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
