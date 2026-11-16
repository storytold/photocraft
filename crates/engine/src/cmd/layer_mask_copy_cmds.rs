//! Copy and paste a layer's pixel mask from the mask-thumbnail menu.

use std::borrow::Cow;

use photocraft_doc::{LayerId, LayerMask};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param, mask_at_depth};
use crate::layer_multi_cmds::targets;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn require_layer_id(cmd: &str, p: &Value) -> Result<()> {
    match p.get("layer") {
        None | Some(Value::Null) => Ok(()),
        Some(v) if v.as_u64().is_some() => Ok(()),
        Some(_) => Err(bad(cmd, "`layer` must be a layer id")),
    }
}

/// `{"layer": null}` is an omitted layer. [`targets`] treats a present key as one named layer,
/// which would paste only the active layer of a multi-selection.
fn omit_null_layer(p: &Value) -> Cow<'_, Value> {
    if p.get("layer").is_some_and(Value::is_null) {
        let mut q = p.clone();
        if let Some(obj) = q.as_object_mut() {
            obj.remove("layer");
        }
        Cow::Owned(q)
    } else {
        Cow::Borrowed(p)
    }
}

/// Layers Paste Layer Mask writes: the named layer, or every selected layer that is not fully locked.
fn paste_ids(s: &Session, p: &Value) -> Result<Vec<LayerId>> {
    let p = omit_null_layer(p);
    let ids = targets(s, &p)?;
    if ids.len() < 2 {
        return Ok(ids);
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let ids: Vec<LayerId> = ids.into_iter().filter(|id| d.doc.layer(*id).is_some_and(|_| !d.doc.effective_locks(*id).all)).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("none of the selected layers can take a layer mask".into()));
    }
    Ok(ids)
}

fn assign(doc: &mut photocraft_doc::Document, id: LayerId, mask: &LayerMask) -> Result<()> {
    crate::extra_cmds::background_to_layer_for_mask(doc, id);
    crate::cmd::enabled::check_all_unlocked(doc, id)?;
    let mask = mask_at_depth(mask.clone(), doc.depth)?;
    doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.mask = Some(mask);
    Ok(())
}

fn can_copy(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if l.mask.is_some() { Ok(()) } else { Err("the layer has no layer mask".into()) }
}

fn can_paste(s: &Session) -> std::result::Result<(), String> {
    if s.layer_mask_clipboard.is_none() {
        return Err("no layer mask has been copied".into());
    }
    let d = s.active().ok_or("no document open")?;
    let sel = d.selected_layers();
    let ids: Vec<LayerId> = if sel.len() > 1 { sel } else { vec![d.active_layer.ok_or("no active layer")?] };
    if ids.iter().any(|id| d.doc.layer(*id).is_some_and(|_| !d.doc.effective_locks(*id).all)) {
        Ok(())
    } else {
        Err("the layer can't take a layer mask".into())
    }
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.layerMask.copy";
    require_layer_id(CMD, p)?;
    let id = layer_param(s, p)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let mask = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?.mask.clone().ok_or_else(|| EngineError::Other("layer has no layer mask".into()))?;
    s.layer_mask_clipboard = Some(mask);
    Ok(json!({ "layer": id.0 }))
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.layerMask.paste";
    require_layer_id(CMD, p)?;
    let mask = s.layer_mask_clipboard.clone().ok_or_else(|| EngineError::Other("no layer mask has been copied".into()))?;
    let ids = paste_ids(s, p)?;
    s.edit("Paste Layer Mask", |doc, _| {
        for id in &ids {
            assign(doc, *id, &mask)?;
        }
        Ok(())
    })?;
    let layers: Vec<u64> = ids.iter().map(|id| id.0).collect();
    Ok(json!({ "layers": layers }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "layer.layerMask.copy",
            label: "Copy Layer Mask",
            menu: &["Layer", "Layer Mask"],
            shortcut: None,
            params: r##"{"layer":id?} → {layer} (copies that layer's layer mask — pixels, density, feather, link and enablement — for Paste Layer Mask)"##,
            enabled: can_copy,
            run: copy,
            journal: true,
        },
        CommandSpec {
            id: "layer.layerMask.paste",
            label: "Paste Layer Mask",
            menu: &["Layer", "Layer Mask"],
            shortcut: None,
            params: r##"{"layer":id?} → {layers:[id,…]} (no layer: every selected layer that is not fully locked; replaces a layer mask already there; one history step)"##,
            enabled: can_paste,
            run: paste,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white", "depth": 8})).unwrap();
        s
    }

    #[test]
    fn copying_a_layer_mask_does_not_edit_and_paste_puts_it_on_another_layer() {
        let mut s = session();
        assert!(s.execute("layer.layerMask.copy", json!({})).is_err());
        s.execute("layer.new.layer", json!({"name": "Src"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 8})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        s.execute("layer.layerMask.edit", json!({"density": 50, "feather": 2})).unwrap();
        let src = s.active().unwrap().active_layer.unwrap();
        let mask = s.active().unwrap().doc.layer(src).unwrap().mask.clone().unwrap();
        let steps = s.active().unwrap().history.past_len();
        assert!(s.execute("layer.layerMask.copy", json!({"layer": "nope"})).is_err());
        s.execute("layer.layerMask.copy", json!({})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        assert_eq!(s.layer_mask_clipboard.as_ref(), Some(&mask));
        assert!(s.is_enabled("edit.purge.clipboard"));

        s.execute("layer.new.layer", json!({"name": "Bare"})).unwrap();
        let bare = s.active().unwrap().active_layer.unwrap();
        assert!(s.is_enabled("layer.layerMask.paste"));
        let before = s.active().unwrap().history.past_len();
        s.execute("layer.layerMask.paste", json!({})).unwrap();
        let pasted = s.active().unwrap().doc.layer(bare).unwrap().mask.clone().unwrap();
        assert_eq!(pasted.density, mask.density);
        assert_eq!(pasted.feather, mask.feather);
        assert!(pasted.surface.sample_channel(1, 1, 0) > 0.99);
        assert!(pasted.surface.sample_channel(6, 1, 0) < 0.01);
        assert!(s.active().unwrap().doc.layer(src).unwrap().mask.is_some());
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        s.undo();
        assert!(s.active().unwrap().doc.layer(bare).unwrap().mask.is_none());

        s.execute("edit.purge.clipboard", json!({})).unwrap();
        assert!(s.layer_mask_clipboard.is_none());
        assert!(!s.is_enabled("layer.layerMask.paste"));
    }

    /// A mask on `Src`, copied. Density 50 and feather 2 distinguish it from a later mask.
    fn copied_mask(s: &mut Session) -> LayerMask {
        s.execute("layer.new.layer", json!({"name": "Src"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 8})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        s.execute("layer.layerMask.edit", json!({"density": 50, "feather": 2})).unwrap();
        let src = s.active().unwrap().active_layer.unwrap();
        let mask = s.active().unwrap().doc.layer(src).unwrap().mask.clone().unwrap();
        s.execute("layer.layerMask.copy", json!({})).unwrap();
        mask
    }

    #[test]
    fn a_null_layer_pastes_onto_every_selected_layer_in_one_step() {
        let mut s = session();
        let copied = copied_mask(&mut s);
        s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
        let a = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
        let b = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
        let before = s.active().unwrap().history.past_len();
        let got = s.execute("layer.layerMask.paste", json!({"layer": null})).unwrap();
        let mut ids: Vec<u64> = got["layers"].as_array().unwrap().iter().filter_map(|v| v.as_u64()).collect();
        ids.sort_unstable();
        let mut want = vec![a.0, b.0];
        want.sort_unstable();
        assert_eq!(ids, want);
        for id in [a, b] {
            let pasted = s.active().unwrap().doc.layer(id).unwrap().mask.clone().unwrap();
            assert_eq!(pasted.density, copied.density);
            assert_eq!(pasted.feather, copied.feather);
        }
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        s.undo();
        assert!(s.active().unwrap().doc.layer(a).unwrap().mask.is_none());
        assert!(s.active().unwrap().doc.layer(b).unwrap().mask.is_none());
    }

    #[test]
    fn pasting_over_a_mask_restores_it_on_undo() {
        let mut s = session();
        let copied = copied_mask(&mut s);
        s.execute("layer.new.layer", json!({"name": "Held"})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("layer.layerMask.edit", json!({"density": 10, "feather": 7})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        let original = s.active().unwrap().doc.layer(id).unwrap().mask.clone().unwrap();
        assert_ne!(original.density, copied.density);
        let before = s.active().unwrap().history.past_len();
        s.execute("layer.layerMask.paste", json!({})).unwrap();
        let replaced = s.active().unwrap().doc.layer(id).unwrap().mask.clone().unwrap();
        assert_eq!(replaced.density, copied.density);
        assert_eq!(replaced.feather, copied.feather);
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        s.undo();
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().mask.as_ref(), Some(&original));
    }

    #[test]
    fn pasting_onto_a_smart_object_sets_its_layer_mask_and_leaves_the_filter_mask() {
        let mut s = session();
        let copied = copied_mask(&mut s);
        s.execute("layer.selectBottom", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        let id = LayerId(s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap());
        {
            let layer = s.active().unwrap().doc.layer(id).unwrap();
            let photocraft_doc::LayerContent::Smart(sm) = &layer.content else { panic!("expected a smart object") };
            assert!(layer.mask.is_none());
            assert!(sm.filter_mask.is_none());
        }
        s.execute("layer.layerMask.paste", json!({"layer": id.0})).unwrap();
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        let pasted = layer.mask.clone().unwrap();
        assert_eq!(pasted.density, copied.density);
        assert_eq!(pasted.feather, copied.feather);
        let photocraft_doc::LayerContent::Smart(sm) = &layer.content else { panic!("paste left the smart object") };
        assert!(sm.filter_mask.is_none());
    }
}
