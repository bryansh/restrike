//! The tick core's input model (D-UI1): `InputEvent`/`ExtKey`, and the
//! engine-owned queue implementing the original's non-plain-pop read
//! semantics.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `engine/seg043.cs` `GetInputKey` (`:55-62`) drains the whole
//!   buffer after reading any nonzero key, keeping only the newest — but
//!   **FD-17 (RESOLVED, docket) confirmed that is a coab transliteration
//!   artifact, not original behavior**: Bryan's live DOSBox mash test showed
//!   the original *buffers type-ahead FIFO* (multiple queued forward steps
//!   commit, not one). So [`InputQueue::read_key`] is a plain FIFO pop, per
//!   the design doc's "the queue read is one function." Our model reads whole
//!   logical [`InputEvent`]s (never raw two-byte extended-key pairs), so the
//!   original's "the `0x00` prefix byte itself doesn't drain" detail has no
//!   analogue here — every queued event is a real keypress.
//! - coab `engine/seg043.cs` `clear_keyboard` (`:88-94`) — an explicit full
//!   drain, layered on top at specific sourced call sites
//!   ([`InputQueue::clear`], e.g. the pagination-release clear at
//!   `seg041.cs:211`). FD-17 keeps this: it is a specific documented behavior,
//!   distinct from the (rejected) general drain-to-newest read policy.
//! - coab `engine/ovr027.cs:124,297-311` (`keypad_ctrl_codes`) — the
//!   9-entry table [`ExtKey::ctrl_code`] transcribes; arrow keys alias their
//!   numpad-direction equivalent (design doc D-UI6).

use std::collections::VecDeque;

/// One tick = 1/60s of game-presentation time (D-UI1).
pub const TICK_HZ: u32 = 60;

/// One logical input event, as collected by a frontend since the last tick
/// and pushed onto the engine's queue in order (D-UI1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InputEvent {
    /// A printable character, `0x20..=0x7A`. Not pre-uppercased by the
    /// frontend — the engine uppercases exactly where the original does.
    Char(u8),
    Enter,
    Escape,
    Backspace,
    /// The original's `0x00`-prefixed extended scancodes (arrows, Home/End/
    /// PgUp/PgDn, numpad).
    Ext(ExtKey),
}

/// The original's extended scancodes this engine models. `Kp5` is included
/// even though it carries no directional meaning: the original maps it to
/// `' '` via `keypad_ctrl_codes[4]` (`ovr027.cs:124`), so the mapping table
/// must be total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExtKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PgUp,
    PgDn,
    Kp1,
    Kp2,
    Kp3,
    Kp4,
    Kp5,
    Kp6,
    Kp7,
    Kp8,
    Kp9,
}

impl ExtKey {
    /// `keypad_ctrl_codes` (`ovr027.cs:124`): the ctrl-code character an
    /// `accept_ext` widget returns for this key. Arrow keys alias their
    /// numpad-direction equivalent (Up/Kp8, Down/Kp2, Left/Kp4, Right/Kp6,
    /// Home/Kp7, End/Kp1, PgUp/Kp9, PgDn/Kp3) — the original's own keyboard
    /// driver funnels both onto the same extended-scancode byte before
    /// `keypad_ctrl_codes` ever sees it (design doc D-UI6).
    pub fn ctrl_code(self) -> u8 {
        match self {
            ExtKey::End | ExtKey::Kp1 => b'O',
            ExtKey::Down | ExtKey::Kp2 => b'P',
            ExtKey::PgDn | ExtKey::Kp3 => b'Q',
            ExtKey::Left | ExtKey::Kp4 => b'K',
            ExtKey::Kp5 => b' ',
            ExtKey::Right | ExtKey::Kp6 => b'M',
            ExtKey::Home | ExtKey::Kp7 => b'G',
            ExtKey::Up | ExtKey::Kp8 => b'H',
            ExtKey::PgUp | ExtKey::Kp9 => b'I',
        }
    }
}

/// The engine-owned input queue (D-UI1): a frontend pushes the events it
/// collected since the last tick, in order; widgets read from it in FIFO
/// order (FD-17 — the original buffers type-ahead, confirmed against DOSBox).
#[derive(Debug, Clone, Default)]
pub struct InputQueue {
    events: VecDeque<InputEvent>,
}

impl InputQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `events`, in order, to the tail of the queue (a tick's worth
    /// of frontend-collected input).
    pub fn push_all(&mut self, events: &[InputEvent]) {
        self.events.extend(events.iter().copied());
    }

    /// Reads the oldest queued event (FIFO), `None` if the queue was empty.
    /// FD-17 (RESOLVED): the original *buffers* type-ahead — mashing forward
    /// five times during a slow redraw commits five queued steps, one per
    /// widget read, not one. coab's `GetInputKey` drain-to-newest
    /// (`seg043.cs:55-62`) is a transliteration/anti-key-repeat artifact,
    /// disproved by Bryan's live DOSBox mash test. The sourced pagination-
    /// release drain still applies, but through [`InputQueue::clear`], not
    /// here.
    pub fn read_key(&mut self) -> Option<InputEvent> {
        self.events.pop_front()
    }

    /// `clear_keyboard` (`seg043.cs:88-94`): an explicit full drain, called
    /// at the original's documented sites (after asset loads, after the
    /// pagination keypress, per-step/menu bookkeeping) independent of any
    /// widget read.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_key_returns_none_on_an_empty_queue() {
        let mut q = InputQueue::new();
        assert_eq!(q.read_key(), None);
    }

    #[test]
    fn read_key_pops_events_oldest_first_fifo() {
        // FD-17: the original buffers type-ahead — every queued key is read,
        // in order, one per call (mashing forward commits every queued step).
        let mut q = InputQueue::new();
        q.push_all(&[
            InputEvent::Char(b'A'),
            InputEvent::Char(b'B'),
            InputEvent::Char(b'C'),
        ]);
        assert_eq!(q.read_key(), Some(InputEvent::Char(b'A')));
        assert_eq!(q.read_key(), Some(InputEvent::Char(b'B')));
        assert_eq!(q.read_key(), Some(InputEvent::Char(b'C')));
        assert!(q.is_empty(), "every queued key must have been readable");
        assert_eq!(q.read_key(), None);
    }

    #[test]
    fn push_all_appends_in_order_across_ticks() {
        let mut q = InputQueue::new();
        q.push_all(&[InputEvent::Char(b'A')]);
        q.push_all(&[InputEvent::Char(b'B')]);
        // FIFO across ticks: the first-pushed event reads back first.
        assert_eq!(q.read_key(), Some(InputEvent::Char(b'A')));
        assert_eq!(q.read_key(), Some(InputEvent::Char(b'B')));
    }

    #[test]
    fn clear_drops_everything_without_returning_it() {
        let mut q = InputQueue::new();
        q.push_all(&[InputEvent::Char(b'A'), InputEvent::Enter]);
        q.clear();
        assert!(q.is_empty());
        assert_eq!(q.read_key(), None);
    }

    #[test]
    fn ext_key_ctrl_code_table_is_total_and_matches_keypad_ctrl_codes() {
        assert_eq!(ExtKey::Kp1.ctrl_code(), b'O');
        assert_eq!(ExtKey::Kp2.ctrl_code(), b'P');
        assert_eq!(ExtKey::Kp3.ctrl_code(), b'Q');
        assert_eq!(ExtKey::Kp4.ctrl_code(), b'K');
        assert_eq!(ExtKey::Kp5.ctrl_code(), b' ');
        assert_eq!(ExtKey::Kp6.ctrl_code(), b'M');
        assert_eq!(ExtKey::Kp7.ctrl_code(), b'G');
        assert_eq!(ExtKey::Kp8.ctrl_code(), b'H');
        assert_eq!(ExtKey::Kp9.ctrl_code(), b'I');
    }

    #[test]
    fn arrow_keys_alias_their_numpad_direction_equivalent() {
        assert_eq!(ExtKey::Up.ctrl_code(), ExtKey::Kp8.ctrl_code());
        assert_eq!(ExtKey::Down.ctrl_code(), ExtKey::Kp2.ctrl_code());
        assert_eq!(ExtKey::Left.ctrl_code(), ExtKey::Kp4.ctrl_code());
        assert_eq!(ExtKey::Right.ctrl_code(), ExtKey::Kp6.ctrl_code());
        assert_eq!(ExtKey::Home.ctrl_code(), ExtKey::Kp7.ctrl_code());
        assert_eq!(ExtKey::End.ctrl_code(), ExtKey::Kp1.ctrl_code());
        assert_eq!(ExtKey::PgUp.ctrl_code(), ExtKey::Kp9.ctrl_code());
        assert_eq!(ExtKey::PgDn.ctrl_code(), ExtKey::Kp3.ctrl_code());
    }
}
