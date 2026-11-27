//! Document-independent recording (#219).
//!
//! The journal is what Actions record, so an entry must mean the same thing on another document.
//! The UI names the layer it clicked by id (`"layer": 7`); ids are per document, so a step kept
//! that way replays only on the document it came from. Before an entry is journaled:
//!
//! - a `layer` that was the active (and only selected) layer when the command ran is dropped:
//!   the step acts on the active layer, as Photoshop's "current layer" does;
//! - any other `layer` id becomes the layer's name when the name is unique in the document, and
//!   `layer.select` always does (selecting is choosing a layer by name, as Photoshop records
//!   it); an ambiguous name keeps the id, which still replays on the recording document;
//! - a mask drop's destination (`layer.layerMask.moveTo`'s `to`) becomes the layer's name, with
//!   `toOccurrence` when several layers share it;
//! - the Type tool's keystrokes fold into the `type.create` that made the layer. On an existing
//!   layer, each replacement stays separate so replay keeps style runs between edited spans.
//!
//! Ids are still accepted everywhere; only what the journal keeps changes.

use std::sync::Arc;

use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::Session;

/// Action recording and playback state ([`Session::recording`]): what the pipeline reads to
/// journal steps into a recording, and the depth and step counters that bound playback, batches
/// and scripts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionRecording {
    /// Journal length when recording started, and the action index being recorded into.
    pub target: Option<(usize, usize)>,
    /// Painting-tool strokes are recorded only when the user enables them for this session.
    pub record_tools: bool,
    /// Bumped by each `actions.record`, so a background job knows whether the recording it was
    /// started in is still the one running when its result lands.
    pub take: u64,
    /// Nesting depth of `actions.play`, bounded by the playback stack limit.
    pub playing: u8,
    /// Active action names, for cycle detection in engine and desktop playback.
    pub playback_stack: Vec<String>,
    /// Number of steps executed by the current top-level playback tree.
    pub playback_steps: u64,
    /// Set for a batch nested in a played action or another batch: the playback counter it carries
    /// is shared by every input, so a played action inside it does not restart the count.
    pub budget_shared: bool,
    /// How many batches (`file.automate.batch`, and droplets through it) are running inside one
    /// another: each runs its steps in a scratch session that carries this on, plus one.
    pub batch_depth: u8,
    /// How many scripts (`file.scripts.browse`) are running inside one another.
    pub script_depth: u8,
    /// Steps run by the current top-level script and everything it calls.
    pub script_steps: u64,
}

/// The active document as it was before a command ran, captured by the dispatcher.
#[derive(Clone, Default)]
pub(crate) struct Before {
    doc: Option<Arc<Document>>,
    active_layer: Option<LayerId>,
    /// Every selected layer: a `layer` that is one of several stays explicit, because without it
    /// a layer command acts on the whole selection ([`crate::layer_multi_cmds::targets`]).
    selected: Vec<LayerId>,
}

impl Before {
    pub(crate) fn capture(s: &Session) -> Self {
        match s.active() {
            Some(st) => Self { doc: Some(st.doc.clone()), active_layer: st.active_layer, selected: st.selected_layers().to_vec() },
            None => Self::default(),
        }
    }

    /// Whether the command left the active document as it was: the same document, unedited.
    pub(crate) fn kept(&self, s: &Session) -> bool {
        matches!((&self.doc, s.active()), (Some(doc), Some(st)) if Arc::ptr_eq(doc, &st.doc))
    }
}

/// Commands whose explicit `layer` names source content rather than a default target.
pub(crate) fn needs_source_layer(id: &str) -> bool {
    matches!(id, "layer.new.frame" | "select.loadSelection" | "type.createSelection" | "edit.checkSpelling")
}

/// `params` with its `layer` id made portable (see the module doc).
pub(crate) fn portable(s: &Session, id: &str, mut params: Value, before: &Before) -> Value {
    if id == "document.activate"
        && let (Some(from), Some(to)) = (before.doc.as_ref().map(|doc| doc.id), s.active().map(|state| state.doc.id))
        && let Some(offset) = s.document_offset_between(from, to)
    {
        return serde_json::json!({"offset": offset});
    }
    let Some(doc) = before.doc.as_deref() else { return params };
    if id == "type.create" && s.recording.target.is_some() && params.get("box").is_none() && doc.size.width > 0 && doc.size.height > 0 {
        let x = params.get("x").and_then(Value::as_f64).unwrap_or(0.0);
        let y = params.get("y").and_then(Value::as_f64).unwrap_or(0.0);
        let x_percent = x / f64::from(doc.size.width) * 100.0;
        let y_percent = y / f64::from(doc.size.height) * 100.0;
        if x_percent.is_finite()
            && y_percent.is_finite()
            && let Some(object) = params.as_object_mut()
        {
            object.remove("x");
            object.remove("y");
            object.insert("xPercent".into(), serde_json::json!(x_percent));
            object.insert("yPercent".into(), serde_json::json!(y_percent));
        }
    }
    if id == "layer.layerMask.moveTo" {
        portable_mask_destination(doc, &mut params);
    }
    let Some(target) = params.get("layer").and_then(Value::as_u64).map(LayerId) else { return params };
    let Some(obj) = params.as_object_mut() else { return params };
    let only_selected = before.selected.len() <= 1;
    if id != "layer.select" && !needs_source_layer(id) && before.active_layer == Some(target) && only_selected {
        obj.remove("layer");
        return params;
    }
    if let Some(layer) = doc.layer(target)
        && !(id == "select.loadSelection" && (layer.name.is_empty() || layer.name == "merged"))
        && doc.walk().iter().filter(|(_, _, l)| l.name == layer.name).count() == 1
    {
        obj.insert("layer".into(), Value::String(layer.name.clone()));
    }
    params
}

/// A mask drop's `to` id becomes the layer's name, and `toOccurrence` which of several layers of
/// that name it is, in document order. Moving a mask leaves the stack as it is, so the count means
/// the same layer when the step replays. It is taken from the document before the step: dropping
/// on the Background renames it "Layer 0".
fn portable_mask_destination(doc: &Document, params: &mut Value) {
    let Some(to) = params.get("to").and_then(Value::as_u64).map(LayerId) else { return };
    let Some(layer) = doc.layer(to) else { return };
    let matches: Vec<_> = doc.walk().into_iter().filter(|(_, _, l)| l.name == layer.name).map(|(_, _, l)| l.id).collect();
    let Some(occurrence) = matches.iter().position(|id| *id == to) else { return };
    let Some(fields) = params.as_object_mut() else { return };
    fields.insert("to".into(), Value::String(layer.name.clone()));
    if matches.len() > 1 {
        fields.insert("toOccurrence".into(), occurrence.into());
    }
}

/// The text layer a journaled `type.create` made or `type.edit` changed, so the keystrokes that
/// follow fold into that entry only when they type into the same layer (the Type tool selects
/// a clicked layer without a command, so the journal alone cannot tell).
pub(crate) fn typed_layer(s: &Session, id: &str, params: &Value) -> Option<(DocId, LayerId)> {
    let doc_id = s.active()?.doc.id;
    match id {
        "type.create" => s.active()?.active_layer.map(|layer| (doc_id, layer)),
        "type.edit" => crate::commands::layer_param(s, params).ok().map(|layer| (doc_id, layer)),
        _ => None,
    }
}

/// What the journal keeps of a Type-tool keystroke.
pub(crate) enum Keystroke {
    /// Not a keystroke: the entry is journaled as it is.
    None,
    /// Folded into the entry before it.
    Folded,
}

/// Fold a Type-tool keystroke into its creation step only. A whole-text step on an existing
/// layer can collapse distinct style runs between separate edited spans on replay (#4514).
/// `previous` is the layer the last entry typed into.
pub(crate) fn fold_keystroke(s: &mut Session, id: &str, params: &Value, previous: Option<(DocId, LayerId)>) -> Keystroke {
    if id != "type.edit" {
        return Keystroke::None;
    }
    let Some(obj) = params.as_object() else { return Keystroke::None };
    if obj.get("coalesce").and_then(Value::as_str).is_none() {
        return Keystroke::None;
    }
    if !obj.contains_key("replace") || !obj.keys().all(|k| matches!(k.as_str(), "layer" | "replace" | "coalesce")) {
        return Keystroke::None;
    }
    let Ok(target) = crate::commands::layer_param(s, params) else { return Keystroke::None };
    // The text as it is now, after this keystroke: the step sets it whole.
    let Some(text) = s.active().and_then(|st| st.doc.layer(target)).and_then(|l| match &l.content {
        LayerContent::Text(t) => Some(t.text.clone()),
        _ => None,
    }) else {
        return Keystroke::None;
    };
    if previous != s.active().map(|st| (st.doc.id, target)) {
        return Keystroke::None;
    }
    // A new recording starts at a journal boundary. Its first edit cannot change an entry
    // before that boundary, or actions.stop would miss the edit altogether.
    if s.recording.target.is_some_and(|(start, _)| s.journal.len() <= start) {
        return Keystroke::None;
    }
    let Some((last, lp)) = s.journal.last_mut() else { return Keystroke::None };
    if last != "type.create" || obj.contains_key("layer") {
        return Keystroke::None;
    }
    let Some(lo) = lp.as_object_mut() else { return Keystroke::None };
    lo.insert("text".into(), Value::String(text));
    Keystroke::Folded
}

/// Replay commands and named action calls, excluding edits to the action definitions and the
/// already-expanded Again wrapper.
pub fn replayable(id: &str) -> bool {
    // Again and Again on a Copy journal their concrete steps (`edit.transform`, after
    // `layer.duplicate` for the copy) and record those: the wrappers look up the last transform in
    // the journal, so they fail where no transform ran yet, e.g. an action played in a new session.
    // A Conditional is recorded with its predicate, not the branch it happened to play (or none),
    // so playback evaluates the condition again on the current document (#7198, #7238).
    matches!(id, "actions.play" | "actions.conditional")
        || (!id.starts_with("actions.")
            // Browse executes its steps through Session::execute, so those concrete
            // commands are already journaled. Recording the wrapper as well replays
            // every script edit twice (#3057).
            && !matches!(id, "edit.transform.again" | "edit.transform.againCopy" | "file.scripts.browse")
            && (shell_view_command(id) || shell_save_command(id) || crate::commands::find(id).is_some_and(|c| c.journal)))
}

/// File › Save, Save As and Save a Copy, and File › Export › Export As, recorded by the desktop
/// shell once the file has a path (#2032, #4373, #7207). The shell replays them; batch and
/// droplets write to their destination folder instead.
pub fn shell_save_command(id: &str) -> bool {
    matches!(id, "file.save" | "file.saveAs" | "file.saveACopy" | "file.export.exportAs")
}

/// View-only menu commands recorded by the desktop shell. Headless playback reports
/// them as unsupported instead of silently dropping them; Batch and droplets skip them.
pub fn shell_view_command(id: &str) -> bool {
    matches!(id, "view.fitOnScreen" | "view.actualPixels" | "view.zoomIn" | "view.zoomOut")
}

/// The point-based painting tools committed by the canvas; fills and gradients remain commands.
pub(crate) fn tool_stroke(id: &str) -> bool {
    matches!(
        id,
        "paint.stroke"
            | "paint.pencil"
            | "paint.mixerBrush"
            | "paint.colorReplacement"
            | "paint.blur"
            | "paint.sharpen"
            | "paint.smudge"
            | "paint.dodge"
            | "paint.burn"
            | "paint.sponge"
            | "paint.cloneStamp"
            | "paint.healingBrush"
            | "paint.historyBrush"
            | "paint.artHistoryBrush"
            | "paint.patternStamp"
            | "paint.backgroundEraser"
            | "paint.spotHealing"
            | "paint.remove"
    )
}

/// Visibility gestures name a layer for replay, since document-local ids change on another image.
pub(crate) fn recorded_params(s: &Session, id: &str, mut p: Value) -> Value {
    if s.recording.target.is_none() || id != "layer.setProps" || p.get("visible").is_none() {
        return p;
    }
    let Some(st) = s.active() else { return p };
    let Some(layer) = p.get("layer").and_then(Value::as_u64).map(LayerId).or(st.active_layer).and_then(|id| st.doc.layer(id)) else { return p };
    let ordinal = st.doc.walk().into_iter().filter(|(_, _, l)| l.name == layer.name).position(|(_, _, l)| l.id == layer.id).unwrap_or(0);
    if let Some(object) = p.as_object_mut() {
        object.remove("layer");
        object.remove("coalesce");
        object.insert("_actionLayer".into(), json!({"name": layer.name, "ordinal": ordinal}));
    }
    p
}

#[cfg(test)]
mod tests;
