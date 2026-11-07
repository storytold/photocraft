//! The seam between PhotoCraft and a learned subject model, reached through the Craft suite's
//! shared AI layer ([craft-ai-integrations](https://github.com/storytold/craft-ai-integrations)).
//!
//! The engine and the UI build for wasm and never open a file or a socket for this, so they only
//! know [`SubjectModel`], which the desktop app implements over the shared client
//! (`apps/photocraft/src/subject_ai.rs`). Without an implementation (the web build, the CLI, tests)
//! Select Subject and Remove Background keep their built-in method.
//!
//! The model runs on this computer: nothing here sends a picture anywhere.

use std::sync::Arc;

/// A picture for the model: 8-bit RGB, row-major, `width * height * 3` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// What a person needs to know before the model is downloaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOffer {
    /// The model's name, such as "InSPyReNet".
    pub model: String,
    /// How much a download takes at most (files already on the computer are not fetched again).
    pub download_bytes: u64,
    /// Where the files go, shown as it is. Empty when this computer gives no place for them.
    pub folder: String,
    /// The terms the download agrees to, shown as they are.
    pub terms: String,
}

/// Whether the model can run now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelStatus {
    /// It can. `model` is its name.
    Ready { model: String },
    /// It can after one download.
    Offer(ModelOffer),
    /// It can't, and a download would not help. `reason` is a sentence for a person.
    Unavailable { reason: String },
}

/// Why [`SubjectModel::matte`] or [`SubjectModel::install`] ended without an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// The person stopped it. Not an error.
    Cancelled,
    /// It failed. The message is safe to show.
    Failed(String),
}

/// A subject model the app can run. Every method blocks, so `matte` and `install` run on a job's
/// worker; `status` only reads settings, so it can be asked before a command starts.
pub trait SubjectModel: Send + Sync {
    /// Whether the model can run now, read again on each call: a person can change their choice
    /// in another Craft app while this one is open.
    fn status(&self) -> ModelStatus;

    /// The subject's coverage over `image`: one byte per pixel, `width * height` bytes, 255 where
    /// the subject is. `cancelled` is asked while it runs.
    fn matte(&self, image: &SubjectImage, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>, ModelError>;

    /// Downloads and checks the model's files and makes them the suite's model for this tool. Only
    /// called after the person agreed to [`ModelOffer::terms`]. `progress` gets a fraction (0–1)
    /// and a message.
    fn install(&self, progress: &dyn Fn(f32, &str), cancelled: &dyn Fn() -> bool) -> Result<(), ModelError>;
}

/// The app's subject model, shared with job workers.
pub type SharedSubjectModel = Arc<dyn SubjectModel>;
