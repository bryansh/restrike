//! Browser `KeyboardEvent` -> `InputEvent` (D-UI6): the web analogue of
//! `restrike-desktop`'s `keymap.rs`. `code` (the physical key, layout- and
//! NumLock-independent) picks numpad digits; `key` (the layout-resolved
//! logical key, so AZERTY users get the character they typed) picks
//! everything else.

use gbx_engine::input::{ExtKey, InputEvent};

pub fn map_key(key: &str, code: &str) -> Option<InputEvent> {
    let kp = match code {
        "Numpad1" => Some(ExtKey::Kp1),
        "Numpad2" => Some(ExtKey::Kp2),
        "Numpad3" => Some(ExtKey::Kp3),
        "Numpad4" => Some(ExtKey::Kp4),
        "Numpad5" => Some(ExtKey::Kp5),
        "Numpad6" => Some(ExtKey::Kp6),
        "Numpad7" => Some(ExtKey::Kp7),
        "Numpad8" => Some(ExtKey::Kp8),
        "Numpad9" => Some(ExtKey::Kp9),
        _ => None,
    };
    if let Some(kp) = kp {
        return Some(InputEvent::Ext(kp));
    }

    match key {
        "Enter" => return Some(InputEvent::Enter),
        "Escape" => return Some(InputEvent::Escape),
        "Backspace" => return Some(InputEvent::Backspace),
        "ArrowUp" => return Some(InputEvent::Ext(ExtKey::Up)),
        "ArrowDown" => return Some(InputEvent::Ext(ExtKey::Down)),
        "ArrowLeft" => return Some(InputEvent::Ext(ExtKey::Left)),
        "ArrowRight" => return Some(InputEvent::Ext(ExtKey::Right)),
        "Home" => return Some(InputEvent::Ext(ExtKey::Home)),
        "End" => return Some(InputEvent::Ext(ExtKey::End)),
        "PageUp" => return Some(InputEvent::Ext(ExtKey::PgUp)),
        "PageDown" => return Some(InputEvent::Ext(ExtKey::PgDn)),
        _ => {}
    }

    // A single-character logical key is the printable case (multi-char
    // names like "Shift"/"Tab"/"F1" fall through to None) --
    // InputEvent::Char's documented 0x20..=0x7A range.
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_none() && ch.is_ascii() {
        let byte = ch as u8;
        if (0x20..=0x7A).contains(&byte) {
            return Some(InputEvent::Char(byte));
        }
    }
    None
}
