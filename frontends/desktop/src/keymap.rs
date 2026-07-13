//! Platform key event -> `InputEvent` (D-UI6): the frontend maps physical
//! presentation, never meaning. Numpad digits are read from the physical
//! key (layout/NumLock-independent); everything else from the logical key
//! so AZERTY users get the character they actually typed.

use gbx_engine::input::{ExtKey, InputEvent};
use winit::event::KeyEvent;
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

pub fn map_key(event: &KeyEvent) -> Option<InputEvent> {
    if let PhysicalKey::Code(code) = event.physical_key {
        let kp = match code {
            KeyCode::Numpad1 => Some(ExtKey::Kp1),
            KeyCode::Numpad2 => Some(ExtKey::Kp2),
            KeyCode::Numpad3 => Some(ExtKey::Kp3),
            KeyCode::Numpad4 => Some(ExtKey::Kp4),
            KeyCode::Numpad5 => Some(ExtKey::Kp5),
            KeyCode::Numpad6 => Some(ExtKey::Kp6),
            KeyCode::Numpad7 => Some(ExtKey::Kp7),
            KeyCode::Numpad8 => Some(ExtKey::Kp8),
            KeyCode::Numpad9 => Some(ExtKey::Kp9),
            _ => None,
        };
        if let Some(kp) = kp {
            return Some(InputEvent::Ext(kp));
        }
    }

    match event.logical_key {
        Key::Named(NamedKey::Enter) => return Some(InputEvent::Enter),
        Key::Named(NamedKey::Escape) => return Some(InputEvent::Escape),
        Key::Named(NamedKey::Backspace) => return Some(InputEvent::Backspace),
        Key::Named(NamedKey::ArrowUp) => return Some(InputEvent::Ext(ExtKey::Up)),
        Key::Named(NamedKey::ArrowDown) => return Some(InputEvent::Ext(ExtKey::Down)),
        Key::Named(NamedKey::ArrowLeft) => return Some(InputEvent::Ext(ExtKey::Left)),
        Key::Named(NamedKey::ArrowRight) => return Some(InputEvent::Ext(ExtKey::Right)),
        Key::Named(NamedKey::Home) => return Some(InputEvent::Ext(ExtKey::Home)),
        Key::Named(NamedKey::End) => return Some(InputEvent::Ext(ExtKey::End)),
        Key::Named(NamedKey::PageUp) => return Some(InputEvent::Ext(ExtKey::PgUp)),
        Key::Named(NamedKey::PageDown) => return Some(InputEvent::Ext(ExtKey::PgDn)),
        _ => {}
    }

    // Layout-resolved printable text (D-UI6: "AZERTY users get what they
    // type"), not scancodes -- InputEvent::Char's documented 0x20..=0x7A
    // range.
    let ch = event.text.as_ref()?.chars().next()?;
    if ch.is_ascii() {
        let byte = ch as u8;
        if (0x20..=0x7A).contains(&byte) {
            return Some(InputEvent::Char(byte));
        }
    }
    None
}
