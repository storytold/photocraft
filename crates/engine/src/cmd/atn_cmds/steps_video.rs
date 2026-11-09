//! The measured Video Layers events (#10723): option-free edits of the active video layer, Show /
//! Hide Altered Video, and the two footage events whose one key is the video file.
use super::{
    AtnItem, Descriptor, Value,
    descriptors::{desc, jstr, path_text, path_value},
    fields::{Mapped, step},
    json,
};

const NO_DESCRIPTOR: &str = "_atnVideoFrameNoDescriptor";
/// The one native command behind both `showAlteredVideo` and `hideAlteredVideo`.
pub(super) const SHOW_ALTERED: &str = "layer.videoLayers.showAlteredVideo";

/// These events take no settings, so a recording with no descriptor keeps that on save.
fn keyless_command(id: &str) -> bool {
    matches!(
        id,
        "layer.videoLayers.insertBlankFrame"
            | "layer.videoLayers.duplicateFrame"
            | "layer.videoLayers.deleteFrame"
            | "layer.videoLayers.restoreFrame"
            | "layer.videoLayers.restoreAllFrames"
            | "layer.videoLayers.reloadFrame"
            | SHOW_ALTERED
    )
}

/// Only the measured empty descriptor is executable; extra settings remain kept.
pub(super) fn from(d: &Descriptor, id: &str) -> Mapped {
    if *d == Descriptor::default() { step(id, json!({})) } else { Mapped::Unsupported { harmless: false } }
}

/// Import's absent-descriptor placeholder must not become a descriptor on save.
pub(super) fn remember(item: &AtnItem, steps: &mut [(String, Value)]) {
    if item.descriptor.is_none() {
        for (id, params) in steps {
            if keyless_command(id)
                && let Some(params) = params.as_object_mut()
            {
                params.insert(NO_DESCRIPTOR.into(), Value::Bool(true));
            }
        }
    }
}

/// Refuse native options and malformed transport data that have no wire field.
pub(super) fn to(p: &Value) -> Option<Descriptor> {
    p.as_object()?.iter().all(|(key, value)| key == NO_DESCRIPTOR && value == &Value::Bool(true)).then(Descriptor::default)
}

/// Called only after `to` has validated the complete native parameter object.
pub(super) fn omitted(id: &str, p: &Value) -> bool {
    keyless_command(id) && p.get(NO_DESCRIPTOR) == Some(&Value::Bool(true))
}

/// `showAlteredVideo` / `hideAlteredVideo`: no settings; the event says which state it sets.
pub(super) fn altered_from(d: &Descriptor, show: bool) -> Mapped {
    match from(d, SHOW_ALTERED) {
        Mapped::Steps(_) => step(SHOW_ALTERED, json!({"show": show})),
        kept => kept,
    }
}

/// Only a step that sets the state saves: the menu item's toggle depends on the layer it plays
/// on, which neither event can say.
pub(super) fn altered_to(p: &Value, show: bool) -> Option<Descriptor> {
    let mut rest = p.as_object()?.clone();
    (rest.remove("show")? == Value::Bool(show)).then_some(())?;
    to(&Value::Object(rest))
}

/// `addVideoLayer` / `replaceFootage`: `null` is the video file. Only the `Pth ` text form that
/// saves back unchanged maps; alias bytes, another header and anything beside the path keep the
/// step as recorded.
pub(super) fn footage_from(d: &Descriptor, id: &str) -> Mapped {
    let path = d.get("null").and_then(path_text).filter(|path| footage_to(&json!({"path": path})).as_ref() == Some(d));
    path.map_or(Mapped::Unsupported { harmless: false }, |path| step(id, json!({"path": path})))
}

/// The path only: a frame rate given to New Video Layer from File has no key in the event.
pub(super) fn footage_to(p: &Value) -> Option<Descriptor> {
    p.as_object()?.keys().all(|key| key == "path").then_some(())?;
    Some(desc("null").with("null", path_value(jstr(p, "path").filter(|s| !s.is_empty())?)))
}
