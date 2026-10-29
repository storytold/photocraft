//! Network printing on craft-print's `wsd` and `ipp` backends. Cargo feature `network-print`; not
//! built for wasm32.
//!
//! The protocols, the print ticket and the job classification are craft-print's; this module is
//! what the app adds on top, shared by the Print dialog and tests:
//!
//! * [`station::Station`]: one printer the person chose, behind a private [`Registry`] that holds
//!   both network backends; the printer is added to the one its address names. It asks what the
//!   printer is (read-only) and makes the tracked print (`craft_print::PrintJob`).
//! * **Formats and conversion are the registry's.** A document in a format the printer lists is
//!   sent as it is. One it does not list goes through the app's [`DocumentConverter`]; with no way
//!   there the print is [`ErrorKind::UnsupportedFormat`] before a job exists.
//! * **No retry of an unknown outcome.** `PrintJob` refuses to resend what may have printed.

use std::sync::Arc;

pub use craft_print::ipp;
pub use craft_print::wsd;
pub use craft_print::{
    AttemptError, CancelToken, ColourMode, Conversion, ConversionPolicy, ConvertContext, Document, DocumentConverter, DocumentFormat, Duplex, ErrorKind, Job,
    JobHint, JobId, JobKind, JobOptions, JobState, JobView, LifecycleState, Orientation, Outcome as PrintOutcome, OwnedDocument, Phase, PrintError, PrintJob,
    PrintJobConfig, PrinterDetails, Progress, Quality, Reconciled as TrackedReconciled, Registry, Resolution, RetryAdvice, RetrySafety, SimpleState, Snapshot,
    StaleReason, UserConfirmed, pwg,
};

mod converter;
pub mod station;

pub use converter::{RenderFn, RenderSpec, RenderedPages};

/// A transport shared with worker threads: the plain-HTTP one of craft-wsd, or an in-memory fake.
pub type Transport = Arc<dyn wsd::adapter::Transport + Send + Sync>;

/// What the person is told when a send failed and the printer may nevertheless have the document.
pub const CHECK_THE_PRINTER: &str = "the request may have reached the printer: check the printer before sending again";
