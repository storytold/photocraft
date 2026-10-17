//! Desktop keyboard tags egui-winit does not provide.
//!
//! egui-winit maps main and numpad Enter to [`egui::Key::Enter`]. Photoshop commits inline type
//! on keypad Enter and inserts a line break on main Return (#6007). The shell polls the OS (see
//! `apps/photocraft/src/numpad_enter.rs`, same idea as Caps Lock) and marks keypad Enter on
//! `physical_key` with an unused function key the type editor recognises.

use egui::{Event, Key, RawInput};

/// Sentinel on [`Event::Key::physical_key`] for keypad Enter (see module docs).
pub(crate) const NUMPAD_ENTER_TAG: Key = Key::F35;

/// When the numpad Enter key is down and main Return is not, tag matching key presses.
pub(crate) fn tag_numpad_enter(raw: &mut RawInput, numpad_down: bool, main_return_down: bool) {
    if !numpad_down || main_return_down {
        return;
    }
    for e in &mut raw.events {
        if let Event::Key { key: Key::Enter, pressed: true, physical_key, .. } = e {
            *physical_key = Some(NUMPAD_ENTER_TAG);
        }
    }
}
