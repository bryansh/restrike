//! Gold Box data format parsers: DAX containers, ECL blocks, GEO maps,
//! images/walldefs/fonts, original save files, and game-detection fingerprints.
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.

pub mod detect;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
