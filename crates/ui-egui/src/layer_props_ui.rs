//! Properties panel for pixel (and other bounded) layers, Photoshop 2026 style: a Transform section
//! (W/H/X/Y of the layer content, editable), Align and Distribute, the kind's sections and Quick Actions. Edits dispatch
//! `edit.transform`, `layer.translate`, `layer.align.*` and selection commands.

use egui::{Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::{Layer, LayerContent};
use serde_json::{Value, json};

use crate::props_layout::{LABEL_GAP, LABEL_W, quick_actions_ui, section};
use crate::theme::{ROW_GAP, Tokens};
use crate::{PhotocraftApp, widgets};

/// `edit.transform` params that scale the content box `b` = [x0, y0, x1, y1] to `w` x `h`, keeping
/// the top-left corner (Photoshop's Properties W/H fields), or `layer.translate` params for X/Y.
pub fn transform_params(layer: u64, b: [i32; 4], w: Option<f32>, h: Option<f32>, linked: bool) -> Option<Value> {
    let (bw, bh) = ((b[2] - b[0]) as f32, (b[3] - b[1]) as f32);
    if bw <= 0.0 || bh <= 0.0 {
        return None;
    }
    let (mut nw, mut nh) = (w.unwrap_or(bw).max(1.0), h.unwrap_or(bh).max(1.0));
    if linked {
        if w.is_some() {
            nh = (nw * bh / bw).max(1.0);
        } else if h.is_some() {
            nw = (nh * bw / bh).max(1.0);
        }
    }
    if (nw - bw).abs() < 0.5 && (nh - bh).abs() < 0.5 {
        return None;
    }
    let (x0, y0) = (b[0] as f32, b[1] as f32);
    let quad = [[x0, y0], [x0 + nw, y0], [x0 + nw, y0 + nh], [x0, y0 + nh]];
    Some(json!({"layer": layer, "rect": b, "quad": quad}))
}

/// A number field that reports a value once committed (drag released, Enter, focus lost).
fn field(ui: &mut egui::Ui, id: &str, label: &str, current: f32, width: f32, u: LenUnit, extent: f64) -> Option<f32> {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 22.0), Sense::hover());
    ui.painter().text(pos2(r.right() - 2.0, r.center().y), egui::Align2::RIGHT_CENTER, tl!(label), egui::FontId::proportional(12.0), t.text_dim);
    let key = ui.make_persistent_id(("layer-props-field", id));
    let mut v = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or_else(|| u.shown(f64::from(current), extent));
    let resp = widgets::value_field(ui, &mut v, -300_000.0..=300_000.0, u.unit.suffix(), width);
    if resp.dragged() || resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, v));
        return None;
    }
    ui.data_mut(|d| d.remove::<f32>(key));
    if !(resp.drag_stopped() || resp.lost_focus() || resp.changed()) {
        return None;
    }
    let px = u.px(v, extent) as f32;
    ((px - current).abs() >= 0.5).then_some(px)
}

/// Lengths in the Units & Rulers unit, as Photoshop's Properties panel shows them (cm when the
/// rulers are in centimeters): document px ↔ that unit at the document's resolution.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LenUnit {
    pub unit: photocraft_engine::prefs::Unit,
    pub dpi: f64,
    pub ppi: f64,
}

impl LenUnit {
    pub(crate) fn of(app: &PhotocraftApp) -> Self {
        let ur = &app.session.prefs().units_and_rulers;
        let dpi = app.session.active().map_or(72.0, |s| f64::from(s.doc.resolution_dpi));
        LenUnit { unit: ur.rulers, dpi, ppi: ur.point_size.per_inch() }
    }
    /// `px` shown in the unit (`extent` is 100% for percent).
    pub(crate) fn shown(&self, px: f64, extent: f64) -> f32 {
        self.unit.from_px(px, self.dpi, extent, self.ppi) as f32
    }
    /// A value typed in the unit, in document px.
    pub(crate) fn px(&self, v: f32, extent: f64) -> f64 {
        self.unit.to_px(f64::from(v), self.dpi, extent, self.ppi)
    }
}

/// Align glyph: a reference line and two bars (Photoshop's align icons).
fn align_glyph(ui: &egui::Ui, r: Rect, kind: &str, color: egui::Color32) {
    let p = ui.painter();
    let c = r.center();
    let s = Stroke::new(1.2, color);
    let vertical_ref = matches!(kind, "leftEdges" | "horizontalCenters" | "rightEdges");
    if vertical_ref {
        let x = match kind {
            "leftEdges" => c.x - 6.0,
            "rightEdges" => c.x + 6.0,
            _ => c.x,
        };
        p.line_segment([pos2(x, c.y - 7.0), pos2(x, c.y + 7.0)], s);
        for (dy, w) in [(-3.0, 10.0), (3.0, 6.0)] {
            let x0 = match kind {
                "leftEdges" => x,
                "rightEdges" => x - w,
                _ => x - w / 2.0,
            };
            p.rect_filled(Rect::from_min_size(pos2(x0, c.y + dy - 1.5), vec2(w, 3.0)), 0.0, color);
        }
    } else {
        let y = match kind {
            "topEdges" => c.y - 6.0,
            "bottomEdges" => c.y + 6.0,
            _ => c.y,
        };
        p.line_segment([pos2(c.x - 7.0, y), pos2(c.x + 7.0, y)], s);
        for (dx, h) in [(-3.0, 10.0), (3.0, 6.0)] {
            let y0 = match kind {
                "topEdges" => y,
                "bottomEdges" => y - h,
                _ => y - h / 2.0,
            };
            p.rect_filled(Rect::from_min_size(pos2(c.x + dx - 1.5, y0), vec2(3.0, h)), 0.0, color);
        }
    }
}

/// Width of the link toggle between the W and H fields (the X/Y row leaves the same gap).
const LINK_W: f32 = 20.0;

fn responsive_rule_params(
    layer: photocraft_doc::LayerId,
    rule: Option<&photocraft_doc::responsive::LayerLayoutRule>,
    mode: &str,
    horizontal: &str,
    vertical: &str,
    text_flow: &str,
) -> Value {
    let mut params = json!({"layer":layer.0,"mode":mode,"horizontal":horizontal,"vertical":vertical});
    if let Some(rule) = rule {
        if let Some(size) = rule.min_size {
            params["minSize"] = json!({"width":size.width,"height":size.height});
        }
        if let Some(size) = rule.max_size {
            params["maxSize"] = json!({"width":size.width,"height":size.height});
        }
        if let Some(size) = rule.min_font_size_pt {
            params["minFontSizePt"] = json!(size);
        }
        if let Some(stack) = &rule.stack {
            let direction = match stack.direction {
                photocraft_doc::responsive::StackDirection::Vertical => "vertical",
                photocraft_doc::responsive::StackDirection::Horizontal => "horizontal",
            };
            let alignment = match stack.alignment {
                photocraft_doc::responsive::StackAlignment::Start => "start",
                photocraft_doc::responsive::StackAlignment::Center => "center",
                photocraft_doc::responsive::StackAlignment::End => "end",
                photocraft_doc::responsive::StackAlignment::Stretch => "stretch",
            };
            params["stack"] = json!({"direction":direction,"alignment":alignment,"gap":stack.gap});
        }
    }
    params["textFlow"] = json!(text_flow);
    params
}

/// Pro Properties body for a non-adjustment layer: Transform, Align and Distribute, the kind's own
/// sections (Character/Paragraph/Type Options for type, Appearance/Shape for shapes), then Quick
/// Actions, all with the same collapsible headers (#155).
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let mut run: Vec<(String, Value)> = Vec::new();
    let t = Tokens::get(ui.ctx());
    if let Some(st) = app.session.active()
        && st.doc.artboard_of(layer.id).is_some_and(|board| board != layer.id)
    {
        let doc = st.doc.clone();
        let suggestion = photocraft_engine::artboard_cmds::semantic_role_suggestion(&doc, layer.id);
        let suggestion_role = suggestion.role;
        let selected = photocraft_engine::artboard_cmds::explicit_semantic_role(&doc, layer.id).unwrap_or("auto");
        let source = photocraft_engine::artboard_cmds::semantic_role_source(&doc, layer.id);
        ui.label(egui::RichText::new(tl!("Responsive layout role")).font(crate::theme::semibold(12.0)).color(t.text));
        let role_label = |role: &str| {
            photocraft_engine::artboard_cmds::SEMANTIC_ROLES.iter().find(|(value, _)| *value == role).map_or("Free placement", |(_, label)| *label)
        };
        let selected_label = if selected == "auto" {
            format!("{} · {}: {}", tl!("Auto"), tl!("suggested"), tl!(role_label(suggestion_role)))
        } else {
            tl!(role_label(selected)).to_owned()
        };
        let source_label = match source {
            "file" => "From file",
            "manual" => "Manual",
            "document" => "Document",
            _ => "Suggested",
        };
        let provenance = if selected == "auto" {
            format!("{} · {}%", tl!("Suggested"), (suggestion.confidence * 100.0).round() as u32)
        } else {
            tl!(source_label).to_owned()
        };
        ui.label(provenance).on_hover_text(format!("{}: {} · {}", tl!("Suggested role"), tl!(role_label(suggestion_role)), tl!(suggestion.evidence)));
        let mut role = selected.to_owned();
        egui::ComboBox::from_id_salt(("artboard-semantic-role", layer.id.0)).selected_text(selected_label).show_ui(ui, |ui| {
            for (value, label) in photocraft_engine::artboard_cmds::SEMANTIC_ROLES {
                let text = if *value == "auto" {
                    format!("{} · {}: {}", tl!("Auto"), tl!("suggested"), tl!(role_label(suggestion_role)))
                } else {
                    tl!(*label).to_owned()
                };
                ui.selectable_value(&mut role, (*value).to_owned(), text);
            }
        });
        let apply_similar = role != "auto" && ui.small_button(tl!("Apply role to similar layers")).clicked();
        if apply_similar {
            run.push(("layer.artboard.semanticRole.set".into(), json!({"layer":layer.id.0,"role":role,"applyToSimilar":true})));
        } else if role != selected {
            run.push(("layer.artboard.semanticRole.set".into(), json!({"layer": layer.id.0, "role": role})));
        }
        let board = doc.artboard_of(layer.id).unwrap_or(layer.id);
        let variant = doc.responsive.variants.iter().find(|entry| entry.artboard == board);
        let master_layer = variant.and_then(|entry| entry.layers.iter().find(|mapping| mapping.variant_layer == layer.id).map(|mapping| mapping.master_layer));
        let has_variant_override = variant.is_some_and(|_| doc.responsive.overrides.iter().any(|entry| entry.variant == board && entry.layer == layer.id));
        let rule = variant
            .and_then(|_| doc.responsive.overrides.iter().find(|entry| entry.variant == board && entry.layer == layer.id).map(|entry| &entry.rule))
            .or_else(|| doc.responsive.rules.iter().find(|rule| rule.layer == layer.id || master_layer == Some(rule.layer)));
        let mode_name = |mode: photocraft_doc::responsive::ResizeMode| match mode {
            photocraft_doc::responsive::ResizeMode::Fixed => "fixed",
            photocraft_doc::responsive::ResizeMode::Stretch => "stretch",
            photocraft_doc::responsive::ResizeMode::Fill => "fill",
            photocraft_doc::responsive::ResizeMode::Fit => "fit",
        };
        let horizontal_name = |anchor: photocraft_doc::responsive::HorizontalAnchor| match anchor {
            photocraft_doc::responsive::HorizontalAnchor::Left => "left",
            photocraft_doc::responsive::HorizontalAnchor::Center => "center",
            photocraft_doc::responsive::HorizontalAnchor::Right => "right",
        };
        let vertical_name = |anchor: photocraft_doc::responsive::VerticalAnchor| match anchor {
            photocraft_doc::responsive::VerticalAnchor::Top => "top",
            photocraft_doc::responsive::VerticalAnchor::Center => "center",
            photocraft_doc::responsive::VerticalAnchor::Bottom => "bottom",
        };
        let mut mode = rule.map_or("fixed", |rule| mode_name(rule.mode)).to_owned();
        let mut horizontal = rule.map_or("left", |rule| horizontal_name(rule.anchor.horizontal)).to_owned();
        let mut vertical = rule.map_or("top", |rule| vertical_name(rule.anchor.vertical)).to_owned();
        let mut min_width = rule.and_then(|rule| rule.min_size).map_or(0, |size| size.width);
        let mut min_height = rule.and_then(|rule| rule.min_size).map_or(0, |size| size.height);
        let mut max_width = rule.and_then(|rule| rule.max_size).map_or(0, |size| size.width);
        let mut max_height = rule.and_then(|rule| rule.max_size).map_or(0, |size| size.height);
        let flow_name = |flow: photocraft_doc::responsive::TextFlow| match flow {
            photocraft_doc::responsive::TextFlow::Preserve => "preserve",
            photocraft_doc::responsive::TextFlow::Rewrap => "rewrap",
            photocraft_doc::responsive::TextFlow::RewrapAutoHeight => "rewrap_auto_height",
            photocraft_doc::responsive::TextFlow::RewrapAndShrink => "rewrap_and_shrink",
        };
        let mut flow = rule.map_or("preserve", |rule| flow_name(rule.text_flow)).to_owned();
        let original = (mode.clone(), horizontal.clone(), vertical.clone(), flow.clone());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Mode")).color(t.text_dim).size(12.0));
            egui::ComboBox::from_id_salt(("artboard-layout-mode", layer.id.0))
                .selected_text(tl!(match mode.as_str() {
                    "stretch" => "Stretch",
                    "fill" => "Fill",
                    "fit" => "Fit",
                    _ => "Fixed",
                }))
                .show_ui(ui, |ui| {
                    for (key, label) in [("fixed", "Fixed"), ("stretch", "Stretch"), ("fill", "Fill"), ("fit", "Fit")] {
                        ui.selectable_value(&mut mode, key.to_owned(), tl!(label));
                    }
                });
        });
        egui::Grid::new(("artboard-anchor-picker", layer.id.0)).spacing([2.0, 2.0]).show(ui, |ui| {
            for v in ["top", "center", "bottom"] {
                for h in ["left", "center", "right"] {
                    let glyph = match (v, h) {
                        ("top", "left") => "↖",
                        ("top", "center") => "↑",
                        ("top", "right") => "↗",
                        ("center", "left") => "←",
                        ("center", "center") => "•",
                        ("center", "right") => "→",
                        ("bottom", "left") => "↙",
                        ("bottom", "center") => "↓",
                        _ => "↘",
                    };
                    let response = ui.selectable_label(horizontal == h && vertical == v, glyph).on_hover_text(format!(
                        "{} {}",
                        tl!(match v {
                            "top" => "Top",
                            "center" => "Center",
                            _ => "Bottom",
                        }),
                        tl!(match h {
                            "left" => "Left",
                            "center" => "Center",
                            _ => "Right",
                        })
                    ));
                    if response.clicked() {
                        horizontal = h.to_owned();
                        vertical = v.to_owned();
                    }
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Text reflow")).color(t.text_dim).size(12.0));
            egui::ComboBox::from_id_salt(("artboard-text-flow", layer.id.0))
                .selected_text(tl!(match flow.as_str() {
                    "rewrap" => "Rewrap",
                    "rewrap_auto_height" => "Rewrap with auto height",
                    "rewrap_and_shrink" => "Rewrap and shrink",
                    _ => "Preserve",
                }))
                .show_ui(ui, |ui| {
                    for (key, label) in [
                        ("preserve", "Preserve"),
                        ("rewrap", "Rewrap"),
                        ("rewrap_auto_height", "Rewrap with auto height"),
                        ("rewrap_and_shrink", "Rewrap and shrink"),
                    ] {
                        ui.selectable_value(&mut flow, key.to_owned(), tl!(label));
                    }
                });
        });
        ui.label(tl!("Size limits in pixels (0 = none)"));
        let mut limits_changed = false;
        egui::Grid::new(("artboard-size-limits", layer.id.0)).num_columns(3).show(ui, |ui| {
            ui.label("");
            ui.label(tl!("Width"));
            ui.label(tl!("Height"));
            ui.end_row();
            ui.label(tl!("Minimum"));
            limits_changed |= ui.add(egui::DragValue::new(&mut min_width).range(0..=1_000_000).speed(1)).changed();
            limits_changed |= ui.add(egui::DragValue::new(&mut min_height).range(0..=1_000_000).speed(1)).changed();
            ui.end_row();
            ui.label(tl!("Maximum"));
            limits_changed |= ui.add(egui::DragValue::new(&mut max_width).range(0..=1_000_000).speed(1)).changed();
            limits_changed |= ui.add(egui::DragValue::new(&mut max_height).range(0..=1_000_000).speed(1)).changed();
            ui.end_row();
        });
        let invalid_limits = min_width > 0 && max_width > 0 && min_width > max_width || min_height > 0 && max_height > 0 && min_height > max_height;
        if invalid_limits {
            ui.colored_label(t.danger, tl!("Maximum size must be at least the minimum size"));
        } else if limits_changed
            || (mode.as_str(), horizontal.as_str(), vertical.as_str(), flow.as_str())
                != (original.0.as_str(), original.1.as_str(), original.2.as_str(), original.3.as_str())
        {
            let mut params = responsive_rule_params(layer.id, rule, &mode, &horizontal, &vertical, &flow);
            params["minSize"] = if min_width == 0 || min_height == 0 { Value::Null } else { json!({"width":min_width,"height":min_height}) };
            params["maxSize"] = if max_width == 0 || max_height == 0 { Value::Null } else { json!({"width":max_width,"height":max_height}) };
            run.push(("layer.artboard.layoutRule.set".into(), params));
        }
        if has_variant_override && ui.button(tl!("Reset to master rule")).clicked() {
            run.push(("layer.artboard.layoutRule.reset".into(), json!({"layer":layer.id.0})));
        }
        ui.add_space(ROW_GAP);
    }
    if let Some(s) = layer.surface() {
        let b = app.cached_bounds(layer.id.0, s);
        let u = LenUnit::of(app);
        let (dw, dh) = app.session.active().map_or((1.0, 1.0), |s| (f64::from(s.doc.size.width), f64::from(s.doc.size.height)));
        if section(ui, "transform", tl!("Transform")) {
            let link_key = egui::Id::new("layer-props-link");
            let linked = ui.data(|d| d.get_temp::<bool>(link_key)).unwrap_or(true);
            let bb = [b.x0, b.y0, b.x1, b.y1];
            // Two label+field columns with the link toggle between them, filling the panel.
            let w = ((ui.available_width() - 2.0 * (LABEL_W + LABEL_GAP) - LINK_W - 2.0 * LABEL_GAP) / 2.0).max(36.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = LABEL_GAP;
                if let Some(v) = field(ui, "w", "W", b.width() as f32, w, u, dw) {
                    run.extend(transform_params(layer.id.0, bb, Some(v), None, linked).map(|p| ("edit.transform".to_string(), p)));
                }
                if crate::icons::button(ui, if linked { "link" } else { "unlink" }, LINK_W, linked, tl!("Link width and height")).clicked() {
                    ui.data_mut(|d| d.insert_temp(link_key, !linked));
                }
                if let Some(v) = field(ui, "h", "H", b.height() as f32, w, u, dh) {
                    run.extend(transform_params(layer.id.0, bb, None, Some(v), linked).map(|p| ("edit.transform".to_string(), p)));
                }
            });
            ui.add_space(ROW_GAP);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = LABEL_GAP;
                if let Some(v) = field(ui, "x", "X", b.x0 as f32, w, u, dw) {
                    run.push(("layer.translate".into(), json!({"layer": layer.id.0, "dx": (v - b.x0 as f32).round() as i32, "dy": 0})));
                }
                ui.add_space(LINK_W + LABEL_GAP);
                if let Some(v) = field(ui, "y", "Y", b.y0 as f32, w, u, dh) {
                    run.push(("layer.translate".into(), json!({"layer": layer.id.0, "dx": 0, "dy": (v - b.y0 as f32).round() as i32})));
                }
            });
            ui.add_space(ROW_GAP);
        }
    }
    // Gradient fills: their gradient first, as in Photoshop's Properties panel.
    let gradient = matches!(layer.content, LayerContent::Fill(photocraft_doc::Fill::Gradient { .. }));
    if gradient {
        crate::gradient_ui::properties(app, ui, layer);
    }
    if !gradient {
        align_and_distribute(app, ui, &mut run);
    }
    for (id, p) in run.drain(..) {
        if let Err(e) = app.run(&id, p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    match &layer.content {
        LayerContent::Text(_) => crate::type_tool::type_properties(app, ui),
        LayerContent::Shape(_) => crate::vector_ui::shape_properties(app, ui, layer.id),
        _ => {}
    }
    // Re-read: a section above may have changed the layer (e.g. point to paragraph type).
    let content = app.session.active().and_then(|s| s.doc.layer(layer.id)).map_or_else(|| layer.content.clone(), |l| l.content.clone());
    if let Some(id) = quick_actions_ui(app, ui, &content) {
        let ctx = ui.ctx().clone();
        if let Err(e) = crate::menus::invoke(app, &ctx, id, json!({})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// The Align and Distribute section: its buttons run `layer.align.*` and `layer.distribute.*` on
/// the selected layers (pushed onto `run`), greyed where the engine says they can't.
fn align_and_distribute(app: &PhotocraftApp, ui: &mut egui::Ui, run: &mut Vec<(String, Value)>) {
    let t = Tokens::get(ui.ctx());
    if !section(ui, "align", tl!("Align and Distribute")) {
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, kind) in ["leftEdges", "horizontalCenters", "rightEdges", "topEdges", "verticalCenters", "bottomEdges"].into_iter().enumerate() {
            if i == 3 {
                ui.add_space(8.0);
            }
            let id = format!("layer.align.{kind}");
            let on = app.session.is_enabled(&id);
            let label = photocraft_engine::commands::find(&id).map_or(kind, |c| c.label);
            let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), if on { Sense::click() } else { Sense::hover() });
            if resp.hovered() && on {
                ui.painter().rect_filled(r, t.radius_sm, t.hover);
            }
            align_glyph(ui, r, kind, if on { t.icon } else { t.text_faint });
            let tip = crate::i18n::fmt(tl!("Align {what}"), &[("what", tl!(&label))]);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, &tip));
            if resp.on_hover_text(tip).clicked() {
                run.push((id, json!({})));
            }
        }
    });
    // The engine already supports equal-gap distribution in both axes. Make those
    // existing commands discoverable here as well as through Layer > Distribute (#2349).
    // Three or more selected layers are required; the engine remains authoritative.
    ui.horizontal_wrapped(|ui| {
        for (kind, text) in [("horizontally", tl!("Horizontally")), ("vertically", tl!("Vertically"))] {
            let id = format!("layer.distribute.{kind}");
            if ui.add_enabled(app.session.is_enabled(&id), egui::Button::new(text)).clicked() {
                run.push((id, json!({})));
            }
        }
    });
    ui.add_space(ROW_GAP);
}

/// Several layers selected with the Background active: Photoshop's Properties keeps Align and
/// Distribute for the selection there, above the Document properties (#4608).
pub fn selection_align(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let mut run = Vec::new();
    align_and_distribute(app, ui, &mut run);
    for (id, p) in run {
        if let Err(e) = app.run(&id, p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_between_gradient_and_raster_keeps_alignment_on_raster_only() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(280.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    return;
                }
                let state = app.session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
                properties(app, ui, &layer);
            },
            app,
        );
        for gradient in [false, true, false] {
            if gradient {
                h.state_mut().run("gradient.fill.create", json!({"from": [20, 60], "to": [180, 60]})).unwrap();
            } else {
                h.state_mut().run("layer.new.layer", json!({})).unwrap();
            }
            h.run_steps(3);
            let sections = crate::props_layout::sections_drawn(&h.ctx);
            assert_eq!(sections.iter().any(|(title, _)| title == "Align and Distribute"), !gradient);
            assert_eq!(sections.iter().any(|(title, _)| title == "Gradient"), gradient);
            assert_eq!(sections.iter().any(|(title, _)| title == "Gradient Options"), gradient);
            assert!(!sections.iter().any(|(title, _)| title == "Color stops" || title == "Opacity stops"));
        }
    }

    #[test]
    fn transform_fields_show_and_take_the_ruler_unit() {
        use photocraft_engine::prefs::Unit;
        // 72 dpi: 1644 px = 58 cm; typing 60 cm sets 1700.8 px
        let u = LenUnit { unit: Unit::Centimeters, dpi: 72.0, ppi: 72.0 };
        assert!((u.shown(1644.0, 1.0) - 58.0).abs() < 0.01);
        assert!((u.px(60.0, 1.0) - 1700.787).abs() < 0.01);
        let px = LenUnit { unit: Unit::Pixels, dpi: 150.0, ppi: 72.0 };
        assert_eq!(px.shown(170.0, 1.0), 170.0);
        assert_eq!(px.px(170.0, 1.0), 170.0);
        let pc = LenUnit { unit: Unit::Percent, dpi: 72.0, ppi: 72.0 };
        assert_eq!(pc.shown(822.0, 1644.0), 50.0);
    }

    #[test]
    fn width_edit_scales_from_the_top_left() {
        let p = transform_params(7, [10, 20, 110, 70], Some(200.0), None, true).unwrap();
        assert_eq!(p["layer"], 7);
        assert_eq!(p["quad"], json!([[10.0, 20.0], [210.0, 20.0], [210.0, 120.0], [10.0, 120.0]]));
        let p = transform_params(7, [10, 20, 110, 70], None, Some(25.0), false).unwrap();
        assert_eq!(p["quad"][2], json!([110.0, 45.0]));
        assert!(transform_params(7, [10, 20, 110, 70], Some(100.0), None, true).is_none());
        assert!(transform_params(7, [0, 0, 0, 0], Some(5.0), None, true).is_none());
    }

    #[test]
    fn transform_params_resize_a_layer_in_the_engine() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 20, "width": 100, "height": 50})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let p = transform_params(id.0, [10, 20, 110, 70], Some(50.0), None, true).unwrap();
        s.execute("edit.transform", p).unwrap();
        let b = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
        assert_eq!((b.x0, b.y0, b.width(), b.height()), (10, 20, 50, 25));
    }
}
