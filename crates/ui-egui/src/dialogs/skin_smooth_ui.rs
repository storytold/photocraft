//! Skin Smooth's session-only colour samples and independent, read-only canvas mask.

use egui::{Color32, TextureHandle, vec2};
use photocraft_algo::skin_smooth_selection::{MAX_SAMPLES, valid_samples, weight};
use photocraft_color::ColorMode;
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_engine::channel_cmds::ChannelTarget;
use photocraft_geom::Rect;
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, filter_dialog::FilterPreviewKey, theme::Tokens, widgets};

const POSITIVE: &str = "positiveColors";
const NEGATIVE: &str = "negativeColors";

pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some("plugin.run")
        && f.get("id").and_then(Value::as_str) == Some("local.skinsmooth.skin-smooth")
        && photocraft_plugins::registry::get("local.skinsmooth.skin-smooth").is_some_and(|p| p.manifest().accelerator == "skinsmooth.v2")
}

fn colors(f: &Map<String, Value>, key: &str) -> Vec<[f32; 3]> {
    f.get(key).and_then(|v| serde_json::from_value(v.clone()).ok()).filter(|c: &Vec<[f32; 3]>| valid_samples(c)).unwrap_or_default()
}

fn source(app: &PhotocraftApp) -> Value {
    app.session.active().map_or(Value::Null, |st| {
        json!([st.doc.id, st.revision, st.active_layer, st.channel_view.target, app.layer_mask_targeted(), app.filter_mask_targeted()])
    })
}

pub(crate) fn bind(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    f.insert("__skinSource".into(), source(app));
    for key in [POSITIVE, NEGATIVE] {
        f.insert(key.into(), json!([]));
    }
    f.remove("__skinTool");
    f.remove("__skinMask");
    f.remove("__skinError");
}

/// Source changes discard photo-specific samples, even while another tab is active or hidden.
pub(crate) fn tick(app: &mut PhotocraftApp) {
    let source = source(app);
    for d in &mut app.ui.dialogs {
        if owns(&d.fields) && d.fields.get("__skinSource") != Some(&source) {
            d.fields.insert("__skinSource".into(), source.clone());
            for key in [POSITIVE, NEGATIVE] {
                d.fields.insert(key.into(), json!([]));
            }
            d.fields.remove("__skinTool");
            d.fields.remove("__skinMask");
            d.fields.remove("__skinError");
        }
    }
    let visible = app.ui.dialogs.last().is_some_and(|d| owns(&d.fields) && d.fields.get("__skinMask") == Some(&json!(true)));
    if !visible || supported(app).is_err() {
        app.skin_smooth_mask = None;
    }
}

fn supported(app: &PhotocraftApp) -> Result<LayerId, &'static str> {
    let st = app.session.active().ok_or("Open an RGB image to sample skin colours.")?;
    if st.channel_view.target != ChannelTarget::Composite || st.doc.quick_mask.is_some() || app.layer_mask_targeted() || app.filter_mask_targeted() {
        return Err("Skin sampling and Mask require the RGB composite view, with layer pixels targeted.");
    }
    photocraft_engine::plugin_cmds::skin_smooth_color_supported(&st.doc)?;
    let id = st.active_layer.ok_or("Select an RGB pixel layer to sample skin colours.")?;
    let path = st.doc.path_of(id).ok_or("The source layer is no longer available.")?;
    for end in 1..=path.len() {
        if let Some(l) = st.doc.layer_at(path.get(..end).unwrap_or(&[]))
            && (l.mask.is_some() || l.vector_mask.is_some() || l.clipped)
        {
            return Err("Skin sampling and Mask are unavailable on masked or clipped layers. Target an unmasked RGB pixel layer.");
        }
    }
    match st.doc.layer(id).map(|l| &l.content) {
        Some(LayerContent::Raster(s)) if s.format().mode == ColorMode::Rgb => Ok(id),
        _ => Err("Skin sampling and Mask require an RGB pixel layer. Rasterize a copy of a Smart Object first."),
    }
}

fn row(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, default: f32, min: f32) {
    let label = crate::filter_dialog::label(key);
    let mut value = f.get(key).and_then(Value::as_f64).unwrap_or(f64::from(default)) as f32;
    ui.push_id(key, |ui| {
        ui.horizontal(|ui| {
            let t = Tokens::get(ui.ctx());
            ui.add_sized([80.0, 24.0], egui::Label::new(egui::RichText::new(&label).color(t.text_dim)).truncate()).on_hover_text(&label);
            let slider_width = (ui.available_width() - 76.0).max(60.0);
            ui.allocate_ui(vec2(slider_width, 24.0), |ui| {
                let response = widgets::slider(ui, &mut value, min..=100.0, None).accessible_name(format!("{label} slider"));
                if response.double_clicked() {
                    value = default;
                }
            });
            widgets::value_field(ui, &mut value, min..=100.0, "", 64.0).accessible_name(&label);
        });
    });
    f.insert(key.into(), json!((value * 10.0).round() / 10.0));
}

fn swatches(ui: &mut egui::Ui, f: &Map<String, Value>, key: &str, label: &str, empty: &str) {
    let samples = colors(f, key);
    ui.horizontal_wrapped(|ui| {
        ui.add_sized([80.0, 20.0], egui::Label::new(label).truncate()).on_hover_text(label);
        if samples.is_empty() {
            ui.label(egui::RichText::new(empty).color(Tokens::get(ui.ctx()).text_dim));
        }
        for rgb in samples {
            let [r, g, b] = rgb.map(|v| (v * 255.0).round() as u8);
            let color = Color32::from_rgb(r, g, b);
            let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColorWell, ui.is_enabled(), format!("{label}: {}", crate::color_picker_ui::hex(rgb))));
            ui.painter().rect_filled(rect, Tokens::get(ui.ctx()).radius_sm, color);
            ui.painter().rect_stroke(
                rect,
                Tokens::get(ui.ctx()).radius_sm,
                egui::Stroke::new(1.0, Tokens::get(ui.ctx()).field_border),
                egui::StrokeKind::Inside,
            );
            response.on_hover_text(format!("{label}: {}", crate::color_picker_ui::hex(rgb)));
        }
    });
}

pub(crate) fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.label(egui::RichText::new(tl!("Smoothing")).strong());
    for (key, default) in [("fine", 45.0), ("medium", 25.0), ("large", 10.0), ("detail", 20.0)] {
        row(ui, f, key, default, 0.0);
    }
    ui.add_space(8.0);
    ui.label(egui::RichText::new(tl!("Tone")).strong());
    for (key, min) in [("brightness", -100.0), ("warmth", -100.0), ("redness", 0.0)] {
        row(ui, f, key, 0.0, min);
    }
    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new(tl!("Skin range")).strong());
    let mut skin = f.get("skinOnly").and_then(Value::as_bool).unwrap_or(true);
    widgets::checkbox(ui, &mut skin, tl!("Skin Only"));
    f.insert("skinOnly".into(), json!(skin));
    let supported = supported(app);
    ui.add_enabled_ui(skin && supported.is_ok(), |ui| {
        ui.horizontal(|ui| {
            for (tool, badge, label) in
                [("sample", "", "Sample skin colour"), ("add", "+", "Add skin colour (Shift)"), ("subtract", "−", "Subtract skin colour (Alt)")]
            {
                let selected = f.get("__skinTool").and_then(Value::as_str) == Some(tool);
                let response = crate::color_range_ui::eyedropper(ui, badge, selected, tl!(&label));
                if response.clicked() {
                    if selected {
                        f.remove("__skinTool");
                    } else {
                        f.insert("__skinTool".into(), json!(tool));
                    }
                }
            }
            let sampled = !colors(f, POSITIVE).is_empty() || !colors(f, NEGATIVE).is_empty();
            if ui.add_enabled(sampled, egui::Button::new(tl!("Clear samples"))).clicked() {
                f.insert(POSITIVE.into(), json!([]));
                f.insert(NEGATIVE.into(), json!([]));
                f.remove("__skinError");
            }
        });
        row(ui, f, "tolerance", 55.0, 0.0);
        swatches(ui, f, POSITIVE, tl!("Include"), tl!("Automatic skin tones"));
        swatches(ui, f, NEGATIVE, tl!("Exclude"), tl!("None"));
    });
    ui.add_space(4.0);
    let hint = match supported {
        Err(reason) => tl!(&reason),
        Ok(_) if f.contains_key("__skinTool") && skin => tl!("Sample original layer colours. Shift adds; Alt subtracts."),
        Ok(_) => tl!("Colours match across the layer; samples last for this dialog."),
    };
    ui.add(egui::Label::new(egui::RichText::new(hint).color(Tokens::get(ui.ctx()).text_dim)).wrap());
    if let Some(error) = f.get("__skinError").and_then(Value::as_str) {
        ui.add(egui::Label::new(error).wrap());
    }
    if let Some(error) = app.skin_smooth_mask.as_ref().and_then(|p| p.error.as_deref()) {
        ui.add(egui::Label::new(error).wrap());
    }
}

pub(crate) fn mask_switch(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut show = f.get("__skinMask").and_then(Value::as_bool).unwrap_or(false);
    ui.add_enabled_ui(supported(app).is_ok(), |ui| {
        widgets::checkbox(ui, &mut show, tl!("Show Mask")).on_hover_text(tl!(
            "White: full weight; black: none. Includes the document selection and source alpha. Independent of Preview; OK always applies smoothing."
        ));
    });
    f.insert("__skinMask".into(), json!(show));
}

pub(crate) fn armed(app: &PhotocraftApp) -> bool {
    supported(app).is_ok()
        && app.ui.dialogs.last().is_some_and(|d| {
            owns(&d.fields)
                && d.fields.get("__skinSource") == Some(&source(app))
                && d.fields.contains_key("__skinTool")
                && d.fields.get("skinOnly") == Some(&json!(true))
        })
}

pub(crate) fn pick(app: &mut PhotocraftApp, at: [f64; 2], mods: egui::Modifiers) -> bool {
    if !armed(app) || at.iter().any(|v| !v.is_finite()) {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    if at[0] < 0.0 || at[1] < 0.0 || at[0] >= f64::from(st.doc.size.width) || at[1] >= f64::from(st.doc.size.height) {
        return false;
    }
    let Some(surface) = st.active_layer.and_then(|id| st.doc.layer(id)).and_then(|l| l.surface()) else { return false };
    // Surface coordinates already include layer translations; never sample the composited canvas.
    let pixel = surface.pixel(at[0].floor() as i32, at[1].floor() as i32);
    if pixel.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) || surface.format().alpha && pixel.last().copied().unwrap_or(0.0) == 0.0 {
        return false;
    }
    let Some(rgb) = pixel.get(..3).and_then(|p| <[f32; 3]>::try_from(p).ok()).filter(|rgb| valid_samples(&[*rgb])) else { return false };
    let Some(d) = app.ui.dialogs.last_mut() else { return false };
    let f = &mut d.fields;
    let tool = if mods.shift {
        "add"
    } else if mods.alt {
        "subtract"
    } else {
        f.get("__skinTool").and_then(Value::as_str).unwrap_or("sample")
    };
    let key = if tool == "subtract" { NEGATIVE } else { POSITIVE };
    let mut samples = if tool == "sample" { Vec::new() } else { colors(f, key) };
    if !samples.contains(&rgb) && samples.len() >= MAX_SAMPLES {
        f.insert("__skinError".into(), json!(tl!("Up to 16 colours per group. Clear samples to start again.")));
        return false;
    }
    if !samples.contains(&rgb) {
        samples.push(rgb);
    }
    if tool == "sample" {
        f.insert(NEGATIVE.into(), json!([]));
    }
    f.insert(key.into(), json!(samples));
    f.remove("__skinError");
    true
}

pub(crate) fn canvas_eyedropper(app: &mut PhotocraftApp, ctx: &egui::Context, over: egui::Pos2, press: Option<[f64; 2]>) -> bool {
    if !armed(app) {
        return false;
    }
    if app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise {
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    } else {
        crate::icons::cursor(ctx, "pipette", over, vec2(2.0, 22.0) / 24.0, 20.0);
        ctx.set_cursor_icon(egui::CursorIcon::None);
    }
    let mods = ctx.input(|i| i.modifiers);
    let tool = app.ui.dialogs.last().and_then(|d| d.fields.get("__skinTool")).and_then(Value::as_str).unwrap_or("sample");
    let badge = if mods.shift || tool == "add" && !mods.alt {
        "+"
    } else if mods.alt || tool == "subtract" {
        "−"
    } else {
        ""
    };
    if !badge.is_empty() {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("skin-sample-cursor")));
        let at = over + vec2(14.0, -8.0);
        // Like Color Range's cursor: readable over both black and white mask pixels.
        painter.text(at + vec2(1.0, 1.0), egui::Align2::CENTER_CENTER, badge, crate::theme::semibold(13.0), Color32::from_black_alpha(200));
        painter.text(at, egui::Align2::CENTER_CENTER, badge, crate::theme::semibold(13.0), Color32::WHITE);
    }
    if let Some(at) = press {
        pick(app, at, mods);
    }
    true
}

#[derive(Default)]
pub(crate) struct MaskPreview {
    key: Option<FilterPreviewKey>,
    texture: Option<TextureHandle>,
    rect: Rect,
    error: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    worker: super::filter_preview_worker::Worker,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn busy_for(app: &PhotocraftApp, dialog: u64) -> bool {
    app.skin_smooth_mask.as_ref().is_some_and(|p| p.worker.busy_for(dialog))
}

/// Weights of stored source pixels, not a composite visibility estimate. Layer masks / clipping
/// are rejected above: their visible contribution would require a different mask definition.
fn mask_buffer(doc: &Document, layer: LayerId, f: &Map<String, Value>, region: Rect, k: u32, cancel: &dyn Fn() -> bool) -> Option<photocraft_compose::Buffer> {
    photocraft_engine::plugin_cmds::skin_smooth_color_supported(doc).ok()?;
    let surface = doc.layer(layer)?.surface()?;
    let plugin = photocraft_plugins::registry::get("local.skinsmooth.skin-smooth")?;
    let area = plugin.output_area(surface, doc.bounds(), doc.selection.as_ref());
    let mut buffer = photocraft_compose::Buffer::filled(region, [0.0, 0.0, 0.0, 1.0]);
    let positive = colors(f, POSITIVE);
    let negative = colors(f, NEGATIVE);
    let tolerance = f.get("tolerance").and_then(Value::as_f64).unwrap_or(55.0) as f32;
    let skin = f.get("skinOnly").and_then(Value::as_bool).unwrap_or(true);
    for (row, y) in (region.y0..region.y1).enumerate() {
        if cancel() {
            return None;
        }
        for (col, x) in (region.x0..region.x1).enumerate() {
            let (sx, sy) = (x.saturating_mul(k as i32), y.saturating_mul(k as i32));
            if sx < area.x0 || sy < area.y0 || sx >= area.x1 || sy >= area.y1 {
                continue;
            }
            let p = surface.pixel(sx, sy);
            if p.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return None;
            }
            let rgb = <[f32; 3]>::try_from(p.get(..3)?).ok()?;
            let alpha = if surface.format().alpha { p.last().copied().unwrap_or(0.0) } else { 1.0 };
            let selected = doc.selection.as_ref().map_or(1.0, |s| s.pixel(sx, sy).first().copied().unwrap_or(0.0).clamp(0.0, 1.0));
            let w = if skin { weight(rgb, tolerance, &positive, &negative) } else { 1.0 } * selected * alpha;
            let pixel = buffer.px.get_mut(row * region.width() as usize + col)?;
            *pixel = [w, w, w, 1.0];
        }
    }
    Some(buffer)
}

fn publish(preview: &mut MaskPreview, ctx: &egui::Context, key: FilterPreviewKey, buffer: Option<photocraft_compose::Buffer>) {
    preview.key = Some(key.clone());
    let Some(buffer) = buffer else {
        preview.texture = None;
        preview.error = Some(tl!("Mask unavailable. Skin Smooth requires SDR RGB values between 0 and 1.").into());
        return;
    };
    preview.error = None;
    let size = [buffer.rect.width() as usize, buffer.rect.height() as usize];
    let pixels = buffer.px.iter().map(|p| Color32::from_gray((p[0].clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    let image = egui::ColorImage::new(size, pixels);
    match &mut preview.texture {
        Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
        None => preview.texture = Some(ctx.load_texture("skin-smooth-mask", image, egui::TextureOptions::LINEAR)),
    }
    preview.rect = Rect::new(
        buffer.rect.x0.saturating_mul(key.k as i32),
        buffer.rect.y0.saturating_mul(key.k as i32),
        buffer.rect.x1.saturating_mul(key.k as i32),
        buffer.rect.y1.saturating_mul(key.k as i32),
    );
}

pub(crate) fn overlay(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    doc_id: photocraft_doc::DocId,
    visible: Rect,
) -> Option<(egui::TextureId, Rect, egui::Rect)> {
    let d = app.ui.dialogs.last()?;
    if !owns(&d.fields) || d.fields.get("__skinMask") != Some(&json!(true)) || d.fields.get("__skinSource") != Some(&source(app)) {
        return None;
    }
    let layer = supported(app).ok()?;
    let st = app.session.active()?;
    if st.doc.id != doc_id {
        return None;
    }
    let doc = st.doc.clone();
    let bounds = doc.bounds();
    let mut k = crate::proxy::view_factor(doc.size.width.max(doc.size.height).div_ceil(1024).max(1), app.current_zoom());
    let visible = visible.intersect(&doc.bounds());
    if visible.is_empty() {
        return None;
    }
    let region = loop {
        let q = k as i32;
        let up = |v: i32| v.div_euclid(q) + i32::from(v.rem_euclid(q) != 0);
        let r = Rect::new(visible.x0.div_euclid(q), visible.y0.div_euclid(q), up(visible.x1), up(visible.y1));
        if u64::from(r.width()) * u64::from(r.height()) <= 1_048_576 {
            break r;
        }
        k = k.saturating_mul(2);
    };
    let mut fields = d.fields.clone();
    fields.retain(|key, _| matches!(key.as_str(), POSITIVE | NEGATIVE | "tolerance" | "skinOnly"));
    let request = FilterPreviewKey {
        doc: doc_id,
        revision: st.revision,
        dialog: d.id,
        active: Some(layer),
        target: ChannelTarget::Composite,
        command: "skin-smooth-mask".into(),
        params: json!({"values":fields,"__previewRegion":[region.x0,region.y0,region.x1,region.y1]}),
        k,
    };
    let preview = app.skin_smooth_mask.get_or_insert_default();
    #[cfg(not(target_arch = "wasm32"))]
    if app.background_jobs {
        preview.worker.supersede(&request, crate::gpu_canvas::now_ms());
        let cancelled = preview.worker.cancelled();
        if let Some((finished, computed)) = preview.worker.poll()
            && finished == request
            && !cancelled
        {
            publish(preview, ctx, finished, computed.ok().and_then(|c| c.result).map(|(_, buffer)| buffer));
        }
        if preview.key.as_ref() != Some(&request) && !preview.worker.busy() {
            let task = request.clone();
            if let Err(error) = preview.worker.start(task.clone(), ctx.clone(), move |cancel| {
                let buffer = mask_buffer(&doc, layer, &fields, region, task.k, &|| cancel.cancelled());
                super::filter_preview_worker::Computed { result: buffer.map(|b| (doc, b)), ms: 0.0 }
            }) {
                crate::notices::error(app, error);
                return None;
            }
        }
        if preview.worker.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    } else if preview.key.as_ref() != Some(&request) {
        publish(preview, ctx, request.clone(), mask_buffer(&doc, layer, &fields, region, k, &|| false));
    }
    #[cfg(target_arch = "wasm32")]
    if preview.key.as_ref() != Some(&request) {
        publish(preview, ctx, request.clone(), mask_buffer(&doc, layer, &fields, region, k, &|| false));
    }
    // Never repaint with a mask from a previous sample set, viewport, source or dialog.
    if preview.key.as_ref() != Some(&request) {
        return None;
    }
    // A partial proxy pixel must stop at the canvas edge, including when rotated or flipped.
    let area = preview.rect.intersect(&bounds);
    let uv = egui::Rect::from_min_max(
        egui::pos2((area.x0 - preview.rect.x0) as f32 / preview.rect.width() as f32, (area.y0 - preview.rect.y0) as f32 / preview.rect.height() as f32),
        egui::pos2((area.x1 - preview.rect.x0) as f32 / preview.rect.width() as f32, (area.y1 - preview.rect.y0) as f32 / preview.rect.height() as f32),
    );
    Some((preview.texture.as_ref()?.id(), area, uv))
}

#[cfg(test)]
mod tests;
