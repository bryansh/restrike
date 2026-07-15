//! The five prompt-line/presentation widgets (D-UI2 `Widget`, design doc
//! §1.5): each a blocking loop in the original, a parked state advanced one
//! tick's input at a time here. `Widget::tick` never blocks: it consumes at
//! most one [`InputQueue::read_key`] per call and returns
//! [`WidgetOutcome::Pending`] until resolved.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `engine/ovr027.cs` `displayInput` (`:132-341`) — [`Hotbar`]'s full
//!   input loop, incl. `accept_ctrlkeys`'s extended-key/digit mapping
//!   through `keypad_ctrl_codes` and the timeout path.
//! - coab `engine/ovr027.cs` `BuildInputKeys` (`:59-86`) — [`build_words`]'s
//!   maximal-run-of-`[0-9A-Z]` word scan.
//! - coab `engine/ovr027.cs` `sl_select_item` (`:532-673`) — [`ListMenu`]'s
//!   highlight/paging semantics, incl. Up/Down being ignored (FD-18 RESOLVED
//!   — confirmed correct against the running game; §1.11 item 10).
//! - coab `engine/ovr008.cs` `sub_317AA`'s callers (HORIZONTAL MENU,
//!   ENCOUNTER MENU, PARLAY, `:1176-1190`) — the `ext_scrolls_party`/
//!   `valid_keys` re-prompt behavior: Esc does not exit these menus, and any
//!   key outside `valid_keys` re-prompts instead of returning.
//! - coab `engine/seg041.cs` `getUserInputString`/`getUserInputShort`
//!   (`:234-294`) — [`TextEntry`]'s echo/backspace/uppercase/numeric-reprompt
//!   semantics.
//! - coab `engine/seg041.cs` `DisplayAndPause` (`:297-303`) — [`PressAnyKey`].
//! - coab `engine/seg041.cs` `GameDelay`/`SysDelay` — [`Delay`].
//!
//! **Implementation note (flagged per D11, not silently absorbed):** the
//! design doc's prose leaves a few byte-level choices unstated pending a
//! DOSBox confirmation pass; each is called out at its point of use below
//! (comma/period cycle direction, sub_317AA `valid_keys` precedence over
//! hotkey matching, numeric re-prompt clearing the buffer). None affect the
//! state machine's structure, only these specific resolutions.

use crate::input::{ExtKey, InputEvent, InputQueue};

/// A hotkey-selectable word: a byte range (`start..end`, exclusive) into the
/// owning [`Hotbar`]'s text.
pub type WordRange = (usize, usize);

/// `BuildInputKeys` (`ovr027.cs:59-86`): maximal runs of `[0-9A-Z]` are the
/// hotkey-selectable words; everything else (spaces, lowercase, punctuation)
/// is separator.
pub fn build_words(text: &str) -> Vec<WordRange> {
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() || bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_uppercase() || bytes[i].is_ascii_digit()) {
                i += 1;
            }
            words.push((start, i));
        } else {
            i += 1;
        }
    }
    words
}

/// `keypad_ctrl_codes`'s digit-key half (`ovr027.cs:124`): plain digit keys
/// `1..=9`, when `accept_ext` is set, map through the same table as their
/// numpad-key equivalent (design doc §1.5: "extended keys ... *and digits*
/// map through keypad_ctrl_codes").
fn keypad_digit_ctrl_code(c: u8) -> u8 {
    debug_assert!((b'1'..=b'9').contains(&c));
    let kp = match c {
        b'1' => ExtKey::Kp1,
        b'2' => ExtKey::Kp2,
        b'3' => ExtKey::Kp3,
        b'4' => ExtKey::Kp4,
        b'5' => ExtKey::Kp5,
        b'6' => ExtKey::Kp6,
        b'7' => ExtKey::Kp7,
        b'8' => ExtKey::Kp8,
        _ => ExtKey::Kp9,
    };
    kp.ctrl_code()
}

/// A menu bar of hotkey-selectable words (`displayInput`, §1.5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Hotbar {
    pub text: String,
    words: Vec<WordRange>,
    pub selected: Option<usize>,
    /// `accept_ctrlkeys`: extended keys/keypad digits resolve through
    /// `keypad_ctrl_codes` instead of being ignored.
    pub accept_ext: bool,
    /// `displayInputSecondsToWait`/`displayInputTimeoutValue`: resolves with
    /// this value once `ticks_left` ticks have elapsed with no input.
    pub timeout: Option<(u32, u8)>,
    /// `sub_317AA` menus: extended keys scroll the party panel while parked
    /// instead of resolving the widget.
    pub ext_scrolls_party: bool,
    /// `sub_317AA` menus: any key outside this set re-prompts instead of
    /// resolving (and Esc never exits — see [`Hotbar::tick`]).
    pub valid_keys: Option<Vec<u8>>,
}

/// One tick's result from any [`Widget`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WidgetOutcome {
    /// Still waiting on input; call `tick` again next tick.
    Pending,
    /// A [`Hotbar`] resolved with this (uppercased) key.
    Hotbar(u8),
    /// A `ext_scrolls_party` [`Hotbar`]'s extended key scrolled the party
    /// panel instead of resolving — the ctrl-code byte identifies direction
    /// (`'H'`=up/`'P'`=down, per §1.5's forward/turn-around aliasing).
    PartyScroll(u8),
    /// A [`ListMenu`] resolved: the *item index* (into the original,
    /// heading-inclusive list) currently highlighted, and the resolving key.
    ListSelected {
        index: usize,
        key: u8,
    },
    ListCancelled,
    TextSubmitted(String),
    TextCancelled,
    /// [`PressAnyKey`] or [`Delay`] resolved — no payload.
    Done,
}

impl Hotbar {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let words = build_words(&text);
        let selected = if words.is_empty() { None } else { Some(0) };
        Hotbar {
            text,
            words,
            selected,
            accept_ext: false,
            timeout: None,
            ext_scrolls_party: false,
            valid_keys: None,
        }
    }

    pub fn words(&self) -> &[WordRange] {
        &self.words
    }

    /// The highlighted word's first character, uppercased — Enter's normal
    /// resolution, and `'\r'` when nothing is highlightable (§1.5).
    pub fn highlighted_char(&self) -> Option<u8> {
        self.selected.map(|i| self.text.as_bytes()[self.words[i].0])
    }

    fn cycle(&mut self, dir: i32) {
        if self.words.is_empty() {
            return;
        }
        let n = self.words.len() as i32;
        let cur = self.selected.map(|i| i as i32).unwrap_or(0);
        let next = ((cur + dir).rem_euclid(n)) as usize;
        self.selected = Some(next);
    }

    fn select_word_starting_with(&mut self, upper: u8) -> bool {
        if let Some(i) = self
            .words
            .iter()
            .position(|&(s, _)| self.text.as_bytes()[s] == upper)
        {
            self.selected = Some(i);
            true
        } else {
            false
        }
    }

    /// Advances the widget by one tick, consuming at most one queued key.
    /// `dt_ticks` drives the optional timeout countdown.
    pub fn tick(&mut self, queue: &mut InputQueue, dt_ticks: u32) -> WidgetOutcome {
        if let Some((ticks_left, value)) = &mut self.timeout {
            if dt_ticks >= *ticks_left {
                return WidgetOutcome::Hotbar(*value);
            }
            *ticks_left -= dt_ticks;
        }

        let Some(key) = queue.read_key() else {
            return WidgetOutcome::Pending;
        };

        if let InputEvent::Ext(ext) = key {
            return if self.accept_ext {
                self.resolve_ctrl_code(ext.ctrl_code())
            } else {
                WidgetOutcome::Pending
            };
        }

        match key {
            InputEvent::Escape => {
                // sub_317AA menus (`valid_keys` set) never exit on Esc
                // (`ovr008.cs:1176-1190`); ordinary Hotbars return '\0'.
                if self.valid_keys.is_some() {
                    WidgetOutcome::Pending
                } else {
                    WidgetOutcome::Hotbar(0)
                }
            }
            InputEvent::Enter => {
                let ch = self.highlighted_char().unwrap_or(b'\r');
                self.resolve_char(ch)
            }
            InputEvent::Char(c) if self.accept_ext && (b'1'..=b'9').contains(&c) => {
                self.resolve_ctrl_code(keypad_digit_ctrl_code(c))
            }
            // Implementation note (flagged): cycle direction (',' = prev,
            // '.' = next) matches left-right keyboard order; unconfirmed
            // against coab's literal branch order (docket).
            InputEvent::Char(b',') => {
                self.cycle(-1);
                WidgetOutcome::Pending
            }
            InputEvent::Char(b'.') => {
                self.cycle(1);
                WidgetOutcome::Pending
            }
            InputEvent::Char(c) => {
                let up = c.to_ascii_uppercase();
                // Implementation note (flagged): a sub_317AA `valid_keys`
                // menu checks membership before hotkey matching — any
                // allowed key resolves even if it isn't a highlightable
                // word's first letter (e.g. a plain 'E' exit key).
                if let Some(valid) = &self.valid_keys {
                    return if valid.contains(&up) {
                        WidgetOutcome::Hotbar(up)
                    } else {
                        WidgetOutcome::Pending
                    };
                }
                if self.select_word_starting_with(up) {
                    WidgetOutcome::Hotbar(up)
                } else if up == b' ' {
                    WidgetOutcome::Hotbar(b' ')
                } else {
                    WidgetOutcome::Pending
                }
            }
            InputEvent::Backspace => WidgetOutcome::Pending,
            InputEvent::Ext(_) => unreachable!("handled above"),
        }
    }

    fn resolve_ctrl_code(&self, code: u8) -> WidgetOutcome {
        if self.ext_scrolls_party {
            return WidgetOutcome::PartyScroll(code);
        }
        self.resolve_char(code)
    }

    fn resolve_char(&self, ch: u8) -> WidgetOutcome {
        match &self.valid_keys {
            Some(valid) if !valid.contains(&ch) => WidgetOutcome::Pending,
            _ => WidgetOutcome::Hotbar(ch),
        }
    }
}

/// One [`ListMenu`] row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ListItem {
    Heading(String),
    Entry(String),
}

impl ListItem {
    fn is_heading(&self) -> bool {
        matches!(self, ListItem::Heading(_))
    }
}

/// `sl_select_item` (`ovr027.cs:532-673`): a vertical list combined with a
/// Hotbar whose text grows `" Next"`/`" Prev"`/`" Exit"` (the growth itself
/// is presentation, not modeled here — this struct owns selection/scroll
/// state only). Movement is transcribed directly from coab's own routines
/// (`menu_scroll_in_page` `:497`, `menu_scroll_page` `:464`, `skipHeadings`
/// `:sub_6CC08`), so it inherits their exact heading-inclusive,
/// page-relative-wrapping behavior — see [`ListMenu::tick`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ListMenu {
    pub items: Vec<ListItem>,
    /// Heading-inclusive cursor into `items` (coab `index_ptr`) — the
    /// currently highlighted row.
    index: usize,
    /// Which `items` row sits at the top of the visible window (coab
    /// `gbl.menuScreenIndex`) — the scroll position, in list coordinates.
    screen_index: usize,
    /// Visible row count (coab `listDisplayHeight` = `endY − startY + 1`).
    /// The list's *screen* origin (§1.5's `textYCol + 1` coupling) is a
    /// presentation concern owned by the consuming screen, not tracked here.
    pub page_size: usize,
}

impl ListMenu {
    /// `page_size` is the visible row count. The cursor initializes on the
    /// first selectable entry (coab's `index_ptr++; scroll_in_page(false)`
    /// normalization off any leading heading); the view starts at row 0.
    pub fn new(items: Vec<ListItem>, page_size: usize) -> Self {
        let mut m = ListMenu {
            items,
            index: 0,
            screen_index: 0,
            page_size: page_size.max(1),
        };
        if m.items.iter().any(|it| !it.is_heading()) {
            m.index = m.scroll_in_page(false, 1);
        }
        m
    }

    /// The `items` index currently highlighted (heading-inclusive
    /// coordinates), or `None` if the cursor isn't on a selectable entry
    /// (an all-heading/empty list).
    pub fn selected_index(&self) -> Option<usize> {
        match self.items.get(self.index) {
            Some(ListItem::Entry(_)) => Some(self.index),
            _ => None,
        }
    }

    /// `menu_scroll_in_page` (`ovr027.cs:497`, `sub_6CDCA`): single-step
    /// highlight movement that wraps *within the current visible page* and
    /// skips headings — never scrolls the window. `backwards_step` follows
    /// coab's own polarity: `false` moves toward row 0 (up, the `'G'`/Home
    /// case), `true` moves toward the end (down, the `'O'`/End case).
    fn scroll_in_page(&self, backwards_step: bool, start: i64) -> usize {
        let count = self.items.len() as i64;
        let page = self.page_size as i64;
        let screen = self.screen_index as i64;
        let mut index = start;
        if backwards_step {
            index += 1;
            if screen + page - 1 < index {
                index = screen;
            }
            if count - 1 < index {
                index = screen;
            }
        } else {
            index -= 1;
            if index < screen {
                index = screen + page - 1;
            }
            if count - 1 < index {
                index = count - 1;
            }
        }
        self.skip_headings(backwards_step, index)
    }

    /// `skipHeadings` (`sub_6CC08`): steps past heading rows in `step`'s
    /// direction, with the same page-relative wrap `scroll_in_page` uses, and
    /// gives up after a full page of headings (matching coab's `var_2 <
    /// listDisplayHeight` bound). Returns a clamped, in-bounds index.
    fn skip_headings(&self, backwards_step: bool, start: i64) -> usize {
        let count = self.items.len() as i64;
        let page = self.page_size as i64;
        let screen = self.screen_index as i64;
        let mut index = start;
        let mut guard = 0;
        while guard < page && (0..count).contains(&index) && self.items[index as usize].is_heading()
        {
            guard += 1;
            if backwards_step {
                index += 1;
                if screen + page - 1 < index {
                    index = screen;
                }
                if count - 1 < index {
                    index = screen;
                }
            } else {
                index -= 1;
                if index < screen {
                    index = screen + page - 1;
                }
                if count - 1 < index {
                    index = count - 1;
                }
            }
        }
        index.clamp(0, (count - 1).max(0)) as usize
    }

    /// `menu_scroll_page` (`ovr027.cs:464`, `sub_6CD38`): scrolls the visible
    /// window by a full page, preserving the cursor's offset within the page,
    /// then skips headings. `backwards_step` matches coab: `false` pages up
    /// (`'I'`/PgUp, `'P'`), `true` pages down (`'Q'`/PgDn, `'N'`).
    fn scroll_page(&mut self, backwards_step: bool) {
        let count = self.items.len() as i64;
        let page = self.page_size as i64;
        let screen_offset = self.index as i64 - self.screen_index as i64;
        let mut screen = self.screen_index as i64;
        if backwards_step {
            screen += page;
            if count - page < screen {
                screen = count - page;
            }
        } else {
            screen -= page;
        }
        // Defensive clamp (coab can momentarily go negative here for a list
        // that fits in one page — an unreachable-in-practice 'P'/'N' edge):
        // keep the window origin in-bounds.
        if screen < 0 {
            screen = 0;
        }
        self.screen_index = screen as usize;
        let index = (screen + screen_offset).clamp(0, (count - 1).max(0));
        self.index = self.skip_headings(backwards_step, index);
    }

    /// Advances by one tick, consuming at most one queued key.
    ///
    /// Per coab's special-key switch (`ovr027.cs:617-653`) and **FD-18
    /// (RESOLVED)**: Home/End (`'G'`/`'O'`, and numpad 7/1) move the highlight
    /// one step within the page (`menu_scroll_in_page`), PgUp/PgDn
    /// (`'I'`/`'Q'`) and the plain letters `'P'`/`'N'` page, and **Up/Down
    /// arrows are ignored** — confirmed correct against the running game
    /// (arrows do nothing; numpad 1/7 drive the highlight), not a bug or a
    /// pending question.
    pub fn tick(&mut self, queue: &mut InputQueue) -> WidgetOutcome {
        let Some(key) = queue.read_key() else {
            return WidgetOutcome::Pending;
        };

        if let InputEvent::Ext(ext) = key {
            match ext.ctrl_code() {
                b'G' => self.index = self.scroll_in_page(false, self.index as i64),
                b'O' => self.index = self.scroll_in_page(true, self.index as i64),
                b'I' => self.scroll_page(false),
                b'Q' => self.scroll_page(true),
                // Up ('H')/Down ('P') and any other extended key: ignored.
                _ => {}
            }
            return WidgetOutcome::Pending;
        }

        match key {
            InputEvent::Escape => WidgetOutcome::ListCancelled,
            InputEvent::Char(c) => match c.to_ascii_uppercase() {
                b'E' => WidgetOutcome::ListCancelled,
                b'P' => {
                    self.scroll_page(false);
                    WidgetOutcome::Pending
                }
                b'N' => {
                    self.scroll_page(true);
                    WidgetOutcome::Pending
                }
                up => match self.selected_index() {
                    Some(index) => WidgetOutcome::ListSelected { index, key: up },
                    None => WidgetOutcome::Pending,
                },
            },
            InputEvent::Enter => match self.selected_index() {
                Some(index) => WidgetOutcome::ListSelected { index, key: b'\r' },
                None => WidgetOutcome::Pending,
            },
            InputEvent::Backspace | InputEvent::Ext(_) => WidgetOutcome::Pending,
        }
    }
}

/// `getUserInputString`/`getUserInputShort` (`seg041.cs:234-294`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextEntry {
    pub prompt: String,
    pub buf: Vec<u8>,
    pub max: usize,
    /// `getUserInputShort`: re-prompts (rather than submitting) until the
    /// buffer parses as `0..=65535`.
    pub numeric: bool,
}

impl TextEntry {
    pub fn new(prompt: impl Into<String>, max: usize, numeric: bool) -> Self {
        TextEntry {
            prompt: prompt.into(),
            buf: Vec::new(),
            max,
            numeric,
        }
    }

    /// Advances by one tick, consuming at most one queued key.
    pub fn tick(&mut self, queue: &mut InputQueue) -> WidgetOutcome {
        let Some(key) = queue.read_key() else {
            return WidgetOutcome::Pending;
        };

        match key {
            InputEvent::Escape => WidgetOutcome::TextCancelled,
            InputEvent::Backspace => {
                self.buf.pop();
                WidgetOutcome::Pending
            }
            InputEvent::Enter => {
                if self.numeric {
                    let text = String::from_utf8_lossy(&self.buf);
                    if text.parse::<u16>().is_ok() {
                        WidgetOutcome::TextSubmitted(text.to_ascii_uppercase())
                    } else {
                        // Implementation note (flagged): "re-runs the string
                        // editor" (§1.5) is transcribed as a full reset of
                        // the buffer rather than an in-place correction —
                        // unconfirmed against coab's exact redraw sequence.
                        self.buf.clear();
                        WidgetOutcome::Pending
                    }
                } else {
                    let text = String::from_utf8_lossy(&self.buf).to_ascii_uppercase();
                    WidgetOutcome::TextSubmitted(text)
                }
            }
            InputEvent::Char(c) if (0x20..=0x7A).contains(&c) => {
                if self.buf.len() < self.max {
                    self.buf.push(c);
                }
                WidgetOutcome::Pending
            }
            InputEvent::Char(_) | InputEvent::Ext(_) => WidgetOutcome::Pending,
        }
    }
}

/// `DisplayAndPause` (`seg041.cs:297-303`): prompt text (owned by the
/// caller/presentation layer) plus one key, any key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct PressAnyKey;

impl PressAnyKey {
    pub fn tick(&mut self, queue: &mut InputQueue) -> WidgetOutcome {
        match queue.read_key() {
            Some(_) => WidgetOutcome::Done,
            None => WidgetOutcome::Pending,
        }
    }
}

/// `GameDelay`/`DELAY`/animation pauses: a fixed tick count, no input
/// consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Delay {
    pub ticks_left: u32,
}

impl Delay {
    pub fn new(ticks: u32) -> Self {
        Delay { ticks_left: ticks }
    }

    pub fn tick(&mut self, dt_ticks: u32) -> WidgetOutcome {
        self.ticks_left = self.ticks_left.saturating_sub(dt_ticks);
        if self.ticks_left == 0 {
            WidgetOutcome::Done
        } else {
            WidgetOutcome::Pending
        }
    }
}

/// The interaction layer (D-UI2): one Widget parked, whatever flow/phase it
/// belongs to (see `shell.rs`'s doc comment on the Fable review finding that
/// generalized this beyond `VmPhase::Gate`/`WorldMenu`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Widget {
    Hotbar(Hotbar),
    ListMenu(ListMenu),
    TextEntry(TextEntry),
    PressAnyKey(PressAnyKey),
    Delay(Delay),
}

impl Widget {
    /// Advances the parked widget by one tick.
    pub fn tick(&mut self, queue: &mut InputQueue, dt_ticks: u32) -> WidgetOutcome {
        match self {
            Widget::Hotbar(h) => h.tick(queue, dt_ticks),
            Widget::ListMenu(l) => l.tick(queue),
            Widget::TextEntry(t) => t.tick(queue),
            Widget::PressAnyKey(p) => p.tick(queue),
            Widget::Delay(d) => d.tick(dt_ticks),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue_of(events: &[InputEvent]) -> InputQueue {
        let mut q = InputQueue::new();
        q.push_all(events);
        q
    }

    #[test]
    fn build_words_finds_maximal_uppercase_digit_runs() {
        let words = build_words("Area Cast View Encamp Search Look");
        // "Area" -> 'A' run of length 1 (only leading 'A' is uppercase).
        let text = "Area Cast View Encamp Search Look";
        let slices: Vec<&str> = words.iter().map(|&(s, e)| &text[s..e]).collect();
        assert_eq!(slices, vec!["A", "C", "V", "E", "S", "L"]);
    }

    #[test]
    fn build_words_handles_full_uppercase_words_and_digits() {
        let words = build_words("1 BASH PICK KNOCK EXIT");
        let text = "1 BASH PICK KNOCK EXIT";
        let slices: Vec<&str> = words.iter().map(|&(s, e)| &text[s..e]).collect();
        assert_eq!(slices, vec!["1", "BASH", "PICK", "KNOCK", "EXIT"]);
    }

    #[test]
    fn hotbar_letter_matching_selects_and_returns_uppercased() {
        let mut h = Hotbar::new("Yes No");
        let mut q = queue_of(&[InputEvent::Char(b'n')]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(b'N'));
        assert_eq!(h.highlighted_char(), Some(b'N'));
    }

    #[test]
    fn hotbar_comma_dot_cycle_the_highlighted_word() {
        let mut h = Hotbar::new("Yes No Maybe");
        assert_eq!(h.highlighted_char(), Some(b'Y'));
        let mut q = queue_of(&[InputEvent::Char(b'.')]);
        h.tick(&mut q, 1);
        assert_eq!(h.highlighted_char(), Some(b'N'));
        let mut q = queue_of(&[InputEvent::Char(b'.')]);
        h.tick(&mut q, 1);
        assert_eq!(h.highlighted_char(), Some(b'M'));
        let mut q = queue_of(&[InputEvent::Char(b'.')]);
        h.tick(&mut q, 1); // wraps back to first
        assert_eq!(h.highlighted_char(), Some(b'Y'));
        let mut q = queue_of(&[InputEvent::Char(b',')]);
        h.tick(&mut q, 1); // wraps backward
        assert_eq!(h.highlighted_char(), Some(b'M'));
    }

    #[test]
    fn hotbar_enter_returns_highlighted_words_first_char() {
        let mut h = Hotbar::new("Yes No");
        let mut q = queue_of(&[InputEvent::Enter]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(b'Y'));
    }

    #[test]
    fn hotbar_enter_with_no_highlightable_word_returns_cr() {
        let mut h = Hotbar::new("hello world"); // no uppercase runs
        let mut q = queue_of(&[InputEvent::Enter]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(b'\r'));
    }

    #[test]
    fn hotbar_esc_returns_null() {
        let mut h = Hotbar::new("Yes No");
        let mut q = queue_of(&[InputEvent::Escape]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(0));
    }

    #[test]
    fn hotbar_space_passes_through() {
        let mut h = Hotbar::new("Yes No");
        let mut q = queue_of(&[InputEvent::Char(b' ')]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(b' '));
    }

    #[test]
    fn hotbar_timeout_resolves_with_the_timeout_value() {
        let mut h = Hotbar::new("Yes No");
        h.timeout = Some((5, 0xFF));
        let mut q = InputQueue::new();
        assert_eq!(h.tick(&mut q, 3), WidgetOutcome::Pending);
        assert_eq!(h.tick(&mut q, 3), WidgetOutcome::Hotbar(0xFF));
    }

    #[test]
    fn hotbar_accept_ext_maps_extended_keys_through_keypad_ctrl_codes() {
        let mut h = Hotbar::new("Area Cast View Encamp Search Look");
        h.accept_ext = true;
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::Up)]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(b'H'));
    }

    #[test]
    fn hotbar_accept_ext_kp5_maps_to_space() {
        let mut h = Hotbar::new("Yes No");
        h.accept_ext = true;
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::Kp5)]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(b' '));
    }

    #[test]
    fn hotbar_ignores_extended_keys_without_accept_ext() {
        let mut h = Hotbar::new("Yes No");
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::Up)]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Pending);
    }

    #[test]
    fn hotbar_accept_ext_digits_map_through_keypad_table() {
        let mut h = Hotbar::new("Yes No");
        h.accept_ext = true;
        let mut q = queue_of(&[InputEvent::Char(b'8')]); // Kp8 -> 'H'
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(b'H'));
    }

    #[test]
    fn hotbar_ext_scrolls_party_diverts_extended_keys() {
        let mut h = Hotbar::new("Encounter");
        h.accept_ext = true;
        h.ext_scrolls_party = true;
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::Up)]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::PartyScroll(b'H'));
    }

    #[test]
    fn hotbar_valid_keys_menu_never_exits_on_escape() {
        let mut h = Hotbar::new("Yes No");
        h.valid_keys = Some(vec![b'Y', b'N']);
        let mut q = queue_of(&[InputEvent::Escape]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Pending);
    }

    #[test]
    fn hotbar_valid_keys_menu_reprompts_on_disallowed_key() {
        let mut h = Hotbar::new("Yes No");
        h.valid_keys = Some(vec![b'Y', b'N']);
        let mut q = queue_of(&[InputEvent::Char(b'X')]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Pending);
    }

    #[test]
    fn hotbar_valid_keys_menu_resolves_an_allowed_non_hotkey_char() {
        let mut h = Hotbar::new("Party Status");
        h.valid_keys = Some(vec![b'E']); // 'E' not a word-first-letter here
        let mut q = queue_of(&[InputEvent::Char(b'e')]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(b'E'));
    }

    fn sample_list() -> ListMenu {
        ListMenu::new(
            vec![
                ListItem::Heading("Spells".into()),
                ListItem::Entry("Magic Missile".into()),
                ListItem::Entry("Sleep".into()),
                ListItem::Entry("Fireball".into()),
                ListItem::Entry("Lightning Bolt".into()),
            ],
            2,
        )
    }

    /// `n` selectable entries, no headings, all visible in one page — the
    /// clean fixture for single-step/wrap movement (a heading inside a small
    /// page collapses movement, which the heading-specific tests cover).
    fn flat_list(n: usize, page: usize) -> ListMenu {
        ListMenu::new(
            (0..n)
                .map(|i| ListItem::Entry(format!("E{i}")))
                .collect::<Vec<_>>(),
            page,
        )
    }

    #[test]
    fn list_menu_headings_are_excluded_from_selection() {
        let list = sample_list();
        assert_eq!(list.selected_index(), Some(1)); // "Magic Missile"
    }

    #[test]
    fn list_menu_up_down_are_ignored() {
        let mut list = sample_list();
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::Down)]);
        assert_eq!(list.tick(&mut q), WidgetOutcome::Pending);
        assert_eq!(list.selected_index(), Some(1), "Down must not move");
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::Up)]);
        assert_eq!(list.tick(&mut q), WidgetOutcome::Pending);
        assert_eq!(list.selected_index(), Some(1), "Up must not move");
    }

    #[test]
    fn list_menu_home_end_step_one_within_the_page() {
        // FD-18: End (numpad 1) moves down one, Home (numpad 7) up one —
        // `menu_scroll_in_page`, not a jump to the ends.
        let mut list = flat_list(5, 5); // cursor starts on entry 0
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::End)]);
        list.tick(&mut q);
        assert_eq!(list.selected_index(), Some(1));
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::End)]);
        list.tick(&mut q);
        assert_eq!(list.selected_index(), Some(2));
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::Home)]);
        list.tick(&mut q);
        assert_eq!(list.selected_index(), Some(1));
    }

    #[test]
    fn list_menu_end_wraps_at_the_bottom_of_the_page() {
        let mut list = flat_list(5, 5); // one page holds every row
        for _ in 0..4 {
            let mut q = queue_of(&[InputEvent::Ext(ExtKey::End)]);
            list.tick(&mut q);
        }
        assert_eq!(list.selected_index(), Some(4), "stepped to the last row");
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::End)]);
        list.tick(&mut q);
        assert_eq!(
            list.selected_index(),
            Some(0),
            "one more End wraps within the page to the top, not off the end"
        );
    }

    #[test]
    fn list_menu_home_wraps_at_the_top_of_the_page() {
        let mut list = flat_list(5, 5); // cursor at the top row
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::Home)]);
        list.tick(&mut q);
        assert_eq!(
            list.selected_index(),
            Some(4),
            "Home from the top wraps down"
        );
    }

    #[test]
    fn list_menu_step_skips_headings() {
        let mut list = ListMenu::new(
            vec![
                ListItem::Entry("A".into()),
                ListItem::Heading("---".into()),
                ListItem::Entry("B".into()),
            ],
            3,
        );
        assert_eq!(list.selected_index(), Some(0)); // "A"
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::End)]);
        list.tick(&mut q);
        assert_eq!(
            list.selected_index(),
            Some(2),
            "End steps over the heading to B"
        );
    }

    #[test]
    fn list_menu_page_keys_scroll_by_a_page() {
        // 6 entries, 2 visible per page: PgDn/'N' page forward, PgUp/'P' back.
        let mut list = flat_list(6, 2);
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::PgDn)]);
        list.tick(&mut q);
        assert_eq!(list.selected_index(), Some(2));
        let mut q = queue_of(&[InputEvent::Char(b'n')]);
        list.tick(&mut q);
        assert_eq!(list.selected_index(), Some(4));
        let mut q = queue_of(&[InputEvent::Char(b'p')]);
        list.tick(&mut q);
        assert_eq!(list.selected_index(), Some(2));
        let mut q = queue_of(&[InputEvent::Ext(ExtKey::PgUp)]);
        list.tick(&mut q);
        assert_eq!(list.selected_index(), Some(0));
    }

    #[test]
    fn list_menu_escape_and_e_cancel() {
        let mut list = sample_list();
        let mut q = queue_of(&[InputEvent::Escape]);
        assert_eq!(list.tick(&mut q), WidgetOutcome::ListCancelled);
        let mut list = sample_list();
        let mut q = queue_of(&[InputEvent::Char(b'e')]);
        assert_eq!(list.tick(&mut q), WidgetOutcome::ListCancelled);
    }

    #[test]
    fn list_menu_enter_selects_the_highlighted_item() {
        let mut list = sample_list();
        let mut q = queue_of(&[InputEvent::Enter]);
        assert_eq!(
            list.tick(&mut q),
            WidgetOutcome::ListSelected {
                index: 1,
                key: b'\r'
            }
        );
    }

    #[test]
    fn list_menu_all_heading_list_has_no_selection() {
        let list = ListMenu::new(vec![ListItem::Heading("only".into())], 3);
        assert_eq!(list.selected_index(), None);
    }

    #[test]
    fn text_entry_echoes_printable_chars_and_uppercases_on_submit() {
        let mut t = TextEntry::new("Name?", 10, false);
        for c in b"hi" {
            let mut q = queue_of(&[InputEvent::Char(*c)]);
            assert_eq!(t.tick(&mut q), WidgetOutcome::Pending);
        }
        let mut q = queue_of(&[InputEvent::Enter]);
        assert_eq!(t.tick(&mut q), WidgetOutcome::TextSubmitted("HI".into()));
    }

    #[test]
    fn text_entry_reads_one_key_per_tick_fifo() {
        let mut t = TextEntry::new("Name?", 10, false);
        let mut q = queue_of(&[
            InputEvent::Char(b'A'),
            InputEvent::Backspace,
            InputEvent::Char(b'B'),
        ]);
        // FD-17 FIFO: one key consumed per tick, oldest first — every queued
        // key is honored across ticks, not drained to the newest.
        t.tick(&mut q);
        assert_eq!(t.buf, vec![b'A']);
        t.tick(&mut q);
        assert!(t.buf.is_empty(), "backspace read next");
        t.tick(&mut q);
        assert_eq!(t.buf, vec![b'B']);
    }

    #[test]
    fn text_entry_backspace_edits_across_ticks() {
        let mut t = TextEntry::new("Name?", 10, false);
        let mut q = queue_of(&[InputEvent::Char(b'A')]);
        t.tick(&mut q);
        let mut q = queue_of(&[InputEvent::Backspace]);
        t.tick(&mut q);
        assert!(t.buf.is_empty());
    }

    #[test]
    fn text_entry_esc_cancels() {
        let mut t = TextEntry::new("Name?", 10, false);
        let mut q = queue_of(&[InputEvent::Escape]);
        assert_eq!(t.tick(&mut q), WidgetOutcome::TextCancelled);
    }

    #[test]
    fn text_entry_respects_max_length() {
        let mut t = TextEntry::new("Name?", 1, false);
        let mut q = queue_of(&[InputEvent::Char(b'A')]);
        t.tick(&mut q);
        let mut q = queue_of(&[InputEvent::Char(b'B')]);
        t.tick(&mut q);
        assert_eq!(t.buf, vec![b'A']);
    }

    #[test]
    fn text_entry_numeric_reprompts_on_invalid_input() {
        let mut t = TextEntry::new("Amount?", 10, true);
        for c in b"xy" {
            let mut q = queue_of(&[InputEvent::Char(*c)]);
            t.tick(&mut q);
        }
        let mut q = queue_of(&[InputEvent::Enter]);
        assert_eq!(t.tick(&mut q), WidgetOutcome::Pending);
        assert!(t.buf.is_empty(), "invalid numeric input resets the buffer");
    }

    #[test]
    fn text_entry_numeric_submits_on_valid_input() {
        let mut t = TextEntry::new("Amount?", 10, true);
        for c in b"42" {
            let mut q = queue_of(&[InputEvent::Char(*c)]);
            t.tick(&mut q);
        }
        let mut q = queue_of(&[InputEvent::Enter]);
        assert_eq!(t.tick(&mut q), WidgetOutcome::TextSubmitted("42".into()));
    }

    #[test]
    fn press_any_key_resolves_on_any_key() {
        let mut p = PressAnyKey;
        let mut q = InputQueue::new();
        assert_eq!(p.tick(&mut q), WidgetOutcome::Pending);
        let mut q = queue_of(&[InputEvent::Char(b'z')]);
        assert_eq!(p.tick(&mut q), WidgetOutcome::Done);
    }

    #[test]
    fn delay_resolves_after_its_tick_count() {
        let mut d = Delay::new(24);
        assert_eq!(d.tick(23), WidgetOutcome::Pending);
        assert_eq!(d.tick(1), WidgetOutcome::Done);
    }

    #[test]
    fn delay_of_zero_ticks_resolves_immediately() {
        let mut d = Delay::new(0);
        assert_eq!(d.tick(0), WidgetOutcome::Done);
    }
}
