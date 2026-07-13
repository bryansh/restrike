//! egui key/text events -> `InputEvent` (the live engine pane's keyboard
//! passthrough, task deliverable 4). Unlike `frontends/desktop/src/keymap.rs`
//! this crate doesn't need a numpad/physical-key distinction: the engine
//! itself already aliases plain digit-key `Char('1'..='9')` events onto the
//! same `keypad_ctrl_codes` a numpad key would resolve to, whenever a widget
//! has `accept_ext` set (`widgets.rs`'s `Hotbar::tick`) — so this module only
//! has to distinguish named/extended keys from plain printable text, not
//! numpad-vs-top-row keys.

use eframe::egui;
use gbx_engine::input::{ExtKey, InputEvent};

/// Maps a named/extended egui key to its `InputEvent`. Returns `None` for
/// any key with no D-UI1 analogue (letters/digits arrive via
/// [`map_text`]'s `egui::Event::Text` instead, since egui doesn't expose a
/// layout-independent "printable character" key variant).
pub fn map_key(key: egui::Key) -> Option<InputEvent> {
    match key {
        egui::Key::Enter => Some(InputEvent::Enter),
        egui::Key::Escape => Some(InputEvent::Escape),
        egui::Key::Backspace => Some(InputEvent::Backspace),
        egui::Key::ArrowUp => Some(InputEvent::Ext(ExtKey::Up)),
        egui::Key::ArrowDown => Some(InputEvent::Ext(ExtKey::Down)),
        egui::Key::ArrowLeft => Some(InputEvent::Ext(ExtKey::Left)),
        egui::Key::ArrowRight => Some(InputEvent::Ext(ExtKey::Right)),
        egui::Key::Home => Some(InputEvent::Ext(ExtKey::Home)),
        egui::Key::End => Some(InputEvent::Ext(ExtKey::End)),
        egui::Key::PageUp => Some(InputEvent::Ext(ExtKey::PgUp)),
        egui::Key::PageDown => Some(InputEvent::Ext(ExtKey::PgDn)),
        _ => None,
    }
}

/// Maps one `egui::Event::Text` payload to `InputEvent::Char`, per D-UI1's
/// `0x20..=0x7A` printable range (layout-resolved text, matching
/// `frontends/desktop`'s own convention).
pub fn map_text(text: &str) -> Option<InputEvent> {
    let ch = text.chars().next()?;
    if !ch.is_ascii() {
        return None;
    }
    let byte = ch as u8;
    (0x20..=0x7A)
        .contains(&byte)
        .then_some(InputEvent::Char(byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_map_to_their_input_event() {
        assert_eq!(map_key(egui::Key::Enter), Some(InputEvent::Enter));
        assert_eq!(map_key(egui::Key::Escape), Some(InputEvent::Escape));
        assert_eq!(map_key(egui::Key::Backspace), Some(InputEvent::Backspace));
        assert_eq!(
            map_key(egui::Key::ArrowUp),
            Some(InputEvent::Ext(ExtKey::Up))
        );
        assert_eq!(
            map_key(egui::Key::PageDown),
            Some(InputEvent::Ext(ExtKey::PgDn))
        );
    }

    #[test]
    fn unmapped_keys_return_none() {
        assert_eq!(map_key(egui::Key::F1), None);
        assert_eq!(map_key(egui::Key::A), None);
    }

    #[test]
    fn printable_text_maps_to_char() {
        assert_eq!(map_text("a"), Some(InputEvent::Char(b'a')));
        assert_eq!(map_text("Z"), Some(InputEvent::Char(b'Z')));
        assert_eq!(map_text(" "), Some(InputEvent::Char(b' ')));
    }

    #[test]
    fn empty_or_non_ascii_text_is_none() {
        assert_eq!(map_text(""), None);
        assert_eq!(map_text("é"), None);
    }

    #[test]
    fn text_outside_the_printable_range_is_none() {
        assert_eq!(map_text("\u{7f}"), None); // DEL, just past 0x7A
    }
}
