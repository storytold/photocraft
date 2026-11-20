//! What the Layers panel's events do: each `LayersEvent` becomes the `(command, params)` PhotoCraft's
//! own commands take, run through one tail (a type session commits before a select, the target
//! switches, `Value::Null` params run as the menu item does).

use craft::behavior::{EditEvent, Mixed, SelectionChange};
use craft::keymap::Mods;
use craft::layers::ui::panel::{
    ActivateOn, LayerColumn, LayerDrop, LayerMove, LayerPosition, LayerToggle, LayersEvent, LayersFooterAction, LayersFooterButton,
};
use craft::layers::ui::rows::LayerPropertyEdit;
use craft::layers::ui::tree::{LayerPart, LayerRowKey, LockKind, SubRowGroup};
use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use super::layers_tree::{self, PhotoLayers};
use crate::PhotocraftApp;
use crate::mask_thumbs_ui::MaskKind;

type Action = (String, Value);

#[cfg(test)]
pub(crate) mod tests;

fn egui_mods(m: Mods) -> egui::Modifiers {
    egui::Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: false, command: m.cmd || m.ctrl }
}

fn mask_kind(part: LayerPart) -> Option<MaskKind> {
    match part {
        LayerPart::LayerMask => Some(MaskKind::Pixel),
        LayerPart::VectorMask => Some(MaskKind::Vector),
        _ => None,
    }
}

fn targets_param(ids: &[LayerId]) -> Value {
    match ids {
        [one] => json!({"layer": one.0}),
        many => json!({"layers": many.iter().map(|id| id.0).collect::<Vec<_>>()}),
    }
}

/// The `layer.moveTo` params of a layer move.
pub(crate) fn move_params(mv: &LayerMove<LayerId>) -> Value {
    let position = match mv.position {
        LayerPosition::Before => "above",
        LayerPosition::After => "below",
        _ => "into",
    };
    let mut p = match mv.layers.as_slice() {
        [one] => json!({"layer": one.0}),
        many => json!({"layers": many.iter().map(|id| id.0).collect::<Vec<_>>()}),
    };
    p["target"] = json!(mv.target.0);
    p["position"] = json!(position);
    if mv.copy {
        p["copy"] = json!(true);
    }
    p
}

/// The value an edit lands on: a `Set` or a gesture's `Commit`.
fn landed<T>(edit: EditEvent<T>) -> Option<T> {
    match edit {
        EditEvent::Set { after, .. } | EditEvent::Commit { after, .. } => Some(after),
        _ => None,
    }
}

/// Applies the panel's events of this pass.
pub(crate) fn apply_layers_events(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<LayersEvent<LayerId>>) {
    let mut actions: Vec<Action> = Vec::new();
    for event in events {
        map_event(app, ctx, event, &mut actions);
    }
    run_actions(app, ctx, actions);
}

/// The panel's lock indicator reports only an unlock: the Background becomes a normal layer, any other layer loses
/// every lock.
fn unlock_action(doc: &photocraft_doc::Document, id: LayerId) -> Action {
    if doc.is_background(id) {
        ("layer.new.layerFromBackground".into(), json!({}))
    } else {
        ("layer.setProps".into(), json!({"layer": id.0, "locks": {"transparency": false, "pixels": false, "position": false, "artboard": false, "all": false}}))
    }
}

/// The commands a click or sweep down a column of the panel makes.
fn toggle_actions(doc: &photocraft_doc::Document, toggle: LayerToggle<LayerId>, actions: &mut Vec<Action>) {
    let layer = |id: &LayerId| doc.layer(*id);
    let filter_index = |id: &LayerId, row: usize| layers_tree::filter_index(doc, id, row);
    // One sweep down the eye column is one gesture, so its steps share a key and are one undo step.
    let coalesce = crate::history_keys::coalesce("layer-eye-sweep", &toggle.edit);
    let value = match toggle.edit {
        EditEvent::Set { after, .. } | EditEvent::Update { value: after, .. } | EditEvent::Commit { after, .. } => Some(after),
        EditEvent::Cancel { before: Some(Mixed::Same(v)), .. } => Some(v),
        _ => None,
    };
    let Some(on) = value else { return };
    for row in &toggle.rows {
        match (toggle.column, row) {
            (LayerColumn::Visibility, LayerRowKey::Layer(id)) => {
                if layer(id).is_some_and(|l| l.visible != on) {
                    let mut p = json!({"layer": id.0, "visible": on});
                    if let Some(key) = &coalesce {
                        p["coalesce"] = json!(key);
                    }
                    actions.push(("layer.setProps".into(), p));
                }
            }
            (LayerColumn::Visibility, LayerRowKey::Effects(id, index)) => {
                let mut p = json!({"layer": id.0, "visible": on});
                if let Some(i) = index {
                    p["index"] = json!(i);
                }
                actions.push(("layer.setEffectsVisible".into(), p));
            }
            (LayerColumn::Visibility, LayerRowKey::Filters(id, row)) => {
                let Some(LayerContent::Smart(sm)) = layer(id).map(|l| &l.content) else { continue };
                match row.and_then(|r| filter_index(id, r)) {
                    None if row.is_none() && sm.filters_enabled != on => actions.push(("layer.smartFilter.disableSmartFilters".into(), json!({"layer": id.0}))),
                    Some(i) if sm.smart_filters.get(i).is_some_and(|f| f.visible != on) => {
                        actions.push(("layer.smartFilter.setVisible".into(), json!({"layer": id.0, "index": i})))
                    }
                    _ => {}
                }
            }
            (LayerColumn::Lock, LayerRowKey::Layer(id)) if layer(id).is_some() => actions.push(unlock_action(doc, *id)),
            _ => {}
        }
    }
}

/// The commands a click on a layer's thumbnail makes.
fn thumb_click_actions(doc: &photocraft_doc::Document, id: LayerId, part: LayerPart, mods: Mods, actions: &mut Vec<Action>) {
    let Some(l) = doc.layer(id) else { return };
    let m = egui_mods(mods);
    if mods.cmd || mods.ctrl {
        let params = match part {
            LayerPart::LayerMask | LayerPart::VectorMask => mask_kind(part).map(|k| crate::mask_thumbs_ui::load_params(l, k, mods)),
            LayerPart::FrameShape if l.frame.is_some() => Some(crate::mask_thumbs_ui::load_params(l, MaskKind::Vector, mods)),
            LayerPart::Pixels | LayerPart::FrameShape if !doc.is_background(id) => {
                Some(json!({"channel": "transparency", "layer": id.0, "operation": craft::behavior::geometry::SelectionOperation::from_mods(mods).name()}))
            }
            _ => None,
        };
        if let Some(p) = params {
            actions.push(("select.loadSelection".into(), p));
        }
        return;
    }
    if part == LayerPart::FilterMask {
        actions.extend(crate::smart_ui::filter_mask_click(l, m));
        return;
    }
    if let Some(cmd) = mask_kind(part).and_then(|k| crate::mask_thumbs_ui::click_command(l, k, m)) {
        actions.push(cmd);
    } else if part == LayerPart::Pixels && m.alt && !m.shift {
        // Alt-click the layer thumbnail: fit the layer on screen, as Photoshop does.
        actions.push(("layer.select".into(), json!({"layer": id.0, "mode": "replace"})));
        actions.push(("view.fitLayersOnScreen".into(), Value::Null));
    }
}

fn map_event(app: &mut PhotocraftApp, ctx: &egui::Context, event: LayersEvent<LayerId>, actions: &mut Vec<Action>) {
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.clone();
    let active = st.active_layer;
    let selection = st.selected_layers();
    let picked = crate::smart_ui::selected(&app.smart_filter_picks, &doc, active);
    let (targets, picked) = {
        let tree = PhotoLayers::new(app, st, &app.ui.layer_filter, false, picked);
        (tree.targets(), tree.selected_filter())
    };
    let layer = |id: &LayerId| doc.layer(*id);
    let filter_index = |id: &LayerId, row: usize| layers_tree::filter_index(&doc, id, row);
    match event {
        LayersEvent::Select(select) => {
            match &select.row {
                Some(LayerRowKey::Filters(id, row)) => {
                    let index = row.and_then(|r| filter_index(id, r));
                    crate::smart_ui::pick(&mut app.smart_filter_picks, doc.id, (id.0, index));
                }
                _ => crate::smart_ui::deselect(&mut app.smart_filter_picks, doc.id),
            }
            let single = match &select.change {
                SelectionChange::Set(ids) if ids.len() == 1 => ids.first().copied(),
                _ => None,
            };
            selection_actions(select.change, selection, actions);
            let Some(id) = single else { return };
            let Some(l) = layer(&id) else { return };
            target_on_click(app, l, active == Some(id), select.part, actions);
        }
        LayersEvent::Toggle(toggle) => toggle_actions(&doc, toggle, actions),
        LayersEvent::ShowOnly(row) | LayersEvent::UndoShowOnly(row) => {
            actions.push(("layer.showOnly".into(), json!({"layer": row.layer().0})));
        }
        LayersEvent::LabelColor { rows, color, .. } => {
            let color = layers_tree::doc_label(color).id();
            if rows.len() > 1 || (rows.len() == 1 && selection.len() > 1 && rows.iter().all(|r| selection.contains(r))) {
                actions.push(("layer.setLabelColor".into(), json!({"color": color})));
            } else if let Some(id) = rows.first() {
                actions.push(("layer.setLabelColor".into(), json!({"layer": id.0, "color": color})));
            }
        }
        LayersEvent::Open(change) => {
            actions.push(("layer.setExpanded".into(), json!({"layer": change.key.0, "expanded": change.open, "all": change.all})));
        }
        LayersEvent::FxOpen { layer, open, all, .. } => {
            actions.push(("layer.setEffectsExpanded".into(), json!({"layer": layer.0, "expanded": open, "all": all})));
        }
        LayersEvent::Rename { layer, text, .. } => {
            actions.push(("layer.renameLayer".into(), json!({"layer": layer.0, "name": text})));
        }
        LayersEvent::Activate { row, on, .. } => activate(app, ctx, &doc, row, on, actions),
        LayersEvent::ThumbClick { layer: id, part, mods, .. } => thumb_click_actions(&doc, id, part, mods, actions),
        LayersEvent::MaskLink { layer: id, part, linked, .. } => {
            let prefix = if part == LayerPart::VectorMask { "layer.vectorMask" } else { "layer.layerMask" };
            actions.push((format!("{prefix}.linked"), json!({"layer": id.0, "linked": linked})));
        }
        LayersEvent::Properties(edit) => properties(app, &targets, edit, actions),
        LayersEvent::Drop(edit) => {
            let Some(drop) = landed(edit) else { return };
            app.reorder_preview = None;
            match drop {
                LayerDrop::Layers(mv) => {
                    if mv.position == LayerPosition::Into
                        && layer(&mv.target).is_some_and(|l| l.frame.is_some())
                        && let [content] = mv.layers.as_slice()
                    {
                        actions.push(("layer.frame.setContent".into(), json!({"layer": mv.target.0, "content": content.0, "copy": mv.copy})));
                    } else {
                        actions.push(("layer.moveTo".into(), move_params(&mv)));
                    }
                }
                LayerDrop::SubRows(mv) if mv.group == SubRowGroup::SmartFilters => {
                    let (Some(from), Some(to)) = (filter_index(&mv.layer, mv.from), filter_index(&mv.layer, mv.to)) else { return };
                    crate::smart_ui::pick(&mut app.smart_filter_picks, doc.id, (mv.layer.0, Some(to)));
                    actions.push(("layer.smartFilter.move".into(), json!({"layer": mv.layer.0, "index": from, "to": to})));
                    if active != Some(mv.layer) {
                        actions.push(("layer.select".into(), json!({"layer": mv.layer.0})));
                    }
                }
                LayerDrop::Transfer(t) => {
                    for row in &t.rows {
                        match row {
                            LayerRowKey::Effects(from, effect) => {
                                let mut p = json!({"from": from.0, "to": t.to.0, "copy": t.copy});
                                if let Some(i) = effect {
                                    p["effect"] = json!(i);
                                }
                                actions.push(("layer.layerStyle.transferEffects".into(), p));
                            }
                            LayerRowKey::Filters(from, r) => {
                                let index = r.and_then(|r| filter_index(from, r));
                                actions.push(crate::smart_ui::drop_action(from.0, index, t.to, t.copy));
                            }
                            _ => {}
                        }
                    }
                }
                LayerDrop::Mask(mv) if mv.part == LayerPart::LayerMask => {
                    actions.push(("layer.layerMask.moveTo".into(), json!({"layer": mv.layer.0, "to": mv.to.0, "copy": mv.copy})));
                }
                _ => {}
            }
        }
        LayersEvent::Footer(action) => footer(app, picked, action, actions),
        LayersEvent::FooterDrop { button, rows, .. } => {
            let layers: Vec<LayerId> = rows.iter().filter_map(|r| if let LayerRowKey::Layer(id) = r { Some(*id) } else { None }).collect();
            let layer_params = || if layers.len() == 1 && !selection.contains(&layers[0]) { json!({"layer": layers[0].0}) } else { json!({}) };
            match button {
                LayersFooterButton::Delete => {
                    if !layers.is_empty() {
                        actions.push(("layer.delete".into(), layer_params()));
                    }
                    for row in &rows {
                        match row {
                            LayerRowKey::Effects(id, effect) => {
                                let mut p = json!({"layer": id.0});
                                if let Some(i) = effect {
                                    p["effect"] = json!(i);
                                }
                                actions.push(("layer.layerStyle.clear".into(), p));
                            }
                            LayerRowKey::Filters(id, r) => actions.push(crate::smart_ui::trash_action((id.0, r.and_then(|r| filter_index(id, r))))),
                            LayerRowKey::Mask(id, part) => {
                                if let Some(kind) = mask_kind(*part) {
                                    actions.push(crate::mask_trash_prompt::drop_action(id.0, kind));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                LayersFooterButton::NewLayer if !layers.is_empty() => actions.push(("layer.duplicate".into(), layer_params())),
                LayersFooterButton::NewGroup if !layers.is_empty() => actions.push(("layer.groupLayers".into(), layer_params())),
                _ => {}
            }
        }
        LayersEvent::ClipLine { upper, release, .. } => {
            let line = crate::clip_line_ui::ClipLine { upper, release };
            actions.extend(line.commands(&doc, &format!("clip-line:{}", ctx.cumulative_pass_nr())));
        }
        LayersEvent::Menu { target, command, .. } => menu_pick(app, ctx, &target.row, &command, actions),
        _ => {}
    }
}

/// A click that selected `l` alone: the thumbnail it landed on becomes the target, as the old
/// thumbnail clicks did; a frame's click picks the frame tool target.
fn target_on_click(app: &mut PhotocraftApp, l: &Layer, was_active: bool, part: Option<LayerPart>, actions: &mut Vec<Action>) {
    if l.frame.is_some() {
        let target = match part {
            Some(LayerPart::FrameShape) => crate::frame_tool::Target::Frame,
            Some(LayerPart::FrameContent) => crate::frame_tool::Target::Content,
            Some(_) => crate::frame_tool::Target::Both,
            None if l.children().is_some_and(|c| !c.is_empty()) => crate::frame_tool::Target::Both,
            None => crate::frame_tool::Target::Frame,
        };
        crate::frame_tool::select(app, l.id, target);
        return;
    }
    match part {
        Some(LayerPart::VectorMask) => {
            app.ui.selected_path = Some("layer".into());
            actions.push(("ui.vectorMaskTarget".into(), json!(true)));
        }
        Some(LayerPart::LayerMask) => {
            // The mask thumbnail also switches Properties to the mask (#3369).
            app.ui.properties_layer = None;
            actions.push(("ui.maskTarget".into(), json!(true)));
        }
        Some(LayerPart::FilterMask) => actions.push(("ui.filterMaskTarget".into(), json!(l.id.0))),
        Some(_) => {
            // Clicking the layer thumbnail leaves mask view (#196).
            if let Some(v) = app.session.active().and_then(photocraft_engine::mask_view_cmds::current).filter(|v| v.layer == l.id) {
                let mask = if v.filter { "filter" } else { "layer" };
                actions.push((photocraft_engine::mask_view_cmds::ID.into(), json!({"layer": l.id.0, "mode": "off", "mask": mask})));
            }
            actions.push(("ui.maskTarget".into(), json!(false)));
        }
        // Selecting an adjustment or fill row targets its mask; another row starts on its pixels.
        None if matches!(l.content, LayerContent::Adjustment(_) | LayerContent::Fill(_)) && l.mask.is_some() => {
            actions.push(("ui.maskTarget".into(), json!(true)))
        }
        None if !was_active => actions.push(("ui.maskTarget".into(), json!(false))),
        None => {}
    }
}

/// A double click or Enter that isn't a rename.
fn activate(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    doc: &photocraft_doc::Document,
    row: LayerRowKey<LayerId>,
    on: ActivateOn,
    actions: &mut Vec<Action>,
) {
    let active = app.session.active().and_then(|st| st.active_layer);
    match row {
        LayerRowKey::Layer(id) => {
            let Some(l) = doc.layer(id) else { return };
            if doc.is_background(id) {
                actions.push(("layer.new.layerFromBackground".into(), json!({})));
                return;
            }
            let command = match on {
                ActivateOn::Thumbnail(LayerPart::LayerMask | LayerPart::VectorMask) => {
                    let kind = if on == ActivateOn::Thumbnail(LayerPart::VectorMask) { MaskKind::Vector } else { MaskKind::Pixel };
                    actions.push((if kind == MaskKind::Vector { "ui.vectorMaskTarget" } else { "ui.maskTarget" }.into(), json!(true)));
                    match crate::mask_thumbs_ui::double_click_command(app, kind) {
                        Some((cmd, val)) => actions.push((cmd.into(), val)),
                        None => actions.push(("ui.properties".into(), json!(true))),
                    }
                    return;
                }
                ActivateOn::Thumbnail(LayerPart::FrameContent) => {
                    if let Some(child) = l.children().and_then(|c| c.first()).filter(|c| matches!(c.content, LayerContent::Smart(_))) {
                        actions.push(("layer.smartObjects.editContents".into(), json!({"layer": child.id.0})));
                    }
                    return;
                }
                ActivateOn::Thumbnail(LayerPart::Pixels) if l.frame.is_none() => match &l.content {
                    LayerContent::Adjustment(_) | LayerContent::Fill(_) | LayerContent::Shape(_) => "layer.layerContentOptions",
                    LayerContent::Smart(_) => "layer.smartObjects.editContents",
                    LayerContent::Text(_) => "type.editText",
                    _ => "layer.layerStyle.blendingOptions",
                },
                _ => "layer.layerStyle.blendingOptions",
            };
            if crate::menus::is_enabled(app, command) {
                // Null params: run like the menu item, dialog included.
                actions.push((command.into(), Value::Null));
            }
        }
        LayerRowKey::Effects(id, index) => {
            let kind = index
                .and_then(|i| doc.layer(id)?.effects.items.get(i).map(|e| e.label()))
                .and_then(|label| crate::layer_style::KINDS.iter().find(|k| k.1 == label).map(|k| k.0));
            if active != Some(id) && app.run("layer.select", json!({"layer": id.0})).is_err() {
                return;
            }
            crate::layer_style::open_for_layer(app, id, kind);
        }
        LayerRowKey::Filters(id, Some(row)) => {
            let Some(LayerContent::Smart(sm)) = doc.layer(id).map(|l| &l.content) else { return };
            let Some(index) = sm.smart_filters.len().checked_sub(1).and_then(|last| last.checked_sub(row)) else { return };
            if active != Some(id) && app.run("layer.select", json!({"layer": id.0})).is_err() {
                return;
            }
            if on == ActivateOn::Options {
                if let Err(e) = crate::smart_filter_dialog::open(app, id, index) {
                    crate::notices::error(app, e);
                }
            } else {
                crate::smart_ui::open_editor(app, ctx, id.0, sm, index);
            }
        }
        _ => {}
    }
}

fn properties(app: &mut PhotocraftApp, targets: &[LayerId], edit: LayerPropertyEdit, actions: &mut Vec<Action>) {
    if targets.is_empty() {
        return;
    }
    let p = targets_param(targets);
    let is_opacity = matches!(edit, LayerPropertyEdit::Opacity(_));
    match edit {
        LayerPropertyEdit::Blend(e) => match e {
            EditEvent::Update { value, .. } => crate::blend_preview::hover(app, targets, layers_tree::blend_of(&value)),
            EditEvent::Set { after, .. } | EditEvent::Commit { after, .. } => {
                crate::blend_preview::hover(app, targets, None);
                if let Some(m) = layers_tree::blend_of(&after) {
                    let mut p = p;
                    p["blend"] = json!(m.label());
                    actions.push(("layer.setProps".into(), p));
                }
            }
            EditEvent::Cancel { .. } => crate::blend_preview::hover(app, targets, None),
            _ => {}
        },
        LayerPropertyEdit::Opacity(e) | LayerPropertyEdit::Fill(e) => {
            let key = if is_opacity { "opacity" } else { "fill" };
            // A scrub or slider drag is one undo step: its gesture coalesces its steps.
            let coalesce = crate::history_keys::coalesce(&format!("layer-{key}-slider"), &e);
            let value = match e {
                EditEvent::Update { value, .. } => Some(value),
                EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(after),
                EditEvent::Cancel { before: Some(Mixed::Same(v)), .. } => Some(v),
                _ => None,
            };
            if let Some(value) = value {
                actions.push(super::options_bar::pct_action(&p, key, value as f32, coalesce));
            }
        }
        LayerPropertyEdit::Lock { kind, on, .. } => {
            let key = match kind {
                LockKind::Transparency => "transparency",
                LockKind::Pixels => "pixels",
                LockKind::Position => "position",
                LockKind::Artboard => "artboard",
                _ => "all",
            };
            let mut p = p;
            p["locks"] = json!({ key: on });
            actions.push(("layer.setProps".into(), p));
        }
        _ => {}
    }
}

/// A pick in the footer.
fn footer(app: &mut PhotocraftApp, picked: Option<crate::smart_ui::FilterDrag>, action: LayersFooterAction, actions: &mut Vec<Action>) {
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.clone();
    let active = st.active_layer.and_then(|id| doc.layer(id));
    match action {
        LayersFooterAction::Link => actions.push(("layer.linkLayers".into(), json!({}))),
        LayersFooterAction::NewLayer { ask, below, .. } => {
            let params = if ask {
                Value::Null
            } else if below {
                json!({"position": "below"})
            } else {
                json!({})
            };
            actions.push(("layer.new.layer".into(), params));
        }
        LayersFooterAction::NewGroup { from_selection, ask, below, .. } => {
            let groups = from_selection && active.is_some();
            if ask {
                // Null params go through the menu path, which opens the dialog.
                let cmd = if groups { crate::new_group_ui::FROM_LAYERS } else { crate::new_group_ui::NEW_GROUP };
                actions.push((cmd.into(), Value::Null));
            } else if below {
                actions.push(("layer.new.group".into(), json!({"position": "below"})));
            } else {
                actions.push((if groups { "layer.groupLayers" } else { "layer.new.group" }.into(), json!({})));
            }
        }
        LayersFooterAction::Delete { confirm, .. } => {
            // A selected Smart Filter row is what Delete removes, as in Photoshop (#4751).
            if let Some(picked) = picked {
                actions.push(crate::smart_ui::trash_action(picked));
                crate::smart_ui::deselect(&mut app.smart_filter_picks, doc.id);
                return;
            }
            let delete = if app.ui.vector_mask_target && active.is_some_and(|l| l.vector_mask.is_some()) {
                "layer.vectorMask.delete"
            } else if app.ui.mask_target && active.is_some_and(|l| l.mask.is_some()) {
                "layer.layerMask.delete"
            } else {
                "layer.delete"
            };
            let params = if confirm && delete == "layer.delete" { json!({"__trash": true}) } else { json!({}) };
            actions.push((delete.into(), params));
        }
        LayersFooterAction::AddMask { invert, .. } => {
            let Some(cmd) = crate::layer_menu_ui::mask_button_command(active, doc.selection.is_some(), invert) else { return };
            // A path selected in the Paths panel becomes the vector mask, as in Photoshop (#4728).
            match crate::vector_ui::selected_document_path(app).filter(|_| cmd.starts_with("layer.vectorMask.")) {
                Some(name) => actions.push(("layer.vectorMask.currentPath".into(), json!({"name": name, "hide": invert}))),
                None => actions.push((cmd.into(), json!({}))),
            }
        }
        LayersFooterAction::Adjustment(id) => actions.push((id, json!({}))),
        LayersFooterAction::Style(kind) => {
            crate::layer_style::open(app, (!kind.is_empty()).then_some(kind.as_str()));
        }
        _ => {}
    }
}

/// A pick in a row menu.
fn menu_pick(app: &mut PhotocraftApp, ctx: &egui::Context, row: &Option<LayerRowKey<LayerId>>, command: &str, actions: &mut Vec<Action>) {
    let layer = row.as_ref().map(|r| *r.layer());
    if command == layers_tree::RENAME {
        if let Some(id) = layer {
            app.layers_rename = Some(id);
        }
        return;
    }
    if let Some(size) = command.strip_prefix(layers_tree::THUMB_SIZE) {
        if let Some(size) = crate::layers_panel_ui::size_of_key(size) {
            app.ui.layers_settings.options.thumbnail_size = size;
            crate::layers_panel_ui::persist_options(app);
        }
        return;
    }
    if let Some(to_layer) = command.strip_prefix(layers_tree::CLIP_THUMBS) {
        let to_layer = to_layer == "true";
        if app.session.prefs().interface.clip_thumbnails_to_layer_bounds != to_layer {
            actions.push(("prefs.set".into(), json!({"path": "interface.clipThumbnailsToLayerBounds", "value": to_layer})));
        }
        return;
    }
    let Some(id) = layer else { return };
    if command == layers_tree::SELECT_PIXELS {
        actions.push(("select.loadSelection".into(), layers_tree::select_pixels(id)));
    } else if let Some(operation) = command.strip_prefix(layers_tree::LOAD_MASK) {
        actions.push(("select.loadSelection".into(), json!({"channel": "mask", "layer": id.0, "operation": operation})));
    } else if command == layers_tree::MASK_REFINE {
        actions.push(("ui.maskTarget".into(), json!(true)));
        actions.push(("select.selectAndMask".into(), Value::Null));
    } else if command == layers_tree::MASK_OPTIONS {
        actions.push(("ui.maskTarget".into(), json!(true)));
        actions.push(("channel.options".into(), Value::Null));
    } else {
        let _ = ctx;
        actions.push((command.to_string(), Value::Null));
    }
}

/// The commands that make `change` the layer selection, `selected` being the layers selected now.
pub(crate) fn selection_actions(change: SelectionChange<LayerId>, selected: &[LayerId], actions: &mut Vec<Action>) {
    match change {
        SelectionChange::Set(ids) if ids.is_empty() => actions.push(("select.deselectLayers".into(), json!({}))),
        SelectionChange::Set(ids) if ids.len() == 1 => {
            if let Some(id) = ids.first() {
                actions.push(("layer.select".into(), json!({"layer": id.0, "mode": "replace"})));
            }
        }
        SelectionChange::Set(ids) => actions.push(("layer.select".into(), json!({"layers": ids.iter().map(|id| id.0).collect::<Vec<_>>()}))),
        SelectionChange::Toggle(id) => actions.push(("layer.select".into(), json!({"layer": id.0, "mode": "toggle"}))),
        SelectionChange::Clear if !selected.is_empty() => {
            actions.push(("select.deselectLayers".into(), json!({})));
        }
        _ => {}
    }
}

/// The layer ids a `layer.select` payload names: the single `layer`, or every id of the `layers`
/// list a multi-row selection carries.
fn select_param_ids(p: &Value) -> Vec<u64> {
    match p.get("layer").and_then(Value::as_u64) {
        Some(id) => vec![id],
        None => p.get("layers").and_then(Value::as_array).map(|ids| ids.iter().filter_map(Value::as_u64).collect()).unwrap_or_default(),
    }
}

/// Runs the actions in order: a type session commits before a select, the mask targets switch,
/// `Value::Null` params (and the fill layers that open a dialog) run as the menu item does.
pub(crate) fn run_actions(app: &mut PhotocraftApp, ctx: &egui::Context, actions: Vec<Action>) {
    for (id, p) in actions {
        // A row click commits even when it selects the layer already being edited (#5584); a
        // multi-row selection that leaves the edited layer out is a layer switch (#6483).
        if id == "layer.select" && app.ui.text_edit.is_some() {
            crate::type_tool::commit_before_selecting(app, &select_param_ids(&p));
        }
        if id == "ui.maskTarget" {
            app.end_filter_mask_view();
            app.ui.mask_target = p.as_bool().unwrap_or(false);
            app.ui.vector_mask_target = false;
            app.ui.filter_mask_target = None;
            if app.ui.mask_target {
                app.target_mask_over_colour_rows();
            }
            app.sync_mask_targets();
            continue;
        }
        if id == "ui.vectorMaskTarget" {
            app.end_filter_mask_view();
            app.ui.vector_mask_target = p.as_bool().unwrap_or(false);
            app.ui.mask_target = false;
            app.ui.filter_mask_target = None;
            app.sync_mask_targets();
            continue;
        }
        if id == "ui.filterMaskTarget" {
            app.select_filter_mask_target(p.as_u64());
            continue;
        }
        // Double-clicking a mask thumbnail opens its settings: show Properties.
        if id == "ui.properties" {
            if let Err(e) = crate::panel_docking::show_panel(app, "properties", Some(true)) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            continue;
        }
        // A command the runtime carries out (Fit Layer(s) on Screen after an Alt-click) fires there; the canvas runs it.
        if crate::shortcut_dispatch::fire_bridged(ctx, &id).is_some() {
            continue;
        }
        if p.is_null() || id == "layer.newFillLayer.solidColor" || id == photocraft_engine::pattern_fill_cmds::CREATE {
            let params = if p.is_null() { json!({}) } else { p };
            if let Err(e) = crate::menus::invoke(app, ctx, &id, params) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            continue;
        }
        if id == "prefs.set" {
            if let Err(e) = app.run(&id, p) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            continue;
        }
        let _ = app.run(&id, p);
    }
}
