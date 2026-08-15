//! ★ **The CotAB code wheel** (`ovr004.copy_protection`, `ovr004.cs:7-111`) —
//! the algorithm, the 6×36 table, and the two rune sets it names.
//!
//! The prompt shows an **Espruar** (elvish) rune and a **Dethek** (dwarvish)
//! rune, a box number 1-6 and one of three path symbols, and asks for the
//! letter the physical translation wheel reveals once the two runes are
//! aligned. `docs/copy-protection.md` recorded the algorithm from Simeon
//! Pilgrim's published reverse-engineering; this module is transcribed from
//! **coab itself** (D11, read-for-behavior, never copied), which settles both
//! of that doc's open items:
//!
//! - **Row 0 really is 36 characters.** The doc flagged "35 shown here; verify
//!   length 36 at impl" — a web-fetch copy artifact. Every row of
//!   `ovr004.codeWheel` is 36 (asserted in this module's tests).
//! - **There is no wheel geometry to pin.** The doc worried about "the exact
//!   rune-index origin/direction on the wheel rim … and the path-symbol →
//!   `code_path` ordering". Neither exists in the program: the runes are
//!   *tile indices*, drawn straight out of `TILES.DAX` by
//!   `DrawIsoTile(var_6, …)` / `DrawIsoTile(var_7 + 0x1A, …)` after
//!   `Load24x24Set(0x1A, 0, 1, "tiles")` / `Load24x24Set(0x16, 0x1A, 2,
//!   "tiles")` (`ovr004.cs:22-23,38-39`), and `code_path` is the raw
//!   `Random(3)` that also picks the path string (`:42-61`). The index the
//!   arithmetic uses IS the index the art is drawn from, so an engine that
//!   draws the same tile computes the same answer by construction. (The real
//!   data confirms the counts: `TILES.DAX` block 1 decodes to **26** 24×24
//!   items, block 2 to **22** — exactly `Random(26)` and `Random(22)`.)
//!
//! Row 5 doubles as the wheel's key row (`A`..`Z`,`1`..`9`,`0` — the label of
//! each of the 36 rim positions) *and* as a real answer row: `code_row` is
//! `Random(6)`, so box number 1 (`code_row == 5`) reads its answer out of it.

/// `ovr004.codeWheel` (`ovr004.cs:7-14`) — six rows of 36, row-major.
///
/// **Not game data** in the D10 sense (nothing is extracted from the shipped
/// files here): it is a table in the program's own code segment, transcribed
/// like `thac0_table` or the frame-border tables in [`crate::frames`].
pub const CODE_WHEEL: [&str; 6] = [
    "CWLNRTESSCEDCSHSISERRRNSHSSTSSNNHSHN",
    "LAASRDAIILIDSUGADAEEOEGRLSELIITESOIO",
    "LRUNIMMORIIGRRIUPTIIUELIMLHMIXACGRIL",
    "Z0LIOHEUVNODSGEOGXYWISIOCRARLRARRHOI",
    "AMTELRLUIYNAEOOITOUELRREREUIMADPPFAB",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890",
];

/// How many characters each [`CODE_WHEEL`] row holds — and the modulus the
/// index arithmetic wraps against (`ovr004.cs:75-83`).
pub const WHEEL_WIDTH: usize = 36;

/// `Load24x24Set(0x1A, 0, 1, "tiles")` (`ovr004.cs:22`): the 26 Espruar runes,
/// `TILES.DAX` block 1, landing in 24×24 cell slots `0..26`.
pub const ESPRUAR_COUNT: u16 = 0x1A;
/// `Load24x24Set(0x16, 0x1A, 2, "tiles")` (`ovr004.cs:23`): the 22 Dethek
/// runes, `TILES.DAX` block 2, landing in cell slots `0x1A..0x30`.
pub const DETHEK_COUNT: u16 = 0x16;
/// The Dethek set's base slot in `gbl.dax24x24Set` — and therefore the `+0x1A`
/// in `DrawIsoTile(var_7 + 0x1A, 7, 0x11)` (`ovr004.cs:39`).
pub const DETHEK_BASE: u8 = 0x1A;
/// The three path symbols, indexed by `code_path` (`ovr004.cs:44-61`).
pub const PATH_STRINGS: [&str; 3] = ["-..-..-..", "- - - - -", "........."];

/// One posed challenge: the two rune indices, the path and the box row — plus
/// the answer, which D-RC4 has us show rather than demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Challenge {
    /// `var_6` — the Espruar rune, `Random(26)`; also its `TILES.DAX` block-1
    /// item index.
    pub espruar: u8,
    /// `var_7` — the Dethek rune, `Random(22)`; `TILES.DAX` block 2, drawn
    /// from cell `espruar + `[`DETHEK_BASE`].
    pub dethek: u8,
    /// `code_path` — `Random(3)`, indexes [`PATH_STRINGS`].
    pub path: u8,
    /// `code_row` — `Random(6)`; the printed box number is `6 - code_row`
    /// (`ovr004.cs:65`).
    pub row: u8,
}

impl Challenge {
    /// Poses one challenge, drawing the four values in the original's own
    /// order: `Random(26)`, `Random(22)`, `Random(3)`, `Random(6)`
    /// (`ovr004.cs:35-36,42,63`).
    ///
    /// The two rune draws come **before** the path/row draws because the
    /// original draws the runes, blits them and only then rolls the path — a
    /// draw-order detail that matters the moment a trace records this stream.
    pub fn pose(rng: &mut crate::rng::EngineRng) -> Self {
        let espruar = rng.random(ESPRUAR_COUNT) as u8;
        let dethek = rng.random(DETHEK_COUNT) as u8;
        let path = rng.random(3) as u8;
        let row = rng.random(6) as u8;
        Challenge {
            espruar,
            dethek,
            path,
            row,
        }
    }

    /// The box number the prompt prints — `6 - code_row` (`ovr004.cs:65`), so
    /// row 0 is box 6 and row 5 is box 1.
    pub fn box_number(&self) -> u8 {
        6 - self.row
    }

    /// The path symbol string for this challenge.
    pub fn path_string(&self) -> &'static str {
        PATH_STRINGS
            .get(self.path as usize)
            .copied()
            // `default: code_path_str = string.Empty` (`ovr004.cs:58-60`) —
            // unreachable from `Random(3)`, transcribed rather than assumed.
            .unwrap_or("")
    }

    /// The wheel index into [`CODE_WHEEL`]`[row]` (`ovr004.cs:73-83`):
    /// `var_6 + 0x22 - var_7 + code_path*12 + ((5 - code_row) << 1)`,
    /// normalized into `0..36`.
    ///
    /// Signed arithmetic on purpose: `var_6 - var_7` can be as low as −21, and
    /// the original's `while (code_index < 0) code_index += 36` loop is
    /// reachable (`espruar 0`, `dethek 21`, `path 0`, `row 5` gives −55 before
    /// wrapping).
    pub fn wheel_index(&self) -> usize {
        let mut index = i32::from(self.espruar) + 0x22 - i32::from(self.dethek)
            + i32::from(self.path) * 12
            + ((5 - i32::from(self.row)) << 1);
        index = index.rem_euclid(WHEEL_WIDTH as i32);
        index as usize
    }

    /// `input_expected` (`ovr004.cs:85`) — the character the wheel reveals.
    pub fn answer(&self) -> char {
        let row = CODE_WHEEL[(self.row as usize).min(CODE_WHEEL.len() - 1)];
        row.as_bytes()[self.wheel_index()] as char
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `docs/copy-protection.md`'s own flag, discharged: the published table
    /// showed 35 characters in row 0. coab's has 36, and so does every other
    /// row — the wheel arithmetic wraps mod 36 and would read out of bounds
    /// otherwise.
    #[test]
    fn every_wheel_row_is_thirty_six_characters() {
        for (i, row) in CODE_WHEEL.iter().enumerate() {
            assert_eq!(row.len(), WHEEL_WIDTH, "row {i}");
            assert!(row.is_ascii(), "row {i} must be ASCII");
        }
    }

    /// Row 5 is the wheel's key row: the 36 rim labels in order.
    #[test]
    fn the_key_row_labels_the_thirty_six_rim_positions() {
        assert_eq!(CODE_WHEEL[5], "ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890");
    }

    /// The index arithmetic, transcribed literally and checked at the corners
    /// the `while` loops exist for.
    #[test]
    fn the_wheel_index_wraps_in_both_directions() {
        // The low corner: 0 + 34 - 21 + 0 + 0 = 13 -> no wrap.
        let low = Challenge {
            espruar: 0,
            dethek: 21,
            path: 0,
            row: 5,
        };
        assert_eq!(low.wheel_index(), 13);
        // The high corner: 25 + 34 - 0 + 24 + 10 = 93 -> 93 - 72 = 21.
        let high = Challenge {
            espruar: 25,
            dethek: 0,
            path: 2,
            row: 0,
        };
        assert_eq!(high.wheel_index(), 21);
        // Every reachable challenge lands inside the row.
        for espruar in 0..26u8 {
            for dethek in 0..22u8 {
                for path in 0..3u8 {
                    for row in 0..6u8 {
                        let c = Challenge {
                            espruar,
                            dethek,
                            path,
                            row,
                        };
                        assert!(c.wheel_index() < WHEEL_WIDTH);
                        assert!(c.answer().is_ascii_alphanumeric());
                    }
                }
            }
        }
    }

    /// The printed box number is `6 - code_row`, so the six rows map onto
    /// boxes 6..1 — never 0, never 7.
    #[test]
    fn the_box_number_counts_down_from_six() {
        for row in 0..6u8 {
            let c = Challenge {
                espruar: 0,
                dethek: 0,
                path: 0,
                row,
            };
            assert_eq!(c.box_number(), 6 - row);
            assert!((1..=6).contains(&c.box_number()));
        }
    }

    /// The three path strings, in `Random(3)` order.
    #[test]
    fn the_path_strings_are_the_three_spirals() {
        let dotted = Challenge {
            espruar: 0,
            dethek: 0,
            path: 0,
            row: 0,
        };
        assert_eq!(dotted.path_string(), "-..-..-..");
        let dashed = Challenge {
            espruar: 0,
            dethek: 0,
            path: 1,
            row: 0,
        };
        assert_eq!(dashed.path_string(), "- - - - -");
        let dots = Challenge {
            espruar: 0,
            dethek: 0,
            path: 2,
            row: 0,
        };
        assert_eq!(dots.path_string(), ".........");
    }

    /// The draw order is the original's: espruar, dethek, path, row — four
    /// draws, no more.
    #[test]
    fn posing_a_challenge_costs_exactly_four_draws() {
        let mut rng = crate::rng::EngineRng::new(12345);
        let mut expected = crate::rng::EngineRng::new(12345);
        let posed = Challenge::pose(&mut rng);
        assert_eq!(posed.espruar as u16, expected.random(ESPRUAR_COUNT));
        assert_eq!(posed.dethek as u16, expected.random(DETHEK_COUNT));
        assert_eq!(posed.path as u16, expected.random(3));
        assert_eq!(posed.row as u16, expected.random(6));
        assert_eq!(rng.state(), expected.state(), "exactly four draws");
        assert!(posed.espruar < 26 && posed.dethek < 22);
    }
}
