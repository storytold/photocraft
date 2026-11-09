//! Images from the system clipboard in the browser (#7842).
//!
//! A page can read the clipboard only in a `paste` event, the default action of Ctrl+V (⌘V),
//! and an image comes in it as a file that has to be read and decoded asynchronously. eframe
//! forwards only the text of a paste, so the app never saw the image and Ctrl+V reported an
//! empty clipboard. Here each paste event's image is read, decoded and kept as what the system
//! clipboard holds: `Services::clipboard_get_image` hands it to the app as the desktop's
//! clipboard read does, and keyboard events wait while it is read, so the Ctrl+V among them
//! pastes it.
//!
//! A paste from the browser's own Edit menu comes with no key press, so once its image has been
//! read the shell presses ⌘V for it ([`WebClipboard::take_press`]).
//!
//! This build doesn't put PhotoCraft's own copies on the system clipboard, so a copy made in
//! PhotoCraft stays newer than the image there. It stops being newer only once the page has lost
//! focus (another app may copy then) and a later paste shows the clipboard holding something
//! else. Losing focus also makes the image the last paste showed unknown: Edit › Paste from
//! PhotoCraft's menu has no paste event to read, so it asks for Ctrl+V until one comes.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::rc::Rc;

use photocraft_ui_egui::i18n;

/// RGBA8 pixels with their width and height, as `Services::clipboard_get_image` returns them.
pub type Pixels = (u32, u32, Vec<u8>);

/// What the page has seen of the system clipboard. The `paste` listener, the clipboard services
/// and the shell share it; the page has a single thread.
#[derive(Clone, Default)]
pub struct WebClipboard(Rc<RefCell<State>>);

#[derive(Default)]
struct State {
    /// The image the last paste event carried; `None` when it carried none (text, nothing) or
    /// before the first paste.
    shown: Option<Shown>,
    /// Paste events whose image is still being read.
    reading: usize,
    /// Counts paste events, so an image that takes longer to read can't replace a newer paste's.
    pastes: u64,
    /// The last copy PhotoCraft made, for as long as it is newer than the system clipboard's
    /// image.
    copied: Option<Copied>,
    /// The page lost focus since the last paste, so the clipboard may hold something else now.
    stale: bool,
    /// Keys held down on the page (`KeyboardEvent.code`): a paste while none is held came from a
    /// menu, not from Ctrl+V.
    keys: BTreeSet<String>,
    /// A paste from a menu is waiting for its image, which is to be pasted once read.
    press: bool,
}

struct Shown {
    /// The file's bytes, hashed: the same file shown again is the same clipboard content.
    /// `None` when the file couldn't be read.
    file: Option<u64>,
    /// The decoded pixels, or why there are none.
    pixels: Result<Pixels, String>,
}

struct Copied {
    /// The file the system clipboard held under the copy, as far as a paste has shown it.
    under: Option<u64>,
    /// The page lost focus since the copy, so another app may have copied something newer.
    left: bool,
}

impl WebClipboard {
    /// A paste event, carrying an image file (`image`) and text (`text`) or not. Returns the
    /// paste's number for [`WebClipboard::read`].
    pub fn paste(&self, image: bool, text: bool) -> u64 {
        let mut s = self.0.borrow_mut();
        s.pastes += 1;
        if image {
            s.reading += 1;
            // eframe turns a paste's text into a paste, and Ctrl+V's key press is one already.
            s.press |= !text && s.keys.is_empty();
        } else {
            s.show(None);
        }
        s.pastes
    }

    /// A key went down (`down`) or up on the page.
    pub fn key(&self, code: &str, down: bool) {
        let keys = &mut self.0.borrow_mut().keys;
        if down {
            keys.insert(code.to_owned());
        } else {
            keys.remove(code);
        }
    }

    /// Paste `number` is still the newest, so its image is worth reading.
    pub fn newest(&self, number: u64) -> bool {
        self.0.borrow().pastes == number
    }

    /// The image file of paste `number` has been read: its bytes, or why they couldn't be.
    /// It is decoded only while no newer paste has come.
    pub fn read(&self, number: u64, file: Result<Vec<u8>, String>) {
        let shown = self.newest(number).then(|| Shown::from_file(file));
        let mut s = self.0.borrow_mut();
        s.reading = s.reading.saturating_sub(1);
        if let Some(shown) = shown
            && s.pastes == number
        {
            s.show(Some(shown));
        }
    }

    /// Paste events are still waiting for their image.
    pub fn reading(&self) -> bool {
        self.0.borrow().reading > 0
    }

    /// A paste from a menu has read its image: true once, when the shell is to press ⌘V for it.
    pub fn take_press(&self) -> bool {
        let mut s = self.0.borrow_mut();
        let press = s.press && s.reading == 0;
        if press {
            s.press = false;
        }
        press
    }

    /// `Services::clipboard_copied`: PhotoCraft made a copy, which stays in the app, newer than
    /// the image on the system clipboard.
    pub fn copied(&self) {
        let mut s = self.0.borrow_mut();
        let under = s.shown.as_ref().and_then(|shown| shown.file);
        s.copied = Some(Copied { under, left: false });
    }

    /// The page lost focus: another app may copy before it is back, and the keys held now are
    /// let go elsewhere.
    pub fn left(&self) {
        let mut s = self.0.borrow_mut();
        s.stale = true;
        s.keys.clear();
        if let Some(copied) = &mut s.copied {
            copied.left = true;
        }
    }

    /// `Services::clipboard_get_image`: the image the last paste showed on the system clipboard,
    /// unless PhotoCraft has copied since and its copy is still the newer one. Once the page has
    /// lost focus that image may be gone, and only a paste event shows what is there now.
    pub fn image(&self) -> Result<Option<Pixels>, String> {
        let s = self.0.borrow();
        match &s.shown {
            _ if s.copied.is_some() => Ok(None),
            Some(_) if s.stale => Err(i18n::t("Press Ctrl+V (⌘V on a Mac) to paste from the system clipboard").to_owned()),
            Some(shown) => shown.pixels.clone().map(Some),
            None => Ok(None),
        }
    }
}

impl State {
    /// A paste showed what the system clipboard holds now.
    fn show(&mut self, shown: Option<Shown>) {
        let file = shown.as_ref().and_then(|shown| shown.file);
        if let Some(copied) = &mut self.copied {
            if !copied.left {
                // Nothing else could copy since PhotoCraft did: this was already there.
                copied.under = file;
            } else if copied.under != file {
                // Another app copied after PhotoCraft: its image is the newer one.
                self.copied = None;
            }
        }
        self.shown = shown;
        self.stale = false;
    }
}

impl Shown {
    fn from_file(file: Result<Vec<u8>, String>) -> Shown {
        match file {
            Ok(bytes) => {
                let mut hasher = DefaultHasher::new();
                bytes.hash(&mut hasher);
                Shown { file: Some(hasher.finish()), pixels: decode(&bytes) }
            }
            Err(error) => Shown { file: None, pixels: Err(unreadable(&error)) },
        }
    }
}

/// An image file from the clipboard (PNG, JPEG, …) as RGBA8 pixels.
pub fn decode(bytes: &[u8]) -> Result<Pixels, String> {
    let image = photocraft_codecs::decode(bytes).map_err(|error| unreadable(&error.to_string()))?;
    Ok((image.width(), image.height(), image.to_rgba8()))
}

fn unreadable(error: &str) -> String {
    i18n::fmt(i18n::t("Couldn't read the pasted image: {error}"), &[("error", error)])
}

/// The largest clipboard file read: the most a decoded image may take ([`photocraft_codecs::Limits`],
/// 1 GiB on wasm32), as a larger file can't decode within it.
pub fn max_file_bytes() -> u64 {
    photocraft_codecs::Limits::default().max_alloc
}

/// Why a file of `bytes` isn't read, or `None` when it is.
pub fn too_large(bytes: u64) -> Option<String> {
    (bytes > max_file_bytes()).then(|| i18n::t("the file is too large").to_owned())
}

/// A clipboard file type that decodes to pixels. SVG is markup, so it isn't one.
pub fn is_raster(mime: &str) -> bool {
    mime.starts_with("image/") && mime != "image/svg+xml"
}

/// Input held back while a pasted image is read: all of it, so the Ctrl+V among it pastes the new
/// image rather than what the clipboard held before, into the document that was active when it
/// was pressed (a click on another tab waits behind it).
#[derive(Default)]
pub struct Held(Vec<egui::Event>);

impl Held {
    /// While `reading`, takes this frame's `events`; once it is done, puts every held event back
    /// in front of them, in the order they came. With `press`, a ⌘V press for a paste from a menu
    /// goes first ([`WebClipboard::take_press`]).
    pub fn hold_or_release(&mut self, reading: bool, press: bool, events: &mut Vec<egui::Event>) {
        if press {
            let key = |pressed| egui::Event::Key { key: egui::Key::V, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::COMMAND };
            self.0.splice(0..0, [key(true), key(false)]);
        }
        if reading {
            self.0.append(events);
        } else if !self.0.is_empty() {
            let mut all = std::mem::take(&mut self.0);
            all.append(events);
            *events = all;
        }
    }
}

/// Reads the image of every paste on the page into `clipboard`, and notes the keys held and when
/// the page loses focus. `ctx` repaints once an image has been read.
#[cfg(target_arch = "wasm32")]
pub fn listen(clipboard: &WebClipboard, ctx: &egui::Context) {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;
    let Some(window) = web_sys::window() else { return };
    let (pastes, ctx) = (clipboard.clone(), ctx.clone());
    let on_paste = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |e: web_sys::ClipboardEvent| {
        // The file must be taken now: the event's data is gone once it has been handled.
        let data = e.clipboard_data();
        let file = data.as_ref().and_then(image_file);
        // What eframe forwards as a paste: the text, when there is some.
        let text = data.and_then(|data| data.get_data("text").ok()).is_some_and(|text| !text.is_empty());
        let number = pastes.paste(file.is_some(), text);
        let Some(file) = file else { return };
        let (clipboard, ctx) = (pastes.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            clipboard.read(number, file_bytes(&file, &clipboard, number).await);
            ctx.request_repaint();
        });
    });
    if let Some(document) = window.document()
        && document.add_event_listener_with_callback("paste", on_paste.as_ref().unchecked_ref()).is_ok()
    {
        on_paste.forget();
    }
    for (name, down) in [("keydown", true), ("keyup", false)] {
        let keys = clipboard.clone();
        let on_key = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| keys.key(&e.code(), down));
        // Captured on the window, so the key is seen before anything on the page can stop it.
        if window.add_event_listener_with_callback_and_bool(name, on_key.as_ref().unchecked_ref(), true).is_ok() {
            on_key.forget();
        }
    }
    let blurred = clipboard.clone();
    let on_blur = Closure::<dyn FnMut()>::new(move || blurred.left());
    if window.add_event_listener_with_callback("blur", on_blur.as_ref().unchecked_ref()).is_ok() {
        on_blur.forget();
    }
}

/// The image a paste carries, when it carries one: PNG first, the type browsers give a copied
/// image.
#[cfg(target_arch = "wasm32")]
fn image_file(data: &web_sys::DataTransfer) -> Option<web_sys::File> {
    let items = data.items();
    let images: Vec<web_sys::DataTransferItem> =
        (0..items.length()).filter_map(|i| items.get(i)).filter(|item| item.kind() == "file" && is_raster(&item.type_())).collect();
    let item = images.iter().find(|item| item.type_() == "image/png").or(images.first())?;
    item.get_as_file().ok().flatten()
}

/// The bytes of paste `number`'s `file`, copied into the app's memory only while that paste is
/// still the newest and the copy fits.
#[cfg(target_arch = "wasm32")]
async fn file_bytes(file: &web_sys::File, clipboard: &WebClipboard, number: u64) -> Result<Vec<u8>, String> {
    if let Some(error) = too_large(file.size() as u64) {
        return Err(error);
    }
    let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await.map_err(|e| format!("{e:?}"))?;
    if !clipboard.newest(number) {
        // A newer paste replaces this one's image: `read` drops it unread.
        return Ok(Vec::new());
    }
    let array = js_sys::Uint8Array::new(&buffer);
    let len = array.length() as usize;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).map_err(|_| i18n::t("the file is too large").to_owned())?;
    bytes.resize(len, 0);
    array.copy_to(&mut bytes);
    Ok(bytes)
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
