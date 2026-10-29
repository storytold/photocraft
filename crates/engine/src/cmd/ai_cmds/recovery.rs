use super::*;

pub(super) fn open_document(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id(p, "ai.result.openDocument")?;
    let variant = match p.get("variant") {
        None => 0,
        Some(value) => value.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| error("AI variant must be a nonnegative integer"))?,
    };
    let name = match p.get("name") {
        None => format!("AI Result #{id}"),
        Some(value) => value
            .as_str()
            .filter(|name| !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control))
            .ok_or_else(|| error("Invalid AI document name"))?
            .to_owned(),
    };
    let target = s.ai.targets.get(&id).ok_or_else(|| error("No such AI result"))?;
    if target.original.max_group_depth() > 64 {
        return Err(error("AI result document nesting exceeds the recovery limit"));
    }
    let layer = target.layers.get(variant).ok_or_else(|| error("No such AI variant"))?.clone();
    let layer_id = layer.id;
    let mut doc = (*target.original).clone();
    validate_document(&doc)?;
    doc.id = DocId::fresh();
    doc.name = name;
    if let Some((size, left, top)) = target.canvas {
        crate::image_cmds::translate_doc(&mut doc, "ai.result.openDocument", left as i32, top as i32)?;
        doc.size = photocraft_geom::Size::new(size.width, size.height);
        crate::canvas_geom::refresh(&mut doc, crate::canvas_geom::Refresh::Shapes);
    }
    doc.layers.push(layer);
    doc.selection = None;
    let document = doc.id;
    s.add_document(doc, None);
    if let Some(state) = s.active_mut() {
        state.active_layer = Some(layer_id);
        state.selected_layers = vec![layer_id];
        state.saved_revision = 0;
    }
    Ok(json!({"result":id,"variant":variant,"document":document.0,"layer":layer_id.0}))
}
