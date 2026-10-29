use super::{
    Adjustment, BlendMode, Color, CommandSpec, EngineError, Fill, Layer, LayerContent, LayerId, LayerMask, Map, Result, Session, Value, adjustment_kind,
    arrange, bad, blend_from_str, color_param, has_doc, has_layer, has_selection, json, layer_copy, layer_panel_option, layer_param, mask_new_layer,
    merge_group, pixels, set_clipped, set_mask, set_mask_and_deselect,
};

/// Layer menu commands, in registry order.
pub(super) fn specs() -> Vec<CommandSpec> {
    [creation(), duplication(), deletion_and_selection(), order_and_clipping(), masks(), merging(), fill_layers(), adjustments(), placement()]
        .into_iter()
        .flatten()
        .collect()
}

/// New layers and groups, and grouping the selected layers.
fn creation() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.new.layer",
            "Layer…",
            ["Layer", "New"],
            Some("Cmd+Shift+N"),
            r##"{"name":str?,"clipToPrevious":bool=false (Use Previous Layer to Create Clipping Mask; inside a clip group the layer joins it anyway),"color":"none|red|orange|yellow|green|seafoam|blue|indigo|magenta|fuchsia|violet|gray"="none","mode":"Normal|Multiply|…"="Normal","opacity":0..1=1,"fillNeutral":bool=false (fill with the mode's neutral color: modes without one are an error),"position":"above|below"="above" (below: the Layers panel's Cmd/Ctrl-click; never under the Background)} (above the active layer, or the top layer inside it when it is an open group)"##,
            has_doc,
            crate::new_layer::run
        ),
        cmd!(
            "layer.new.group",
            "Group…",
            ["Layer", "New"],
            None,
            r##"{"name":text?,"color":"none|red|orange|yellow|green|seafoam|blue|indigo|magenta|fuchsia|violet|gray"="none","mode":str="passThrough" (blend mode),"opacity":0..100=100 (percent),"position":"above|below"="above" (below: the Layers panel's Cmd/Ctrl-click; never under the Background)} (above the active layer, or inside it when it is an open group; clipped when it lands inside a clipping mask)"##,
            has_doc,
            |s, p| {
                let opts = crate::group_options::GroupOptions::parse("layer.new.group", p)?;
                let below = crate::group_options::below("layer.new.group", p)?;
                let place = if below { crate::layer_place::Place::Below } else { crate::layer_place::Place::Auto };
                let id = s.edit("New Group", |doc, active| {
                    let name = opts.name.clone().unwrap_or_else(|| doc.next_layer_name("Group"));
                    let mut group = Layer::group(name, vec![]);
                    opts.apply(&mut group);
                    let bg = crate::background_cmds::background_id(doc);
                    let id = crate::layer_place::insert_new(doc, *active, group, place);
                    crate::background_cmds::keep_background_at_bottom(bg, doc)?;
                    *active = Some(id);
                    Ok(id)
                })?;
                Ok(json!({ "layer": id.0 }))
            }
        ),
        cmd!(
            "layer.groupLayers",
            "Group Layers",
            ["Layer"],
            Some("Cmd+G"),
            r##"{"layer":id?,"name":text?,"color":"none|red|orange|yellow|green|seafoam|blue|indigo|magenta|fuchsia|violet|gray"="none","mode":str="passThrough" (blend mode),"opacity":0..100=100 (percent)} (no layer: every selected layer; the new group starts collapsed)"##,
            has_layer,
            crate::layer_multi_cmds::group_layers
        ),
    ]
}

/// Duplicate Layer, into the same document or a new one.
fn duplication() -> Vec<CommandSpec> {
    vec![cmd!(
        "layer.duplicate",
        "Duplicate Layer…",
        ["Layer"],
        None,
        r##"{"layer":id?,"inPlace":bool=false (artboards: keep the copy on the original instead of beside it),"name":str? (one layer: the copy's name),"document":index|"new"=same document (Photoshop's Destination: "new" makes a new document the size of this canvas holding just the copies),"documentName":str? (the new document's name)} (no layer: every selected layer)"##,
        has_layer,
        |s, p| {
            if let Some(d) = p.get("document") {
                if d.as_str() == Some("new") {
                    return crate::layer_copy_cmds::duplicate_to_new_document(s, p);
                }
                let same = d.as_u64().is_some_and(|i| s.active_index().is_some_and(|a| i == a as u64));
                if !same {
                    return Err(EngineError::BadParams {
                        cmd: "layer.duplicate".into(),
                        msg: if d.as_u64().is_some() {
                            "the destination is another open document (use layer.copyToDocument)".into()
                        } else {
                            "`document` must be a document index or \"new\"".into()
                        },
                    });
                }
            }
            let in_place = p.get("inPlace").and_then(Value::as_bool).unwrap_or(false);
            let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
            if crate::layer_multi_cmds::multi(s, p) {
                return crate::layer_multi_cmds::duplicate_selected(s, in_place, "Duplicate Layers");
            }
            let id = layer_param(s, p)?;
            let add_copy = layer_panel_option(s, "addCopyToCopiedLayersAndGroups", true);
            let nid = s.edit("Duplicate Layer", |doc, active| {
                let mut dup = layer_copy(doc, id, add_copy)?;
                if let Some(name) = &name {
                    dup.name.clone_from(name);
                }
                let nid = doc.insert_above(Some(id), dup);
                if !in_place {
                    crate::artboard_cmds::place_copy(doc, nid)?;
                }
                *active = Some(nid);
                Ok(nid)
            })?;
            Ok(json!({ "layer": nid.0 }))
        }
    )]
}

/// Deleting and selecting layers, and setting their properties.
fn deletion_and_selection() -> Vec<CommandSpec> {
    vec![
        cmd!("layer.delete", "Delete Layer", ["Layer", "Delete"], None, r##"{"layer":id?} (no layer: every selected layer)"##, has_layer, |s, p| {
            if crate::layer_multi_cmds::multi(s, p) {
                return crate::layer_multi_cmds::delete_selected(s);
            }
            let id = layer_param(s, p)?;
            s.edit("Delete Layer", |doc, active| {
                let neighbours = crate::layer_multi_cmds::deletion_neighbours(doc, id);
                let before = crate::clip_group::Before::read(doc, &[id]);
                doc.remove(id).ok_or(EngineError::NoLayer(id))?;
                crate::clip_group::settle(doc, &[], &before, false);
                if doc.layers.is_empty() {
                    return Err(EngineError::Other("a document must keep at least one layer".into()));
                }
                if active.is_some_and(|current| doc.layer(current).is_none()) {
                    *active = neighbours.into_iter().find(|candidate| doc.layer(*candidate).is_some());
                }
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!(
            "layer.select",
            "Select Layer",
            [],
            None,
            r##"{"layer":id|str | "name":str,"occurrence":index?,"mode":"replace|toggle|range|add|remove"="replace"} (name occurrence: zero-based document walk order; required for duplicate names; toggle = ⌘-click, range = ⇧-click, remove = deselect it, keeping at least one)"##,
            has_doc,
            crate::layer_multi_cmds::select
        ),
        cmd!(
            "layer.setProps",
            "Layer Properties",
            [],
            None,
            r##"{"layer":id?,"layers":[id,…]? (the same change to each, one step: the Layers panel's controls with several layers selected),"name":str? (one layer only),"visible":bool?,"opacity":0..1?,"fill":0..1?,"blend":"Multiply|…"?,"clipped":bool?,"locked":bool?,"locks":{"transparency","pixels","position","artboard","all":bool}?,"channels":[bool,…]? (Advanced Blending: which colour channels blend, R G B / C M Y K / L a b)}"##,
            has_layer,
            set_props
        ),
    ]
}

/// Arranging layers and clipping masks.
fn order_and_clipping() -> Vec<CommandSpec> {
    vec![
        cmd!("layer.arrange.bringForward", "Bring Forward", ["Layer", "Arrange"], Some("Cmd+]"), r##"{"layer":id?}"##, has_layer, |s, p| arrange(s, p, 1)),
        cmd!("layer.arrange.sendBackward", "Send Backward", ["Layer", "Arrange"], Some("Cmd+["), r##"{"layer":id?}"##, has_layer, |s, p| arrange(s, p, -1)),
        cmd!("layer.arrange.bringToFront", "Bring to Front", ["Layer", "Arrange"], Some("Cmd+Shift+]"), r##"{"layer":id?}"##, has_layer, |s, p| arrange(
            s,
            p,
            i32::MAX
        )),
        cmd!("layer.arrange.sendToBack", "Send to Back", ["Layer", "Arrange"], Some("Cmd+Shift+["), r##"{"layer":id?}"##, has_layer, |s, p| arrange(
            s,
            p,
            i32::MIN
        )),
        cmd!(
            "layer.createClippingMask",
            "Create Clipping Mask",
            ["Layer"],
            Some("Cmd+Alt+G"),
            r##"{"layer":id?} (no layer: every selected layer but the lowest clips to the layer below it)"##,
            has_layer,
            |s, p| set_clipped(s, p, true)
        ),
        cmd!(
            "layer.releaseClippingMask",
            "Release Clipping Mask",
            ["Layer"],
            None,
            r##"{"layer":id?} (no layer: every selected layer)"##,
            has_layer,
            |s, p| set_clipped(s, p, false)
        ),
    ]
}

/// Layer mask commands.
fn masks() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.layerMask.revealAll",
            "Reveal All",
            ["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?} (no layer: every selected layer that can take a mask)"##,
            has_layer,
            |s, p| set_mask(s, p, "Add Layer Mask", Some(LayerMask::reveal_all()))
        ),
        cmd!(
            "layer.layerMask.hideAll",
            "Hide All",
            ["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?} (no layer: every selected layer that can take a mask)"##,
            has_layer,
            |s, p| set_mask(s, p, "Add Layer Mask", Some(LayerMask::hide_all()))
        ),
        cmd!(
            "layer.layerMask.revealSelection",
            "Reveal Selection",
            ["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?} (no layer: every selected layer that can take a mask)"##,
            has_selection,
            |s, p| {
                let sel = s.active().and_then(|d| d.doc.selection.clone()).ok_or(EngineError::Other("no selection".into()))?;
                let mask = LayerMask { surface: sel, ..LayerMask::reveal_all() };
                // The selection becomes the mask, so Photoshop drops it (as Hide Selection does): a
                // following Invert (⌘I) on the targeted mask then inverts all of it, not just the
                // part inside the old selection. Same undo step as the mask.
                set_mask_and_deselect(s, p, "Add Layer Mask", mask)
            }
        ),
        cmd!("layer.layerMask.delete", "Delete", ["Layer", "Layer Mask"], None, r##"{"layer":id?}"##, has_layer, |s, p| set_mask(
            s,
            p,
            "Delete Layer Mask",
            None
        )),
    ]
}

/// Merge Down, Merge Group and Flatten Image.
fn merging() -> Vec<CommandSpec> {
    vec![
        cmd!("layer.mergeDown", "Merge Down", ["Layer"], None, r##"{"layer":id?}"##, has_layer, |s, p| {
            let id = layer_param(s, p)?;
            s.edit("Merge Down", |doc, active| {
                let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
                let Some((&idx, parent)) = path.split_last() else { return Err(EngineError::NoLayer(id)) };
                if idx == 0 {
                    return Err(EngineError::Other("no layer below to merge into".into()));
                }
                let mut below = parent.to_vec();
                below.push(idx - 1);
                let lower = doc.layer_at(&below).ok_or(EngineError::NoLayer(id))?.clone();
                let upper = doc.layer(id).ok_or(EngineError::NoLayer(id))?.clone();
                let merged = pixels::merge_down(doc, &lower, &upper);
                doc.remove(id);
                *doc.layer_at_mut(&below).ok_or(EngineError::NoLayer(lower.id))? = merged;
                *active = Some(lower.id);
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!("layer.mergeGroup", "Merge Group", ["Layer"], None, r##"{"layer":id?}"##, has_layer, |s, p| {
            let id = layer_param(s, p)?;
            if !s.active().and_then(|d| d.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Group(_))) {
                return Err(EngineError::Other("Merge Group needs a group".into()));
            }
            merge_group(s, id)
        }),
        cmd!("layer.flattenImage", "Flatten Image", ["Layer"], None, "{}", has_doc, |s, _| {
            s.edit("Flatten Image", |doc, active| {
                let fmt = doc.pixel_format();
                let mut bg = Layer::raster("Background", fmt);
                bg.locks.transparency = true;
                bg.locks.position = true;
                *crate::pixels_mut(&mut bg)? = photocraft_compose::flatten_to_surface(doc, fmt, Some([1.0, 1.0, 1.0]));
                *active = Some(bg.id);
                doc.layers = vec![bg];
                Ok(())
            })?;
            Ok(Value::Null)
        }),
    ]
}

/// Solid color and gradient fill layers.
fn fill_layers() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.newFillLayer.solidColor",
            "Solid Color…",
            ["Layer", "New Fill Layer"],
            None,
            r##"{"color":"#rrggbb"=foreground,"path":"work"|saved path name|{…path}? (the active path: becomes the vector mask, making a shape; else the selection is the layer mask)}"##,
            has_doc,
            |s, p| {
                let c = color_param("layer.newFillLayer.solidColor", p, "color", s.tools.foreground)?;
                let default_mask = layer_panel_option(s, "useDefaultMasksOnFillLayers", true);
                let id = s.edit("New Color Fill Layer", |doc, active| {
                    let fill = Fill::Solid(Color::rgba(c[0], c[1], c[2], c[3])).in_mode(doc.mode);
                    let mut l = Layer::new(doc.next_layer_name("Color Fill"), LayerContent::Fill(fill));
                    mask_new_layer(doc, &mut l, p, "layer.newFillLayer.solidColor", default_mask)?;
                    let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::Auto);
                    *active = Some(id);
                    Ok(id)
                })?;
                Ok(json!({ "layer": id.0 }))
            }
        ),
        cmd!(
            "layer.newFillLayer.gradient",
            "Gradient…",
            ["Layer", "New Fill Layer"],
            None,
            r##"{"from":"#rrggbb","to":"#rrggbb","angle":deg=90,"style":"linear|radial|angle|reflected|diamond","reverse":bool,"path":"work"|saved path name|{…path}? (the active path: becomes the vector mask; else the selection is the layer mask)}"##,
            has_doc,
            |s, p| {
                let a = color_param("layer.newFillLayer.gradient", p, "from", s.tools.foreground)?;
                let b = color_param("layer.newFillLayer.gradient", p, "to", s.tools.background)?;
                let angle = crate::gradient_fill_cmds::angle_param("layer.newFillLayer.gradient", p, 90.0)?;
                let style = crate::layer_style::gradient_style(p.get("style").and_then(Value::as_str).unwrap_or("linear"));
                let reverse = p.get("reverse").and_then(Value::as_bool).unwrap_or(false);
                let default_mask = layer_panel_option(s, "useDefaultMasksOnFillLayers", true);
                let id = s.edit("New Gradient Fill Layer", |doc, active| {
                    let fill = Fill::gradient(
                        vec![(0.0, Color::rgba(a[0], a[1], a[2], a[3])), (1.0, Color::rgba(b[0], b[1], b[2], b[3]))],
                        angle,
                        1.0,
                        style,
                        reverse,
                    )
                    .in_mode(doc.mode);
                    let mut l = Layer::new(doc.next_layer_name("Gradient Fill"), LayerContent::Fill(fill));
                    mask_new_layer(doc, &mut l, p, "layer.newFillLayer.gradient", default_mask)?;
                    let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::Auto);
                    *active = Some(id);
                    Ok(id)
                })?;
                Ok(json!({ "layer": id.0 }))
            }
        ),
    ]
}

/// Editing an adjustment layer's properties.
fn adjustments() -> Vec<CommandSpec> {
    vec![cmd!(
        "layer.setAdjustment",
        "Adjustment Properties",
        [],
        None,
        r##"{"layer":id?, …params of that adjustment kind} (Levels, Curves: "eyedropper":{"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1} = the Properties panel's Set Black/Neutral Gray/White Point, sampling the image below the layer, RGB and Grayscale (no gray) only)"##,
        has_layer,
        |s, p| {
            let id = layer_param(s, p)?;
            let eyedropped = crate::adjust_params::layer_eyedropper(s, id, p)?;
            s.edit("Modify Adjustment", |doc, _| {
                let mode = doc.mode;
                let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                let LayerContent::Adjustment(adj) = &mut l.content else { return Err(EngineError::Other("not an adjustment layer".into())) };
                if matches!(adj, Adjustment::Unsupported { .. }) {
                    return Err(EngineError::Other("this adjustment can't be edited (kept as imported)".into()));
                }
                let kind = adjustment_kind(adj);
                // No params resets to the defaults; otherwise params merge over the current values.
                *adj = if let Some(eyedropped) = eyedropped {
                    eyedropped
                } else if crate::adjust_params::user_keys(p).is_empty() {
                    crate::adjust_params::default_for(kind, mode)?
                } else {
                    crate::adjust_params::from_params(kind, p, Some(adj), mode)?
                };
                Ok(())
            })?;
            Ok(Value::Null)
        }
    )]
}

/// Moving layers in the stack and on the canvas.
fn placement() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.moveTo",
            "Reorder Layer",
            [],
            None,
            r##"{"layer":id?| "layers":[id,…]?, "target":id|name,"targetOccurrence":n? (which of several layers named `target`, in document order),"position":"above|below|into"="above","copy":bool=false} (layers: one undoable move, document order preserved; copy: place duplicates there and leave the layers, as ⌥-dragging a Layers panel row; a recorded action keeps the target by name)"##,
            has_layer,
            crate::layer_multi_cmds::move_to
        ),
        cmd!(
            "layer.translate",
            "Move Layer",
            [],
            None,
            r##"{"layer":id?,"dx":i32,"dy":i32,"units":"px"|"pt"="px" (pt: points, whole pixels at the document's resolution),"target":"pixels"|"mask"="pixels","selection":bool=false} (no layer: every selected layer; linked layers follow; target mask: a layer whose mask is unlinked moves that mask alone, a linked mask moves with the layer; selection: the selection moves too, in the same step)"##,
            has_layer,
            crate::layer_multi_cmds::translate
        ),
    ]
}

/// Applies layer.setProps to its target layers as one history step.
fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = prop_targets(s, p)?;
    if ids.len() > 1 && p.get("name").is_some() {
        return Err(bad("layer.setProps", "`name` renames one layer, not several"));
    }
    let (opacity, fill) = (unit_prop(p, "opacity")?, unit_prop(p, "fill")?);
    let label = props_label(p);
    // The eye: a history state only with History Options › Make Layer Visibility
    // Changes Undoable.
    let record = label != "Layer Visibility" || s.visibility_undoable();
    s.edit_with(label, record, |doc, _| {
        // The Background has no opacity, fill or blend mode: like Photoshop, setting
        // one that changes its look turns it into a normal layer ("Layer 0") first.
        let num = |k: &str| p.get(k).and_then(Value::as_f64).is_some_and(|v| v < 1.0);
        let blends = p.get("blend").and_then(Value::as_str).and_then(blend_from_str).is_some_and(|b| b != photocraft_color::BlendMode::Normal);
        for &id in &ids {
            if num("opacity") || num("fill") || blends {
                crate::extra_cmds::background_to_layer(doc, id);
            }
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            apply_props(l, p, opacity, fill)?;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Several layers at once (the Layers panel with several selected, as in Photoshop),
/// else the given or active one.
fn prop_targets(s: &Session, p: &Value) -> Result<Vec<LayerId>> {
    match p.get("layers") {
        None | Some(Value::Null) => Ok(vec![layer_param(s, p)?]),
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= 10_000 => {
            let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
            let mut ids = Vec::with_capacity(a.len());
            for v in a {
                let id = v.as_u64().map(LayerId).ok_or_else(|| bad("layer.setProps", format!("`layers` holds layer ids, got {v}")))?;
                if doc.layer(id).is_none() {
                    return Err(EngineError::NoLayer(id));
                }
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            Ok(ids)
        }
        Some(v) => Err(bad("layer.setProps", format!("`layers` is a non-empty list of layer ids, got {v}"))),
    }
}

/// Opacity and Fill are fractions here, while the Layer Style commands take
/// percentages: a value outside 0..1 is refused, never clamped to 100 % (#5242).
fn unit_prop(p: &Value, key: &str) -> Result<Option<f32>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    match v.as_f64() {
        Some(x) if (0.0..=1.0).contains(&x) => Ok(Some(x as f32)),
        _ => Err(bad("layer.setProps", format!("{key} must be a number in 0..1 (1 = 100 %), got {v}"))),
    }
}

/// Photoshop's History names for a single kind of change; a mix is "Layer Properties".
fn props_label(p: &Value) -> &'static str {
    let changed: Vec<&str> =
        p.as_object().map(|o| o.keys().map(String::as_str).filter(|k| !matches!(*k, "layer" | "layers" | "coalesce")).collect()).unwrap_or_default();
    match changed.as_slice() {
        ["visible"] => "Layer Visibility",
        ["opacity"] => "Opacity Change",
        ["fill"] => "Fill Opacity Change",
        ["blend"] => "Blending Change",
        ["name"] => "Rename Layer",
        _ => "Layer Properties",
    }
}

/// Sets the requested name, visibility, opacity, fill and blend mode on one layer.
fn apply_props(l: &mut Layer, p: &Value, opacity: Option<f32>, fill: Option<f32>) -> Result<()> {
    if let Some(v) = p.get("name").and_then(Value::as_str) {
        l.name = v.to_string();
    }
    if let Some(v) = p.get("visible").and_then(Value::as_bool) {
        l.visible = v;
    }
    if let Some(v) = opacity {
        l.opacity = v;
    }
    if let Some(v) = fill {
        l.fill_opacity = v;
    }
    if let Some(v) = p.get("blend").and_then(Value::as_str) {
        l.blend = blend_prop(l, v)?;
    }
    apply_clip_and_locks(l, p)
}

/// Sets the requested clipping, channel and lock flags on one layer.
fn apply_clip_and_locks(l: &mut Layer, p: &Value) -> Result<()> {
    if let Some(v) = p.get("clipped").and_then(Value::as_bool) {
        l.clipped = v;
    }
    if let Some(v) = p.get("locked").and_then(Value::as_bool) {
        l.locks.all = v;
    }
    if let Some(Value::Array(ch)) = p.get("channels") {
        // Blending Options › Advanced Blending › Channels: unchecked = left out.
        l.excluded_channels = ch.iter().take(32).enumerate().filter(|(_, v)| v.as_bool() == Some(false)).fold(0, |m, (i, _)| m | 1 << i);
    }
    if let Some(Value::Object(m)) = p.get("locks") {
        set_locks(l, m)?;
    }
    Ok(())
}

/// Parses a blend mode for the layer, refusing Pass Through on anything but a group.
fn blend_prop(l: &Layer, v: &str) -> Result<BlendMode> {
    let b = blend_from_str(v).ok_or_else(|| bad("layer.setProps", format!("unknown blend mode `{v}`")))?;
    // Pass Through is a group's mode only, as in Photoshop.
    if b == BlendMode::PassThrough && !l.is_group() {
        return Err(bad("layer.setProps", format!("Pass Through applies to groups only; layer {} isn't one", l.id.0)));
    }
    Ok(b)
}

/// Sets each named lock, refusing an unknown name.
fn set_locks(l: &mut Layer, m: &Map<String, Value>) -> Result<()> {
    for (k, v) in m {
        let v = v.as_bool().unwrap_or(false);
        if !l.locks.set(k, v) {
            return Err(bad("layer.setProps", format!("unknown lock `{k}`")));
        }
    }
    Ok(())
}
