//! Local experimental mask-effect list, visible only while a mask is selected in 3D.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{Layer, mask_paint::EffectKind};
use serde_json::{Value, json};

const MASK_EFFECT_ROW_INDENT: f32 = 28.0;
const MASK_EFFECT_BLEND_WIDTH: f32 = 58.0;
const MASK_EFFECT_OPACITY_WIDTH: f32 = 40.0;

#[derive(Clone, Copy)]
struct LevelsPicker {
    doc: u64,
    layer: u64,
    index: usize,
    point: u8,
    tool: crate::state::Tool,
}

fn levels_picker_id() -> egui::Id {
    egui::Id::unique("uv-mask-levels-picker")
}

pub(crate) fn levels_picker_active(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<LevelsPicker>(levels_picker_id())).is_some_and(|picker| {
        picker.tool == app.ui.tool
            && app.session.active().is_some_and(|st| {
                st.doc.id.0 == picker.doc
                    && st.active_layer.is_some_and(|layer| layer.0 == picker.layer)
                    && selected_effect(ctx, picker.doc, picker.layer) == Some(picker.index)
            })
    })
}

fn picker_point(app: &PhotocraftApp, ctx: &egui::Context) -> Option<LevelsPicker> {
    ctx.data(|data| data.get_temp::<LevelsPicker>(levels_picker_id())).filter(|picker| {
        picker.tool == app.ui.tool
            && app.session.active().is_some_and(|st| {
                st.doc.id.0 == picker.doc
                    && st.active_layer.is_some_and(|layer| layer.0 == picker.layer)
                    && selected_effect(ctx, picker.doc, picker.layer) == Some(picker.index)
            })
    })
}

/// Apply one bounded grayscale sample from the Levels input prefix.
pub(crate) fn sample_levels_at(app: &mut PhotocraftApp, ctx: &egui::Context, point: [f64; 2]) -> bool {
    sample_levels_at_inner(app, ctx, point)
}

pub(crate) fn sample_levels_at_uv(app: &mut PhotocraftApp, ctx: &egui::Context, material: usize, uv: [f64; 2]) -> bool {
    let Some(picker) = picker_point(app, ctx) else { return false };
    let Some(st) = app.session.active() else { return false };
    if uv.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return true;
    }
    let channel = app.session.uv.current_channel();
    if app.session.uv.map_bindings(channel).get(material).copied().flatten() != Some(st.doc.id) {
        return true;
    }
    let point = [uv[0] * f64::from(st.doc.size.width), (1.0 - uv[1]) * f64::from(st.doc.size.height)];
    let _ = picker;
    sample_levels_at_inner(app, ctx, point)
}

fn sample_levels_at_inner(app: &mut PhotocraftApp, ctx: &egui::Context, point: [f64; 2]) -> bool {
    let Some(picker) = picker_point(app, ctx) else { return false };
    let Some(st) = app.session.active() else { return false };
    if !point[0].is_finite()
        || !point[1].is_finite()
        || point[0] < 0.0
        || point[1] < 0.0
        || point[0] >= f64::from(st.doc.size.width)
        || point[1] >= f64::from(st.doc.size.height)
    {
        return true;
    }
    let Some(stack) = st.doc.layer(photocraft_doc::LayerId(picker.layer)).and_then(|layer| layer.mask.as_ref()).and_then(|mask| mask.paint.as_ref()) else {
        return false;
    };
    let (x, y) = (point[0].floor() as i32, point[1].floor() as i32);
    let Some(sample) =
        photocraft_compose::mask_paint::compose_prefix(stack, picker.index, photocraft_geom::Rect::new(x, y, x.saturating_add(1), y.saturating_add(1)))
            .and_then(|values| values.first().copied())
    else {
        return true;
    };
    let Some(effect) = stack.layers.get(picker.index) else { return true };
    let EffectKind::Levels { input_black, input_white, .. } = &effect.kind else { return true };
    let sample = sample.clamp(0.0, 1.0);
    let params = match picker.point {
        0 => json!({"layer":picker.layer,"index":picker.index,"input_black":sample.min(*input_white - 0.002)}),
        1 => {
            let t = ((sample - *input_black) / (*input_white - *input_black).max(0.002)).clamp(0.01, 0.99);
            let next_gamma = (t.ln() / 0.5f32.ln()).clamp(0.01, 9.99);
            json!({"layer":picker.layer,"index":picker.index,"gamma":next_gamma})
        }
        _ => json!({"layer":picker.layer,"index":picker.index,"input_white":sample.max(*input_black + 0.002)}),
    };
    if let Err(error) = app.run("uv.maskPaint.modify", params) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    true
}

pub(crate) fn prune_levels_picker(app: &PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) || !levels_picker_active(app, ctx) {
        ctx.data_mut(|data| data.remove::<LevelsPicker>(levels_picker_id()));
    }
}

fn effect_selection_id(doc: u64, layer: u64) -> egui::Id {
    egui::Id::unique(("uv-mask-effect-selection", doc, layer))
}

fn expanded_effects_id() -> egui::Id {
    egui::Id::unique("uv-mask-paint-expanded-layer")
}

/// Open the selected layer's mask stack without switching the canvas to mask display.
pub(crate) fn expand_effects(ctx: &egui::Context, doc: u64, layer: u64) {
    ctx.data_mut(|data| data.insert_temp(expanded_effects_id(), (doc, layer)));
}

/// Collapse the mask stack when the user returns to the layer's image thumbnail/row.
pub(crate) fn collapse_effects(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove_temp::<(u64, u64)>(expanded_effects_id()));
}

fn effects_expanded(ctx: &egui::Context, doc: u64, layer: u64) -> bool {
    ctx.data(|data| data.get_temp::<(u64, u64)>(expanded_effects_id())) == Some((doc, layer))
}

pub(crate) fn clear_effect_selection(ctx: &egui::Context, doc: u64, layer: u64) {
    ctx.data_mut(|data| data.remove::<usize>(effect_selection_id(doc, layer)));
}

pub(crate) fn select_effect(ctx: &egui::Context, doc: u64, layer: u64, index: usize) {
    ctx.data_mut(|data| data.insert_temp(effect_selection_id(doc, layer), index));
}

pub(crate) fn selected_effect(ctx: &egui::Context, doc: u64, layer: u64) -> Option<usize> {
    ctx.data_mut(|data| data.get_temp::<usize>(effect_selection_id(doc, layer)))
}

#[derive(Clone, Copy)]
struct EffectDrag {
    doc: u64,
    layer: u64,
    index: usize,
}

#[derive(Clone, Copy)]
struct PendingEffectClick {
    doc: u64,
    layer: u64,
    index: usize,
    at: f64,
    alt: bool,
}

fn effect_drag_id() -> egui::Id {
    egui::Id::unique("uv-mask-effect-drag")
}

fn pending_effect_click_id() -> egui::Id {
    egui::Id::unique("uv-mask-effect-click")
}

fn effect_rename_id(doc: u64, layer: u64, index: usize) -> egui::Id {
    egui::Id::unique(("uv-mask-effect-rename", doc, layer, index))
}

fn queue_effect_selection(app: &PhotocraftApp, layer: photocraft_doc::LayerId, index: usize, alt: bool, actions: &mut Vec<(String, Value)>) {
    let already_showing_this_mask = app
        .session
        .active()
        .and_then(photocraft_engine::mask_view_cmds::current)
        .is_some_and(|view| view.layer == layer && view.mode == photocraft_engine::mask_view_cmds::MaskViewMode::Gray);
    let show_mask = gray_view_after_effect_click(already_showing_this_mask, alt);
    actions.push(("ui.maskTarget".into(), json!({"enabled":true,"show":show_mask})));
    actions.push(("uv.maskPaint.select".into(), json!({"layer":layer.0,"index":index})));
}

/// Convert a visual insertion boundary to the final engine index after removing the source row.
fn effect_drop_index(count: usize, source: usize, target: usize, above: bool) -> Option<usize> {
    if count == 0 || source >= count || target >= count {
        return None;
    }
    // Rows are displayed in reverse stack order. Inserting above a visual row means inserting
    // after its engine index; below means before it. The engine's `to` parameter is the final
    // index after removing the source row, so shift the boundary left when the source precedes it.
    let boundary = if above { target + 1 } else { target };
    Some(if source < boundary { boundary - 1 } else { boundary }.min(count - 1))
}

/// Mirror the engine's active mask-effect identity into the Properties selection after add/select.
pub(crate) fn show_active_effect_properties(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let Some((doc, layer, index)) = app.session.active().and_then(|state| {
        let layer = state.active_layer?;
        let index = state.doc.layer(layer)?.mask.as_ref()?.paint.as_ref()?.active;
        Some((state.doc.id.0, layer.0, index))
    }) else {
        return false;
    };
    select_effect(ctx, doc, layer, index);
    app.ui.mask_target = true;
    app.ui.vector_mask_target = false;
    app.ui.filter_mask_target = None;
    app.sync_mask_targets();
    app.ui.dock_tabs.properties = 0;
    if app.ui.docking.is_some() {
        if let Some(Err(error)) = crate::panel_docking::command(app, "window.panel.activate", &json!({"panel":"properties"})) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    } else {
        // The legacy dock needs its tab enabled as well as the panel group shown. `reveal`
        // handles both and expands a collapsed group; custom docks use the activate command above.
        crate::dock::reveal(app, crate::dock::Group::Properties);
    }
    true
}

fn effect_is_selected(ctx: &egui::Context, doc: u64, layer: u64, index: usize) -> bool {
    selected_effect(ctx, doc, layer) == Some(index)
}

pub(crate) fn rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer, actions: &mut Vec<(String, Value)>) -> bool {
    let Some(doc_id) = app.session.active().map(|st| st.doc.id.0) else { return false };
    if app.ui.workspace != "3D" || !effects_expanded(ui.ctx(), doc_id, layer.id.0) || app.session.active().and_then(|s| s.active_layer) != Some(layer.id) {
        return false;
    }
    let Some(stack) = layer.mask.as_ref().and_then(|m| m.paint.as_ref()) else { return false };
    let tokens = Tokens::get(ui.ctx());
    if let Some(pending) = ui.ctx().data(|data| data.get_temp::<PendingEffectClick>(pending_effect_click_id())) {
        let now = ui.input(|input| input.time);
        if pending.doc != doc_id || pending.layer != layer.id.0 {
            ui.ctx().data_mut(|data| data.remove::<PendingEffectClick>(pending_effect_click_id()));
        } else if now - pending.at > 0.30 {
            queue_effect_selection(app, layer.id, pending.index, pending.alt, actions);
            ui.ctx().data_mut(|data| data.remove::<PendingEffectClick>(pending_effect_click_id()));
        }
    }
    let mut row_rects = Vec::with_capacity(stack.layers.len());
    for (index, paint) in stack.layers.iter().enumerate().rev() {
        ui.push_id(("mask-paint", layer.id.0, index), |ui| {
            let row_bounds = ui.available_rect_before_wrap().intersect(ui.clip_rect());
            let response = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    let has_blend = matches!(&paint.kind, EffectKind::Paint | EffectKind::Fill { .. } | EffectKind::Blur { .. });
                    let blend_width = if has_blend { MASK_EFFECT_BLEND_WIDTH } else { 0.0 };
                    // Compact controls stay readable while leaving the name the remaining width.
                    let opacity_width = MASK_EFFECT_OPACITY_WIDTH;
                    let delete_width = 16.0;
                    let spacing = ui.spacing().item_spacing.x;
                    let gaps = (if has_blend { 6.0 } else { 5.0 }) * spacing;
                    // Squeeze the child indent first on narrow panels, then give the name every
                    // remaining pixel between its icon and Blend. The small reserve covers rounding.
                    let available_width = ui.available_width().min(row_bounds.width());
                    let fixed_width = 16.0 + 14.0 + blend_width + opacity_width + delete_width + gaps + 2.0;
                    let indent = MASK_EFFECT_ROW_INDENT.min((available_width - fixed_width - 24.0).max(0.0));
                    let name_width = (available_width - fixed_width - indent).max(0.0);
                    ui.add_space(indent);
                    let (eye, eye_response) = ui.allocate_exact_size(egui::vec2(16.0, 20.0), egui::Sense::click());
                    crate::icons::paint(
                        ui,
                        eye,
                        if paint.visible { "eye" } else { "eye-off" },
                        12.0,
                        if paint.visible { tokens.icon } else { tokens.text_faint },
                    );
                    let eye_response = eye_response.on_hover_text(tl!("Visibility"));
                    eye_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Visibility")));
                    if eye_response.clicked() {
                        actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer.id.0,"index":index,"visible":!paint.visible})));
                    }
                    let (icon_rect, icon_response) = ui.allocate_exact_size(egui::vec2(14.0, 20.0), egui::Sense::click());
                    let icon = match &paint.kind {
                        EffectKind::Paint => "brush",
                        EffectKind::Fill { .. } => "paint-bucket",
                        EffectKind::Levels { .. } => "contrast",
                        EffectKind::Blur { .. } => "droplet",
                    };
                    crate::icons::paint(ui, icon_rect, icon, 12.0, tokens.icon);
                    let selected = effect_is_selected(ui.ctx(), doc_id, layer.id.0, index);
                    icon_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Select mask effect")));
                    if icon_response.clicked_by(egui::PointerButton::Primary) {
                        queue_effect_selection(app, layer.id, index, ui.input(|input| input.modifiers.alt), actions);
                    }
                    let rename_id = effect_rename_id(doc_id, layer.id.0, index);
                    let editing_name = ui.ctx().data(|data| data.get_temp::<String>(rename_id));
                    if let Some(mut edited_name) = editing_name {
                        let input_id = rename_id.with("input");
                        let focus_id = rename_id.with("focus-on-entry");
                        let response = ui
                            .add_sized(
                                egui::vec2(name_width, 20.0),
                                egui::TextEdit::singleline(&mut edited_name).id(input_id).desired_width(name_width).char_limit(128),
                            )
                            .accessible_name(tl!("Name"));
                        if ui.ctx().data_mut(|data| data.remove_temp::<bool>(focus_id).unwrap_or(false)) {
                            response.request_focus();
                            crate::field_tab::select_all(ui.ctx(), input_id, &edited_name);
                        }
                        if response.changed() {
                            ui.ctx().data_mut(|data| data.insert_temp(rename_id, edited_name.clone()));
                        }
                        let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
                        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                        if escape {
                            ui.ctx().data_mut(|data| {
                                data.remove::<String>(rename_id);
                                data.remove::<bool>(focus_id);
                            });
                        } else if enter || response.lost_focus() {
                            if edited_name != paint.name {
                                actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer.id.0,"index":index,"name":edited_name})));
                            }
                            ui.ctx().data_mut(|data| {
                                data.remove::<String>(rename_id);
                                data.remove::<bool>(focus_id);
                            });
                        }
                    } else {
                        let name = ui
                            .add_sized(
                                egui::vec2(name_width, 20.0),
                                egui::Button::selectable(selected, &paint.name).truncate().sense(egui::Sense::click_and_drag()),
                            )
                            .on_hover_text(&paint.name);
                        if name.double_clicked_by(egui::PointerButton::Primary) {
                            ui.ctx().data_mut(|data| {
                                data.remove::<PendingEffectClick>(pending_effect_click_id());
                                data.insert_temp(rename_id, paint.name.clone());
                                data.insert_temp(rename_id.with("focus-on-entry"), true);
                            });
                            ui.ctx().memory_mut(|memory| memory.request_focus(rename_id.with("input")));
                        } else if name.drag_started_by(egui::PointerButton::Primary) {
                            ui.ctx().data_mut(|data| {
                                data.remove::<PendingEffectClick>(pending_effect_click_id());
                                data.insert_temp(effect_drag_id(), EffectDrag { doc: doc_id, layer: layer.id.0, index });
                            });
                        } else if name.clicked_by(egui::PointerButton::Primary) {
                            let now = ui.input(|input| input.time);
                            let alt = ui.input(|input| input.modifiers.alt);
                            let pending = ui.ctx().data(|data| data.get_temp::<PendingEffectClick>(pending_effect_click_id()));
                            ui.ctx().data_mut(|data| data.remove::<PendingEffectClick>(pending_effect_click_id()));
                            if let Some(pending) = pending
                                && (pending.doc != doc_id || pending.layer != layer.id.0 || pending.index != index || now - pending.at > 0.30)
                            {
                                queue_effect_selection(app, photocraft_doc::LayerId(pending.layer), pending.index, pending.alt, actions);
                            }
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(pending_effect_click_id(), PendingEffectClick { doc: doc_id, layer: layer.id.0, index, at: now, alt })
                            });
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(310));
                        }
                        name.context_menu(|ui| {
                            add_menu(app, ui, layer.id.0, actions);
                            ui.separator();
                            for (label, to, enabled) in [
                                (tl!("Move Up"), index.saturating_add(1), index + 1 < stack.layers.len()),
                                (tl!("Move Down"), index.saturating_sub(1), index > 0),
                            ] {
                                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                                    actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer.id.0,"index":index,"to":to})));
                                    ui.close();
                                }
                            }
                        });
                    }
                    if has_blend {
                        blend_selector(ui, doc_id, layer.id.0, index, paint.blend, actions);
                    }
                    opacity_popup(app, ui, layer.id, index, paint.opacity, actions);
                    delete_effect_button(ui, layer.id.0, index, &paint.kind, actions);
                })
                .response;
            if matches!(&paint.kind, EffectKind::Fill { .. }) {
                let drop_rect =
                    egui::Rect::from_min_max(egui::pos2(row_bounds.left(), response.rect.top()), egui::pos2(row_bounds.right(), response.rect.bottom()));
                app.uv_cache.library.fill_targets.push((drop_rect, doc_id, layer.id.0, index));
            }
            row_rects.push((index, response.rect));
            ui.painter().hline(response.rect.x_range(), response.rect.bottom(), egui::Stroke::new(0.5, tokens.text_faint));
        });
    }
    let drag = ui.ctx().data(|data| data.get_temp::<EffectDrag>(effect_drag_id()));
    if let Some(drag) = drag.filter(|drag| drag.doc == doc_id && drag.layer == layer.id.0) {
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let released = ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary));
        if let Some(pointer) = pointer {
            if let Some((target, rect)) = row_rects.iter().find(|(_, rect)| rect.contains(pointer)) {
                let above = pointer.y < rect.center().y;
                let to = effect_drop_index(stack.layers.len(), drag.index, *target, above);
                let line_y = if above { rect.top() } else { rect.bottom() };
                ui.painter().hline(rect.x_range(), line_y, egui::Stroke::new(2.0, tokens.accent));
                if let Some(to) = to.filter(|to| released && *to != drag.index) {
                    actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer.id.0,"index":drag.index,"to":to})));
                }
            }
        }
        if released {
            ui.ctx().data_mut(|data| data.remove::<EffectDrag>(effect_drag_id()));
        }
    }
    !stack.layers.is_empty()
}

fn delete_effect_label(kind: &EffectKind) -> &'static str {
    match kind {
        EffectKind::Paint => tl!("Delete Paint layer"),
        EffectKind::Fill { .. } => tl!("Delete Fill layer"),
        EffectKind::Levels { .. } => tl!("Delete Levels layer"),
        EffectKind::Blur { .. } => tl!("Delete Blur layer"),
    }
}

fn delete_effect_button(ui: &mut egui::Ui, layer: u64, index: usize, kind: &EffectKind, actions: &mut Vec<(String, Value)>) {
    let label = delete_effect_label(kind);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, 20.0), egui::Sense::click());
    crate::icons::paint(ui, rect, "trash", 12.0, Tokens::get(ui.ctx()).text_dim);
    let response = response.on_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if response.clicked() {
        actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer,"index":index,"remove":true})));
    }
}

fn opacity_popup(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: photocraft_doc::LayerId, index: usize, canonical: f32, actions: &mut Vec<(String, Value)>) {
    let value = crate::uv_mask_paint_preview::value(app, layer, index, canonical);
    let (button_rect, button) = ui.allocate_exact_size(egui::vec2(MASK_EFFECT_OPACITY_WIDTH, 20.0), egui::Sense::click());
    let button = button.on_hover_text(tl!("Opacity"));
    let visuals = ui.style().interact(&button);
    ui.painter().rect_filled(button_rect, visuals.corner_radius, visuals.bg_fill);
    ui.painter().rect_stroke(button_rect, visuals.corner_radius, visuals.bg_stroke, egui::StrokeKind::Inside);
    ui.painter().text(
        button_rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{:.0}%", value * 100.0),
        egui::TextStyle::Button.resolve(ui.style()),
        visuals.fg_stroke.color,
    );
    button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Opacity")));
    let mut released = false;
    let popup = egui::Popup::from_toggle_button_response(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(144.0);
        ui.label(tl!("Opacity"));
        let current = crate::uv_mask_paint_preview::value(app, layer, index, canonical);
        let mut opacity = current * 100.0;
        let response = crate::widgets::slider(ui, &mut opacity, 0.0..=100.0, None);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, tl!("Opacity")));
        if response.changed() {
            crate::uv_mask_paint_preview::set(app, layer, index, opacity / 100.0);
            ui.ctx().request_repaint();
        }
        if response.drag_stopped() || (response.changed() && !response.dragged()) {
            released = true;
        }
    });
    let pending = crate::uv_mask_paint_preview::pending(app).filter(|commit| commit.layer == layer && commit.index == index);
    if (released || (popup.is_none() && pending.is_some()))
        && let Some(commit) = pending
        && (commit.opacity - canonical).abs() > f32::EPSILON
    {
        actions.push(("uv.maskPaint.modify".into(), commit.params()));
    }
}

/// Levels mask effects use the familiar Properties tab instead of expanding controls in Layers.
pub(crate) fn levels_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    if app.ui.workspace != "3D" {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return false };
    let Some(stack) = layer.mask.as_ref().and_then(|mask| mask.paint.as_ref()) else { return false };
    let Some(index) = selected_effect(ui.ctx(), st.doc.id.0, layer.id.0).filter(|index| *index == stack.active) else { return false };
    let Some(effect) = stack.layers.get(index) else { return false };
    let EffectKind::Levels { input_black, gamma, input_white, output_black, output_white } = &effect.kind else { return false };
    let (doc_id, revision, size) = (st.doc.id.0, st.revision, st.doc.size);
    let (layer_id, index) = (layer.id, index);
    let effect_name = effect.name.clone();
    let mut visible = effect.visible;
    let (input_black, gamma, input_white, output_black, output_white) = (*input_black, *gamma, *input_white, *output_black, *output_white);
    let canonical = serde_json::json!({
        "inBlack": (input_black * 255.0).round(),
        "gamma": gamma,
        "inWhite": (input_white * 255.0).round(),
        "outBlack": (output_black * 255.0).round(),
        "outWhite": (output_white * 255.0).round(),
    });
    let values_id = egui::Id::unique(("mask-levels-values", doc_id, layer_id.0, index));
    let mut values = ui
        .ctx()
        .data(|data| data.get_temp::<(u64, Value)>(values_id))
        .filter(|(cached_revision, _)| *cached_revision == revision)
        .map_or(canonical, |(_, values)| values);
    let hist = mask_levels_histogram(ui.ctx(), doc_id, revision, layer_id.0, index, stack, size);
    let cx = crate::adjust_editors::EditorCx {
        mem: egui::Id::unique(("mask-levels", doc_id, layer_id.0, index)),
        hist: Some(hist),
        gray: true,
        swatches: [[0.0; 3]; 2],
        gradients: Vec::new(),
        dialog: false,
        dynamic_color_sliders: false,
        isolated_curve: None,
    };
    let mut actions: Vec<(String, Value)> = Vec::new();
    let mut arm_picker = None;
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
        crate::icons::paint(ui, icon_rect, "contrast", 16.0, Tokens::get(ui.ctx()).icon);
        ui.strong(&effect_name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(tl!("Levels"));
        });
    });
    ui.horizontal(|ui| {
        let current =
            picker_point(app, ui.ctx()).filter(|picker| picker.doc == doc_id && picker.layer == layer_id.0 && picker.index == index).map(|picker| picker.point);
        let choices = [
            (0, tl!("Set Black Point"), egui::Color32::BLACK),
            (1, tl!("Set Neutral Gray Point"), egui::Color32::from_gray(128)),
            (2, tl!("Set White Point"), egui::Color32::WHITE),
        ];
        for (point, label, color) in choices {
            if crate::adjust_dialog::picker_button(ui, current == Some(point), true, label, color).clicked() {
                if current == Some(point) {
                    ui.ctx().data_mut(|data| data.remove::<LevelsPicker>(levels_picker_id()));
                } else {
                    arm_picker = Some(point);
                }
            }
        }
    });
    ui.add_space(8.0);
    let edit = crate::adjust_editors::editor(ui, "levels", &mut values, &cx);
    if edit.changed && !edit.commit {
        ui.ctx().data_mut(|data| data.insert_temp(values_id, (revision, values.clone())));
    }
    if edit.commit {
        let scale = 1.0 / 255.0;
        let black = values["inBlack"].as_f64().unwrap_or(0.0) as f32 * scale;
        let white = values["inWhite"].as_f64().unwrap_or(255.0) as f32 * scale;
        let out_black = values["outBlack"].as_f64().unwrap_or(0.0) as f32 * scale;
        let out_white = values["outWhite"].as_f64().unwrap_or(255.0) as f32 * scale;
        let next_gamma = values["gamma"].as_f64().unwrap_or(1.0) as f32;
        if black != input_black || white != input_white || out_black != output_black || out_white != output_white || next_gamma != gamma {
            actions.push((
                "uv.maskPaint.modify".to_string(),
                serde_json::json!({
                    "layer":layer_id.0,"index":index,
                    "input_black":black,"gamma":next_gamma,"input_white":white,
                    "output_black":out_black,"output_white":out_white
                }),
            ));
        }
        ui.ctx().data_mut(|data| data.remove::<(u64, Value)>(values_id));
    }
    ui.horizontal(|ui| {
        if ui.checkbox(&mut visible, tl!("Visible")).changed() {
            actions.push(("uv.maskPaint.modify".to_string(), serde_json::json!({"layer":layer_id.0,"index":index,"visible":visible})));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Reset")).clicked() {
                ui.ctx().data_mut(|data| data.remove::<(u64, Value)>(values_id));
                actions.push((
                    "uv.maskPaint.modify".to_string(),
                    serde_json::json!({"layer":layer_id.0,"index":index,"input_black":0.0,"gamma":1.0,"input_white":1.0,"output_black":0.0,"output_white":1.0}),
                ));
            }
        });
    });
    if edit.changed {
        ui.ctx().request_repaint();
    }
    for (command, params) in actions {
        if let Err(error) = app.run(&command, params) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
    if let Some(point) = arm_picker
        && (!app.ui.uv_workbench.select_shells || crate::uv_ui::commit_current_polygon_selection(app))
    {
        app.ui.uv_workbench.select_shells = false;
        ui.ctx().data_mut(|data| data.insert_temp(levels_picker_id(), LevelsPicker { doc: doc_id, layer: layer_id.0, index, point, tool: app.ui.tool }));
    }
    true
}

/// A small, distributed histogram of the editable prefix before the selected Levels layer.
/// Sampling 32×32 spread-out 4×4 patches keeps this useful on 4k masks without recompositing the
/// full mask on every Properties frame. The cached entry is replaced after any document revision.
fn mask_levels_histogram(
    ctx: &egui::Context,
    doc: u64,
    revision: u64,
    layer: u64,
    index: usize,
    stack: &photocraft_doc::mask_paint::Stack,
    size: photocraft_doc::Size,
) -> std::sync::Arc<crate::tone::Histograms> {
    let id = egui::Id::unique(("mask-levels-histogram", doc, layer, index));
    if let Some((cached_revision, cached)) = ctx.data(|data| data.get_temp::<(u64, std::sync::Arc<crate::tone::Histograms>)>(id))
        && cached_revision == revision
    {
        return cached;
    }
    let mut bins = [0u32; 256];
    let (width, height) = (size.width as i32, size.height as i32);
    for gy in 0..32 {
        for gx in 0..32 {
            let x = (gx * width / 32).min((width - 1).max(0));
            let y = (gy * height / 32).min((height - 1).max(0));
            let rect = photocraft_geom::Rect::new(x, y, (x + 4).min(width), (y + 4).min(height));
            if let Some(values) = photocraft_compose::mask_paint::compose_prefix(stack, index, rect) {
                for value in values {
                    let bin = (value.clamp(0.0, 1.0) * 255.0).round() as usize;
                    bins[bin] = bins[bin].saturating_add(1);
                }
            }
        }
    }
    let cached = std::sync::Arc::new(crate::tone::Histograms { tag: doc ^ revision ^ layer ^ index as u64, ch: vec![bins], ..Default::default() });
    ctx.data_mut(|data| data.insert_temp(id, (revision, cached.clone())));
    cached
}

/// Fill settings live in Properties; the rectangle itself is edited directly on the 2D canvas.
pub(crate) fn fill_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    if app.ui.workspace != "3D" {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return false };
    let Some(stack) = layer.mask.as_ref().and_then(|mask| mask.paint.as_ref()) else { return false };
    let Some(index) = selected_effect(ui.ctx(), st.doc.id.0, layer.id.0).filter(|index| *index == stack.active) else { return false };
    let Some(effect) = stack.layers.get(index) else { return false };
    let EffectKind::Fill { value, rotation, repeat_u, repeat_v, mirror_u, mirror_v } = &effect.kind else { return false };
    let layer_id = layer.id.0;
    let (_, rotation) = crate::uv_mask_paint_preview::fill_transform(
        app,
        layer.id,
        index,
        effect.fill_bounds.unwrap_or([0, 0, st.doc.size.width as i32, st.doc.size.height as i32]),
        *rotation,
    );
    let mut actions = Vec::new();
    ui.heading(&effect.name);
    ui.label(tl!("Fill"));
    gradient_parameter(ui, st.doc.id.0, st.revision, layer_id, index, *value, &mut actions);
    edit_parameter_speed(ui, st.doc.id.0, st.revision, layer_id, index, "rotation", tl!("Rotation"), rotation, -360.0..=360.0, 1.0, &mut actions);
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        let mut next = *repeat_u;
        if ui.checkbox(&mut next, tl!("Repeat U")).changed() {
            actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer_id,"index":index,"repeat_u":next})));
        }
        let mut next = *repeat_v;
        if ui.checkbox(&mut next, tl!("Repeat V")).changed() {
            actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer_id,"index":index,"repeat_v":next})));
        }
    });
    ui.horizontal_wrapped(|ui| {
        let mut next = *mirror_u;
        if ui.checkbox(&mut next, tl!("Mirror U")).changed() {
            actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer_id,"index":index,"mirror_u":next})));
        }
        let mut next = *mirror_v;
        if ui.checkbox(&mut next, tl!("Mirror V")).changed() {
            actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer_id,"index":index,"mirror_v":next})));
        }
    });
    ui.add_space(6.0);
    if let Some(map) = effect.fill_map.as_ref() {
        let [source_w, source_h] = effect.fill_map_size;
        if source_w > 0 && source_h > 0 {
            ui.add_space(6.0);
            let preview_w = ui.available_width().clamp(1.0, 120.0);
            let preview_h = (preview_w * (source_h as f32 / source_w as f32)).clamp(1.0, 72.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(preview_w, preview_h), egui::Sense::hover());
            let tex_w = 120usize;
            let tex_h = ((source_h as f64 / source_w as f64 * tex_w as f64).round() as usize).clamp(1, 72);
            let texture_id = egui::Id::unique(("uv-fill-map-preview", st.doc.id.0, layer_id, index));
            let cached = ui.ctx().data_mut(|data| data.get_temp::<(u64, egui::TextureHandle)>(texture_id));
            let handle = if let Some((_, handle)) = cached.filter(|(revision, _)| *revision == st.revision) {
                handle
            } else {
                let pixels = (0..tex_h)
                    .flat_map(|y| (0..tex_w).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        let sx = ((x as u64 * u64::from(source_w)) / tex_w as u64).min(u64::from(source_w - 1)) as i32;
                        let sy = ((y as u64 * u64::from(source_h)) / tex_h as u64).min(u64::from(source_h - 1)) as i32;
                        egui::Color32::from_gray((map.sample_channel(sx, sy, 0).clamp(0.0, 1.0) * 255.0).round() as u8)
                    })
                    .collect();
                let image = egui::ColorImage::new([tex_w, tex_h], pixels);
                let handle = ui.ctx().load_texture(format!("uv-fill-map-{}-{layer_id}-{index}", st.doc.id.0), image, egui::TextureOptions::LINEAR);
                ui.ctx().data_mut(|data| data.insert_temp(texture_id, (st.revision, handle.clone())));
                handle
            };
            ui.painter().image(handle.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        if ui.button(tl!("Remove image")).clicked() {
            actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer_id,"index":index,"clear_fill_map":true})));
        }
    } else {
        ui.weak(tl!("Uniform grayscale Fill"));
    }
    for (command, params) in actions {
        if let Err(error) = app.run(&command, params) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
    true
}

/// Paint and Blur use Properties for their effect-specific settings; shared blend and opacity
/// controls live in their Layers rows.
pub(crate) fn simple_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    if app.ui.workspace != "3D" {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return false };
    let Some(stack) = layer.mask.as_ref().and_then(|mask| mask.paint.as_ref()) else { return false };
    let Some(index) = selected_effect(ui.ctx(), st.doc.id.0, layer.id.0).filter(|index| *index == stack.active) else { return false };
    let Some(effect) = stack.layers.get(index) else { return false };
    if !matches!(&effect.kind, EffectKind::Paint | EffectKind::Blur { .. }) {
        return false;
    }
    let layer_id = layer.id.0;
    ui.heading(&effect.name);
    ui.label(if matches!(&effect.kind, EffectKind::Paint) { tl!("Paint") } else { tl!("Blur") });
    let mut actions: Vec<(String, Value)> = Vec::new();
    if let EffectKind::Blur { radius } = &effect.kind {
        let canonical_radius = f32::from(*radius);
        let value_id = egui::Id::unique(("mask-effect-radius", st.doc.id.0, layer_id, index));
        let mut next_radius = ui.ctx().data_mut(|data| data.get_temp::<f32>(value_id).unwrap_or(canonical_radius));
        let response = crate::widgets::slider_row(ui, tl!("Radius"), &mut next_radius, 1.0..=64.0, "px", None);
        if response.changed() {
            ui.ctx().data_mut(|data| data.insert_temp(value_id, next_radius));
        }
        if response.drag_stopped() || (response.changed() && !response.dragged()) {
            let staged = ui.ctx().data_mut(|data| {
                let staged = data.get_temp::<f32>(value_id).unwrap_or(next_radius);
                data.remove::<f32>(value_id);
                staged
            });
            if staged.round() != canonical_radius {
                actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer_id,"index":index,"radius":staged.round() as u8})));
            }
        }
    }
    for (command, params) in actions {
        if let Err(error) = app.run(&command, params) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
    true
}

fn mask_blend_label(mode: photocraft_color::BlendMode) -> &'static str {
    match mode {
        photocraft_color::BlendMode::Normal => "Normal",
        photocraft_color::BlendMode::Darken => "Darken",
        photocraft_color::BlendMode::Multiply => "Multiply",
        photocraft_color::BlendMode::Lighten => "Lighten",
        photocraft_color::BlendMode::Screen => "Screen",
        photocraft_color::BlendMode::LinearDodge => "Linear Dodge",
        photocraft_color::BlendMode::Subtract => "Subtract",
        photocraft_color::BlendMode::Difference => "Difference",
        _ => "Legacy blend mode",
    }
}

fn mask_blend_button_label(mode: photocraft_color::BlendMode) -> &'static str {
    match mode {
        photocraft_color::BlendMode::Normal => "Nrm",
        photocraft_color::BlendMode::Darken => "Drk",
        photocraft_color::BlendMode::Multiply => "Mul",
        photocraft_color::BlendMode::Lighten => "Lit",
        photocraft_color::BlendMode::Screen => "Scrn",
        photocraft_color::BlendMode::LinearDodge => "Lin+",
        photocraft_color::BlendMode::Subtract => "Sub",
        photocraft_color::BlendMode::Difference => "Diff",
        _ => "Legacy",
    }
}

fn mask_blend_command(mode: photocraft_color::BlendMode) -> Option<&'static str> {
    Some(match mode {
        photocraft_color::BlendMode::Normal => "normal",
        photocraft_color::BlendMode::Darken => "darken",
        photocraft_color::BlendMode::Multiply => "multiply",
        photocraft_color::BlendMode::Lighten => "lighten",
        photocraft_color::BlendMode::Screen => "screen",
        photocraft_color::BlendMode::LinearDodge => "linear_dodge",
        photocraft_color::BlendMode::Subtract => "subtract",
        photocraft_color::BlendMode::Difference => "difference",
        _ => return None,
    })
}

fn blend_selector(ui: &mut egui::Ui, doc: u64, layer: u64, index: usize, canonical: photocraft_color::BlendMode, actions: &mut Vec<(String, Value)>) {
    use photocraft_doc::mask_paint::MASK_BLEND_MODES;
    let _ = doc;
    let mut selected = mask_blend_command(canonical).unwrap_or("legacy").to_owned();
    let previous = selected.clone();
    let selected_label = MASK_BLEND_MODES
        .iter()
        .copied()
        .find(|mode| mask_blend_command(*mode) == Some(selected.as_str()))
        .map(mask_blend_button_label)
        .unwrap_or_else(|| mask_blend_button_label(canonical));
    let full_selected_label = MASK_BLEND_MODES
        .iter()
        .copied()
        .find(|mode| mask_blend_command(*mode) == Some(selected.as_str()))
        .map(mask_blend_label)
        .unwrap_or_else(|| mask_blend_label(canonical));
    let (button_rect, response) = ui.allocate_exact_size(egui::vec2(MASK_EFFECT_BLEND_WIDTH, 20.0), egui::Sense::click());
    let response = response.on_hover_text(tl!(full_selected_label));
    let visuals = ui.style().interact(&response);
    ui.painter().rect_filled(button_rect, visuals.corner_radius, visuals.bg_fill);
    ui.painter().rect_stroke(button_rect, visuals.corner_radius, visuals.bg_stroke, egui::StrokeKind::Inside);
    ui.painter().text(
        egui::pos2(button_rect.left() + 5.0, button_rect.center().y),
        egui::Align2::LEFT_CENTER,
        tl!(selected_label),
        egui::TextStyle::Button.resolve(ui.style()),
        visuals.fg_stroke.color,
    );
    crate::icons::paint(
        ui,
        egui::Rect::from_center_size(egui::pos2(button_rect.right() - 8.0, button_rect.center().y), egui::vec2(9.0, 9.0)),
        "chevron-down",
        9.0,
        visuals.fg_stroke.color,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tl!("Blend mode")));
    let mut picked = None;
    egui::Popup::from_toggle_button_response(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(140.0);
        for mode in MASK_BLEND_MODES {
            if let Some(command) = mask_blend_command(mode)
                && ui.selectable_label(selected == command, tl!(mask_blend_label(mode))).clicked()
            {
                picked = Some(command.to_owned());
                ui.close();
            }
        }
    });
    if let Some(value) = picked {
        selected = value;
    }
    let picked = MASK_BLEND_MODES.iter().copied().find(|mode| mask_blend_command(*mode) == Some(selected.as_str()));
    if selected != previous
        && let Some(mode) = picked
    {
        if let Some(value) = mask_blend_command(mode) {
            actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer,"index":index,"blend":value})));
        }
    }
}

/// Black-to-white source-value control. It stages pointer movement in UI memory and commits one
/// mask recomposition after release, just like the numeric controls above.
fn gradient_parameter(ui: &mut egui::Ui, doc: u64, revision: u64, layer: u64, index: usize, canonical: f32, actions: &mut Vec<(String, Value)>) {
    let id = egui::Id::unique(("mask-effect-param", doc, revision, layer, index, "value"));
    let mut value = ui.ctx().data_mut(|data| data.get_temp::<f32>(id).unwrap_or(canonical));
    ui.horizontal(|ui| {
        ui.label(tl!("Gray value"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(format!("{value:.3}"));
        });
    });
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click_and_drag());
    let track = rect.shrink2(egui::vec2(1.0, 3.0));
    let painter = ui.painter();
    let steps = 64;
    for step in 0..steps {
        let x0 = egui::lerp(track.x_range(), step as f32 / steps as f32);
        let x1 = egui::lerp(track.x_range(), (step + 1) as f32 / steps as f32);
        let gray = (step * 255 / (steps - 1)) as u8;
        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(x0, track.top()), egui::pos2(x1 + 0.5, track.bottom())), 0.0, egui::Color32::from_gray(gray));
    }
    painter.rect_stroke(track, 2.0, egui::Stroke::new(1.0, Tokens::get(ui.ctx()).text_faint), egui::StrokeKind::Inside);
    let pointer_active = response.drag_started_by(egui::PointerButton::Primary)
        || response.dragged_by(egui::PointerButton::Primary)
        || response.drag_stopped_by(egui::PointerButton::Primary)
        || response.clicked_by(egui::PointerButton::Primary);
    if pointer_active && let Some(pointer) = response.interact_pointer_pos() {
        value = ((pointer.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
        ui.ctx().data_mut(|data| data.insert_temp(id, value));
    }
    let thumb_x = egui::lerp(track.x_range(), value.clamp(0.0, 1.0));
    painter.line_segment(
        [egui::pos2(thumb_x, track.top() - 2.0), egui::pos2(thumb_x, track.bottom() + 2.0)],
        egui::Stroke::new(2.0, Tokens::get(ui.ctx()).accent),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, tl!("Gray value")));
    if response.drag_stopped_by(egui::PointerButton::Primary) || response.clicked_by(egui::PointerButton::Primary) {
        let staged = ui.ctx().data_mut(|data| {
            let staged = data.get_temp::<f32>(id).unwrap_or(value);
            data.remove::<f32>(id);
            staged
        });
        if (staged - canonical).abs() > f32::EPSILON {
            actions.push(("uv.maskPaint.modify".into(), json!({"layer":layer,"index":index,"value":staged})));
        }
    }
}

#[allow(clippy::too_many_arguments)] // The fields form one explicit UI parameter binding.
fn edit_parameter_speed(
    ui: &mut egui::Ui,
    doc: u64,
    revision: u64,
    layer: u64,
    index: usize,
    key: &'static str,
    label: &str,
    canonical: f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f64,
    actions: &mut Vec<(String, Value)>,
) {
    let id = egui::Id::unique(("mask-effect-param", doc, revision, layer, index, key));
    let mut value = ui.ctx().data_mut(|data| data.get_temp::<f32>(id).unwrap_or(canonical));
    let response = ui
        .add(egui::DragValue::new(&mut value).range(range).speed(speed).suffix(if key == "rotation" { "°" } else { "" }).prefix(format!("{label}: ")))
        .on_hover_text(format!("{label} — {}", tl!("Applied when you release the control")));
    if response.changed() {
        ui.ctx().data_mut(|data| data.insert_temp(id, value));
    }
    if response.drag_stopped() || response.lost_focus() {
        let staged = ui.ctx().data_mut(|data| {
            let staged = data.get_temp::<f32>(id).unwrap_or(value);
            data.remove::<f32>(id);
            staged
        });
        if (staged - canonical).abs() > f32::EPSILON {
            let mut params = json!({"layer":layer,"index":index});
            if let Some(object) = params.as_object_mut() {
                object.insert(key.to_owned(), json!(staged));
                actions.push(("uv.maskPaint.modify".into(), params));
            }
        }
    }
}

fn polygon_selection_action(app: &PhotocraftApp, layer: u64) -> Option<(String, Value)> {
    let state = &app.ui.uv_workbench;
    if app.ui.workspace != "3D" || !state.select_shells || state.selection_target != crate::uv_ui::SelectionTarget::Polygons {
        return None;
    }
    let owner = state.polygon_document?;
    let active = app.session.active()?;
    if active.doc.id != owner || active.doc.layer(photocraft_doc::LayerId(layer)).is_none() {
        return None;
    }
    let scene = app.session.uv.scene.as_ref()?;
    let polygons: Vec<_> = state
        .selected_polygons
        .iter()
        .filter_map(|&id| {
            let triangle = scene.triangles.get(id)?;
            Some(triangle.corners.map(|corner| [corner.uv[0] * f64::from(active.doc.size.width), (1.0 - corner.uv[1]) * f64::from(active.doc.size.height)]))
        })
        .collect();
    (!polygons.is_empty()).then(|| ("select.uvPolygons".into(), json!({"document":owner.0,"polygons":polygons})))
}

fn fill_selection_preparation(app: &PhotocraftApp, layer: u64) -> Option<(String, Value)> {
    polygon_selection_action(app, layer).or_else(|| shell_selection_needs_apply(app, layer).then(|| ("uv.mask.apply".into(), json!({}))))
}

fn has_fill_selection(app: &PhotocraftApp, layer: u64) -> bool {
    if has_active_uv_selection(app) {
        fill_selection_preparation(app, layer).is_some()
    } else {
        app.session.active().is_some_and(|active| active.doc.selection.is_some())
    }
}

fn has_active_uv_selection(app: &PhotocraftApp) -> bool {
    let state = &app.ui.uv_workbench;
    app.ui.workspace == "3D"
        && state.select_shells
        && match state.selection_target {
            crate::uv_ui::SelectionTarget::Polygons => !state.selected_polygons.is_empty(),
            crate::uv_ui::SelectionTarget::UvShells => !app.session.uv.selected.is_empty(),
        }
}

fn fill_selection_add_params(layer: u64) -> Value {
    json!({"layer":layer,"kind":"fill","from_selection":true,"repeat_u":false,"repeat_v":false,"name":"Selection Fill"})
}

fn shell_selection_needs_apply(app: &PhotocraftApp, layer: u64) -> bool {
    let state = &app.ui.uv_workbench;
    if app.ui.workspace != "3D"
        || !state.select_shells
        || state.selection_target != crate::uv_ui::SelectionTarget::UvShells
        || app.session.uv.selected.is_empty()
    {
        return false;
    }
    let Some(active) = app.session.active() else { return false };
    if active.doc.layer(photocraft_doc::LayerId(layer)).is_none() {
        return false;
    }
    app.session.uv.map_bindings(app.session.uv.current_channel()).get(app.session.uv.material).copied().flatten() == Some(active.doc.id)
}

pub(crate) fn add_menu(app: &PhotocraftApp, ui: &mut egui::Ui, layer: u64, actions: &mut Vec<(String, Value)>) {
    for (label, kind, icon) in [
        (tl!("Add Paint layer"), "paint", "brush"),
        (tl!("Add Fill layer"), "fill", "paint-bucket"),
        (tl!("Add Levels layer"), "levels", "contrast"),
        (tl!("Add Blur layer"), "blur", "droplet"),
    ] {
        if menu_item(ui, icon, label) {
            actions.push(("layer.select".into(), json!({"layer":layer})));
            actions.push(("uv.maskPaint.add".into(), json!({"layer":layer,"kind":kind})));
            ui.close();
        }
    }
    if menu_item_enabled(ui, "quick-selection", tl!("Fill from selection"), has_fill_selection(app, layer)) {
        actions.push(("layer.select".into(), json!({"layer":layer})));
        if let Some(action) = fill_selection_preparation(app, layer) {
            actions.push(action);
        }
        actions.push(("uv.maskPaint.add".into(), fill_selection_add_params(layer)));
        ui.close();
    }
}

fn menu_item(ui: &mut egui::Ui, icon: &str, label: &str) -> bool {
    menu_item_enabled(ui, icon, label, true)
}

fn menu_item_enabled(ui: &mut egui::Ui, icon: &str, label: &str, enabled: bool) -> bool {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
        crate::icons::paint(ui, rect, icon, 15.0, if enabled { Tokens::get(ui.ctx()).icon } else { Tokens::get(ui.ctx()).text_faint });
        ui.add_enabled(enabled, egui::Button::new(label)).clicked()
    })
    .inner
}

fn gray_view_after_effect_click(already_gray: bool, alt: bool) -> bool {
    if already_gray { !alt } else { alt }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::Role;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use photocraft_color::PixelFormat;
    use photocraft_doc::{
        LayerMask,
        mask_paint::{Paint, Stack},
    };
    use photocraft_raster::Surface;

    #[test]
    fn mask_effect_rows_require_expansion_and_mask_target_has_no_selected_child() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":16,"height":16})).unwrap();
        session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
        let layer_id = session.active().unwrap().active_layer.unwrap();
        let state = session.active_mut().unwrap();
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        let mut mask = LayerMask::hide_all();
        mask.paint = Some(Stack {
            base,
            layers: vec![Paint {
                name: "Fill".into(),
                visible: true,
                opacity: 1.0,
                blend: photocraft_color::BlendMode::Normal,
                surface: Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]),
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: Some([0, 0, 16, 16]),
                kind: EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false },
            }],
            active: 0,
        });
        std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer_id).unwrap().mask = Some(mask);
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.workspace = "3D".into();
        app.ui.mask_target = true;
        let layer = app.session.active().unwrap().doc.layer(layer_id).unwrap().clone();
        let ctx = egui::Context::default();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 120.0))), ..Default::default() };
        let mut visible = false;
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ui| visible = rows(&mut app, ui, &layer, &mut actions));
        output.textures_delta.clear();
        assert!(!visible, "targeting the mask alone keeps its effect stack collapsed");
        expand_effects(&ctx, doc_id(&app), layer.id.0);
        let mut output = ctx.run_ui(Default::default(), |ui| visible = rows(&mut app, ui, &layer, &mut actions));
        output.textures_delta.clear();
        assert!(visible, "expanding the mask exposes its effect rows");
        assert!(actions.is_empty(), "targeting a mask does not select a child effect");
        assert_eq!(selected_effect(&ctx, doc_id(&app), layer.id.0), None);
        collapse_effects(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| visible = rows(&mut app, ui, &layer, &mut actions));
        output.textures_delta.clear();
        assert!(!visible, "returning to layer pixels collapses its effect rows");
    }

    #[test]
    fn targeting_mask_thumbnail_clears_selected_child_effect() {
        let ctx = egui::Context::default();
        let doc = 41;
        let layer = 9;
        ctx.data_mut(|data| data.insert_temp(effect_selection_id(doc, layer), 2usize));
        assert_eq!(selected_effect(&ctx, doc, layer), Some(2));

        // This is the state transition used by the Layers-panel mask-thumbnail handler.
        clear_effect_selection(&ctx, doc, layer);
        assert_eq!(selected_effect(&ctx, doc, layer), None, "the mask is targeted without selecting a child effect");
    }

    #[test]
    fn selecting_another_mask_effect_keeps_gray_view_until_alt_toggles_it_off() {
        assert!(gray_view_after_effect_click(true, false));
        assert!(!gray_view_after_effect_click(true, true));
        assert!(!gray_view_after_effect_click(false, false));
        assert!(gray_view_after_effect_click(false, true));
    }

    #[test]
    fn effect_drag_drop_indices_follow_descending_layer_rows() {
        // Same-row drops are no-ops on either half of the row.
        assert_eq!(effect_drop_index(3, 1, 1, true), Some(1));
        assert_eq!(effect_drop_index(3, 1, 1, false), Some(1));

        // Move the bottom layer upward and the top layer downward across a middle row.
        assert_eq!(effect_drop_index(3, 0, 1, true), Some(1));
        assert_eq!(effect_drop_index(3, 2, 1, false), Some(1));

        // Crossing the full stack reaches the top and bottom final positions.
        assert_eq!(effect_drop_index(3, 0, 2, true), Some(2));
        assert_eq!(effect_drop_index(3, 2, 0, false), Some(0));

        assert_eq!(effect_drop_index(0, 0, 0, true), None);
        assert_eq!(effect_drop_index(3, 3, 0, true), None);
        assert_eq!(effect_drop_index(3, 0, 3, true), None);
    }

    #[test]
    fn dragging_an_effect_name_reorders_without_changing_opacity_or_selection() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":16,"height":16})).unwrap();
        session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
        let layer_id = session.active().unwrap().active_layer.unwrap();
        let state = session.active_mut().unwrap();
        let base = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        let make_paint = |name: &str, opacity| Paint {
            name: name.into(),
            visible: true,
            opacity,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind: EffectKind::Paint,
        };
        let mut mask = LayerMask::hide_all();
        mask.paint = Some(Stack { base, layers: vec![make_paint("Lower", 0.35), make_paint("Upper", 0.8)], active: 0 });
        std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer_id).unwrap().mask = Some(mask);
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.workspace = "3D".into();
        app.ui.mask_target = true;
        let mut h = Harness::builder().with_size(egui::vec2(520.0, 180.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let doc = app.session.active().unwrap().doc.id.0;
                expand_effects(ui.ctx(), doc, layer_id.0);
                let layer = app.session.active().unwrap().doc.layer(layer_id).unwrap().clone();
                let mut actions = Vec::new();
                rows(app, ui, &layer, &mut actions);
                for (command, params) in actions {
                    if command == "uv.maskPaint.modify" {
                        app.run(&command, params).unwrap();
                        show_active_effect_properties(app, ui.ctx());
                    }
                }
            },
            app,
        );
        h.run_steps(2);
        let from = h.get_by_label("Upper").rect().center(); // top row is the engine's index 1
        let to = h.get_by_label("Lower").rect().center(); // lower row is index 0
        h.event(egui::Event::PointerMoved(from));
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(to));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);

        let stack = h.state().session.active().unwrap().doc.layer(layer_id).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
        assert_eq!(stack.layers.iter().map(|effect| effect.name.as_str()).collect::<Vec<_>>(), ["Upper", "Lower"]);
        assert_eq!(stack.layers[0].opacity, 0.8);
        assert_eq!(stack.layers[1].opacity, 0.35);
        assert_eq!(stack.active, 1, "engine active-effect identity follows the moved layer");
        assert_eq!(selected_effect(&h.ctx, h.state().session.active().unwrap().doc.id.0, layer_id.0), Some(1));
    }

    #[test]
    fn double_clicking_a_mask_effect_name_renames_without_selecting_or_changing_view() {
        let (app, layer_id) = app_with_paint_and_fill_rows();
        let mut h = Harness::builder()
            .with_size(egui::vec2(520.0, 220.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut PhotocraftApp| draw_mask_rows(app, ui, layer_id), app);
        h.run_steps(3);
        let name = h.get_by_label("Fill").rect().center();
        h.event(egui::Event::PointerMoved(name));
        h.step();
        for pressed in [true, false, true, false] {
            h.event(egui::Event::PointerButton { pos: name, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(2);
        assert!(h.query_all_by_label("Name").next().is_some(), "double click opens the inline rename field");
        h.event(egui::Event::Text("Renamed Fill".into()));
        h.run_steps(1);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        let state = h.state().session.active().unwrap();
        let stack = state.doc.layer(layer_id).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
        assert_eq!(stack.layers[1].name, "Renamed Fill");
        assert_eq!(stack.active, 0, "renaming does not select another effect");
        assert_eq!(selected_effect(&h.ctx, state.doc.id.0, layer_id.0), None);
        assert!(photocraft_engine::mask_view_cmds::current(state).is_none(), "renaming does not enter Gray mask view");
    }

    #[test]
    fn creating_blur_exposes_opacity_radius_and_blend_mode() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":16,"height":16})).unwrap();
        session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
        let layer_id = session.active().unwrap().active_layer.unwrap();
        session.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.workspace = "3D".into();
        app.ui.panels.properties = false;
        let mut h = Harness::builder().with_size(egui::vec2(520.0, 400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let mut actions = Vec::new();
                add_menu(app, ui, layer_id.0, &mut actions);
                for (command, params) in actions {
                    if command == "layer.select" {
                        let _ = app.run(&command, params);
                    } else if command == "ui.maskTarget" {
                        app.ui.mask_target = true;
                        app.ui.vector_mask_target = false;
                    } else {
                        let _ = app.run(&command, params);
                        if command == "uv.maskPaint.add" {
                            show_active_effect_properties(app, ui.ctx());
                        }
                    }
                }
                let doc = app.session.active().unwrap().doc.id.0;
                expand_effects(ui.ctx(), doc, layer_id.0);
                let layer = app.session.active().unwrap().doc.layer(layer_id).unwrap().clone();
                let mut row_actions = Vec::new();
                rows(app, ui, &layer, &mut row_actions);
                simple_properties(app, ui);
            },
            app,
        );
        h.run_steps(25);
        h.get_by_label("Add Blur layer").click();
        h.run_steps(3);
        let state = h.state().session.active().unwrap();
        let stack = state.doc.layer(layer_id).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap();
        assert_eq!(stack.active, 0);
        assert!(matches!(&stack.layers[0].kind, EffectKind::Blur { radius } if *radius == 8));
        assert!(h.state().ui.panels.properties, "creation opens Properties");
        assert_eq!(h.state().ui.dock_tabs.properties, 0, "the Properties tab is active");
        let doc = h.state().session.active().unwrap().doc.id.0;
        select_effect(&h.ctx, doc, layer_id.0, 0);
        h.run_steps(3);
        assert!(h.query_all_by_label("Opacity").any(|node| node.accesskit_node().role() == Role::Button));
        assert!(h.query_all_by_label("Radius").next().is_some());
        assert!(
            h.query_all_by_label("Blend mode").any(|node| node.accesskit_node().role() == Role::ComboBox),
            "global Blur exposes its mask blend mode in the stack row"
        );
    }

    #[test]
    fn adding_an_effect_selects_properties_without_changing_mask_display_mode() {
        use photocraft_engine::mask_view_cmds::{self, MaskViewMode};
        for gray_view in [false, true] {
            let mut session = photocraft_engine::Session::new();
            session.execute("file.new", json!({"width":16,"height":16})).unwrap();
            session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
            let layer_id = session.active().unwrap().active_layer.unwrap();
            session.execute("layer.layerMask.revealAll", json!({})).unwrap();
            if gray_view {
                session.execute("view.layerMask", json!({"mode":"gray"})).unwrap();
            }
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.ui.workspace = "3D".into();
            let before = app.session.active().and_then(mask_view_cmds::current).map(|view| view.mode);
            app.run("uv.maskPaint.add", json!({"layer":layer_id.0,"kind":"blur"})).unwrap();
            assert!(show_active_effect_properties(&mut app, &egui::Context::default()));
            let after = app.session.active().and_then(mask_view_cmds::current).map(|view| view.mode);
            assert_eq!(after, before, "adding a mask effect must preserve the existing display channel");
            assert!(app.ui.mask_target, "the new effect remains the active brush target");
            assert!(app.ui.panels.properties, "Properties opens for the new effect");
            assert_eq!(app.ui.dock_tabs.properties, 0);
            assert!(
                crate::dock::visible_tabs(&app, crate::dock::Group::Properties, app.ui.theme.is_pro()).contains(&(0, "Properties")),
                "the native dock exposes the Properties tab"
            );
            assert!(!app.ui.dock.is_collapsed(crate::dock::Group::Properties), "the native Properties group is expanded");
            assert_eq!(app.session.active().unwrap().doc.layer(layer_id).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().active, 0);
            if gray_view {
                assert_eq!(after, Some(MaskViewMode::Gray));
            } else {
                assert_eq!(after, None);
            }
        }
    }

    #[test]
    fn selecting_mask_effect_reopens_and_activates_properties_in_custom_dock() {
        let (mut app, layer_id) = app_with_paint_and_fill_rows();
        app.run("window.panel.activate", json!({"panel":"layers"})).unwrap();
        assert!(app.ui.docking.is_some(), "the fixture now uses the custom docking model");
        app.run("window.panel.close", json!({"panel":"properties"})).unwrap();
        assert!(!app.ui.docking.as_ref().unwrap().layout.contains(&"properties".to_owned()), "Properties begins hidden");
        app.run("uv.maskPaint.select", json!({"layer":layer_id.0,"index":1})).unwrap();

        let ctx = egui::Context::default();
        assert!(show_active_effect_properties(&mut app, &ctx));

        let workspace = app.ui.docking.as_ref().unwrap();
        assert!(workspace.layout.contains(&"properties".to_owned()), "activation restores a closed Properties panel");
        assert_eq!(workspace.layout.panel_state(&"properties".to_owned()), craft_ui::docking::PanelState::Front);
        assert_eq!(app.ui.docking_reveal.as_deref(), Some("properties"), "the docking renderer raises the Properties panel");
        assert!(app.ui.panels.properties);
        assert_eq!(app.ui.dock_tabs.properties, 0);
        assert_eq!(selected_effect(&ctx, doc_id(&app), layer_id.0), Some(1));
    }

    #[test]
    fn add_menu_click_keeps_composite_or_gray_view_and_opens_properties() {
        use photocraft_engine::mask_view_cmds::{self, MaskViewMode};
        for gray_view in [false, true] {
            let mut session = photocraft_engine::Session::new();
            session.execute("file.new", json!({"width":16,"height":16})).unwrap();
            session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
            let layer_id = session.active().unwrap().active_layer.unwrap();
            session.execute("layer.layerMask.revealAll", json!({})).unwrap();
            if gray_view {
                session.execute("view.layerMask", json!({"mode":"gray"})).unwrap();
            }
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.ui.workspace = "3D".into();
            if gray_view {
                app.run("window.panel.activate", json!({"panel":"layers"})).unwrap();
                app.run("window.panel.close", json!({"panel":"properties"})).unwrap();
            }
            let mut h = Harness::builder().with_size(egui::vec2(520.0, 280.0)).with_step_dt(1.0 / 60.0).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    let mut actions = Vec::new();
                    add_menu(app, ui, layer_id.0, &mut actions);
                    for (command, params) in actions {
                        let _ = app.run(&command, params);
                        if command == "uv.maskPaint.add" {
                            show_active_effect_properties(app, ui.ctx());
                        }
                    }
                    simple_properties(app, ui);
                },
                app,
            );
            h.run_steps(2);
            h.get_by_label("Add Fill layer").click();
            h.run_steps(2);
            let state = h.state().session.active().unwrap();
            let mode = mask_view_cmds::current(state).map(|view| view.mode);
            assert_eq!(mode, if gray_view { Some(MaskViewMode::Gray) } else { None });
            assert!(h.state().ui.mask_target);
            assert!(h.state().ui.panels.properties);
            assert_eq!(h.state().ui.dock_tabs.properties, 0);
            assert!(selected_effect(&h.ctx, state.doc.id.0, layer_id.0).is_some());
            if gray_view {
                let dock = h.state().ui.docking.as_ref().unwrap();
                assert_eq!(dock.layout.panel_state(&"properties".to_owned()), craft_ui::docking::PanelState::Front);
                assert_eq!(h.state().ui.docking_reveal.as_deref(), Some("properties"));
            }
        }
    }

    #[test]
    fn fill_from_selection_uses_active_uv_polygon_or_shell_selection_without_repeat() {
        let make_app = || {
            let mut session = photocraft_engine::Session::new();
            session.execute("uv.workspace.activate", json!({"active":true})).unwrap();
            let mesh = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
            session.execute("uv.model.import", json!({"path":mesh})).unwrap();
            session.execute("uv.texture.create", json!({"width":32,"height":32})).unwrap();
            session.execute("layer.layerMask.revealAll", json!({})).unwrap();
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.ui.workspace = "3D".into();
            app.ui.uv_workbench.select_shells = true;
            app
        };

        let mut shell_app = make_app();
        let layer = shell_app.session.active().unwrap().active_layer.unwrap();
        let owner = shell_app.session.active().unwrap().doc.id;
        let stale_triangle =
            shell_app.session.uv.topology.as_ref().and_then(|topology| topology.shells.get(1)).and_then(|shell| shell.triangles.first()).copied().unwrap();
        let stale_polygon = shell_app.session.uv.scene.as_ref().and_then(|scene| scene.triangles.get(stale_triangle)).unwrap().corners.map(|corner| {
            let doc = &shell_app.session.active().unwrap().doc;
            [corner.uv[0] * f64::from(doc.size.width), (1.0 - corner.uv[1]) * f64::from(doc.size.height)]
        });
        shell_app.run("select.uvPolygons", json!({"document":owner.0,"polygons":[stale_polygon]})).unwrap();
        let stale_selection = shell_app.session.active().unwrap().doc.selection.clone().unwrap();
        shell_app.ui.uv_workbench.selection_target = crate::uv_ui::SelectionTarget::UvShells;
        shell_app.session.uv.selected.insert(0);
        let (command, params) = fill_selection_preparation(&shell_app, layer.0).unwrap();
        assert_eq!(command, "uv.mask.apply", "the active UV shell selection replaces a stale document selection before creating its Fill");
        shell_app.run(&command, params).unwrap();
        let current_selection = shell_app.session.active().unwrap().doc.selection.as_ref().unwrap();
        assert_ne!(
            current_selection.read_region(shell_app.session.active().unwrap().doc.bounds()),
            stale_selection.read_region(shell_app.session.active().unwrap().doc.bounds())
        );
        shell_app.run("uv.maskPaint.add", fill_selection_add_params(layer.0)).unwrap();
        let fill = &shell_app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0];
        assert!(matches!(&fill.kind, EffectKind::Fill { repeat_u: false, repeat_v: false, .. }));
        assert!(fill.fill_map.is_some(), "the fill is baked from the selected shell coverage");

        let mut polygon_app = make_app();
        let layer = polygon_app.session.active().unwrap().active_layer.unwrap();
        let document = polygon_app.session.active().unwrap().doc.id;
        polygon_app.ui.uv_workbench.selection_target = crate::uv_ui::SelectionTarget::Polygons;
        polygon_app.ui.uv_workbench.polygon_document = Some(document);
        polygon_app.ui.uv_workbench.selected_polygons.insert(0);
        let (command, params) = fill_selection_preparation(&polygon_app, layer.0).unwrap();
        assert_eq!(command, "select.uvPolygons", "the current polygon highlight is committed before Fill reads document selection");
        polygon_app.run(&command, params).unwrap();
        assert!(polygon_app.session.active().unwrap().doc.selection.is_some());
        polygon_app.run("uv.maskPaint.add", fill_selection_add_params(layer.0)).unwrap();
        let fill = &polygon_app.session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0];
        assert!(matches!(&fill.kind, EffectKind::Fill { repeat_u: false, repeat_v: false, .. }));
        assert!(fill.fill_map.is_some(), "the fill is baked from the selected polygon coverage");
    }

    #[test]
    fn mask_effect_blend_dropdown_excludes_color_dependent_modes() {
        let (app, layer_id) = app_with_paint_and_fill_rows();
        let mut h = Harness::builder()
            .with_size(egui::vec2(520.0, 220.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut PhotocraftApp| draw_mask_rows(app, ui, layer_id), app);
        h.run_steps(3);
        assert_eq!(h.query_all_by_label("Blend mode").filter(|node| node.accesskit_node().role() == Role::ComboBox).count(), 2);
        assert_eq!(h.query_all_by_label("Opacity").filter(|node| node.accesskit_node().role() == Role::Button).count(), 2);

        let paint_y = h.get_by_label("Paint").rect().center().y;
        let paint_blend = widget_center(&h, "Blend mode", Role::ComboBox, paint_y);
        click_at(&mut h, paint_blend);
        h.run_steps(2);
        for allowed in ["Normal", "Darken", "Multiply", "Lighten", "Screen", "Linear Dodge", "Subtract", "Difference"] {
            assert!(h.query_all_by_label(allowed).next().is_some(), "Paint offers scalar mask mode {allowed}");
        }
        for excluded in ["Color Burn", "Color Dodge", "Overlay", "Soft Light", "Hue", "Saturation", "Color", "Luminosity"] {
            assert!(h.query_all_by_label(excluded).next().is_none(), "mask blend list must exclude {excluded}");
        }
        let multiply = h.get_by_label("Multiply").rect().center();
        click_at(&mut h, multiply);
        h.run_steps(3);
        let layer = h.state().session.active().unwrap().doc.layer(layer_id).unwrap();
        assert_eq!(layer.mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].blend, photocraft_color::BlendMode::Multiply);

        let fill_y = h.get_by_label("Fill").rect().center().y;
        let fill_blend = widget_center(&h, "Blend mode", Role::ComboBox, fill_y);
        click_at(&mut h, fill_blend);
        h.run_steps(2);
        for allowed in ["Normal", "Darken", "Multiply", "Lighten", "Screen", "Linear Dodge", "Subtract", "Difference"] {
            assert!(h.query_all_by_label(allowed).next().is_some(), "Fill offers scalar mask mode {allowed}");
        }
        for excluded in ["Color Burn", "Overlay", "Hue", "Saturation", "Color", "Luminosity"] {
            assert!(h.query_all_by_label(excluded).next().is_none(), "Fill blend list must exclude {excluded}");
        }
        let difference = h.get_by_label("Difference").rect().center();
        click_at(&mut h, difference);
        h.run_steps(3);
        let layer = h.state().session.active().unwrap().doc.layer(layer_id).unwrap();
        assert_eq!(layer.mask.as_ref().unwrap().paint.as_ref().unwrap().layers[1].blend, photocraft_color::BlendMode::Difference);

        for (name, value) in [("Paint", 0.55), ("Fill", 0.35)] {
            let row_y = h.get_by_label(name).rect().center().y;
            let opacity_button = widget_center(&h, "Opacity", Role::Button, row_y);
            click_at(&mut h, opacity_button);
            h.run_steps(2);
            let slider = widget_center(&h, "Opacity", Role::Slider, h.ctx.content_rect().center().y);
            let slider_rect = h.query_all_by_label("Opacity").find(|node| node.accesskit_node().role() == Role::Slider).unwrap().rect();
            click_at(&mut h, egui::pos2(slider_rect.left() + slider_rect.width() * value, slider.y));
            h.run_steps(3);
            let index = if name == "Paint" { 0 } else { 1 };
            let layer = h.state().session.active().unwrap().doc.layer(layer_id).unwrap();
            assert!(
                (layer.mask.as_ref().unwrap().paint.as_ref().unwrap().layers[index].opacity - value).abs() < 0.03,
                "{name} row opacity updates from its popup slider"
            );
        }
    }

    #[test]
    fn mask_blur_row_exposes_and_updates_its_blend_mode() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":16,"height":16})).unwrap();
        session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
        let layer_id = session.active().unwrap().active_layer.unwrap();
        let state = session.active_mut().unwrap();
        let mut mask = LayerMask::hide_all();
        mask.paint = Some(Stack {
            base: Surface::with_default(PixelFormat::GRAY8, &[0.0]),
            layers: vec![Paint {
                name: "Blur".into(),
                visible: true,
                opacity: 1.0,
                blend: photocraft_color::BlendMode::Normal,
                surface: Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]),
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Blur { radius: 8 },
            }],
            active: 0,
        });
        std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer_id).unwrap().mask = Some(mask);
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.workspace = "3D".into();
        app.ui.mask_target = true;
        let mut h = Harness::builder()
            .with_size(egui::vec2(360.0, 140.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut PhotocraftApp| draw_mask_rows(app, ui, layer_id), app);
        h.run_steps(3);
        assert_eq!(h.query_all_by_label("Blend mode").filter(|node| node.accesskit_node().role() == Role::ComboBox).count(), 1);

        let row_y = h.get_by_label("Blur").rect().center().y;
        let blend_button = widget_center(&h, "Blend mode", Role::ComboBox, row_y);
        click_at(&mut h, blend_button);
        h.run_steps(2);
        let multiply = h.get_by_label("Multiply").rect().center();
        click_at(&mut h, multiply);
        h.run_steps(3);

        let blend = h.state().session.active().unwrap().doc.layer(layer_id).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().layers[0].blend;
        assert_eq!(blend, photocraft_color::BlendMode::Multiply, "the global Blur row updates its scalar mask blend mode");
    }

    #[test]
    fn mask_effect_row_controls_stay_on_one_line_at_narrow_width() {
        let width = 220.0;
        let (app, layer_id) = app_with_paint_and_fill_rows();
        let mut h = Harness::builder()
            .with_size(egui::vec2(width, 220.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, app: &mut PhotocraftApp| draw_mask_rows(app, ui, layer_id), app);
        h.run_steps(3);
        let right_edge = h.ctx.content_rect().right();
        for label in ["Paint", "Fill", "Opacity", "Blend mode", "Delete Paint layer", "Delete Fill layer"] {
            for node in h.query_all_by_label(label) {
                assert!(node.rect().right() <= right_edge + 1.0, "{label} clipped outside the narrow Layers panel: {:?}", node.rect());
            }
        }
        for (name, delete_label) in [("Paint", "Delete Paint layer"), ("Fill", "Delete Fill layer")] {
            let name_node = h.get_by_label(name);
            let row_y = name_node.rect().center().y;
            let visibility = widget_rect(&h, "Visibility", Role::Button, row_y);
            let effect_icon = widget_rect(&h, "Select mask effect", Role::Button, row_y);
            let blend = widget_rect(&h, "Blend mode", Role::ComboBox, row_y);
            let opacity = widget_rect(&h, "Opacity", Role::Button, row_y);
            let delete = widget_rect(&h, delete_label, Role::Button, row_y);
            assert!(visibility.left() >= h.ctx.content_rect().left() + 8.0, "{name} eye keeps its child-row indent");
            assert!(name_node.rect().left() >= effect_icon.right() - 1.0, "{name} starts after its effect icon");
            assert!(name_node.rect().right() <= blend.left() + 1.0, "{name} uses the available space up to Blend");
            assert!(blend.width() <= MASK_EFFECT_BLEND_WIDTH + 1.0, "{name} Blend control stays compact");
            assert!((opacity.width() - MASK_EFFECT_OPACITY_WIDTH).abs() <= 1.0, "{name} Opacity control uses its compact width");
            assert!(blend.right() <= opacity.left() + 1.0, "Blend and Opacity stay adjacent in order");
            assert!(opacity.right() <= delete.left() + 1.0, "Opacity and delete stay adjacent in order");
            assert!(name_node.rect().width() >= 24.0, "{name} keeps enough width to remain readable");
            for (label, role) in [("Visibility", Role::Button), ("Blend mode", Role::ComboBox), ("Opacity", Role::Button), (delete_label, Role::Button)] {
                let control_y = widget_center(&h, label, role, row_y).y;
                assert!((control_y - row_y).abs() <= 1.0, "{label} stays on the {name} control row");
            }
        }
    }

    #[test]
    fn clicking_a_sibling_effect_preserves_the_active_gray_mask_view() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":16,"height":16})).unwrap();
        session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
        let layer_id = session.active().unwrap().active_layer.unwrap();
        let state = session.active_mut().unwrap();
        let mut mask = LayerMask::hide_all();
        mask.paint = Some(Stack {
            base: Surface::with_default(PixelFormat::GRAY8, &[0.0]),
            layers: ["First", "Second"]
                .into_iter()
                .map(|name| Paint {
                    name: name.into(),
                    visible: true,
                    opacity: 1.0,
                    blend: photocraft_color::BlendMode::Normal,
                    surface: Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]),
                    fill_map: None,
                    fill_map_size: [0, 0],
                    fill_bounds: None,
                    kind: EffectKind::Paint,
                })
                .collect(),
            active: 0,
        });
        std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer_id).unwrap().mask = Some(mask);
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.workspace = "3D".into();
        app.ui.mask_target = true;
        app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
        app.run("window.panel.activate", json!({"panel":"layers"})).unwrap();
        app.run("window.panel.close", json!({"panel":"properties"})).unwrap();
        assert!(!app.ui.docking.as_ref().unwrap().layout.contains(&"properties".to_owned()));
        let observed_actions = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed_for_ui = observed_actions.clone();
        let mut h = Harness::builder().with_size(egui::vec2(440.0, 180.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let Some(layer) = app.session.active().and_then(|state| state.active_layer.and_then(|id| state.doc.layer(id))).cloned() else {
                    return;
                };
                let mut actions = Vec::new();
                rows(app, ui, &layer, &mut actions);
                *observed_for_ui.lock().unwrap() = actions.clone();
                let ctx = ui.ctx().clone();
                for (command, params) in actions {
                    if command == "ui.maskTarget" {
                        let show = params.get("show").and_then(Value::as_bool).unwrap_or(false);
                        if let Err(error) = app.run("view.layerMask", json!({"mode":if show {"gray"} else {"off"}})) {
                            app.ui.status = error;
                            app.ui.status_error = true;
                        }
                    } else {
                        if let Err(error) = app.run(&command, params) {
                            app.ui.status = error;
                            app.ui.status_error = true;
                        }
                        if command == "uv.maskPaint.select" {
                            show_active_effect_properties(app, &ctx);
                        }
                    }
                }
            },
            app,
        );
        let doc = h.state().session.active().unwrap().doc.id.0;
        expand_effects(&h.ctx, doc, layer_id.0);
        h.run_steps(25);
        let second = h.get_by_label("Second").rect().center();
        h.hover_at(second);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: second, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(20); // Name clicks are delayed 300 ms to distinguish them from double-click rename.
        let state = h.state().session.active().unwrap();
        assert_eq!(
            state.doc.layer(layer_id).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().active,
            1,
            "actions: {:?}, UI status: {}",
            observed_actions.lock().unwrap(),
            h.state().ui.status
        );
        let view = photocraft_engine::mask_view_cmds::current(state).unwrap();
        assert_eq!(view.mode, photocraft_engine::mask_view_cmds::MaskViewMode::Gray);
        let dock = h.state().ui.docking.as_ref().unwrap();
        assert_eq!(dock.layout.panel_state(&"properties".to_owned()), craft_ui::docking::PanelState::Front);
        assert_eq!(h.state().ui.docking_reveal.as_deref(), Some("properties"));
        assert!(h.state().ui.panels.properties);
    }

    fn doc_id(app: &PhotocraftApp) -> u64 {
        app.session.active().unwrap().doc.id.0
    }

    fn app_with_paint_and_fill_rows() -> (PhotocraftApp, photocraft_doc::LayerId) {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":16,"height":16})).unwrap();
        session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
        let layer_id = session.active().unwrap().active_layer.unwrap();
        let state = session.active_mut().unwrap();
        let make_effect = |name: &str, kind| Paint {
            name: name.into(),
            visible: true,
            opacity: 1.0,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 0.0]),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind,
        };
        let mut mask = LayerMask::hide_all();
        mask.paint = Some(Stack {
            base: Surface::with_default(PixelFormat::GRAY8, &[0.0]),
            layers: vec![
                make_effect("Paint", EffectKind::Paint),
                make_effect("Fill", EffectKind::Fill { value: 1.0, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false }),
            ],
            active: 0,
        });
        std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer_id).unwrap().mask = Some(mask);
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.workspace = "3D".into();
        app.ui.mask_target = true;
        (app, layer_id)
    }

    fn draw_mask_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer_id: photocraft_doc::LayerId) {
        let doc = app.session.active().unwrap().doc.id.0;
        expand_effects(ui.ctx(), doc, layer_id.0);
        let layer = app.session.active().unwrap().doc.layer(layer_id).unwrap().clone();
        let mut actions = Vec::new();
        rows(app, ui, &layer, &mut actions);
        for (command, params) in actions {
            if command == "ui.maskTarget" {
                let show = params.get("show").and_then(Value::as_bool).unwrap_or(false);
                let mode = if show { "gray" } else { "off" };
                let _ = app.run("view.layerMask", json!({"mode":mode}));
            } else {
                let reorders = command == "uv.maskPaint.modify" && params.get("to").is_some();
                let _ = app.run(&command, params);
                if command == "uv.maskPaint.select" || command == "uv.maskPaint.add" || reorders {
                    show_active_effect_properties(app, ui.ctx());
                }
            }
        }
    }

    fn widget_rect(h: &Harness<'_, PhotocraftApp>, label: &str, role: Role, row_y: f32) -> egui::Rect {
        h.query_all_by_label(label)
            .filter(|node| node.accesskit_node().role() == role)
            .min_by(|a, b| (a.rect().center().y - row_y).abs().total_cmp(&(b.rect().center().y - row_y).abs()))
            .unwrap_or_else(|| panic!("missing {role:?} labeled {label}"))
            .rect()
    }

    fn widget_center(h: &Harness<'_, PhotocraftApp>, label: &str, role: Role, row_y: f32) -> egui::Pos2 {
        widget_rect(h, label, role, row_y).center()
    }

    fn click_at(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2) {
        h.event(egui::Event::PointerMoved(at));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
    }
}
