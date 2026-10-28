//! The `postMessage` bridge to a same-origin parent page (`?host=parent`), for pages that embed
//! PhotoCraft in an `<iframe>` (such as a Nextcloud app). Messages are plain objects with a `type`:
//!
//! - `photocraft:ready` (to the parent): `{ version }`, the app is listening; send a file now.
//! - `photocraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. It arrives like a file picked with File › Open or dropped on the window.
//! - `photocraft:pick-save` (to the parent): `{ id, name }` when Save, Save As or Export asks
//!   where to save, with the suggested file name (its extension sets the format).
//! - `photocraft:picked` (from the parent): `{ id, path }` answers it; a missing or empty `path`
//!   cancels.
//! - `photocraft:save` (to the parent): `{ name, bytes }` (a transferred `ArrayBuffer`) for every
//!   file PhotoCraft writes (`Services::write`): `name` is the document's own file name for a
//!   save in place, or the path the parent picked. The parent stores it and reports failures
//!   itself.
//! - `photocraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
//! - `photocraft:failed` (to the parent): `{ error }` when the app couldn't start.
//!
//! Only the parent window on the page's own origin is heard or answered. In host mode the parent
//! guards leaving the page with unsaved changes.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest, Services};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

type Inbox = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

#[derive(Clone)]
pub struct Bridge {
    window: web_sys::Window,
    parent: web_sys::Window,
    origin: String,
    /// Save locations asked of the parent and not answered yet, by request id.
    pending: Rc<RefCell<HashMap<u32, FileDialogReply>>>,
    next_id: Rc<Cell<u32>>,
    /// The unsaved-changes state last reported to the parent.
    reported: Rc<Cell<Option<bool>>>,
}

impl Bridge {
    /// `None` when the page isn't framed (there is no parent to talk to).
    pub fn new() -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let origin = window.location().origin().ok()?;
        Some(Self { window, parent, origin, pending: Rc::default(), next_id: Rc::default(), reported: Rc::default() })
    }

    /// Ask the parent where to save and send it what PhotoCraft writes, and hear files and
    /// answers from it. File › Open keeps the browser's own file picker.
    pub fn connect(&self, services: &mut Services, inbox: Inbox, ctx: egui::Context) {
        self.listen(inbox, ctx);
        let bridge = self.clone();
        let mut open_dialog = services.file_dialog.take();
        services.file_dialog = Some(Box::new(move |request, frame, reply| match request {
            FileDialogRequest::Save { suggested } => bridge.ask(&suggested, reply),
            open => match open_dialog.as_mut() {
                Some(dialog) => dialog(open, frame, reply),
                None => reply.send(None),
            },
        }));
        let bridge = self.clone();
        services.write = Some(Box::new(move |path: &str, bytes: &[u8]| bridge.post_save(path, bytes)));
        // Unlike a browser download (#4962), what the parent is handed it stores, reporting a
        // failure itself: a save to it counts as done.
        services.write_unconfirmed = false;
    }

    pub fn post_ready(&self) {
        self.send(&[("type", "photocraft:ready".into()), ("version", env!("CARGO_PKG_VERSION").into())]);
    }

    pub fn post_failed(&self, error: &str) {
        self.send(&[("type", "photocraft:failed".into()), ("error", error.into())]);
    }

    /// After each frame: tell the parent when unsaved changes appear or go away.
    pub fn report_dirty(&self, dirty: bool) {
        if self.reported.get() != Some(dirty) && self.send(&[("type", "photocraft:dirty".into()), ("dirty", dirty.into())]) {
            self.reported.set(Some(dirty));
        }
    }

    /// Ask the parent where to save; the answer arrives as `photocraft:picked`.
    fn ask(&self, suggested: &str, reply: FileDialogReply) {
        let name = std::path::Path::new(suggested).file_name().map_or_else(|| suggested.to_string(), |n| n.to_string_lossy().to_string());
        let id = self.next_id.get().wrapping_add(1);
        self.next_id.set(id);
        if self.send(&[("type", "photocraft:pick-save".into()), ("id", id.into()), ("name", name.as_str().into())]) {
            self.pending.borrow_mut().insert(id, reply);
        } else {
            reply.send(None);
        }
    }

    /// A path the parent picked stays a path (it starts with `/`); a save in place goes by the
    /// document's own file name.
    fn post_save(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let name = if path.starts_with('/') {
            path.to_string()
        } else {
            std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().to_string())
        };
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let message = message(&[("type", "photocraft:save".into()), ("name", name.as_str().into()), ("bytes", buffer.clone().into())])
            .ok_or_else(|| "couldn't build the message".to_string())?;
        self.parent
            .post_message_with_transfer(&message, &self.origin, &js_sys::Array::of1(&buffer))
            .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
    }

    fn listen(&self, inbox: Inbox, ctx: egui::Context) {
        let parent = self.parent.clone();
        let origin = self.origin.clone();
        let pending = self.pending.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                return;
            }
            let data = event.data();
            match string(&data, "type").as_deref() {
                Some("photocraft:open") => {
                    let name = string(&data, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Image.png".to_string());
                    let Some(bytes) = bytes(&data) else { return };
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                }
                Some("photocraft:picked") => {
                    let Some(id) = number(&data, "id") else { return };
                    let Some(reply) = pending.borrow_mut().remove(&id) else { return };
                    let path = string(&data, "path").filter(|p| !p.trim().is_empty());
                    reply.send(path.map(FileDialogAnswer::SaveTo));
                }
                _ => return,
            }
            ctx.request_repaint();
        });
        if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            // The listener lives as long as the page.
            on_message.forget();
        }
    }

    /// Post a message to the parent; false if it couldn't be sent.
    fn send(&self, fields: &[(&str, JsValue)]) -> bool {
        message(fields).is_some_and(|m| self.parent.post_message(&m, &self.origin).is_ok())
    }
}

fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
    let m = js_sys::Object::new();
    for (key, value) in fields {
        js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
    }
    Some(m)
}

fn string(data: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_string()
}

/// A request id: a whole number that fits in `u32`.
fn number(data: &JsValue, key: &str) -> Option<u32> {
    let n = js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_f64()?;
    (n.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&n)).then_some(n as u32)
}

/// `bytes` as an `ArrayBuffer` or `Uint8Array`.
fn bytes(data: &JsValue) -> Option<Vec<u8>> {
    let bytes = js_sys::Reflect::get(data, &JsValue::from_str("bytes")).ok()?;
    if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        Some(array.to_vec())
    } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
        Some(js_sys::Uint8Array::new(&bytes).to_vec())
    } else {
        None
    }
}
