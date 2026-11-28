//! Confirms copying a layer mask onto an adjustment layer that already has one.

use std::sync::{Arc, Weak};

use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use super::alerts;
use crate::PhotocraftApp;

const COMMAND: &str = "layer.layerMask.moveTo";

struct MaskReplacement {
    // Reopening a saved document can reuse its ID and revision. Keep the identity without retaining its pixels.
    document: Weak<Document>,
    revision: u64,
    from: LayerId,
    to: LayerId,
}

pub(crate) fn can_replace(layer: &Layer) -> bool {
    layer.mask.is_some() && matches!(layer.content, LayerContent::Adjustment(_))
}

pub(crate) fn ask(app: &mut PhotocraftApp, from: LayerId, to: LayerId) {
    let Some(state) = app.session.active() else { return };
    let Some(target) = state.doc.layer(to).filter(|layer| can_replace(layer)) else { return };
    if from == to || !state.doc.layer(from).is_some_and(|layer| layer.mask.is_some()) {
        return;
    }
    let message = crate::i18n::fmt(tl!("Replace the layer mask on “{layer}”?"), &[("layer", target.name.as_str())]);
    let prompt = StandardPrompt::custom(message)
        .title(tl!("Replace Layer Mask"))
        .button(PromptChoice::Proceed, tl!("Replace"))
        .button(PromptChoice::Cancel, tl!("Cancel"))
        .tag(MaskReplacement { document: Arc::downgrade(&state.doc), revision: state.revision, from, to });
    let _ = alerts::open_prompt(app, prompt);
}

pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(replacement) = answer.take_tag::<MaskReplacement>() else { return false };
    if answer.choice == PromptChoice::Proceed {
        let result = confirm(app, replacement);
        alerts::report(app, result);
    }
    true
}

fn confirm(app: &mut PhotocraftApp, replacement: MaskReplacement) -> Result<Value, String> {
    let same_document =
        app.session.active().is_some_and(|state| state.revision == replacement.revision && replacement.document.ptr_eq(&Arc::downgrade(&state.doc)));
    if !same_document {
        return Err(tl!("The document changed. Drag the mask again to replace it.").into());
    }
    app.run_command(COMMAND, json!({"layer": replacement.from.0, "to": replacement.to.0, "copy": true, "replace": true}))
}
