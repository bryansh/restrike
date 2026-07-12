//! The canonical 16-entry EGA palette every Gold Box screen composites
//! against. Uncopyrightable fact (the standard IBM EGA RGB triples), landed
//! here per `docs/design/renderer-ui-shell.md` D-UI5 ("the EGA palette
//! canon land here as evidence-tagged data (D6) — first real rules-pack
//! entries").
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `Classes/Display.cs:25-26` (`OrigEgaColors`/`egaColors`) — the
//!   16-entry `{R, G, B}` table `SetEgaPalette` remaps slots against. Values
//!   confirmed against the design doc's own citation (§1.1: "color 10 =
//!   `{82,255,82}`").

/// One EGA palette slot's RGB triple.
pub type Rgb = [u8; 3];

/// The 16-entry canonical EGA palette (index = 4-bit palette code), in the
/// original's slot order. Every composited asset in this codebase indexes
/// pixels `0..=15` into a palette starting from this table; `SetEgaPalette`
/// remaps individual slots at runtime (palette effects), never the table
/// itself.
pub const EGA_PALETTE: [Rgb; 16] = [
    [0, 0, 0],
    [0, 0, 173],
    [0, 173, 0],
    [0, 173, 173],
    [173, 0, 0],
    [173, 0, 173],
    [173, 82, 0],
    [173, 173, 173],
    [82, 82, 82],
    [82, 82, 255],
    [82, 255, 82],
    [82, 255, 255],
    [255, 82, 82],
    [255, 82, 255],
    [255, 255, 82],
    [255, 255, 255],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_sixteen_entries() {
        assert_eq!(EGA_PALETTE.len(), 16);
    }

    #[test]
    fn color_10_matches_the_design_docs_own_citation() {
        // docs/design/renderer-ui-shell.md §1.1: "color 10 = {82,255,82}".
        assert_eq!(EGA_PALETTE[10], [82, 255, 82]);
    }

    #[test]
    fn color_0_is_black_and_15_is_white() {
        assert_eq!(EGA_PALETTE[0], [0, 0, 0]);
        assert_eq!(EGA_PALETTE[15], [255, 255, 255]);
    }
}
