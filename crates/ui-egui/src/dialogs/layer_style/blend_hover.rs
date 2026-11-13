//! Layer Style: hovering a mode in an open Blend Mode list previews it on the canvas (#5945), as
//! the Layers panel's list does (#970). The hovered mode is laid over a copy of the fields the
//! canvas renders, like the Color Picker's live colour: the dialog's pending fields, the document
//! and its history stay as they are until a mode is chosen. Leaving the list, closing it, or a list
//! no longer drawn (another page, Preview off, the dialog closed) shows the pending style again.
//! A mode chosen with the wheel (#1747) is a pending edit like a click, never a hover.

use std::borrow::Cow;

use serde_json::{Map, Value, json};

use super::{BLENDING, defaults, entry, set_param};
use crate::PhotocraftApp;

/// A mode hovered in a Blend Mode list of one page of a Layer Style dialog.
pub(crate) struct BlendHover {
    /// The dialog, and its page (an effect instance id or Blending Options) the list is on.
    dialog: u64,
    page: String,
    /// The parameter the list edits.
    key: &'static str,
    mode: String,
    /// The frame the list was last drawn in: a list no longer drawn previews nothing.
    frame: u64,
}

/// Dialog `dialog`'s body, each frame it draws page `page`: the mode under the pointer in one of
/// the page's open Blend Mode lists (with the parameter it edits), if any.
pub(crate) fn hover(app: &mut PhotocraftApp, ctx: &egui::Context, dialog: u64, page: &str, hovered: Option<(&'static str, String)>) {
    let frame = app.frame;
    let old = app.style_blend_hover.as_ref().filter(|h| h.dialog == dialog);
    if hovered.is_none() && app.style_blend_hover.is_some() && old.is_none() {
        return; // another dialog's hover
    }
    let changed = match (old, &hovered) {
        (Some(h), Some((key, mode))) => h.page != page || h.key != *key || h.mode != *mode,
        (old, hovered) => old.is_some() != hovered.is_some(),
    };
    // The canvas is drawn before the dialogs: show the change even if the pointer then rests.
    if changed {
        ctx.request_repaint();
    }
    app.style_blend_hover = hovered.map(|(key, mode)| BlendHover { dialog, page: page.into(), key, mode, frame });
}

/// Dialog `dialog`'s `fields` as the canvas renders them: with the hovered mode, while its list
/// is drawn on the page shown and the mode differs from the page's pending one.
pub(crate) fn preview_fields<'a>(app: &PhotocraftApp, dialog: u64, fields: Cow<'a, Map<String, Value>>) -> Cow<'a, Map<String, Value>> {
    let Some(h) = app.style_blend_hover.as_ref().filter(|h| h.dialog == dialog && h.frame.saturating_add(1) >= app.frame) else { return fields };
    if fields.get("selected").and_then(Value::as_str) != Some(h.page.as_str()) {
        return fields;
    }
    let pending = if h.page == BLENDING {
        fields.get("p:blendingOptions").filter(|p| p.is_object()).map(|p| p.get(h.key).cloned().unwrap_or_else(|| json!("Normal")))
    } else {
        // A switched-off effect draws nothing in any mode.
        entry(&fields, &h.page).filter(|e| e.get("on").and_then(Value::as_bool).unwrap_or(true)).and_then(|e| {
            let kind = e.get("kind").and_then(Value::as_str)?;
            let params = e.get("params").filter(|p| p.is_object())?;
            Some(params.get(h.key).cloned().or_else(|| defaults(kind).get(h.key).cloned()).unwrap_or_else(|| json!("Normal")))
        })
    };
    if pending.is_none_or(|p| p.as_str() == Some(h.mode.as_str())) {
        return fields;
    }
    let mut shown = fields.into_owned();
    set_param(&mut shown, &h.page, h.key, json!(h.mode));
    Cow::Owned(shown)
}

#[cfg(test)]
mod tests;
