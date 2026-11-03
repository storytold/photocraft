//! Background replacement beneath an existing cutout; the subject's pixels and mask stay intact.

use photocraft_color::Color;
use photocraft_doc::{Fill, GradientStyle, Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::{EngineError, Result, Session};

pub const COMMAND: &str = "layer.backgroundFill";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: msg.into() }
}

pub(super) fn subject(s: &Session, id: LayerId) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    let layer = st.doc.layer(id).ok_or("no subject layer")?;
    if !matches!(layer.content, LayerContent::Raster(_) | LayerContent::Smart(_)) {
        return Err("choose a pixel or Smart Object subject layer".into());
    }
    if !layer.mask.as_ref().is_some_and(|m| m.enabled) && !layer.vector_mask.as_ref().is_some_and(|m| m.enabled) {
        return Err("remove the background or add a subject mask first".into());
    }
    if layer.clipped || st.doc.effective_locks(id).all {
        return Err("the subject must be unlocked and outside a clipping mask".into());
    }
    Ok(())
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let id = s.active().and_then(|st| st.active_layer).ok_or("no subject layer selected")?;
    subject(s, id)
}

/// Whether an existing image layer can be moved behind this subject without breaking a clipping group.
pub fn image_source(s: &Session, target: LayerId, source: LayerId) -> Result<()> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if source == target {
        return Err(bad("the subject cannot also be the background"));
    }
    let layer = st.doc.layer(source).ok_or(EngineError::NoLayer(source))?;
    if !matches!(layer.content, LayerContent::Raster(_) | LayerContent::Smart(_)) {
        return Err(bad("sourceLayer must be a pixel or Smart Object layer"));
    }
    let locks = st.doc.effective_locks(source);
    if locks.all
        || locks.position
        || layer.clipped
        || !crate::clip_group::clipped_above(&st.doc, source).is_empty()
        || crate::background_cmds::background_id(&st.doc) == Some(source)
    {
        return Err(bad("unlock the background image and release its clipping mask before moving it"));
    }
    Ok(())
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    if !p.is_object() || p.get("layer").is_some_and(|v| v.as_u64().is_none()) {
        return Err(bad("params must be an object and layer an unsigned id"));
    }
    let id = layer_param(s, p)?;
    subject(s, id).map_err(EngineError::Other)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if p.get("document").is_some_and(|v| v.as_u64() != Some(st.doc.id.0)) {
        return Err(bad("the target document is no longer active"));
    }
    let kind = match p.get("kind") {
        None => "solid",
        Some(Value::String(kind)) => kind.as_str(),
        _ => return Err(bad("kind must be solid, gradient or image")),
    };
    let fg = s.tools.foreground;
    let bg = s.tools.background;
    let original = Color::rgba(fg[0], fg[1], fg[2], fg[3]);
    let color = |key: &str, fallback: Color| {
        crate::solid_fill_cmds::color_param(p.get(key).unwrap_or(&Value::Null), fallback, st.doc.mode)
            .map(|color| color.in_mode(st.doc.mode))
            .map_err(|e| bad(e.to_string()))
    };
    let (content, source) = match kind {
        "solid" => {
            let c = if p.get("color").is_some() { color("color", original)? } else { original.in_mode(st.doc.mode) };
            (Some(LayerContent::Fill(Fill::Solid(c))), None)
        }
        "gradient" => {
            let from = if p.get("color").is_some() { color("color", original)? } else { original.in_mode(st.doc.mode) };
            let end = Color::rgba(bg[0], bg[1], bg[2], bg[3]);
            let to = if p.get("to").is_some() { color("to", end)? } else { end.in_mode(st.doc.mode) };
            let angle = crate::gradient_fill_cmds::angle_param(COMMAND, p, 90.0)?;
            (Some(LayerContent::Fill(Fill::gradient(vec![(0.0, from), (1.0, to)], angle, 1.0, GradientStyle::Linear, false))), None)
        }
        "image" => {
            let source = LayerId(p.get("sourceLayer").and_then(Value::as_u64).ok_or_else(|| bad("choose an image sourceLayer"))?);
            image_source(s, id, source)?;
            (None, Some(source))
        }
        _ => return Err(bad("kind must be solid, gradient or image")),
    };
    let layer = s.edit("Background Fill", move |doc, _| {
        let mut layer = match (content, source) {
            (Some(content), _) => Layer::new(doc.next_layer_name("Background Fill"), content),
            (_, Some(source)) => doc.remove(source).ok_or(EngineError::NoLayer(source))?,
            _ => return Err(bad("missing background content")),
        };
        layer.visible = true;
        // Direct insertion avoids inheriting a neighbour's clipping state.
        Ok(doc.insert_below(Some(id), layer))
    })?;
    Ok(json!({"layer": layer.0, "subject": id.0}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "Background Fill…",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":subject id?,"document":id?,"kind":"solid|gradient|image"="solid","color":"#rrggbb"|[r,g,b,a?]|Color? (foreground),"to":color? (background; gradient),"angle":degrees=90,"sourceLayer":id? (image: move this existing layer below the subject, retaining its transform)} → {layer,subject}. Requires an enabled subject mask. Creates an unmasked full-canvas fill or moves an image layer; preserves the subject, selection and active layer. One undo step."##,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;
