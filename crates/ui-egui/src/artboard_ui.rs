//! Artboards in the shell: the pasteboard between boards and the board names on the canvas,
//! View › Fit Artboard on Screen, and the Properties panel section (X/Y/W/H, preset,
//! background). Everything that changes the document is `layer.artboard.set` and friends.

use egui::{Align2, Color32, Pos2, Rect, Stroke};
use photocraft_doc::{ArtboardBackground, Document, Layer};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::theme::Tokens;

fn safe_area_bounds(board: photocraft_geom::Rect, insets: photocraft_doc::responsive::SafeInsets) -> photocraft_geom::Rect {
    let fraction = |value: f32| f64::from(value.clamp(0.0, 0.5));
    let x0 = (f64::from(board.x0) + f64::from(board.width()) * fraction(insets.left)).round() as i32;
    let x1 = (f64::from(board.x1) - f64::from(board.width()) * fraction(insets.right)).round() as i32;
    let y0 = (f64::from(board.y0) + f64::from(board.height()) * fraction(insets.top)).round() as i32;
    let y1 = (f64::from(board.y1) - f64::from(board.height()) * fraction(insets.bottom)).round() as i32;
    photocraft_geom::Rect::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1))
}

/// Screen rects covering the document image minus every artboard (vertical slabs between the
/// boards' x edges, split around the boards they cross).
pub fn pasteboard_rects(xf: &ViewXform, doc: &Document) -> Vec<Rect> {
    let canvas = doc.bounds();
    let boards: Vec<photocraft_geom::Rect> = doc.artboards().iter().map(|b| b.2.rect.intersect(&canvas)).filter(|r| !r.is_empty()).collect();
    let mut xs: Vec<i32> = vec![canvas.x0, canvas.x1];
    for b in &boards {
        xs.extend([b.x0, b.x1]);
    }
    xs.sort_unstable();
    xs.dedup();
    let mut out = Vec::new();
    for w in xs.windows(2) {
        let (x0, x1) = (w[0], w[1]);
        let mut spans: Vec<(i32, i32)> = boards.iter().filter(|b| b.x0 <= x0 && b.x1 >= x1).map(|b| (b.y0, b.y1)).collect();
        spans.sort_unstable();
        let mut y = canvas.y0;
        for (a, b) in spans {
            if a > y {
                out.push(xf.doc_rect(photocraft_geom::Rect::new(x0, y, x1, a)));
            }
            y = y.max(b);
        }
        if y < canvas.y1 {
            out.push(xf.doc_rect(photocraft_geom::Rect::new(x0, y, x1, canvas.y1)));
        }
    }
    out
}

/// Board outlines and names (the active board's name is highlighted, as in Photoshop).
pub fn draw_frames(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(st) = app.session.active() else { return };
    let t = Tokens::get(painter.ctx());
    let active = st.active_layer.and_then(|id| st.doc.artboard_of(id));
    for (id, name, a) in st.doc.artboards() {
        let r = xf.doc_rect(a.rect);
        let on = active == Some(id);
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, if on { t.accent } else { Color32::from_black_alpha(90) }), egui::StrokeKind::Outside);
        let pos = Pos2::new(r.left(), r.top() - 4.0);
        painter.text(pos, Align2::LEFT_BOTTOM, name, egui::FontId::proportional(11.0), if on { t.text } else { t.text_dim });
        if app.ui.extras.safe_areas {
            for safe_area in st.doc.responsive.safe_areas.iter().filter(|area| area.artboard == id) {
                let safe = xf.doc_rect(safe_area_bounds(a.rect, safe_area.insets));
                painter.rect_stroke(safe, 0.0, Stroke::new(1.0, t.warning), egui::StrokeKind::Inside);
                painter.text(safe.left_top() + egui::vec2(3.0, 3.0), Align2::LEFT_TOP, &safe_area.name, egui::FontId::proportional(10.0), t.warning);
            }
        }
    }
}

/// Responsive issue outlines are painted after layer/tool overlays so selection handles cannot
/// hide them. Diagnostics themselves are cached per document revision by the overview state.
pub fn draw_responsive_markers(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if !app.ui.extras.responsive_warnings {
        return;
    }
    let Some(state) = app.session.active() else { return };
    let t = Tokens::get(painter.ctx());
    for marker in crate::artboard_overview::canvas_markers(&app.artboard_overview, state.doc.id, state.revision) {
        let screen = xf.doc_rect(marker.bounds);
        match marker.kind {
            crate::artboard_overview::MarkerKind::Overflow => {
                painter.rect_stroke(screen, 0.0, Stroke::new(2.0, t.warning), egui::StrokeKind::Outside);
            }
            crate::artboard_overview::MarkerKind::Collision => {
                painter.circle_filled(screen.left_top() + egui::vec2(5.0, 5.0), 4.0, t.danger);
            }
            crate::artboard_overview::MarkerKind::SafeArea => {
                painter.circle_filled(screen.left_top() + egui::vec2(14.0, 5.0), 4.0, t.accent);
            }
        }
    }
}

/// View › Fit Artboard on Screen: zoom to the active artboard (or the first one).
pub fn fit_artboard(app: &mut PhotocraftApp) -> Result<Value, String> {
    let i = app.session.active_index().ok_or("no document")?;
    let st = app.session.active().ok_or("no document")?;
    let id = st.active_layer.and_then(|l| st.doc.artboard_of(l)).or_else(|| st.doc.artboards().last().map(|b| b.0)).ok_or("the document has no artboards")?;
    let b = st.doc.layer(id).and_then(Layer::artboard).map(|a| a.rect).ok_or("no artboard")?;
    let size = [st.doc.size.width, st.doc.size.height];
    let area = app.last_canvas_rect.size();
    let area = if area.x > 50.0 { area } else { egui::vec2(1200.0, 800.0) };
    let ppp = app.canvas_ppp();
    let v = &mut app.ui.views[i];
    // Leave room for the name above the board. The area is in egui points; the stored zoom is
    // device pixels per document pixel.
    v.zoom = ((area.x - 60.0) / b.width().max(1) as f32).min((area.y - 80.0) / b.height().max(1) as f32) * ppp;
    v.zoom = crate::zoom_levels::clamp(v.zoom, size);
    // Widen before adding: a board near ±2^30, or one whose far edge saturated at i32::MAX,
    // overflows an i32 sum (#981).
    v.center = [((f64::from(b.x0) + f64::from(b.x1)) / 2.0) as f32, ((f64::from(b.y0) + f64::from(b.y1)) / 2.0) as f32];
    v.fit_pending = false;
    Ok(json!({"zoom": v.zoom, "artboard": id.0, "bounds": [b.x0, b.y0, b.x1, b.y1]}))
}

/// Properties panel for an artboard.
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let Some(a) = layer.artboard().cloned() else { return };
    let t = Tokens::get(ui.ctx());
    let key = |k: &str| format!("artboard-{}-{k}", layer.id.0);
    let mut edit: Option<Value> = None;
    let mut lifecycle_action: Option<(&str, Value)> = None;
    let mut safe_area_action: Option<(&str, Value)> = None;
    let responsive = app.session.active().map(|state| state.doc.responsive.clone()).unwrap_or_default();
    let is_variant = responsive.variants.iter().any(|variant| variant.artboard == layer.id);
    let is_master = responsive.master == Some(layer.id);
    // The custom background swatch was clicked: its colour, for the Color Picker.
    let mut pick: Option<[f32; 3]> = None;
    ui.label(egui::RichText::new(tl!("Artboard")).font(crate::theme::semibold(12.0)).color(t.text));
    ui.add_space(4.0);
    let mut preset = a.preset.clone();
    let mut opts: Vec<(String, &str)> = vec![(String::new(), tl!("Custom"))];
    opts.extend(photocraft_engine::artboard_cmds::PRESETS.iter().map(|(n, _, _)| (n.to_string(), *n)));
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Size")).color(t.text_dim).size(12.0));
        if crate::widgets::dropdown(ui, &key("preset"), &mut preset, &opts, 150.0) && !preset.is_empty() {
            edit = Some(json!({"layer": layer.id.0, "preset": preset}));
        }
    });
    let num = |ui: &mut egui::Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>| -> bool {
        ui.label(egui::RichText::new(tl!(&label)).color(t.text_dim).size(12.0));
        crate::widgets::value_field(ui, v, range, "px", 64.0).changed()
    };
    let r = a.rect;
    ui.horizontal(|ui| {
        let (mut w, mut h) = (r.width() as f32, r.height() as f32);
        let cw = num(ui, "W", &mut w, 1.0..=300_000.0);
        let ch = num(ui, "H", &mut h, 1.0..=300_000.0);
        if cw || ch {
            edit = Some(json!({"layer": layer.id.0, "width": w.round(), "height": h.round(), "coalesce": key("wh")}));
        }
    });
    ui.horizontal(|ui| {
        let (mut x, mut y) = (r.x0 as f32, r.y0 as f32);
        let cx = num(ui, "X", &mut x, -300_000.0..=300_000.0);
        let cy = num(ui, "Y", &mut y, -300_000.0..=300_000.0);
        if cx || cy {
            edit = Some(json!({"layer": layer.id.0, "x": x.round(), "y": y.round(), "coalesce": key("xy")}));
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Artboard Background Color")).color(t.text_dim).size(12.0));
    });
    ui.horizontal(|ui| {
        let mut bg = a.background.name().to_string();
        let opts = [
            ("white".to_string(), tl!("White")),
            ("black".to_string(), tl!("Black")),
            ("transparent".to_string(), tl!("Transparent")),
            ("custom".to_string(), tl!("Other…")),
        ];
        if crate::widgets::dropdown(ui, &key("bg"), &mut bg, &opts, 120.0) {
            let mut params = json!({"layer": layer.id.0, "background": bg});
            if bg == "custom" {
                let hex = match a.background {
                    ArtboardBackground::Custom(c) => {
                        let [r8, g8, b8, _] = c.to_rgba8();
                        format!("#{:02x}{:02x}{:02x}", r8, g8, b8)
                    }
                    _ => "#ffffff".to_string(),
                };
                params["color"] = json!(hex);
            }
            edit = Some(params);
        }
        if let ArtboardBackground::Custom(c) = a.background {
            let [r8, g8, b8, _] = c.to_rgba8();
            // PhotoCraft's Color Picker, as everywhere else (#2144): OK sets the background.
            if crate::widgets::color_swatch_button(ui, egui::Color32::from_rgb(r8, g8, b8), tl!("Color")).clicked() {
                pick = Some(c.to_rgb());
            }
        }
    });
    egui::CollapsingHeader::new(tl!("Responsive variants")).id_salt(key("variants")).default_open(false).show(ui, |ui| {
        if is_variant {
            let current_name = responsive.variants.iter().find(|variant| variant.artboard == layer.id).map_or_else(String::new, |variant| variant.name.clone());
            let name_id = egui::Id::new(key("variant-name"));
            let mut variant_name = ui.ctx().data(|data| data.get_temp::<String>(name_id)).unwrap_or(current_name.clone());
            ui.horizontal(|ui| {
                ui.label(tl!("Variant"));
                let response = ui.add(egui::TextEdit::singleline(&mut variant_name).desired_width(110.0).hint_text(tl!("Variant name")));
                if response.changed() {
                    ui.ctx().data_mut(|data| data.insert_temp(name_id, variant_name.clone()));
                }
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) && variant_name.trim() != current_name {
                    lifecycle_action = Some(("layer.artboard.variant.rename", json!({"layer":layer.id.0,"name":variant_name.trim()})));
                }
                if ui.button(tl!("Set as master")).clicked() {
                    lifecycle_action = Some(("layer.artboard.master.set", json!({"layer":layer.id.0})));
                }
                if ui.button(tl!("Delete")).clicked() {
                    lifecycle_action = Some(("layer.artboard.variant.delete", json!({"layer":layer.id.0})));
                }
            });
        } else if is_master {
            let variant_count = responsive.variants.iter().filter(|variant| variant.master == layer.id).count();
            ui.label(variant_capacity_label(variant_count)).on_hover_text(tl!("Responsive variants"));
            if variant_count > 0 && ui.button(tl!("Update variants")).clicked() {
                lifecycle_action = Some(("layer.artboard.variants.update", json!({"layer":layer.id.0})));
            }
        } else if ui.button(tl!("Set as master")).clicked() {
            lifecycle_action = Some(("layer.artboard.master.set", json!({"layer":layer.id.0})));
        }
        let variants: Vec<_> = responsive.variants.iter().filter(|variant| variant.master == responsive.master.unwrap_or(layer.id)).collect();
        if !variants.is_empty() {
            let switch_id = egui::Id::new(key("switch-variant"));
            let current = ui.ctx().data(|data| data.get_temp::<u64>(switch_id).unwrap_or(layer.id.0));
            let current_name =
                variants.iter().find(|variant| variant.artboard.0 == current).map_or_else(|| tl!("Switch variant").to_owned(), |variant| variant.name.clone());
            egui::ComboBox::from_id_salt(switch_id).selected_text(current_name).show_ui(ui, |ui| {
                for variant in &variants {
                    if ui.selectable_label(current == variant.artboard.0, &variant.name).clicked() {
                        ui.ctx().data_mut(|data| data.insert_temp(switch_id, variant.artboard.0));
                        lifecycle_action = Some(("layer.select", json!({"layer":variant.artboard.0})));
                    }
                }
            });
        }
        const VARIANT_SIZES: &[(&str, u32, u32)] =
            &[("4:5 · 1080×1350", 1080, 1350), ("9:16 · 1080×1920", 1080, 1920), ("16:9 · 1920×1080", 1920, 1080), ("1:1 · 1080×1080", 1080, 1080)];
        let variant_id = egui::Id::new(key("variant-size"));
        let mut variant = ui.ctx().data(|d| d.get_temp::<usize>(variant_id).unwrap_or(0)).min(VARIANT_SIZES.len() - 1);
        let layout_id = egui::Id::new(key("variant-layout"));
        let mut layout = ui.ctx().data(|d| d.get_temp::<usize>(layout_id).unwrap_or(0).min(1));
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(variant_id).selected_text(VARIANT_SIZES[variant].0).width(150.0).show_ui(ui, |ui| {
                for (i, (label, _, _)) in VARIANT_SIZES.iter().enumerate() {
                    ui.selectable_value(&mut variant, i, *label);
                }
            });
            ui.ctx().data_mut(|d| d.insert_temp(variant_id, variant));
            if ui.button(tl!("Create")).clicked() {
                let (_, width, height) = VARIANT_SIZES[variant];
                let mode = if layout == 0 { "semantic" } else { "proportional" };
                if let Err(e) = app.run("layer.artboard.createVariant", json!({"layer": layer.id.0, "width": width, "height": height, "layout": mode})) {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Reflow")).color(t.text_dim).size(12.0));
            egui::ComboBox::from_id_salt(layout_id).selected_text(if layout == 0 { tl!("Semantic reflow") } else { tl!("Proportional fit") }).show_ui(
                ui,
                |ui| {
                    ui.selectable_value(&mut layout, 0, tl!("Semantic reflow · keep type size"));
                    ui.selectable_value(&mut layout, 1, tl!("Proportional fit · scale all"));
                },
            );
            ui.ctx().data_mut(|d| d.insert_temp(layout_id, layout));
        });
        if ui.button(tl!("Overview · all formats")).clicked() {
            app.artboard_overview.open = true;
        }
    });
    egui::CollapsingHeader::new(tl!("Safe area")).id_salt(key("safe-area")).default_open(false).show(ui, |ui| {
        let areas: Vec<_> = responsive.safe_areas.iter().filter(|area| area.artboard == layer.id).collect();
        let selected_id = egui::Id::new(key("safe-area-selection"));
        let default_name = areas.first().map_or_else(|| "Content".to_owned(), |area| area.name.clone());
        let mut selected_name = ui.ctx().data(|data| data.get_temp::<String>(selected_id)).unwrap_or(default_name);
        if !areas.iter().any(|area| area.name == selected_name) && !areas.is_empty() {
            selected_name = areas[0].name.clone();
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(selected_id).selected_text(&selected_name).show_ui(ui, |ui| {
                for area in &areas {
                    ui.selectable_value(&mut selected_name, area.name.clone(), &area.name);
                }
            });
            ui.ctx().data_mut(|data| data.insert_temp(selected_id, selected_name.clone()));
            if ui.button(tl!("Add")).clicked() {
                let mut suffix = areas.len().saturating_add(1);
                let name = loop {
                    let candidate = format!("{} {suffix}", tl!("Safe area"));
                    if !areas.iter().any(|area| area.name == candidate) { break candidate }
                    suffix = suffix.saturating_add(1);
                };
                selected_name = name.clone();
                ui.ctx().data_mut(|data| data.insert_temp(selected_id, name.clone()));
                safe_area_action = Some(("layer.artboard.safeArea.set", json!({"layer":layer.id.0,"name":name,"top":0.05,"right":0.05,"bottom":0.05,"left":0.05,"requiredRoles":["cta"]})));
            }
            if areas.iter().any(|area| area.name == selected_name) && ui.button(tl!("Delete")).clicked() {
                safe_area_action = Some(("layer.artboard.safeArea.delete", json!({"layer":layer.id.0,"name":selected_name})));
            }
        });
        let existing = responsive.safe_areas.iter().find(|area| area.artboard == layer.id && area.name == selected_name).cloned();
        let inset = existing.as_ref().map_or(0.05, |area| area.insets.top);
        let mut top = ui.ctx().data(|data| data.get_temp::<f32>(egui::Id::new(key("safe-top")))).unwrap_or(inset);
        let mut right = ui.ctx().data(|data| data.get_temp::<f32>(egui::Id::new(key("safe-right")))).unwrap_or(existing.as_ref().map_or(0.05, |area| area.insets.right));
        let mut bottom = ui.ctx().data(|data| data.get_temp::<f32>(egui::Id::new(key("safe-bottom")))).unwrap_or(existing.as_ref().map_or(0.05, |area| area.insets.bottom));
        let mut left = ui.ctx().data(|data| data.get_temp::<f32>(egui::Id::new(key("safe-left")))).unwrap_or(existing.as_ref().map_or(0.05, |area| area.insets.left));
        let mut changed = false;
        ui.label(tl!("Insets as a fraction of artboard size"));
        let mut commit = false;
        ui.horizontal(|ui| {
            for (value, label) in [(&mut top, "Top"), (&mut right, "Right"), (&mut bottom, "Bottom"), (&mut left, "Left")] {
                let response = ui.add(egui::DragValue::new(value).range(0.0..=0.5).speed(0.005).prefix(format!("{} ", tl!(label))));
                changed |= response.changed();
                commit |= response.drag_stopped() || response.lost_focus();
            }
        });
        if changed {
            let values = [top, right, bottom, left];
            for (side, value) in ["top", "right", "bottom", "left"].into_iter().zip(values) {
                ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(key(&format!("safe-{side}"))), value));
            }
            if commit {
                safe_area_action = Some(("layer.artboard.safeArea.set", json!({"layer":layer.id.0,"name":existing.as_ref().map_or(selected_name.as_str(), |area| area.name.as_str()),"top":top,"right":right,"bottom":bottom,"left":left,"requiredRoles":existing.as_ref().map_or_else(|| vec!["cta".to_owned()], |area| area.required_roles.clone())})));
            }
        }
    });
    if let Some((command, params)) = lifecycle_action
        && let Err(error) = app.run(command, params)
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    if let Some((command, params)) = safe_area_action
        && let Err(error) = app.run(command, params)
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    if let Some(p) = edit
        && let Err(e) = app.run("layer.artboard.set", p)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if let Some(rgb) = pick {
        let title = crate::color_picker_ui::title_for(tl!("Color"));
        crate::color_picker_ui::open_for_command(app, &title, rgb, "layer.artboard.set", json!({"layer": layer.id.0, "background": "custom"}));
    }
}

fn variant_capacity_label(count: usize) -> String {
    format!("{} · {count}/{}", tl!("Master"), photocraft_doc::responsive::MAX_RESPONSIVE_VARIANTS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::{invoke as menu, is_live, menu_items};

    fn app() -> (PhotocraftApp, egui::Context) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
        app.sync_views();
        (app, egui::Context::default())
    }

    #[test]
    fn master_shows_current_variant_count_and_document_limit() {
        assert!(variant_capacity_label(7).ends_with("7/64"));
    }

    #[test]
    fn pasteboard_is_the_canvas_minus_the_boards() {
        let (mut app, _) = app();
        app.run("layer.new.artboard", json!({"rect": [0, 0, 40, 50]})).unwrap();
        app.run("layer.new.artboard", json!({"rect": [60, 10, 40, 30]})).unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let xf = ViewXform {
            rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 50.0)),
            zoom: 1.0,
            center: [50.0, 25.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        };
        let rects = pasteboard_rects(&xf, &doc);
        let area: f32 = rects.iter().map(|r| r.area()).sum();
        // 100×50 canvas − 40×50 − 40×30 boards.
        assert!((area - (5000.0 - 2000.0 - 1200.0)).abs() < 0.5, "{rects:?}");
        assert!(rects.iter().all(|r| !r.intersects(Rect::from_min_max(Pos2::new(0.5, 0.5), Pos2::new(39.5, 49.5)))));
    }

    #[test]
    fn safe_area_overlay_uses_fractional_insets_and_clamps_bad_values() {
        let board = photocraft_geom::Rect::new(0, 0, 100, 80);
        assert_eq!(
            safe_area_bounds(board, photocraft_doc::responsive::SafeInsets { top: 0.1, right: 0.2, bottom: 0.25, left: 0.3 }),
            photocraft_geom::Rect::new(30, 8, 80, 60)
        );
        assert_eq!(
            safe_area_bounds(board, photocraft_doc::responsive::SafeInsets { top: f32::NAN, right: 0.0, bottom: 0.0, left: f32::INFINITY }),
            photocraft_geom::Rect::new(50, 0, 100, 80)
        );
    }

    #[test]
    fn fit_artboard_and_menu_items() {
        let (mut app, ctx) = app();
        for id in [
            "view.fitArtboardOnScreen",
            "view.artboardOverview",
            "view.show.safeAreas",
            "window.panel.layerComps",
            "layer.new.artboard",
            "file.export.artboardsToFiles",
            "file.export.artboardsToPdf",
            "file.export.layerCompsToFiles",
            "view.clearSelectedArtboardGuides",
        ] {
            assert!(is_live(id), "{id}");
        }
        let enabled = |app: &PhotocraftApp, id: &str| menu_items(app).into_iter().find(|i| i.id == id).is_some_and(|i| i.enabled);
        let checked = |app: &PhotocraftApp, id: &str| menu_items(app).into_iter().find(|i| i.id == id).and_then(|i| i.checked);
        assert_eq!(checked(&app, "view.show.safeAreas"), Some(false));
        menu(&mut app, &ctx, "view.show.safeAreas", json!({})).unwrap();
        assert!(app.ui.extras.safe_areas);
        assert_eq!(checked(&app, "view.show.safeAreas"), Some(true));
        assert!(!enabled(&app, "view.fitArtboardOnScreen"));
        assert!(!app.artboard_overview.open);
        // Menu clicks without params open dialogs for the parameterised commands.
        let r = menu(&mut app, &ctx, "layer.new.artboard", json!({})).unwrap();
        assert!(r.get("dialog").is_some());
        app.run("layer.new.artboard", json!({"rect": [200, 0, 50, 20]})).unwrap();
        assert!(enabled(&app, "view.fitArtboardOnScreen"));
        menu(&mut app, &ctx, "view.artboardOverview", json!({})).unwrap();
        assert!(app.artboard_overview.open);
        app.artboard_overview.open = false;
        let r = menu(&mut app, &ctx, "view.fitArtboardOnScreen", json!({})).unwrap();
        assert_eq!(r["bounds"], json!([200, 0, 250, 20]));
        let i = app.session.active_index().unwrap();
        assert_eq!(app.ui.views[i].center, [225.0, 10.0]);
        assert!(menu(&mut app, &ctx, "file.export.artboardsToFiles", json!({})).unwrap().get("dialog").is_some());
        // Window › Layer Comps shows the third History tab.
        menu(&mut app, &ctx, "window.panel.layerComps", json!({})).unwrap();
        assert!(app.ui.panels.history);
        assert_eq!(app.ui.dock_tabs.history, 2);
    }

    #[test]
    fn fit_artboard_centres_a_board_with_large_coordinates() {
        // #981: `(b.x0 + b.x1) as f32` overflowed i32 for boards near ±2^30 or the i32 edges,
        // panicking outside any dispatch guard.
        for (params, want) in [
            (json!({"x": 2147483000, "y": 0, "width": 600, "height": 1000}), [(2147483000.0 + 2147483600.0) / 2.0, 500.0]),
            (json!({"x": 1073741824, "width": 50, "height": 20}), [1073741849.0, 10.0]),
            (json!({"y": -1073741840, "width": 50, "height": 20}), [25.0, -1073741830.0]),
        ] {
            let (mut app, ctx) = app();
            let i = app.session.active_index().unwrap();
            app.run("layer.new.artboard", params.clone()).unwrap();
            app.sync_views();
            menu(&mut app, &ctx, "view.fitArtboardOnScreen", json!({})).unwrap();
            // The view centre is f32, so compare against the true midpoint rounded to f32.
            let c = app.ui.views[i].center;
            assert_eq!(c, [want[0] as f32, want[1] as f32], "{params}");
        }
    }

    fn drag(app: &mut PhotocraftApp, from: [f64; 2], by: [f64; 2], mods: egui::Modifiers) {
        use crate::canvas::{ToolEvent, tool_event};
        tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
        for t in [0.5, 1.0] {
            tool_event(app, ToolEvent::Move { x: from[0] + by[0] * t, y: from[1] + by[1] * t, pressure: 1.0 }, mods);
        }
        tool_event(app, ToolEvent::Up { x: from[0] + by[0], y: from[1] + by[1] }, mods);
    }

    fn boards(app: &PhotocraftApp) -> Vec<photocraft_geom::Rect> {
        app.session.active().unwrap().doc.artboards().iter().map(|b| b.2.rect).collect()
    }

    /// #1531: the Move tool drags a board by an empty spot on it (Auto-Select picks the board, not
    /// the Background under it), and ⌥-drag drags a copy off the original.
    #[test]
    fn move_tool_drags_an_artboard_by_its_empty_area() {
        let (mut app, _) = app();
        app.run("layer.new.artboard", json!({"rect": [0, 0, 40, 40]})).unwrap();
        let bg = app.session.active().unwrap().doc.layers[0].id.0;
        app.run("layer.select", json!({"layer": bg})).unwrap();
        app.ui.tool = crate::state::Tool::Move;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        drag(&mut app, [20.0, 20.0], [10.0, 5.0], egui::Modifiers::NONE);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(10, 5, 50, 45)]);
        let st = app.session.active().unwrap();
        assert_eq!(st.active_layer, Some(st.doc.artboards()[0].0));
        // ⌥-drag: the copy follows the pointer, the original stays.
        drag(&mut app, [20.0, 20.0], [50.0, 0.0], egui::Modifiers::ALT);
        let mut got = boards(&app);
        got.sort_by_key(|r| r.x0);
        assert_eq!(got, vec![photocraft_geom::Rect::new(10, 5, 50, 45), photocraft_geom::Rect::new(60, 5, 100, 45)]);
        // One undo step takes the copy back.
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app).len(), 1);
    }

    #[test]
    fn comp_target_follows_selection_then_last_applied() {
        let (mut app, _) = app();
        assert_eq!(crate::comps_ui::target(&app), None);
        let a = app.run("layerComp.new", json!({})).unwrap()["comp"].as_u64().unwrap() as u32;
        let b = app.run("layerComp.new", json!({})).unwrap()["comp"].as_u64().unwrap() as u32;
        assert_eq!(crate::comps_ui::target(&app), Some(b));
        app.ui.layer_comp_selected = Some(a);
        assert_eq!(crate::comps_ui::target(&app), Some(a));
        app.ui.layer_comp_selected = Some(999);
        assert_eq!(crate::comps_ui::target(&app), Some(b));
    }
}
