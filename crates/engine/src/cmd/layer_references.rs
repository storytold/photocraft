//! Recorded indexes count the full stack; navigation ordinals follow visible panel rows.

use super::{Document, EngineError, Layer, LayerId, Result, Session, Value};

#[derive(Clone, Copy)]
enum Entry<'a> {
    Layer(&'a Layer),
    GroupEnd(LayerId),
}

/// Bottom-to-top recording order, independent of collapsed Layers panel rows.
fn stack(doc: &Document) -> Vec<Entry<'_>> {
    let mut pending: Vec<_> = doc.layers.iter().rev().map(|layer| (layer, false)).collect();
    let mut out = Vec::new();
    while let Some((layer, visited)) = pending.pop() {
        if !visited && let Some(children) = layer.children() {
            out.push(Entry::GroupEnd(layer.id));
            pending.push((layer, true));
            pending.extend(children.iter().rev().map(|child| (child, false)));
        } else {
            out.push(Entry::Layer(layer));
        }
    }
    out
}

/// Photoshop's first name match starts at the bottom, even in a hidden group.
pub(super) fn named_from_bottom(doc: &Document, name: &str) -> Option<LayerId> {
    let entries = stack(doc);
    let layers = || {
        entries.iter().filter_map(|entry| match entry {
            Entry::Layer(layer) => Some(*layer),
            Entry::GroupEnd(_) => None,
        })
    };
    layers()
        .find(|l| l.name == name)
        .or_else(|| {
            let folded = name.to_lowercase();
            layers().find(|l| l.name.to_lowercase() == folded)
        })
        .map(|l| l.id)
}

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}

/// A Background occupies index zero; without it the first stack entry is one.
fn at_index(doc: &Document, index: u64) -> Result<Entry<'_>> {
    let first = u64::from(doc.background().is_none());
    let entry = index.checked_sub(first).and_then(|n| usize::try_from(n).ok()).and_then(|n| stack(doc).get(n).copied());
    entry.ok_or_else(|| bad(format!("no layer at stack index {index}")))
}

/// Resolve one typed selector; a group-end index succeeds without selecting a layer.
pub(crate) fn recorded_layer(s: &Session, selector: &Value) -> Result<Option<LayerId>> {
    let fields = selector.as_object().filter(|fields| fields.len() == 1).ok_or_else(|| bad("layer reference needs one index or ordinal"))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if let Some(index) = fields.get("index") {
        let index = index.as_u64().ok_or_else(|| bad("layer index must be a non-negative integer"))?;
        return Ok(match at_index(&st.doc, index)? {
            Entry::Layer(layer) => Some(layer.id),
            Entry::GroupEnd(_) => None,
        });
    }
    let ordinal = fields.get("ordinal").and_then(Value::as_str).ok_or_else(|| bad("layer reference needs an index or ordinal"))?;
    crate::layer_nav_cmds::ordinal_target(st, ordinal).map(Some)
}

/// Convert a pre-move stack boundary to an existing layer.moveTo drop position.
pub(crate) fn recorded_drop(doc: &Document, index: u64) -> Result<(LayerId, &'static str)> {
    if index == 0 {
        if doc.background().is_some() {
            return Err(bad("cannot move a layer below the Background"));
        }
        return doc.layers.first().map(|layer| (layer.id, "below")).ok_or_else(|| bad("the document has no layers"));
    }
    match at_index(doc, index)? {
        Entry::GroupEnd(id) => Ok((id, "below")),
        Entry::Layer(layer) => {
            let position = if layer.is_group() { "into" } else { "below" };
            Ok((layer.id, position))
        }
    }
}
