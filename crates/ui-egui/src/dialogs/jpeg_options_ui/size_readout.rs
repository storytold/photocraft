//! JPEG Options' file size: while choosing a quality, the hosted dialog shows how big the JPEG
//! will be, and the answer follows Quality and Format Options. The parked save's document is
//! encoded in memory with the settings OK would write it with (the same encoder and
//! [`ExportSettings`] as the save, so the size is the file's, metadata and profile included).
//!
//! The encode runs on a worker thread in the native app, so the dialog never waits for it; the web
//! build has no threads and encodes in the frame, so its limit is lower. One encode runs at a time,
//! and the last few answers are kept for their settings: a pass that changes nothing encodes
//! nothing, and going back to settings already worked out answers at once. The dialog shows the
//! newest answered size while a new one is worked out. Documents over [`MAX_PX`] aren't encoded.

use std::collections::VecDeque;

use std::sync::{Arc, Mutex, PoisonError};

use photocraft_codecs::JpegFormat;
use photocraft_doc::{DocId, Document};

use crate::{ExportSettings, PhotocraftApp};

/// The largest document, in pixels, whose size is worked out. Flattening and encoding 40 MP takes
/// a second or two on a worker and about 160 MB at 8 bits, as Export As's exact size does (#6595).
#[cfg(not(target_arch = "wasm32"))]
const MAX_PX: u64 = 40_000_000;
/// The web build encodes in the frame: up to 4 MP keeps a settings change under a frame or so.
#[cfg(target_arch = "wasm32")]
const MAX_PX: u64 = 4_000_000;

/// Whether a document of `size` gets a size readout (see [`MAX_PX`]).
pub(super) fn estimable(size: photocraft_doc::Size) -> bool {
    size.area() <= MAX_PX
}

/// The bytes `doc` takes written as a JPEG with `settings`, as the save writes it; `None` when it
/// can't be encoded.
pub(super) fn encoded_size(doc: &Document, settings: &ExportSettings) -> Option<u64> {
    let bytes = photocraft_io::export(doc, "size.jpg", &settings.io_options()).ok()?.bytes;
    u64::try_from(bytes.len()).ok()
}

/// What one size is for: the document as it is, and the encoder quality (1–100) and format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key {
    doc: DocId,
    revision: u64,
    quality: u8,
    format: JpegFormat,
}

/// How many worked-out sizes are kept, so stepping between a few qualities and formats doesn't
/// encode again.
const KEPT: usize = 8;

/// Where a worker leaves its answer: `Some` once done (`Some(None)`: couldn't be encoded).
type Slot = Arc<Mutex<Option<Option<u64>>>>;

/// The readout between passes (kept in the parked save).
#[derive(Default)]
pub(super) struct Readout {
    /// The last [`KEPT`] sizes worked out and what each is for, newest last.
    known: VecDeque<(Key, Option<u64>)>,
    /// The encode running, and what it is for.
    running: Option<(Key, Slot)>,
}

impl Readout {
    /// The size for `key`: `Some(size)` once worked out (`Some(None)`: it couldn't be encoded),
    /// `None` while the encode runs. Unless `key`'s size is known or being worked out, starts the
    /// job `make` returns, which calls `done` when it finishes; a change made while another encode
    /// runs starts once that one is done.
    pub(super) fn update<J>(&mut self, key: Key, make: impl FnOnce() -> J, done: impl FnOnce() + Send + 'static) -> Option<Option<u64>>
    where
        J: FnOnce() -> Option<u64> + Send + 'static,
    {
        self.collect();
        if self.running.is_none() && self.size_of(key).is_none() {
            let slot = Slot::default();
            run(make(), Arc::clone(&slot), done);
            self.running = Some((key, slot));
            // The web build encodes in the frame: its answer is already there.
            self.collect();
        }
        self.size_of(key)
    }

    /// `key`'s size if it has been worked out (`Some(None)`: it couldn't be).
    fn size_of(&self, key: Key) -> Option<Option<u64>> {
        self.known.iter().find(|(k, _)| *k == key).map(|&(_, size)| size)
    }

    /// Takes a finished encode's answer.
    fn collect(&mut self) {
        let Some((key, slot)) = &self.running else { return };
        let Some(size) = slot.lock().unwrap_or_else(PoisonError::into_inner).take() else { return };
        let key = *key;
        self.running = None;
        self.known.retain(|(k, _)| *k != key);
        if self.known.len() == KEPT {
            self.known.pop_front();
        }
        self.known.push_back((key, size));
    }
}

/// Runs `job` on a worker (in the web build, right here), leaves its answer in `slot`, then calls
/// `done`. A job that panics answers `None`.
fn run(job: impl FnOnce() -> Option<u64> + Send + 'static, slot: Slot, done: impl FnOnce() + Send + 'static) {
    let answer = Arc::clone(&slot);
    let work = move || {
        let size = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or(None);
        *answer.lock().unwrap_or_else(PoisonError::into_inner) = Some(size);
        done();
    };
    #[cfg(not(target_arch = "wasm32"))]
    if std::thread::Builder::new().name("jpeg-options-size".into()).spawn(work).is_err() {
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(None);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = slot;
        work();
    }
}

/// What the readout encodes: the waiting save's document, with the settings its save starts from.
pub(super) struct Source {
    doc: Arc<Document>,
    revision: u64,
    base: ExportSettings,
    own_quality: Option<u8>,
}

impl Source {
    /// For the parked JPEG save, if any, as its document is now; `None` when the document is
    /// gone.
    pub(super) fn of(app: &mut PhotocraftApp) -> Option<Self> {
        // Out of the app while its settings are read, which activates the document for a moment.
        let save = app.parked_save.take()?;
        let source = Self::from_save(app, &save);
        app.parked_save = Some(save);
        source
    }

    fn from_save(app: &mut PhotocraftApp, save: &crate::format_prompts::ParkedSave) -> Option<Self> {
        let crate::format_prompts::ParkedFormat::Jpeg(parked) = &save.format else { return None };
        let st = app.session.documents().iter().find(|st| st.doc.id == save.doc)?;
        let (doc, revision) = (Arc::clone(&st.doc), st.revision);
        Some(Source { doc, revision, base: save.settings(app), own_quality: parked.own_quality })
    }

    /// Whether the document is small enough to encode (see [`MAX_PX`]).
    pub(super) fn estimable(&self) -> bool {
        estimable(self.doc.size)
    }

    /// What the size for `quality` (0–12) and `format` is keyed by.
    pub(super) fn key(&self, quality: u8, format: JpegFormat) -> Key {
        Key { doc: self.doc.id, revision: self.revision, quality: self.encoder_quality(quality), format }
    }

    /// The encode of the size for `quality` (0–12) and `format`, to run on a worker.
    pub(super) fn job(&self, quality: u8, format: JpegFormat) -> impl FnOnce() -> Option<u64> + Send + 'static {
        let (doc, settings) = (Arc::clone(&self.doc), self.settings(quality, format));
        move || encoded_size(&doc, &settings)
    }

    /// The encoder quality (1–100) OK writes for `quality` (0–12): the opened file's own when left
    /// on its step, as `confirm` keeps it.
    fn encoder_quality(&self, quality: u8) -> u8 {
        let own = self.own_quality.filter(|&q| super::quality_from_percent(u32::from(q)) == quality);
        own.unwrap_or_else(|| photocraft_engine::file_cmds::jpeg_quality(f64::from(quality)))
    }

    /// The settings OK writes with for `quality` (0–12) and `format`.
    fn settings(&self, quality: u8, format: JpegFormat) -> ExportSettings {
        ExportSettings { jpeg_quality: Some(self.encoder_quality(quality)), jpeg_format: format, ..self.base.clone() }
    }
}

#[cfg(test)]
mod tests;
