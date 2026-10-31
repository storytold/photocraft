//! G'MIC filters for PhotoCraft, through a user-installed `gmic` command-line program.
//!
//! [G'MIC](https://gmic.eu) is a separate program (CeCILL licensed) that PhotoCraft neither ships
//! nor links: this crate finds the `gmic` binary ([`runner::locate`]), hands it the layer's pixels
//! in its own uncompressed `.cimg` format ([`cimg`]) and reads the result back, so nothing of
//! G'MIC enters this process. The filter catalogue G'MIC-Qt shows (`#@gui` definitions) is read
//! from the program's own `parse_gui json` output ([`catalog`]) and turned into dialog parameter
//! specs and command lines.
//!
//! Everything is data in, data out: malformed files, catalogue JSON, parameters, a missing binary,
//! a crash or a hang of the child process are all [`Error`]s, never panics. Spawning processes is
//! native only; on `wasm32` [`runner`] returns [`Error::Unsupported`].
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod apply;
pub mod catalog;
pub mod cimg;
pub mod runner;

pub use apply::{apply, check_format};
pub use catalog::{Catalog, Filter, Param, ParamKind};
pub use cimg::Image;
pub use runner::{Gmic, RunOptions};

/// A G'MIC error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// No usable `gmic` program was found.
    #[error("G'MIC is not installed or was not found: {0}")]
    NotFound(String),
    /// Not available on this platform (the web build cannot start programs).
    #[error("{0}")]
    Unsupported(String),
    /// `gmic` ran and reported an error (its message), or exited abnormally.
    #[error("G'MIC: {0}")]
    Failed(String),
    /// The run took longer than its time limit and was stopped.
    #[error("G'MIC took longer than {0} s and was stopped")]
    Timeout(u64),
    /// The run was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// A malformed `.cimg` file.
    #[error("bad .cimg data: {0}")]
    Cimg(String),
    /// The filter catalogue is missing or malformed.
    #[error("G'MIC filter list: {0}")]
    Catalog(String),
    /// Bad filter parameters.
    #[error("G'MIC parameters: {0}")]
    Params(String),
    /// The image or the filter's output can't be used.
    #[error("{0}")]
    Image(String),
    /// A file-system or process error.
    #[error("{0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;
