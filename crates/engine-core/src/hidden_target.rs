//! Photoshop refuses to paint on or move a hidden layer (#571): the painting and retouching
//! tools, the Paint Bucket, the Gradient and the Move tool stop with "the target layer is hidden"
//! instead of changing pixels nobody can see. A layer inside a hidden group is hidden too. An
//! alpha channel or Quick Mask target has no layer, so it is never refused.

use photocraft_doc::{Document, LayerId};
use serde_json::Value;

use crate::Session;

/// Photoshop's alert text.
pub const HIDDEN: &str = "Could not complete your request because the target layer is hidden.";

/// The Art History Brush's own alert text (#12638).
pub const ART_HISTORY_HIDDEN: &str = "Could not use the art history brush because the target layer is hidden.";

/// Commands that paint on, or move, the target layer.
const GATED: &[&str] = &[
    "paint.stroke",
    "paint.pencil",
    "paint.mixerBrush",
    "paint.colorReplacement",
    "paint.bucket",
    "paint.gradient",
    "paint.magicEraser",
    "paint.backgroundEraser",
    "paint.cloneStamp",
    "paint.healingBrush",
    "paint.spotHealing",
    "paint.patch",
    "paint.dodge",
    "paint.burn",
    "paint.sponge",
    "paint.blur",
    "paint.sharpen",
    "paint.smudge",
    "paint.historyBrush",
    "paint.artHistoryBrush",
    "layer.translate",
];

/// Is the layer shown: it and every group around it visible.
fn shown(doc: &Document, id: LayerId) -> bool {
    doc.path_of(id).is_some_and(|path| (1..=path.len()).all(|n| path.get(..n).and_then(|p| doc.layer_at(p)).is_some_and(|l| l.visible)))
}

/// Why command `id` with `params` must not run: its target layer is hidden. `None` for other
/// commands, channel targets, and visible layers.
pub fn refusal(s: &Session, id: &str, params: &Value) -> Option<&'static str> {
    if !GATED.contains(&id) {
        return None;
    }
    let params = crate::channel_cmds::inject_target(s, id, params.clone());
    if crate::channel_cmds::is_channel_target(&params) {
        return None;
    }
    let st = s.active()?;
    let layer = crate::commands::layer_param(s, &params).ok()?;
    // The Art History Brush's text was measured on a layer's pixels only.
    let pixels = crate::channel_cmds::target_of(&params) == crate::channel_cmds::Target::Pixels;
    let why = if id == "paint.artHistoryBrush" && pixels { ART_HISTORY_HIDDEN } else { HIDDEN };
    (st.doc.layer(layer).is_some() && !shown(&st.doc, layer)).then_some(why)
}
