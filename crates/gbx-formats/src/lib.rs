//! Gold Box data format parsers: DAX containers, ECL blocks, GEO maps,
//! images/walldefs/fonts, original save files, and game-detection fingerprints.
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.

pub mod anim;
pub mod dax;
pub mod detect;
pub mod ecl_text;
pub mod exepack;
pub mod font;
pub mod game_data;
pub mod geo;
pub mod image;
pub mod walldef;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
