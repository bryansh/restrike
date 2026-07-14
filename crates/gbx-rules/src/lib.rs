//! Rules packs (THAC0, saves, XP, ability modifiers, weapons, spell/skill
//! parameters) and per-flavor traits (adnd1, xxvc), verified against the
//! user's data files at first run.
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.

pub mod adnd1;
pub mod bash_door;
pub mod pack;
pub mod palette;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
