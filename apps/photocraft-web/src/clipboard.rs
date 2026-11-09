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
//! This build doesn't put PhotoCraft's own copies on the system clipboard, so a copy made in
//! PhotoCraft stays newer than the image there. It stops being newer only once the page has lost
//! focus (another app may copy then) and a later paste shows the clipboard holding something
//! else.

use std::cell::RefCell;
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
    /// A paste event, carrying an image file (`image`) or not. Returns the paste's number for
    /// [`WebClipboard::read`].
    pub fn paste(&self, image: bool) -> u64 {
        let mut s = self.0.borrow_mut();
        s.pastes += 1;
        if image {
            s.reading += 1;
        } else {
            s.show(None);
        }
        s.pastes
    }

    /// The image file of paste `number` has been read: its bytes, or why they couldn't be.
    /// It is decoded only while no newer paste has come.
    pub fn read(&self, number: u64, file: Result<Vec<u8>, String>) {
        let newest = self.0.borrow().pastes == number;
        let shown = newest.then(|| Shown::from_file(file));
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

    /// `Services::clipboard_copied`: PhotoCraft made a copy, which stays in the app, newer than
    /// the image on the system clipboard.
    pub fn copied(&self) {
        let mut s = self.0.borrow_mut();
        let under = s.shown.as_ref().and_then(|shown| shown.file);
        s.copied = Some(Copied { under, left: false });
    }

    /// The page lost focus: another app may copy before it is back.
    pub fn left(&self) {
        if let Some(copied) = &mut self.0.borrow_mut().copied {
            copied.left = true;
        }
    }

    /// `Services::clipboard_get_image`: the image the last paste showed on the system clipboard,
    /// unless PhotoCraft has copied since and its copy is still the newer one.
    pub fn image(&self) -> Result<Option<Pixels>, String> {
        let s = self.0.borrow();
        match &s.shown {
            Some(shown) if s.copied.is_none() => shown.pixels.clone().map(Some),
            _ => Ok(None),
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

/// A clipboard file type that decodes to pixels. SVG is markup, so it isn't one.
pub fn is_raster(mime: &str) -> bool {
    mime.starts_with("image/") && mime != "image/svg+xml"
}

/// Whether `event` waits while a pasted image is read: the keyboard and clipboard events, so the
/// Ctrl+V among them pastes the new image rather than what the clipboard held before.
pub fn waits(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::PasteImage(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Ime(_)
            | egui::Event::ImeResetCompleted { .. }
            | egui::Event::ModifiersChanged(_)
    )
}

/// Keyboard events held back while a pasted image is read ([`waits`]).
#[derive(Default)]
pub struct Held(Vec<egui::Event>);

impl Held {
    /// While `reading`, takes the waiting events out of this frame's `events`; once it is done,
    /// puts every held event back in front of them, in the order they came.
    pub fn hold_or_release(&mut self, reading: bool, events: &mut Vec<egui::Event>) {
        if reading {
            let (wait, rest): (Vec<_>, Vec<_>) = std::mem::take(events).into_iter().partition(waits);
            self.0.extend(wait);
            *events = rest;
        } else if !self.0.is_empty() {
            let mut all = std::mem::take(&mut self.0);
            all.append(events);
            *events = all;
        }
    }
}

/// Reads the image of every paste on the page into `clipboard`, and notes when the page loses
/// focus. `ctx` repaints once an image has been read.
#[cfg(target_arch = "wasm32")]
pub fn listen(clipboard: &WebClipboard, ctx: &egui::Context) {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;
    let Some(window) = web_sys::window() else { return };
    let (pastes, ctx) = (clipboard.clone(), ctx.clone());
    let on_paste = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |e: web_sys::ClipboardEvent| {
        // The file must be taken now: the event's data is gone once it has been handled.
        let file = e.clipboard_data().and_then(|data| image_file(&data));
        let number = pastes.paste(file.is_some());
        let Some(file) = file else { return };
        let (clipboard, ctx) = (pastes.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            clipboard.read(number, file_bytes(&file).await);
            ctx.request_repaint();
        });
    });
    if let Some(document) = window.document()
        && document.add_event_listener_with_callback("paste", on_paste.as_ref().unchecked_ref()).is_ok()
    {
        on_paste.forget();
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

#[cfg(target_arch = "wasm32")]
async fn file_bytes(file: &web_sys::File) -> Result<Vec<u8>, String> {
    let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await.map_err(|e| format!("{e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
