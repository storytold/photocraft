//! File › Import › Video Frames to Layers, recorded as `importVideoToLayers`, as `docs/atn.md`
//! ("Recorded with source files") measures it (#10723): `Usng` = the video (a path),
//! `makeAnimation` (bool: Make Frame Animation) and `frameSkip` (long: Limit To Every n Frames,
//! absent when off).
//!
//! PhotoCraft has no frame animation and imports every frame, so only a step with Make Frame
//! Animation off and no frame limit plays here; any other stays a kept step.

use super::descriptors::{desc, path_text, path_value};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};

/// The native command the step plays as.
pub(super) const COMMAND: &str = "file.import.videoFramesToLayers";

/// The descriptor the step holds for `path`: the measured keys, in the measured order.
fn descriptor(path: &str) -> Descriptor {
    desc("null").with("Usng", path_value(path)).with("makeAnimation", D::Boolean(false))
}

/// Map the measured form with Make Frame Animation off and no frame limit: `Usng` a path that
/// decodes, then `makeAnimation` false, and nothing else. Anything else stays a complete kept step.
pub(super) fn from(d: &Descriptor) -> Mapped {
    match d.get("Usng").and_then(path_text) {
        Some(path) if d.items == descriptor(&path).items => step(COMMAND, json!({"path": path})),
        _ => Mapped::Unsupported { harmless: false },
    }
}

/// Write a native step that names its file. A step with any other setting is refused rather
/// than written without it.
pub(super) fn to(_: &str, p: &Value) -> Option<Descriptor> {
    let m = p.as_object()?;
    let path = m.get("path")?.as_str()?;
    (m.len() == 1 && !path.is_empty() && !path.contains('\0')).then(|| descriptor(path))
}
