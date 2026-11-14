//! Artboards to Files' Export Options (#13636): the per-file-type settings beyond the format.
//! Each one is optional; without any, an export writes what it wrote before.
//!
//! - `"iccProfile": bool`: embed the document's ICC profile (PSD: its ICC resource). Unset, as
//!   before: flat files embed it, a PSD only when the document has a profile of its own.
//! - `"bmpBits": 16 | 24 | 32`: BMP depth. 16 is refused: the encoder writes 24 and 32 only.
//! - `"pngInterlaced": bool`: an Adam7-interlaced PNG.
//! - `"transparency": bool`: PNG; `false` composites each board over white, so the file has no alpha.
//! - `"preserveArtboard": bool`: PSD; `false` writes the board as a plain group.
//! - `"tiffCompression": "none" | "lzw" | "zip"`: as Save As; `"jpeg"` is refused, the TIFF
//!   encoder has no JPEG compression yet.
//! - `"quality": 0..12` (JPEG), `"maximizeCompatibility": bool` (PSD) and `"tgaBits": 24 | 32`
//!   (Targa) as for every save ([`SaveOpts::from_params`]).

use photocraft_color::Color;
use photocraft_doc::{Document, LayerContent};
use serde_json::Value;

use crate::file_cmds::SaveOpts;
use crate::{EngineError, Result, Session};

/// What the options change in each board's document before it is written.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct BoardOptions {
    /// PNG Transparency; `Some(false)` puts a white Background under the board.
    transparency: Option<bool>,
    /// PSD Preserve Artboard; `Some(false)` turns the board into a plain group.
    preserve_artboard: Option<bool>,
    /// Include ICC Profile; `Some(false)` leaves a PSD's ICC resource out, `Some(true)` writes
    /// one even for a document without a profile of its own.
    icc_profile: Option<bool>,
}

fn bad(cmd: &str, msg: String) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg }
}

/// `key` in `p` as a bool; absent or null: `None`. Anything else is a bad parameter of `cmd`.
fn bool_param(p: &Value, key: &str, cmd: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(v) => Err(bad(cmd, format!("`{key}` must be true or false, got {v}"))),
    }
}

/// The save options and the per-board changes `p` asks for.
pub(super) fn parse(s: &Session, p: &Value, cmd: &str) -> Result<(SaveOpts, BoardOptions)> {
    if p.get("tiffCompression").and_then(Value::as_str) == Some("jpeg") {
        return Err(bad(cmd, "JPEG compression in a TIFF is not supported yet; use \"none\", \"lzw\" or \"zip\"".into()));
    }
    match p.get("quality") {
        None | Some(Value::Null) => {}
        Some(v) if v.as_f64().is_some() => {}
        Some(v) => return Err(bad(cmd, format!("`quality` must be a number from 0 to 12, got {v}"))),
    }
    let mut save = SaveOpts::from_params(s, p, cmd)?;
    save.bmp_bits = match p.get("bmpBits") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_u64().and_then(photocraft_io::BmpBits::from_bits) {
            Some(photocraft_io::BmpBits::Bits16) => return Err(bad(cmd, "16-bit BMP is not supported yet; use 24 or 32 bits".into())),
            Some(bits) => Some(bits),
            None => return Err(bad(cmd, format!("`bmpBits` must be 16, 24 or 32, got {v}"))),
        },
    };
    let icc_profile = bool_param(p, "iccProfile", cmd)?;
    save.embed_icc = icc_profile;
    save.png_interlaced = bool_param(p, "pngInterlaced", cmd)?;
    let board = BoardOptions { transparency: bool_param(p, "transparency", cmd)?, preserve_artboard: bool_param(p, "preserveArtboard", cmd)?, icc_profile };
    Ok((save, board))
}

impl BoardOptions {
    /// Applies the options of `format` (lower case, no dot) to one board's document.
    pub(super) fn apply(&self, one: &mut Document, format: &str) {
        if format == "png" && self.transparency == Some(false) {
            // A white Background in the document's own mode and depth: the composite is opaque,
            // so the file is written without alpha.
            let bg = Document::with_background("Background", one.size, one.mode, one.depth, Color::WHITE);
            one.layers.splice(0..0, bg.layers);
        }
        if matches!(format, "psd" | "psb") {
            if self.preserve_artboard == Some(false) {
                for layer in &mut one.layers {
                    if let LayerContent::Group(g) = &mut layer.content {
                        g.artboard = None;
                    }
                }
            }
            match self.icc_profile {
                Some(false) => one.icc_profile = None,
                // A document without a profile of its own gets the one its colours are taken to
                // be in, as a flat file embeds it.
                Some(true) if one.icc_profile.is_none() => one.icc_profile = Some(crate::color_cmds::working_profile(one.mode).to_bytes()),
                _ => {}
            }
        }
    }
}
