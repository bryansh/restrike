//! The combat screen's geometry (`docs/design/combat-visualizer.md` §1.1–1.2)
//! — every region, row and pixel origin the scene draws into, as constants
//! with their coab sites.
//!
//! Text units are 8×8 cells on the 40×25 grid; the map viewport's units are
//! 24×24 cells, three text cells to a side (`IconColumnSize = 3`,
//! `ovr033.cs:316`).
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/seg037.cs` `DrawFrame_Combat` (`:148-172`) — the border rows and
//!   columns (drawn by [`crate::frames::draw_frame_combat`]).
//! - `engine/ovr033.cs` `ScreenMapCheck` (`:316-334`) — the 7×7 tile loop and
//!   its `screenRowY/screenColX += 3` stride.
//! - `engine/seg040.cs` `draw_combat_picture` (`:115-118`) — the hard pixel
//!   clip `[8,176) × [8,176)` every map draw passes through, and
//!   `OverlayUnbounded`'s `rowY + 1` / `colX + 1` cell offset (`:23-26`).
//! - `engine/ovr034.cs` `draw_combat_icon` (`:90-98`) — `(tileY*3)+1`,
//!   `(tileX*3)+1`.
//! - `engine/seg041.cs:119-123` — the `CombatSummary` text region
//!   (`{0x15, 0x26, 1, 0x17}`), which is our existing
//!   [`crate::text::COMBAT_SUMMARY`].
//! - `engine/ovr025.cs` `ClearPlayerTextArea` (`:815-825`) — the message
//!   region, rows `0x0A..=0x15` of the panel.
//! - `engine/ovr027.cs` `ClearPromptAreaNoUpdate` (`:352-355`) — row 0x18.
//! - `engine/ovr014.cs:1904,2231` / `ovr023.cs:810` — the status line's own
//!   clear, `draw8x8_clear_area(0x17, 0x27, 0x17, 0)`.

use crate::draw::Clip;
use crate::text::TextRegion;

/// The map viewport is 7×7 combat cells (`ScreenMapCheck`'s `i/j <= 6`; the
/// combat core's [`crate::combat::SCREEN_MAX`] `+ 1`).
pub const VIEWPORT_CELLS: usize = 7;

/// One combat cell is 3 text cells / 24 pixels a side (`IconColumnSize`).
pub const CELL_TEXT_COLS: usize = 3;

/// The map viewport's top-left pixel — `OverlayUnbounded`'s `+1` cell offset
/// applied to screen cell (0,0): text cell (1,1) = pixel (8,8).
pub const VIEWPORT_ORIGIN_PX: (usize, usize) = (8, 8);

/// The hard clip every map-viewport draw runs under
/// (`draw_combat_picture`'s `8, 176, 8, 176`) — pixels (8,8)–(175,175).
/// Identical to [`Clip::OVERLAY`], named here so the scene's call sites read
/// as the map viewport rather than as a generic overlay.
pub const VIEWPORT_CLIP: Clip = Clip::OVERLAY;

/// The right-panel summary region (`seg041.bounds[CombatSummary]`) — rows
/// 1..=0x15, cols 0x17..=0x26.
pub const PANEL_REGION: TextRegion = crate::text::COMBAT_SUMMARY;

/// The panel's **message** region — `ClearPlayerTextArea`'s
/// `draw8x8_clear_area(0x15, 0x26, 0x0a, 0x17)`, i.e. rows 10..=0x15 of the
/// same columns (§1.1: "messages print in the same panel from row 10 down").
/// Slice 3 clears it and pins its geometry; slice 4 fills it.
pub const MESSAGE_REGION: TextRegion = TextRegion {
    y_start: 0x0A,
    y_end: 0x15,
    x_start: 0x17,
    x_end: 0x26,
};

/// The status line ("Range = N", "Spell:&lt;name&gt;") — row 0x17, all 40
/// columns.
pub const STATUS_ROW: usize = 0x17;

/// The prompt/menu line — row 0x18, all 40 columns (`ClearPromptArea`).
pub const PROMPT_ROW: usize = 0x18;

/// The last text column (`0x27`), the right edge of both full-width rows.
pub const LAST_COL: usize = 0x27;

// --- the panel's own coordinates (`CombatDisplayPlayerSummary`,
// `ovr025.cs:291-335`) ------------------------------------------------------

/// The panel's left text column — every label starts here.
pub const PANEL_COL: usize = 0x17;
/// The combatant's name row (`DisplayPlayerStatusString(false, 1, …)`).
pub const PANEL_NAME_ROW: usize = 1;
/// The row `DisplayPlayerStatusString`'s wrapped text starts on.
pub const PANEL_STATUS_TEXT_ROW: usize = 2;
/// The "Hitpoints" label row (`line + 1` with `line == 2`).
pub const PANEL_HP_ROW: usize = 3;
/// The hit-point value's column.
pub const PANEL_HP_COL: usize = 0x21;
/// The "AC" label row (`line + 1` with `line == 4`).
pub const PANEL_AC_ROW: usize = 5;
/// The AC value's column.
pub const PANEL_AC_COL: usize = 0x1A;
/// The readied weapon's wrap region — rows `line+1..=line+3` with `line == 6`.
pub const PANEL_WEAPON_REGION: TextRegion = TextRegion {
    y_start: 7,
    y_end: 9,
    x_start: PANEL_COL,
    x_end: 0x26,
};

// --- panel colours (`ovr025.cs:270-289,827-847`) ---------------------------

/// A party combatant's name colour.
pub const NAME_COLOR_PARTY: u8 = 0x0B;
/// An enemy combatant's name colour.
pub const NAME_COLOR_ENEMY: u8 = 0x0E;
/// A removed (`in_combat == false`) combatant's name colour — checked FIRST,
/// so a downed party member's name goes this colour, not the party one.
pub const NAME_COLOR_REMOVED: u8 = 0x0C;
/// Hit points at full — green.
pub const HP_COLOR_FULL: u8 = 0x0A;
/// Hit points below max — yellow.
pub const HP_COLOR_DAMAGED: u8 = 0x0E;
/// The label colour ("Hitpoints", "AC", the AC value, wrapped text).
pub const LABEL_COLOR: u8 = 10;
/// The health-status / "(Helpless)" line's colour.
pub const STATUS_COLOR: u8 = 15;

/// Screen cell → its top-left **text cell** `(row, col)`: `(cy*3)+1`,
/// `(cx*3)+1` (`draw_combat_icon`, and `DrawIsoTile` via
/// `OverlayUnbounded`'s `+1`).
pub fn screen_cell_text_origin(cx: usize, cy: usize) -> (usize, usize) {
    (cy * CELL_TEXT_COLS + 1, cx * CELL_TEXT_COLS + 1)
}

/// Screen cell → its top-left **pixel**: (8 + 24·cx, 8 + 24·cy) (§1.2).
pub fn screen_cell_pixel_origin(cx: usize, cy: usize) -> (usize, usize) {
    let (row, col) = screen_cell_text_origin(cx, cy);
    (col * 8, row * 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_spans_pixels_8_to_175() {
        assert_eq!(screen_cell_pixel_origin(0, 0), VIEWPORT_ORIGIN_PX);
        let (x, y) = screen_cell_pixel_origin(VIEWPORT_CELLS - 1, VIEWPORT_CELLS - 1);
        assert_eq!((x, y), (152, 152));
        // The last cell's far edge is the clip's exclusive bound.
        assert_eq!(x + 24, VIEWPORT_CLIP.x1);
        assert_eq!(y + 24, VIEWPORT_CLIP.y1);
        assert_eq!((VIEWPORT_CLIP.x0, VIEWPORT_CLIP.y0), VIEWPORT_ORIGIN_PX);
    }

    #[test]
    fn viewport_cells_match_the_combat_cores_screen_max() {
        assert_eq!(VIEWPORT_CELLS as i32, crate::combat::SCREEN_MAX + 1);
    }

    #[test]
    fn the_middle_border_column_sits_between_viewport_and_panel() {
        // The viewport's last text column is 21 (0x15); the border is 0x16;
        // the panel starts at 0x17.
        let (_, col) = screen_cell_text_origin(VIEWPORT_CELLS - 1, 0);
        assert_eq!(col + CELL_TEXT_COLS - 1, 0x15);
        assert_eq!(PANEL_REGION.x_start, 0x17);
    }
}
