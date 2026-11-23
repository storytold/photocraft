//! A mask dragged onto the Layers panel's Trash asks first, as in Photoshop. A layer mask (#3113):
//! "Apply mask to layer before removing?" Apply runs `layer.layerMask.apply`, Delete
//! `layer.layerMask.delete`, Cancel keeps the mask. A vector mask (#3119): "Delete the vector
//! mask?" OK runs `layer.vectorMask.delete`, Cancel keeps it. A click on the Trash still deletes the
//! targeted mask at once (#2197).
//!
//! The prompt is a `StandardPrompt` on the dialog host: `ui.inspect` shows it, `ui.dialog.confirm`
//! applies (or deletes the vector mask), `ui.dialog.set` with `"choice": "delete"` then
//! `ui.dialog.confirm` deletes a layer mask, and `ui.dialog.cancel` keeps the mask.

use craft::ui::dialog::{PromptAnswer, PromptChoice, PromptKind, StandardPrompt};
use photocraft_doc::LayerId;
use serde_json::{Value, json};

use super::alerts;
use crate::PhotocraftApp;
use crate::mask_thumbs_ui::MaskKind;

/// Marks the params of a [`drop_action`], which [`intercept`] answers with the prompt.
const MARK: &str = "__maskTrash";
const LAYER_MASK_DELETE: &str = "layer.layerMask.delete";
const LAYER_MASK_APPLY: &str = "layer.layerMask.apply";
const VECTOR_MASK_DELETE: &str = "layer.vectorMask.delete";

/// What the prompt asked about: the mask of `layer` in document `doc`, a vector mask when `vector`.
pub(crate) struct MaskTrash {
    doc: u64,
    layer: u64,
    vector: bool,
}

/// The action a mask thumbnail dropped on the Trash pushes; [`intercept`] turns it into the prompt.
pub fn drop_action(layer: u64, kind: MaskKind) -> (String, Value) {
    let command = if kind == MaskKind::Vector { VECTOR_MASK_DELETE } else { LAYER_MASK_DELETE };
    (command.into(), json!({"layer": layer, MARK: true}))
}

/// Opens the prompt for a [`drop_action`]; `None` for every other command.
pub fn intercept(app: &mut PhotocraftApp, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let vector = match command {
        LAYER_MASK_DELETE => false,
        VECTOR_MASK_DELETE => true,
        _ => return None,
    };
    if params.get(MARK).and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let layer = params.get("layer").and_then(Value::as_u64)?;
    let state = app.session.active()?;
    state.doc.layer(LayerId(layer)).filter(|l| if vector { l.vector_mask.is_some() } else { l.mask.is_some() })?;
    let tag = MaskTrash { doc: state.doc.id.0, layer, vector };
    let prompt = if vector {
        StandardPrompt::new(PromptKind::TrashVectorMask)
    } else {
        // Apply needs a pixel layer (Layer › Layer Mask › Apply is greyed out otherwise).
        StandardPrompt::trash_mask().can_apply(app.session.is_enabled_with(LAYER_MASK_APPLY, &json!({"layer": layer})))
    };
    Some(Ok(match alerts::open_prompt(app, prompt.tag(tag)) {
        Ok(id) => json!({"dialog": id.value(), "confirmationPending": true}),
        Err(_) => json!({"confirmationPending": true}),
    }))
}

/// The `PROMPTS` entry: Proceed applies a layer mask (or deletes a vector mask), Alternate deletes a
/// layer mask, Cancel keeps it. `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(tag) = answer.take_tag::<MaskTrash>() else { return false };
    let command = match (tag.vector, answer.choice) {
        (true, PromptChoice::Proceed) => VECTOR_MASK_DELETE,
        (false, PromptChoice::Proceed) => LAYER_MASK_APPLY,
        (false, PromptChoice::Alternate) => LAYER_MASK_DELETE,
        _ => return true,
    };
    let result = confirm(app, &tag, command);
    alerts::report(app, result);
    true
}

/// Runs `command` on the mask when the document, the layer and its mask are still the ones asked about.
fn confirm(app: &mut PhotocraftApp, tag: &MaskTrash, command: &str) -> Result<Value, String> {
    let state = app.session.active().ok_or(tl!("The document is closed"))?;
    let has_mask = |l: &photocraft_doc::Layer| if tag.vector { l.vector_mask.is_some() } else { l.mask.is_some() };
    if state.doc.id.0 != tag.doc || !state.doc.layer(LayerId(tag.layer)).is_some_and(has_mask) {
        return Err(tl!("The layer or its mask changed. Drag the mask onto the Trash again.").into());
    }
    app.run_command(command, json!({"layer": tag.layer}))
}
