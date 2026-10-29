//! A PNG import in resumable steps, for a caller that must not block for the whole file (the web
//! build has no threads): the same document [`crate::import_with_options_and_interrupt`] gives.

use std::sync::Arc;

use photocraft_codecs::{self as codecs, DecodeOptions, Format, Image, Limits, PngRows, SharedBytes};

use crate::{ImportResult, IoError, Route};

enum Stage {
    Rows(Box<PngRows<SharedBytes>>),
    Decoded(Box<Image>),
    Spent,
}

/// A plain PNG file being imported as a document, a few rows at a time.
pub struct PngImport {
    name: String,
    bytes: Arc<Vec<u8>>,
    stage: Stage,
}

impl PngImport {
    /// Starts importing `bytes` (named `name`) when the import would decode them as a plain PNG,
    /// which reads the header and refuses what the import refuses; `None` for any other file,
    /// which only [`crate::import_with_options_and_interrupt`] reads. The decoder shares `bytes`.
    pub fn new(name: &str, bytes: &Arc<Vec<u8>>) -> Result<Option<Self>, IoError> {
        if crate::route(name, bytes) != Route::Flat || codecs::detect(bytes) != Some(Format::Png) {
            return Ok(None);
        }
        let rows = PngRows::new(SharedBytes(Arc::clone(bytes)), &Limits::default())?;
        Ok(Some(Self { name: name.to_string(), bytes: Arc::clone(bytes), stage: Stage::Rows(Box::new(rows)) }))
    }

    /// How much of the decoding is done, `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        match &self.stage {
            Stage::Rows(rows) => rows.progress(),
            Stage::Decoded(_) | Stage::Spent => 1.0,
        }
    }

    /// Decodes rows while `more()` returns true (at least one per call); then, in a step of its
    /// own, turns the image into a document. `Some` once that is done, after which the import is
    /// spent.
    pub fn step(&mut self, more: &mut dyn FnMut() -> bool) -> Result<Option<ImportResult>, IoError> {
        match std::mem::replace(&mut self.stage, Stage::Spent) {
            Stage::Rows(mut rows) => {
                self.stage = match rows.step(more)? {
                    Some(img) => Stage::Decoded(Box::new(img)),
                    None => Stage::Rows(rows),
                };
                Ok(None)
            }
            Stage::Decoded(img) => {
                let img = codecs::finish_png_decode(*img, &DecodeOptions::default())?;
                crate::flat::flat_document(&self.name, &self.bytes, Some(Format::Png), img).map(Some)
            }
            Stage::Spent => Err(IoError::Unsupported("the PNG was already imported".into())),
        }
    }
}
