//! ★ **`MapCursor`** (`engine/ovr028.cs`) — the blinking white square that
//! marks where the party is on the Dalelands map (roll-credits D-S7b).
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/ovr028.cs` (the whole 41-line class): the two 33-entry
//!   coordinate tables, `SetPosition` (`sub_6E005`), `Draw` (`sub_6E02E`) and
//!   `Restore` (`sub_6E05D`).
//! - `engine/ovr027.cs` `displayInput` (`:165-172,176-183,315-323,331-336`):
//!   the four sites that prime, blink and finally erase it, and the cadence
//!   they blink at.
//! - `engine/seg001.cs:199-202`: what the cursor *is* — a `DaxBlock(0, 1, 1, 8)`
//!   (one item, one 8-pixel column, eight rows) filled with colour `0x0F`, i.e.
//!   a solid white 8×8 cell.
//! - `engine/seg040.cs` `ega_backup` (`:35-56`): the backup is a straight
//!   pixel-for-pixel read of the same 8×8 cell out of the framebuffer.
//!
//! ## Where it blinks, and where it does not
//!
//! Only `displayInput` blinks it. `VertMenuSelect` (`ovr027.cs`'s other wait
//! loop, the one VERTICAL MENU parks in) has no cursor code at all — so the
//! JOURNEY destination list is the one overland prompt the cursor sits out.
//! That asymmetry is the original's, and it is transcribed rather than
//! smoothed: the blink lives at the same seam
//! [`crate::picture::menu_wait_animation`] does, which is the `Widget::Hotbar`
//! (HORIZONTAL MENU) park.
//!
//! ## The cadence
//!
//! `displayInput` opens with `timeCursorOn = now + 300ms` and
//! `timeCursorOff = timeCursorOn + 500ms`, and each event re-arms the *other*
//! one: drawing sets `timeCursorOn = timeCursorOff + 300ms`, restoring sets
//! `timeCursorOff = timeCursorOn + 500ms`. Unrolled, that is **500 ms lit,
//! 300 ms dark**, after an initial 300 ms of dark — not the symmetric blink
//! the two constants suggest.
//!
//! ## The backup
//!
//! `Draw` re-runs `ega_backup` every time, and `Restore` blits it back, so the
//! saved cell is always the pristine background. Ours does the same against
//! [`Framebuffer`]. The pre-loop `Draw(); Restore();` pair at
//! `ovr027.cs:170-171` is a no-op in a model that captures on draw (it exists
//! in the original only to prime `cursor_bkup` before the loop can restore
//! from it), so it has no counterpart here.

use crate::framebuffer::Framebuffer;
use crate::shell::GameState;

/// `city_map_x` (`ovr028.cs:7-11`, `unk_16D5A`) — 33 entries, in CELLS.
pub const CITY_MAP_X: [u8; CITY_COUNT] = [
    0x04, 0x0C, 0x15, 0x0B, 0x1D, 0x14, 0x26, 0x15, //
    0x1E, 0x1F, 0x19, 0x25, 0x1C, 0x1D, 0x03, 0x0C, //
    0x19, 0x1D, 0x1D, 0x21, 0x13, 0x10, 0x09, 0x10, //
    0x14, 0x15, 0x19, 0x19, 0x1A, 0x1F, 0x25, 0x22, //
    0x0F,
];

/// `city_map_y` (`ovr028.cs:13-17`, `unk_16D7A`) — 33 entries, in CELLS.
pub const CITY_MAP_Y: [u8; CITY_COUNT] = [
    0x0F, 0x08, 0x0B, 0x04, 0x0A, 0x04, 0x01, 0x02, //
    0x0D, 0x0F, 0x03, 0x05, 0x02, 0x08, 0x0C, 0x0D, //
    0x0A, 0x0C, 0x09, 0x09, 0x08, 0x06, 0x06, 0x03, //
    0x02, 0x02, 0x03, 0x02, 0x03, 0x04, 0x02, 0x01, //
    0x00,
];

/// Both tables' length. The first fourteen entries are the named regions
/// `ECL1#80 @0x9012`'s `ON GOTO` knows (TILVERTON … MYTH DRANNOR); the rest
/// are the en-route waypoints `@0x910C`'s `GETTABLE 0x9D13` points the cursor
/// at while a journey's encounter is on screen. `ovr011.cs`'s `CityInfo`
/// terrain table is indexed by the very same value and is also 33 long.
pub const CITY_COUNT: usize = 33;

/// `gbl.bigpic_block_id == 0x79` (`ovr027.cs:166`) — the Dalelands map. The
/// cursor is drawn in map coordinates, so it only means anything over that one
/// backdrop; the other three overland BIGPICs (`0x7B`, `0x78`, `0x7A`) get no
/// cursor.
pub const DALELANDS_BIGPIC: u8 = 0x79;

/// `gbl.cursor` (`seg001.cs:200-202`): `DaxBlock(0, 1, 1, 8)` — `width = 1`
/// column of 8 pixels, `height = 8` rows — every byte `0x0F`.
const CURSOR_W: usize = 8;
const CURSOR_H: usize = 8;
const CURSOR_COLOUR: u8 = 0x0F;

/// `300` and `500` milliseconds at [`crate::input::TICK_HZ`] — the same
/// 100 ms = 6 ticks conversion `crate::picture::menu_wait_animation` uses.
const TICKS_ON: u32 = 30;
const TICKS_OFF: u32 = 18;

/// `MapCursor.SetPosition` (`ovr028.cs:22-26`): the cell the cursor occupies,
/// as `(x, y)`. `None` for an index past the tables — the original would read
/// out of bounds, which is not a behaviour worth reproducing.
pub fn position(city: u8) -> Option<(u8, u8)> {
    let i = city as usize;
    Some((*CITY_MAP_X.get(i)?, *CITY_MAP_Y.get(i)?))
}

/// The three-condition gate all four `displayInput` sites share, verbatim
/// (`ovr027.cs:165-167`): `game_state == WildernessMap && bigpic_block_id ==
/// 0x79 && lastDaxBlockId != 0x50`.
///
/// The third conjunct is the city-scene guard
/// ([`crate::picture::CITY_SCENE_PIC_BLOCK`]): inside a city menu the map is
/// not on screen, so blinking a cursor over the city picture would be
/// nonsense.
pub fn blinks(game_state: GameState, bigpic_block: Option<u8>, last_dax_block: u8) -> bool {
    game_state == GameState::WildernessMap
        && bigpic_block == Some(DALELANDS_BIGPIC)
        && last_dax_block != crate::picture::CITY_SCENE_PIC_BLOCK
}

/// One parked prompt's cursor blink — `displayInput`'s two timers plus
/// `gbl.cursor_bkup`, which in the original are a stack local pair and a
/// global scratch block. Transient by construction: a restored save simply
/// starts its next prompt's blink from dark.
#[derive(Debug, Clone, Default)]
pub struct MapCursorBlink {
    /// Ticks since the prompt opened (`displayInput`'s `timeStart`).
    elapsed: u32,
    /// `timeCursorOn` / `timeCursorOff`, in ticks from `timeStart`.
    next_on: u32,
    next_off: u32,
    /// `gbl.cursor_bkup` + whether it currently holds a pristine background.
    backup: Option<([u8; CURSOR_W * CURSOR_H], usize, usize)>,
    armed: bool,
}

impl MapCursorBlink {
    /// `displayInput`'s prologue (`ovr027.cs:151-153`): both timers set from
    /// `timeStart`. Idempotent — the first tick of a park arms it, later ticks
    /// leave it alone.
    fn arm(&mut self) {
        if self.armed {
            return;
        }
        self.armed = true;
        self.elapsed = 0;
        self.next_on = TICKS_OFF; // `timeStart + 300ms`
        self.next_off = TICKS_OFF + TICKS_ON; // `timeCursorOn + 500ms`
    }

    /// One wait-loop iteration (`ovr027.cs:176-183` then `:315-323`): draw at
    /// `timeCursorOn`, restore at `timeCursorOff`, each re-arming the other.
    ///
    /// `pos` is the cell [`position`] resolved for the live `current_city`;
    /// the original resolves it once at `:169` and never again inside the
    /// loop, so a mid-prompt `current_city` change would not move the original's
    /// cursor either — the caller passes the value it read when the prompt
    /// opened.
    pub fn tick(&mut self, fb: &mut Framebuffer, pos: (u8, u8), dt_ticks: u32) {
        self.arm();
        self.elapsed = self.elapsed.saturating_add(dt_ticks);
        if self.elapsed >= self.next_on {
            self.draw(fb, pos);
            self.next_on = self.next_off + TICKS_OFF; // `:182`
        }
        if self.elapsed >= self.next_off {
            self.restore(fb);
            self.next_off = self.next_on + TICKS_ON; // `:322`
        }
    }

    /// `MapCursor.Draw` (`ovr028.cs:29-33`): back the cell up, then paint it
    /// solid `0x0F`.
    fn draw(&mut self, fb: &mut Framebuffer, pos: (u8, u8)) {
        let (x0, y0) = (pos.0 as usize * 8, pos.1 as usize * 8);
        let mut saved = [0u8; CURSOR_W * CURSOR_H];
        for dy in 0..CURSOR_H {
            for dx in 0..CURSOR_W {
                saved[dy * CURSOR_W + dx] = fb.get_pixel(x0 + dx, y0 + dy);
                fb.set_pixel(x0 + dx, y0 + dy, CURSOR_COLOUR);
            }
        }
        self.backup = Some((saved, x0, y0));
    }

    /// `MapCursor.Restore` (`ovr028.cs:36-39`): blit the backup back. A no-op
    /// when nothing was ever drawn, which is what makes `:331-336`'s
    /// unconditional exit restore safe.
    pub fn restore(&mut self, fb: &mut Framebuffer) {
        let Some((saved, x0, y0)) = self.backup.take() else {
            return;
        };
        for dy in 0..CURSOR_H {
            for dx in 0..CURSOR_W {
                fb.set_pixel(x0 + dx, y0 + dy, saved[dy * CURSOR_W + dx]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables are 33 long and every coordinate lands inside the 40×25 cell
    /// grid the Dalelands BIGPIC is drawn on.
    #[test]
    fn every_city_lands_on_the_screen() {
        assert_eq!(CITY_MAP_X.len(), CITY_COUNT);
        assert_eq!(CITY_MAP_Y.len(), CITY_COUNT);
        for city in 0..CITY_COUNT as u8 {
            let (x, y) = position(city).expect("in range");
            assert!(x < 40, "city {city} x={x}");
            assert!(y < 25, "city {city} y={y}");
        }
        assert_eq!(position(CITY_COUNT as u8), None);
    }

    /// Two spot checks against `ovr028.cs`'s literal rows: the first entry and
    /// the odd-one-out last, whose `y` of 0 puts it on the frame's top row.
    #[test]
    fn the_table_matches_the_original_at_both_ends() {
        assert_eq!(position(0), Some((0x04, 0x0F)));
        assert_eq!(position(9), Some((0x1F, 0x0F)));
        assert_eq!(position(32), Some((0x0F, 0x00)));
    }

    #[test]
    fn the_blink_needs_all_three_conditions() {
        use crate::picture::{CITY_SCENE_PIC_BLOCK, NO_DAX_BLOCK};
        assert!(blinks(
            GameState::WildernessMap,
            Some(DALELANDS_BIGPIC),
            NO_DAX_BLOCK
        ));
        assert!(!blinks(
            GameState::DungeonMap,
            Some(DALELANDS_BIGPIC),
            NO_DAX_BLOCK
        ));
        assert!(!blinks(GameState::WildernessMap, Some(0x7B), NO_DAX_BLOCK));
        assert!(!blinks(
            GameState::WildernessMap,
            Some(DALELANDS_BIGPIC),
            CITY_SCENE_PIC_BLOCK
        ));
    }

    /// ★ The cadence, unrolled: dark for 300 ms, then **lit for 500 ms and
    /// dark for 300 ms**, forever. The two timers re-arm each other
    /// (`ovr027.cs:182,322`), which is what makes the duty cycle asymmetric.
    #[test]
    fn the_blink_is_lit_500ms_and_dark_300ms_after_a_300ms_lead_in() {
        let mut fb = Framebuffer::new();
        for y in 0..25 * 8 {
            for x in 0..40 * 8 {
                fb.set_pixel(x, y, 3);
            }
        }
        let mut blink = MapCursorBlink::default();
        let pos = (4u8, 4u8);
        let lit = |fb: &Framebuffer| fb.get_pixel(4 * 8, 4 * 8) == CURSOR_COLOUR;

        let mut history = Vec::new();
        for _ in 0..(TICKS_OFF + 3 * (TICKS_ON + TICKS_OFF)) {
            blink.tick(&mut fb, pos, 1);
            history.push(lit(&fb));
        }

        // Ticks 1..=17 dark (the 300 ms lead-in), 18..=47 lit, 48..=65 dark,
        // 66..=95 lit — measured off the history rather than asserted
        // tick-by-tick so the intent survives a tick-rate change.
        let flips: Vec<usize> = (1..history.len())
            .filter(|&i| history[i] != history[i - 1])
            .collect();
        assert!(flips.len() >= 3, "history: {history:?}");
        assert_eq!(flips[0] + 1, TICKS_OFF as usize, "the 300ms lead-in");
        assert_eq!(flips[1] - flips[0], TICKS_ON as usize, "500ms lit");
        assert_eq!(flips[2] - flips[1], TICKS_OFF as usize, "300ms dark");
        assert!(history[flips[0]], "the first flip is to lit");
    }

    /// Restore puts back exactly what Draw covered, so a blink leaves no
    /// residue on the map — `displayInput`'s exit restore (`:335`) depends on
    /// it, and so does every iteration in between.
    #[test]
    fn a_full_blink_leaves_the_background_untouched() {
        let mut fb = Framebuffer::new();
        for y in 0..25 * 8 {
            for x in 0..40 * 8 {
                fb.set_pixel(x, y, ((x + y) % 15) as u8);
            }
        }
        let before: Vec<u8> = (0..64)
            .map(|i| fb.get_pixel(4 * 8 + i % 8, 4 * 8 + i / 8))
            .collect();

        let mut blink = MapCursorBlink::default();
        for _ in 0..(TICKS_OFF + TICKS_ON + TICKS_OFF) {
            blink.tick(&mut fb, (4, 4), 1);
        }
        blink.restore(&mut fb);

        let after: Vec<u8> = (0..64)
            .map(|i| fb.get_pixel(4 * 8 + i % 8, 4 * 8 + i / 8))
            .collect();
        assert_eq!(before, after);
    }
}
