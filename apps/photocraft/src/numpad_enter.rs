//! Keypad Enter vs main Return, read from the OS each frame (#6007).
//!
//! egui-winit maps both to [`egui::Key::Enter`]. Photoshop commits inline type on the keypad key
//! and inserts a line break on Return. [`photocraft_ui_egui::keyboard_tags`] tags keypad presses
//! when this service reports the numpad key down without main Return (same polling idea as
//! [`caps_lock`]).
//!
//! - **macOS**: `CGEventSource::key_state` for keypad Enter (`0x4C`) and Return (`0x24`).
//! - **X11 (and XWayland)**: `XQueryKeymap` for `XK_KP_Enter` and `XK_Return`.
//! - **Windows**: both keys share `VK_RETURN` in `GetAsyncKeyState`; scan codes (`0xE01C` numpad vs
//!   `0x001C` main) are checked with `MapVirtualKey` against `VK::RETURN` only, then
//!   `GetAsyncKeyState(VK::RETURN)` (no `VK::from_raw` — workspace forbids `unsafe`).
//! - **Native Wayland / web**: no global keyboard query; the service is `None`.

pub use platform::service;

#[cfg(target_os = "macos")]
mod platform {
    use core_graphics2::event_source::CGEventSource;
    use core_graphics2::event_types::CGEventSourceStateID;
    use photocraft_ui_egui::EnterKeysFn;

    const KEYPAD_ENTER: u16 = 0x4c;
    const RETURN: u16 = 0x24;

    pub fn service(_cc: &eframe::CreationContext<'_>) -> Option<EnterKeysFn> {
        Some(Box::new(|| {
            let st = CGEventSourceStateID::CombinedSessionState;
            (CGEventSource::key_state(st, KEYPAD_ENTER), CGEventSource::key_state(st, RETURN))
        }))
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use photocraft_ui_egui::EnterKeysFn;
    use winsafe::co::{MAPVK, VK};

    const RETURN_RAW: u32 = VK::RETURN.raw() as u32;

    fn sc_maps_return(sc: u32, map: MAPVK) -> bool {
        winsafe::MapVirtualKey(sc, map) == RETURN_RAW
    }

    pub fn service(_cc: &eframe::CreationContext<'_>) -> Option<EnterKeysFn> {
        Some(Box::new(|| {
            let down = winsafe::GetAsyncKeyState(VK::RETURN);
            if !down {
                return (false, false);
            }
            let numpad = sc_maps_return(0x00e0_001c, MAPVK::VSC_TO_VK_EX);
            let main = sc_maps_return(0x001c, MAPVK::VSC_TO_VK);
            if numpad && main {
                // `GetAsyncKeyState` is keyed by VK only; both scan codes map to Return.
                (false, true)
            } else {
                (numpad, main)
            }
        }))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
    use photocraft_ui_egui::EnterKeysFn;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt as _;
    use x11rb::rust_connection::RustConnection;

    const XK_RETURN: u32 = 0xff0d;
    const XK_KP_ENTER: u32 = 0xff8d;

    struct State {
        conn: RustConnection,
        main_kc: u8,
        numpad_kc: u8,
    }

    fn keycodes(conn: &RustConnection) -> Result<(u8, u8), String> {
        let setup = conn.setup();
        let min_kc = setup.min_keycode;
        let count = setup.max_keycode - min_kc + 1;
        let map = conn.get_keyboard_mapping(min_kc, count).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        let per = map.keysyms_per_keycode as usize;
        let mut main_kc = None;
        let mut numpad_kc = None;
        for i in 0..count as usize {
            let kc = min_kc + i as u8;
            let start = i * per;
            let end = start.saturating_add(per).min(map.keysyms.len());
            let group = &map.keysyms[start..end];
            if group.contains(&XK_RETURN) {
                main_kc = Some(kc);
            }
            if group.contains(&XK_KP_ENTER) {
                numpad_kc = Some(kc);
            }
        }
        Ok((main_kc.ok_or("no XK_Return keycode")?, numpad_kc.ok_or("no XK_KP_Enter keycode")?))
    }

    fn down(conn: &RustConnection, kc: u8) -> bool {
        let map = match conn.query_keymap().ok().and_then(|c| c.reply().ok()) {
            Some(m) => m,
            None => return false,
        };
        let bit = kc as usize;
        map.keys.get(bit / 8).is_some_and(|b| b & (1 << (bit % 8)) != 0)
    }

    pub fn service(cc: &eframe::CreationContext<'_>) -> Option<EnterKeysFn> {
        let is_x11 = matches!(cc.window_handle().ok()?.as_raw(), RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_));
        if !is_x11 {
            return None;
        }
        let (conn, _) = match RustConnection::connect(None) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("enter_keys: {e}");
                return None;
            }
        };
        let (main_kc, numpad_kc) = match keycodes(&conn) {
            Ok(k) => k,
            Err(e) => {
                log::warn!("enter_keys: {e}");
                return None;
            }
        };
        let state = State { conn, main_kc, numpad_kc };
        Some(Box::new(move || (down(&state.conn, state.numpad_kc), down(&state.conn, state.main_kc))))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod platform {
    use photocraft_ui_egui::EnterKeysFn;

    pub fn service(_cc: &eframe::CreationContext<'_>) -> Option<EnterKeysFn> {
        None
    }
}
