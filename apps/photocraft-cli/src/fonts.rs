//! `--font-dir` (#3393): fonts loaded before anything opens, in the process-wide text engine, so
//! PSD import's font matching and every session of the process see them.

use std::io::Write;

use crate::args::FontArgs;
use crate::{CliError, Result};

/// Registers each `--font-dir` (a font folder or file) in the process-wide text engine. Called
/// before anything opens: PSD import resolves font names then, and every session of the process
/// (each file of a batch, a server's session) shares the fonts. Files that fail are warnings;
/// a missing path or a folder without fonts is an error.
pub(crate) fn load(fonts: &FontArgs, err: &mut dyn Write) -> Result {
    for path in &fonts.font_dirs {
        let registered = photocraft_engine::font_cmds::register_font_path(path).map_err(|e| match e {
            photocraft_engine::EngineError::BadParams { msg, .. } => CliError::Other(format!("--font-dir: {msg}")),
            other => CliError::Engine(other),
        })?;
        for (file, why) in &registered.skipped {
            let _ = writeln!(err, "warning: --font-dir {}: skipped {file}: {why}", path.display());
        }
    }
    Ok(())
}
