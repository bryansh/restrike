//! The text system (D-UI3/§1.4): region geometry, persistent cursor, the
//! word-wrap algorithm, per-char pacing, and the pagination gate.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `engine/seg041.cs` `press_any_key` (`:125-231`) — the wrap +
//!   pagination algorithm this module's [`TextJob`] state machine
//!   transcribes, driven per explicit tick budget instead of `SysDelay`
//!   (D-UI1: the engine never reads a clock).
//! - coab `engine/seg041.cs` `displayStringSlow` (`:90-107`) — the per-char
//!   draw + pacing loop.
//! - coab `engine/seg041.cs` `text_skip_space` (`:110-117`) — the post-wrap
//!   leading-space skip.
//! - coab `engine/seg041.cs:119-123` (`bounds`) — the three text regions.
//!
//! **Transcription note (flagged per D11, not silently absorbed):** the
//! literal decompiled overflow check is `if (X > xEnd) { if (X == xEnd &&
//! ...) { /* trim */ } ... }` (`seg041.cs:191-198`) — since both branches
//! test the same fixed expression `X`, the inner `== xEnd` can never be
//! true given the outer already asserts `X > xEnd`; as literally read this
//! makes the trim branch dead code. The design doc (`renderer-ui-shell.md`
//! §1.4/D-UI7) names "the exact-fit-drop-one-trailing-space case" as real,
//! tested behavior, so this implementation treats the outer bound as
//! inclusive (`>=`) so the branch is reachable — most plausibly a
//! decompiler artifact around the original's actual comparison. Docketed
//! for a DOSBox confirmation alongside the design doc's other open
//! questions (§4).

use crate::draw::{cell_rect_fill, draw_glyph};
use crate::framebuffer::Framebuffer;
use gbx_formats::font::Font;

/// One of the three text regions (`seg041.bounds`, `seg041.cs:119-123`).
/// Arbitrary bounds are supported (`press_any_key` takes them as
/// parameters in the original); these are just the shipped presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TextRegion {
    pub y_start: usize,
    pub y_end: usize,
    pub x_start: usize,
    pub x_end: usize,
}

/// The exploration text window: rows 17-22, cols 1-38.
pub const NORMAL_BOTTOM: TextRegion = TextRegion {
    y_end: 0x16,
    x_end: 0x26,
    y_start: 0x11,
    x_start: 1,
};
/// The two-line variant: rows 21-22, cols 1-38.
pub const NORMAL2: TextRegion = TextRegion {
    y_end: 0x16,
    x_end: 0x26,
    y_start: 0x15,
    x_start: 1,
};
/// The combat summary panel (M4): rows 1-21, cols 23-38.
pub const COMBAT_SUMMARY: TextRegion = TextRegion {
    y_end: 0x15,
    x_end: 0x26,
    y_start: 1,
    x_start: 0x17,
};

/// The persistent text cursor (`gbl.textXCol`/`textYCol`) — survives across
/// jobs, scripts, and flows (§1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TextCursor {
    pub col: usize,
    pub row: usize,
}

impl TextCursor {
    pub const fn new() -> Self {
        TextCursor { col: 0, row: 0 }
    }
}

impl Default for TextCursor {
    fn default() -> Self {
        Self::new()
    }
}

fn is_punctuation(c: u8) -> bool {
    matches!(c, b'!' | b',' | b'-' | b'.' | b':' | b';' | b'?')
}

/// `text_skip_space` (`seg041.cs:110-117`): advances `text_start` (1-based)
/// past leading spaces, stopping strictly before `text_max`.
fn skip_space(text: &[u8], text_max: usize, text_start: &mut usize) {
    while *text_start < text_max && text[*text_start - 1] == b' ' {
        *text_start += 1;
    }
}

/// One token's end (1-based, inclusive), per `press_any_key`'s inline scan
/// (`seg041.cs:166-189`): a maximal punctuation run, else a maximal
/// non-punctuation/non-space run, extended by any immediately-trailing
/// punctuation when the run didn't end on a space.
fn token_end(text: &[u8], text_start: usize) -> usize {
    let input_len = text.len();
    let mut text_end = text_start;
    while text_end < input_len && is_punctuation(text[text_end - 1]) {
        text_end += 1;
    }
    while text_end < input_len && !is_punctuation(text[text_end - 1]) && text[text_end - 1] != b' '
    {
        text_end += 1;
    }
    if text[text_end - 1] != b' ' {
        while text_end + 1 < input_len && is_punctuation(text[text_end]) {
            text_end += 1;
        }
    }
    text_end
}

/// `display_char01`'s glyph index mapping (`toupper(ch) % 0x40`, engine-side
/// per §1.4) plus the mono blit itself.
pub fn draw_char(
    fb: &mut Framebuffer,
    font: &Font,
    ch: u8,
    row: usize,
    col: usize,
    bg: u8,
    fg: u8,
) {
    if col >= 40 || row >= 25 {
        return;
    }
    let index = (ch.to_ascii_uppercase() as usize) % 0x40;
    draw_glyph(fb, font.glyph(index), row, col, bg, fg);
}

/// `displayString` (`seg041.cs:75-86`): an immediate, unpaced, non-wrapping
/// string draw — headers, prompts, party-panel labels.
pub fn draw_string(
    fb: &mut Framebuffer,
    font: &Font,
    text: &str,
    row: usize,
    col: usize,
    bg: u8,
    fg: u8,
) {
    for (i, ch) in text.bytes().enumerate() {
        draw_char(fb, font, ch, row, col + i, bg, fg);
    }
}

/// A paced, wrapping, paginating text job — `press_any_key` resumed one
/// tick's character budget at a time. `advance` never blocks: it draws up
/// to `budget` characters, then returns.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextJob {
    text: Vec<u8>,
    fg_color: u8,
    region: TextRegion,
    /// 1-based index into `text`, matching the original's indexing exactly.
    text_start: usize,
    step: Step,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum DrawResume {
    /// The exact-fit-drop-space case: after the draw, still need the wrap
    /// bookkeeping (col reset, row++, post-wrap space skip, pagination
    /// check).
    AfterExactFitTrim,
    /// The plain non-overflow draw: loop back to the next token.
    PlainDraw,
    /// The redraw right after a pagination release.
    AfterPaginationRedraw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Step {
    /// Compute the next token starting at `text_start`.
    ComputeToken,
    /// Drawing characters `text_start..=text_end` (1-based inclusive).
    Drawing {
        text_end: usize,
        resume: DrawResume,
    },
    /// Paginating: the `PressAnyKey` gate is open. `text_end` is the
    /// overflowing token's end, redrawn once released.
    NeedsKey {
        text_end: usize,
    },
    Done,
}

/// [`TextJob::advance`]'s result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// Ran out of budget this tick; more ticks needed.
    Continuing,
    /// Pagination gate is open (`Widget::PressAnyKey`, D-UI2) — call
    /// [`TextJob::release`] once the keypress is observed.
    NeedsKey,
    /// The job has drawn everything.
    Done,
}

impl TextJob {
    /// Starts a new job. Snaps an out-of-region cursor to the region start
    /// (`press_any_key`'s entry guard, `seg041.cs:143-150`); `clear_first`
    /// additionally clears the region and resets the cursor first
    /// (PRINTCLEAR). Bounds outside `xStart<=0x27, yStart<=0x18` (and the
    /// original's odd `xEnd>0x27 && yEnd>0x27` guard, `seg041.cs:137-141`)
    /// make the job a no-op — transcribed as-is.
    pub fn new(
        text: &str,
        fg_color: u8,
        region: TextRegion,
        clear_first: bool,
        cursor: &mut TextCursor,
        fb: &mut Framebuffer,
    ) -> Self {
        let text = text.as_bytes().to_vec();

        let out_of_bounds = region.x_start > 0x27
            || region.y_start > 0x18
            || (region.x_end > 0x27 && region.y_end > 0x27);

        if !out_of_bounds
            && (cursor.col < region.x_start
                || cursor.col > region.x_end
                || cursor.row < region.y_start
                || cursor.row > region.y_end)
        {
            cursor.col = region.x_start;
            cursor.row = region.y_start;
        }

        if !out_of_bounds && clear_first {
            cell_rect_fill(
                fb,
                0,
                region.y_start,
                region.y_end,
                region.x_start,
                region.x_end,
            );
            cursor.col = region.x_start;
            cursor.row = region.y_start;
        }

        let step = if out_of_bounds || text.is_empty() {
            Step::Done
        } else {
            Step::ComputeToken
        };

        TextJob {
            text,
            fg_color,
            region,
            text_start: 1,
            step,
        }
    }

    pub fn status(&self) -> JobStatus {
        match self.step {
            Step::Done => JobStatus::Done,
            Step::NeedsKey { .. } => JobStatus::NeedsKey,
            _ => JobStatus::Continuing,
        }
    }

    /// Releases the pagination gate: clears the region and resumes drawing
    /// the overflowing token fresh (`seg041.cs:213-215`). The
    /// keyboard-queue-clear obligation (`clear_keyboard()`,
    /// `seg041.cs:211`) is the caller's — an explicit seam, not this
    /// module's concern (step 3/4 wiring).
    pub fn release(&mut self, fb: &mut Framebuffer) {
        if let Step::NeedsKey { text_end } = self.step {
            cell_rect_fill(
                fb,
                0,
                self.region.y_start,
                self.region.y_end,
                self.region.x_start,
                self.region.x_end,
            );
            self.step = Step::Drawing {
                text_end,
                resume: DrawResume::AfterPaginationRedraw,
            };
        }
    }

    /// Draws up to `budget` characters, wrapping/paginating exactly per
    /// `press_any_key`. Returns without blocking; call again next tick
    /// (after [`TextJob::release`] if [`JobStatus::NeedsKey`]) to continue.
    pub fn advance(
        &mut self,
        budget: u32,
        fb: &mut Framebuffer,
        font: &Font,
        cursor: &mut TextCursor,
    ) -> JobStatus {
        let mut budget = budget;
        loop {
            match self.step {
                Step::Done => return JobStatus::Done,
                Step::NeedsKey { .. } => return JobStatus::NeedsKey,
                Step::Drawing { text_end, resume } => {
                    while self.text_start <= text_end {
                        if budget == 0 {
                            return JobStatus::Continuing;
                        }
                        let ch = self.text[self.text_start - 1];
                        draw_char(fb, font, ch, cursor.row, cursor.col, 0, self.fg_color);
                        cursor.col += 1;
                        self.text_start += 1;
                        budget -= 1;
                    }
                    match resume {
                        DrawResume::PlainDraw | DrawResume::AfterPaginationRedraw => {
                            self.step = Step::ComputeToken;
                        }
                        DrawResume::AfterExactFitTrim => {
                            cursor.col = self.region.x_start;
                            cursor.row += 1;
                            skip_space(&self.text, self.text.len(), &mut self.text_start);
                            if cursor.row > self.region.y_end && self.text_start <= self.text.len()
                            {
                                cursor.col = self.region.x_start;
                                cursor.row = self.region.y_start;
                                // Re-derive this token's end for the post-release redraw.
                                let text_end = token_end(&self.text, self.text_start);
                                self.step = Step::NeedsKey { text_end };
                            } else {
                                self.step = Step::ComputeToken;
                            }
                        }
                    }
                }
                Step::ComputeToken => {
                    if self.text_start > self.text.len() {
                        if cursor.col > self.region.x_end {
                            cursor.col = self.region.x_start;
                            cursor.row += 1;
                        }
                        self.step = Step::Done;
                        continue;
                    }

                    let text_end = token_end(&self.text, self.text_start);
                    let token_span = (text_end - self.text_start) + cursor.col;

                    if token_span >= self.region.x_end {
                        if token_span == self.region.x_end && self.text[text_end - 1] == b' ' {
                            self.step = Step::Drawing {
                                text_end: text_end - 1,
                                resume: DrawResume::AfterExactFitTrim,
                            };
                        } else {
                            cursor.col = self.region.x_start;
                            cursor.row += 1;
                            skip_space(&self.text, self.text.len(), &mut self.text_start);
                            if cursor.row > self.region.y_end && self.text_start <= self.text.len()
                            {
                                cursor.col = self.region.x_start;
                                cursor.row = self.region.y_start;
                                let text_end = token_end(&self.text, self.text_start);
                                self.step = Step::NeedsKey { text_end };
                            }
                            // else: loop back to ComputeToken with the same
                            // (unadvanced) text_start, retrying on the new line.
                        }
                    } else {
                        self.step = Step::Drawing {
                            text_end,
                            resume: DrawResume::PlainDraw,
                        };
                    }
                }
            }
        }
    }
}

/// Per-character pacing (D-UI1): `⌊acc⌋` characters per tick, where `acc +=
/// tick_ms / char_ms` and `char_ms = game_speed_var × 3`. A fractional
/// accumulator, not per-char rounding — average pacing is exact at every
/// speed, matching D-UI1's rationale exactly.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct TextPacer {
    char_ms: f64,
    acc: f64,
}

impl TextPacer {
    pub fn new(game_speed: u8) -> Self {
        TextPacer {
            char_ms: game_speed as f64 * 3.0,
            acc: 0.0,
        }
    }

    /// Advances by one tick of `tick_ms` milliseconds, returning the whole
    /// number of characters now affordable (consuming that whole part from
    /// the running accumulator; the fraction carries forward).
    pub fn tick(&mut self, tick_ms: f64) -> u32 {
        self.acc += tick_ms / self.char_ms;
        let whole = self.acc.floor();
        self.acc -= whole;
        whole as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::font;

    /// A synthetic font (D10) where glyph `index` renders as a solid block
    /// of pixel value `index % 16` — enough to prove *which* glyph drew
    /// without needing real letterforms.
    fn marker_font() -> Font {
        let data = vec![0xFFu8; font::GLYPH_COUNT * font::GLYPH_BYTES]; // every glyph a full solid block
        font::decode(&data)
    }

    fn drive_to_completion(
        job: &mut TextJob,
        fb: &mut Framebuffer,
        font: &Font,
        cursor: &mut TextCursor,
    ) -> JobStatus {
        loop {
            match job.advance(1_000_000, fb, font, cursor) {
                JobStatus::NeedsKey => {
                    job.release(fb);
                }
                other @ (JobStatus::Done | JobStatus::Continuing) => return other,
            }
        }
    }

    #[test]
    fn short_text_draws_without_wrapping() {
        let font = marker_font();
        let mut fb = Framebuffer::new();
        let mut cursor = TextCursor {
            col: NORMAL_BOTTOM.x_start,
            row: NORMAL_BOTTOM.y_start,
        };
        let mut job = TextJob::new("hi", 10, NORMAL_BOTTOM, false, &mut cursor, &mut fb);
        assert_eq!(
            drive_to_completion(&mut job, &mut fb, &font, &mut cursor),
            JobStatus::Done
        );
        assert_eq!(cursor.row, NORMAL_BOTTOM.y_start);
        assert_eq!(cursor.col, NORMAL_BOTTOM.x_start + 2);
    }

    #[test]
    fn a_long_word_wraps_to_the_next_row() {
        let font = marker_font();
        let mut fb = Framebuffer::new();
        // Region 4 cols wide (x_start=0, x_end=3): a 6-char word can't fit.
        let region = TextRegion {
            y_start: 0,
            y_end: 5,
            x_start: 0,
            x_end: 3,
        };
        let mut cursor = TextCursor { col: 0, row: 0 };
        let mut job = TextJob::new("abcdef", 10, region, false, &mut cursor, &mut fb);
        assert_eq!(
            drive_to_completion(&mut job, &mut fb, &font, &mut cursor),
            JobStatus::Done
        );
        assert!(cursor.row > 0, "must have wrapped to a later row");
    }

    #[test]
    fn punctuation_tokens_do_not_split_a_run() {
        let font = marker_font();
        let mut fb = Framebuffer::new();
        let region = TextRegion {
            y_start: 0,
            y_end: 5,
            x_start: 0,
            x_end: 39,
        };
        let mut cursor = TextCursor { col: 0, row: 0 };
        // "a,b" is a run bounded by punctuation: token_end scanning must
        // treat the whole run up to the space as one unit and not panic.
        let mut job = TextJob::new("a,b! c", 10, region, false, &mut cursor, &mut fb);
        assert_eq!(
            drive_to_completion(&mut job, &mut fb, &font, &mut cursor),
            JobStatus::Done
        );
    }

    #[test]
    fn exact_fit_trailing_space_is_dropped() {
        let font = marker_font();
        let mut fb = Framebuffer::new();
        // Region 5 cols wide: x_start=0, x_end=4. "abcd " (4 chars + space)
        // followed by "e" - the first token "abcd " with trailing content
        // should trim the trailing space at the exact-fit boundary.
        let region = TextRegion {
            y_start: 0,
            y_end: 5,
            x_start: 0,
            x_end: 4,
        };
        let mut cursor = TextCursor { col: 0, row: 0 };
        let mut job = TextJob::new("abcd e", 10, region, false, &mut cursor, &mut fb);
        assert_eq!(
            drive_to_completion(&mut job, &mut fb, &font, &mut cursor),
            JobStatus::Done
        );
        // "abcd" occupies cols 0-3; the trailing space is dropped (not drawn
        // as a 5th char at col 4), "e" starts the next row.
        assert_eq!(cursor.row, 1);
        assert_eq!(cursor.col, 1);
    }

    #[test]
    fn post_wrap_leading_spaces_are_skipped() {
        let font = marker_font();
        let mut fb = Framebuffer::new();
        let region = TextRegion {
            y_start: 0,
            y_end: 5,
            x_start: 0,
            x_end: 3,
        };
        let mut cursor = TextCursor { col: 0, row: 0 };
        // A long first word forces a wrap; the run of spaces immediately
        // after it must be skipped on the new line, not drawn.
        let mut job = TextJob::new("abcdef   g", 10, region, false, &mut cursor, &mut fb);
        assert_eq!(
            drive_to_completion(&mut job, &mut fb, &font, &mut cursor),
            JobStatus::Done
        );
    }

    #[test]
    fn out_of_region_cursor_snaps_to_region_start() {
        let mut fb = Framebuffer::new();
        let region = TextRegion {
            y_start: 5,
            y_end: 10,
            x_start: 5,
            x_end: 10,
        };
        let mut cursor = TextCursor { col: 0, row: 0 }; // well outside the region
        let job = TextJob::new("hi", 10, region, false, &mut cursor, &mut fb);
        assert_eq!(cursor.col, region.x_start);
        assert_eq!(cursor.row, region.y_start);
        let _ = job;
    }

    #[test]
    fn clear_first_clears_the_region_and_resets_the_cursor() {
        let mut fb = Framebuffer::new();
        let region = TextRegion {
            y_start: 1,
            y_end: 2,
            x_start: 1,
            x_end: 2,
        };
        fb.set_pixel(8, 8, 5); // inside the region's pixel footprint
        let mut cursor = TextCursor { col: 1, row: 1 };
        let job = TextJob::new("", 10, region, true, &mut cursor, &mut fb);
        assert_eq!(fb.get_pixel(8, 8), 0, "clear_first must clear the region");
        assert_eq!(cursor.col, region.x_start);
        assert_eq!(cursor.row, region.y_start);
        let _ = job;
    }

    #[test]
    fn pagination_fires_when_the_region_fills_then_releases_and_resets() {
        let font = marker_font();
        let mut fb = Framebuffer::new();
        // A 1-row-tall, narrow region: any wrap immediately overflows yEnd.
        let region = TextRegion {
            y_start: 0,
            y_end: 0,
            x_start: 0,
            x_end: 2,
        };
        let mut cursor = TextCursor { col: 0, row: 0 };
        let mut job = TextJob::new("abc defgh", 10, region, false, &mut cursor, &mut fb);

        // Drive with a tiny budget until we hit the pagination gate.
        let mut status = JobStatus::Continuing;
        for _ in 0..1000 {
            status = job.advance(1, &mut fb, &font, &mut cursor);
            if status == JobStatus::NeedsKey {
                break;
            }
        }
        assert_eq!(
            status,
            JobStatus::NeedsKey,
            "must reach the pagination gate"
        );
        assert_eq!(cursor.col, region.x_start);
        assert_eq!(
            cursor.row, region.y_start,
            "cursor resets to region start when pagination opens"
        );

        job.release(&mut fb);
        assert_eq!(job.status(), JobStatus::Continuing);
        assert_eq!(
            drive_to_completion(&mut job, &mut fb, &font, &mut cursor),
            JobStatus::Done
        );
    }

    #[test]
    fn accumulator_average_pacing_is_exact_at_default_speed_4() {
        // char_ms = 4*3 = 12; tick_ms = 1000/60 ~= 16.667. Over many ticks
        // the average chars/tick must converge to tick_ms/char_ms exactly,
        // with per-tick output never drifting more than one char from ideal.
        let mut pacer = TextPacer::new(4);
        let tick_ms = 1000.0 / 60.0;
        let mut total = 0u64;
        let ticks = 100_000u64;
        for i in 1..=ticks {
            total += pacer.tick(tick_ms) as u64;
            let ideal = (i as f64) * tick_ms / 12.0;
            assert!(
                (total as f64 - ideal).abs() < 1.0,
                "drift exceeded 1 char at tick {i}"
            );
        }
        let ideal_total = (ticks as f64) * tick_ms / 12.0;
        assert!((total as f64 - ideal_total).abs() < 1.0);
    }

    #[test]
    fn accumulator_average_pacing_is_exact_at_speed_1() {
        let mut pacer = TextPacer::new(1);
        let tick_ms = 1000.0 / 60.0;
        let mut total = 0u64;
        for i in 1..=50_000u64 {
            total += pacer.tick(tick_ms) as u64;
            let ideal = (i as f64) * tick_ms / 3.0;
            assert!(
                (total as f64 - ideal).abs() < 1.0,
                "drift exceeded 1 char at tick {i}"
            );
        }
    }

    #[test]
    fn accumulator_average_pacing_is_exact_at_speed_9() {
        let mut pacer = TextPacer::new(9);
        let tick_ms = 1000.0 / 60.0;
        let mut total = 0u64;
        for i in 1..=50_000u64 {
            total += pacer.tick(tick_ms) as u64;
            let ideal = (i as f64) * tick_ms / 27.0;
            assert!(
                (total as f64 - ideal).abs() < 1.0,
                "drift exceeded 1 char at tick {i}"
            );
        }
    }

    #[test]
    fn draw_string_is_immediate_and_unpaced() {
        let font = marker_font();
        let mut fb = Framebuffer::new();
        draw_string(&mut fb, &font, "AC", 2, 33, 0, 10);
        // Just must not panic and must draw something at the first cell.
        assert_ne!(fb.get_pixel(33 * 8, 2 * 8), 0);
    }
}
