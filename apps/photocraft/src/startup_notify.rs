//! Ends the desktop's launch feedback (the spinning "busy" cursor) once the window exists.
//!
//! A launcher that starts us from a `.desktop` file with `StartupNotify=true` passes a startup id
//! in `DESKTOP_STARTUP_ID` (X11) and shows a busy cursor until the app says the launch is over
//! with a `remove: ID=…` message (freedesktop startup-notification spec). winit can send it but
//! eframe never hands it the token, so without this GNOME kept the cursor spinning until its
//! timeout (about 15 s) after the window was already up.
//!
//! - **X11**: tag our window with `_NET_STARTUP_ID` and broadcast the `remove` message on a
//!   second, pure-Rust X connection (off the UI thread: a slow X server never delays startup).
//! - **Wayland**: the token (`XDG_ACTIVATION_TOKEN`) is only used for focus; nothing spins.
//!
//! No `unsafe`, so the variable stays in the environment (edition 2024's `remove_var` is unsafe);
//! a child that inherits it only refers to a launch that is already over.

/// Finish the launch for the main window, when the launcher started one.
pub fn finish(cc: &eframe::CreationContext<'_>) {
    use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
    let Some(id) = std::env::var("DESKTOP_STARTUP_ID").ok().filter(|id| !id.is_empty()) else { return };
    let window = match cc.window_handle().ok().map(|h| h.as_raw()) {
        Some(RawWindowHandle::Xlib(h)) => u32::try_from(h.window).ok(),
        Some(RawWindowHandle::Xcb(h)) => Some(h.window.get()),
        _ => return,
    };
    let spawned = std::thread::Builder::new().name("startup-notify".into()).spawn(move || {
        if let Err(e) = send_remove(window, &id) {
            log::warn!("couldn't end the launch feedback: {e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("couldn't end the launch feedback: {e}");
    }
}

fn send_remove(window: Option<u32>, id: &str) -> Result<(), String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, PropMode, WindowClass};
    use x11rb::wrapper::ConnectionExt as _;
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let (conn, screen) = x11rb::rust_connection::RustConnection::connect(None).map_err(|e| err(&e))?;
    let root = conn.setup().roots.get(screen).ok_or("no X screen")?.root;
    let atom = |name: &[u8]| -> Result<u32, String> {
        Ok(conn.intern_atom(false, name).map_err(|e| err(&e))?.reply().map_err(|e| err(&e))?.atom)
    };
    let (begin, more) = (atom(b"_NET_STARTUP_INFO_BEGIN")?, atom(b"_NET_STARTUP_INFO")?);
    if let Some(window) = window {
        let prop = atom(b"_NET_STARTUP_ID")?;
        conn.change_property8(PropMode::REPLACE, window, prop, AtomEnum::STRING, id.as_bytes()).map_err(|e| err(&e))?;
    }
    // The spec sends the message from a window of the sender's own.
    let sender = conn.generate_id().map_err(|e| err(&e))?;
    let aux = CreateWindowAux::new().override_redirect(1);
    conn.create_window(0, sender, root, -100, -100, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &aux).map_err(|e| err(&e))?;
    for (i, chunk) in chunks(&remove_message(id)).into_iter().enumerate() {
        let event = ClientMessageEvent::new(8, sender, if i == 0 { begin } else { more }, chunk);
        conn.send_event(false, root, EventMask::PROPERTY_CHANGE, event).map_err(|e| err(&e))?;
    }
    conn.destroy_window(sender).map_err(|e| err(&e))?;
    conn.sync().map_err(|e| err(&e))?;
    Ok(())
}

/// `remove: ID="<id>"` with `"` and `\` escaped, NUL-terminated.
fn remove_message(id: &str) -> Vec<u8> {
    let mut msg = b"remove: ID=\"".to_vec();
    for b in id.bytes() {
        if b == b'"' || b == b'\\' {
            msg.push(b'\\');
        }
        msg.push(b);
    }
    msg.extend_from_slice(b"\"\0");
    msg
}

/// The message split into the 20-byte payloads of format-8 client messages, zero-padded.
fn chunks(msg: &[u8]) -> Vec<[u8; 20]> {
    msg.chunks(20)
        .map(|c| {
            let mut out = [0u8; 20];
            for (o, b) in out.iter_mut().zip(c) {
                *o = *b;
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_message_quotes_the_id() {
        assert_eq!(remove_message("abc_TIME42"), b"remove: ID=\"abc_TIME42\"\0");
        assert_eq!(remove_message("a\"b\\c"), b"remove: ID=\"a\\\"b\\\\c\"\0");
        assert_eq!(remove_message(""), b"remove: ID=\"\"\0");
    }

    #[test]
    fn chunks_pad_the_last_payload() {
        let msg = remove_message("gnome-shell-1234-host-photocraft-1_TIME5678");
        let parts = chunks(&msg);
        assert_eq!(parts.len(), msg.len().div_ceil(20));
        let joined: Vec<u8> = parts.concat();
        assert_eq!(&joined[..msg.len()], &msg[..]);
        assert!(joined[msg.len()..].iter().all(|&b| b == 0));
        assert!(chunks(b"").is_empty());
    }
}
