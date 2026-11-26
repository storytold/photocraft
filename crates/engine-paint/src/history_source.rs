//! The History Brush's, the Art History Brush's and Fill › History's source document, checked
//! against the document they paint on.

use std::sync::Arc;

use photocraft_color::ColorMode;
use photocraft_doc::Document;
use serde_json::Value;

use crate::{DocState, EngineError, Result, history_source_cmds};

/// The document the History Brush and Fill › History paint from: the `state` (History panel row)
/// or `snapshot` passed, else the source set in the History panel. A source of another canvas
/// size is refused rather than painting shifted pixels (after Crop, Canvas Size or Image Size):
/// Photoshop refuses too, "the current canvas size does not match that of the history state"
/// (its message as users report it; Adobe's help doesn't document the case).
pub fn paint_source(st: &DocState, p: &Value, cmd: &str) -> Result<Arc<Document>> {
    checked_source(st, p, cmd)?.map_err(|why| {
        EngineError::Other(match why {
            SourceRefusal::NoSource => "the History Brush source was deleted: set a new source in the History panel".into(),
            SourceRefusal::CanvasSize { source } => format!(
                "the current canvas size ({}×{}) does not match that of the History Brush source ({}×{}): set a source of this size in the History panel",
                st.doc.size.width, st.doc.size.height, source.width, source.height
            ),
        })
    })
}

/// Why [`checked_source`] found no source to paint from, so each tool can word it (#12638).
#[derive(Debug)]
pub enum SourceRefusal {
    /// No `state` or `snapshot` passed, and the History panel's source was deleted.
    NoSource,
    /// The source has another canvas size than the document.
    CanvasSize { source: photocraft_geom::Size },
}

/// [`paint_source`] with the refusal left to the caller: bad params and a `state` or `snapshot`
/// that is gone are errors, the two refusals the tools word themselves are the inner `Err`.
pub fn checked_source(st: &DocState, p: &Value, cmd: &str) -> Result<std::result::Result<Arc<Document>, SourceRefusal>> {
    let doc = match history_source_cmds::parse(st, p, cmd)? {
        Some(src) => history_source_cmds::source_of(st, src).ok_or_else(|| EngineError::Other("that history state or snapshot is gone".into()))?,
        None => match history_source_cmds::source(st).and_then(|src| history_source_cmds::source_of(st, src)) {
            Some(doc) => doc,
            None => return Ok(Err(SourceRefusal::NoSource)),
        },
    };
    if doc.size != st.doc.size {
        return Ok(Err(SourceRefusal::CanvasSize { source: doc.size }));
    }
    Ok(Ok(doc))
}

/// The History Brush's refusals after Image › Mode, with Photoshop's alert texts (#12842): only
/// the History Brush's pairs were measured (RGB to Grayscale, a depth change), so any other change,
/// or both at once, converts the source as before. Document modes are compared: the pixel format
/// reads Indexed and Multichannel as RGB.
pub fn refuse_other_format(st: &DocState, source: &Document) -> Result<()> {
    let (mode_differs, depth_differs) = (source.mode != st.doc.mode, source.depth != st.doc.depth);
    if source.mode == ColorMode::Rgb && st.doc.mode == ColorMode::Grayscale && !depth_differs {
        return Err(EngineError::Other("Could not use the history brush because the current color mode does not match that of the history state.".into()));
    }
    if depth_differs && !mode_differs {
        return Err(EngineError::Other("Could not use the history brush because the current bit depth does not match that of the history state.".into()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "history_brush_format_mismatch_tests.rs"]
mod history_brush_format_mismatch_tests;
