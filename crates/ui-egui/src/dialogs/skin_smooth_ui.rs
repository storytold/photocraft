//! Skin Smooth's session-only colour samples and independent, read-only canvas mask.

use craft::{
    behavior::cursor::CursorBadge,
    keymap::Mods,
    ui::dialog::{DialogId, StackDialog},
};
use egui::{Color32, TextureHandle, vec2};
use photocraft_algo::skin_smooth_selection::{MAX_SAMPLES, valid_samples, weight};
use photocraft_color::ColorMode;
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_engine::channel_cmds::ChannelTarget;
use photocraft_geom::Rect;
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, filter_dialog::FilterPreviewKey, theme::Tokens};

const POSITIVE: &str = "positiveColors";
const NEGATIVE: &str = "negativeColors";

/// Enables this UI only for the installed Skin Smooth plug-in with the expected skinsmooth.v2 contract.
pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some("plugin.run")
        && f.get("id").and_then(Value::as_str) == Some("local.skinsmooth.skin-smooth")
        && photocraft_plugins::registry::get("local.skinsmooth.skin-smooth").is_some_and(|p| p.manifest().accelerator == "skinsmooth.v2")
}

/// Reads a bounded normalized RGB group, treating malformed or absent dialog data as empty.
fn colors(f: &Map<String, Value>, key: &str) -> Vec<[f32; 3]> {
    f.get(key).and_then(|v| serde_json::from_value(v.clone()).ok()).filter(|c: &Vec<[f32; 3]>| valid_samples(c)).unwrap_or_default()
}

/// Identifies the document revision, layer and channel/mask target to invalidate photo-specific samples.
pub(super) fn source(app: &PhotocraftApp) -> Value {
    app.session.active().map_or(Value::Null, |st| {
        json!([st.doc.id, st.revision, st.active_layer, st.channel_view.target, app.layer_mask_targeted(), app.filter_mask_targeted()])
    })
}

/// Binds a fresh dialog to its source and clears samples, sampler state, mask visibility and prior errors.
pub(crate) fn bind(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    f.insert("__skinSource".into(), source(app));
    for key in [POSITIVE, NEGATIVE] {
        f.insert(key.into(), json!([]));
    }
    f.remove("__skinTool");
    f.remove("__pointerDown");
    f.remove("__skinMask");
    f.remove("__skinError");
}

/// Source changes discard photo-specific samples, even while another tab is active or hidden.
pub(crate) fn tick(app: &mut PhotocraftApp) {
    let source = source(app);
    if let Some(d) = super::plugin_dialog::get_mut(app) {
        if owns(&d.fields) && d.fields.get("__skinSource") != Some(&source) {
            d.fields.insert("__skinSource".into(), source.clone());
            for key in [POSITIVE, NEGATIVE] {
                d.fields.insert(key.into(), json!([]));
            }
            d.fields.remove("__skinTool");
            d.fields.remove("__pointerDown");
            d.fields.remove("__skinMask");
            d.fields.remove("__skinError");
        }
    }
    let visible = super::plugin_dialog::top(app).is_some_and(|d| owns(&d.fields) && d.fields.get("__skinMask") == Some(&json!(true)));
    if !visible || supported(app).is_err() {
        app.skin_smooth_mask = None;
    }
}

/// Returns a sampleable RGB raster layer under the host's sRGB policy.
/// Rejects channel/mask targets and masked or clipped ancestors whose visible weight is not represented.
pub(super) fn supported(app: &PhotocraftApp) -> Result<LayerId, &'static str> {
    let st = app.session.active().ok_or("Open an RGB image to sample skin colours.")?;
    // Profile rejection also applies to Quick Mask and smart objects; show it before target guidance.
    photocraft_engine::plugin_cmds::skin_smooth_color_supported(&st.doc)?;
    if st.channel_view.target != ChannelTarget::Composite || st.doc.quick_mask.is_some() || app.layer_mask_targeted() || app.filter_mask_targeted() {
        return Err("Skin sampling and Mask require the RGB composite view, with layer pixels targeted.");
    }
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

/// Edits a percent control with a slider and numeric field; double-click restores its default.
fn row(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, default: f32, min: f32, focus: bool) {
    let label = super::plugin_dialog::label(key);
    let mut value = f.get(key).and_then(Value::as_f64).unwrap_or(f64::from(default)) as f32;
    ui.push_id(key, |ui| {
        ui.horizontal(|ui| {
            let t = Tokens::get(ui.ctx());
            ui.add_sized([80.0, 24.0], egui::Label::new(egui::RichText::new(&label).color(t.text_dim)).truncate()).on_hover_text(&label);
            let slider_width = (ui.available_width() - 76.0).max(60.0);
            ui.allocate_ui(vec2(slider_width, 24.0), |ui| {
                let response = ui.add(egui::Slider::new(&mut value, min..=100.0).show_value(false)).accessible_name(format!("{label} slider"));
                if response.double_clicked() {
                    value = default;
                }
            });
            let field = ui.add_sized([64.0, 24.0], egui::DragValue::new(&mut value).range(min..=100.0).max_decimals(1)).accessible_name(&label);
            if focus && !ui.is_sizing_pass() {
                field.request_focus();
                let mut edit = egui::TextEdit::load_state(ui.ctx(), field.id).unwrap_or_default();
                edit.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&value.to_string())));
                edit.store(ui.ctx(), field.id);
            }
        });
    });
    f.insert(key.into(), json!((value * 10.0).round() / 10.0));
}

/// Shows encoded source RGB samples with a hover/focus removal cross and per-sample deletion.
fn swatches(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str, empty: &str) {
    let mut samples = colors(f, key);
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        ui.add_sized([80.0, 20.0], egui::Label::new(label).truncate()).on_hover_text(label);
        if samples.is_empty() {
            ui.label(egui::RichText::new(empty).color(Tokens::get(ui.ctx()).text_dim));
        }
        for (index, &rgb) in samples.iter().enumerate() {
            let [r, g, b] = rgb.map(|v| (v * 255.0).round() as u8);
            let color = Color32::from_rgb(r, g, b);
            let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::click());
            let hint = format!("{}: {}", tl!(&format!("Remove {label} sample")), crate::color_picker_ui::hex(rgb));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &hint));
            ui.painter().rect_filled(rect, Tokens::get(ui.ctx()).radius_sm, color);
            ui.painter().rect_stroke(
                rect,
                Tokens::get(ui.ctx()).radius_sm,
                egui::Stroke::new(1.0, Tokens::get(ui.ctx()).field_border),
                egui::StrokeKind::Inside,
            );
            if response.hovered() || response.has_focus() {
                let t = Tokens::get(ui.ctx());
                let center = rect.center();
                ui.painter().circle_filled(center, 7.0, t.field);
                for direction in [-1.0, 1.0] {
                    let offset = vec2(3.0, 3.0 * direction);
                    ui.painter().line_segment([center - offset, center + offset], egui::Stroke::new(1.5, t.text));
                }
                if response.has_focus() {
                    ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                }
            }
            if response.clicked() {
                remove = Some(index);
            }
            response.on_hover_text(hint).on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    });
    if let Some(index) = remove {
        samples.remove(index);
        f.insert(key.into(), json!(samples));
        f.remove("__skinError");
    }
}

/// Draws smoothing, tone and skin-range controls, disabling sampling when the source cannot be represented.
pub(super) fn body(supported: Result<(), &str>, mask_error: Option<&str>, ui: &mut egui::Ui, f: &mut Map<String, Value>, first: bool) {
    ui.label(egui::RichText::new(tl!("Smoothing")).strong());
    for (key, default) in [("fine", 45.0), ("medium", 25.0), ("large", 10.0), ("detail", 20.0)] {
        row(ui, f, key, default, 0.0, first && key == "fine");
    }
    ui.add_space(8.0);
    ui.label(egui::RichText::new(tl!("Tone")).strong());
    for (key, min) in [("brightness", -100.0), ("warmth", -100.0), ("redness", 0.0)] {
        row(ui, f, key, 0.0, min, false);
    }
    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new(tl!("Skin range")).strong());
    let mut skin = f.get("skinOnly").and_then(Value::as_bool).unwrap_or(true);
    ui.checkbox(&mut skin, tl!("Skin Only"));
    f.insert("skinOnly".into(), json!(skin));
    ui.add_enabled_ui(skin && supported.is_ok(), |ui| {
        ui.horizontal(|ui| {
            for (tool, badge, label) in
                [("sample", "", "Sample skin colour"), ("add", "+", "Add skin colour (Shift)"), ("subtract", "−", "Subtract skin colour (Alt)")]
            {
                let selected = f.get("__skinTool").and_then(Value::as_str) == Some(tool);
                let response = ui
                    .selectable_label(selected, if badge.is_empty() { tl!("Sample").to_owned() } else { badge.to_owned() })
                    .accessible_name(tl!(&label))
                    .on_hover_text(tl!(&label));
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
        row(ui, f, "tolerance", 55.0, 0.0, false);
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
    if let Some(error) = mask_error {
        ui.add(egui::Label::new(error).wrap());
    }
}

/// Edits read-only mask visibility independently of filter Preview and the pixels committed by OK.
pub(super) fn mask_switch(supported: bool, ui: &mut egui::Ui, show: &mut bool) {
    ui.add_enabled_ui(supported, |ui| {
        ui.checkbox(show, tl!("Show Mask")).on_hover_text(tl!(
            "White: full weight; black: none. Includes the document selection and source alpha. Independent of Preview; OK always applies smoothing."
        ));
    });
}

/// Allows sampling only for the current source and top Skin Smooth dialog with Skin Only enabled.
pub(crate) fn armed(app: &PhotocraftApp) -> bool {
    supported(app).is_ok()
        && super::plugin_dialog::top(app).is_some_and(|d| {
            owns(&d.fields)
                && d.fields.get("__skinSource") == Some(&source(app))
                && d.fields.contains_key("__skinTool")
                && d.fields.get("skinOnly") == Some(&json!(true))
        })
}

/// Samples original layer pixels at document coordinates, never the composite or displayed mask.
/// Shift adds, Alt subtracts; the Sample tool replaces both groups. Invalid or transparent picks return false.
pub(crate) fn pick(app: &mut PhotocraftApp, at: [f64; 2], mods: Mods) -> bool {
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
    let Some(d) = super::plugin_dialog::get_mut(app) else { return false };
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
    } else if tool == "add" {
        // Restore the picked colour without changing the positive selection's base on exclusion.
        let tolerance = f.get("tolerance").and_then(Value::as_f64).unwrap_or(55.0) as f32;
        let mut negative = colors(f, NEGATIVE);
        negative.retain(|sample| weight(rgb, tolerance, &[*sample], &[]) == 0.0);
        f.insert(NEGATIVE.into(), json!(negative));
    }
    f.insert(key.into(), json!(samples));
    f.remove("__skinError");
    true
}

/// The canvas tool's cursor follows the sampler button and held modifiers.
pub(super) fn cursor_badge(app: &PhotocraftApp, mods: Mods) -> Option<CursorBadge> {
    let tool = super::plugin_dialog::top(app)?.fields.get("__skinTool").and_then(Value::as_str);
    if mods.shift || tool == Some("add") && !mods.alt {
        Some(CursorBadge::Add)
    } else if mods.alt || tool == Some("subtract") {
        Some(CursorBadge::Subtract)
    } else {
        None
    }
}

pub(super) fn mask_error(app: &PhotocraftApp) -> Option<String> {
    app.skin_smooth_mask.as_ref()?.error.clone()
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

/// Reports active background mask work belonging to the requested dialog.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn busy_for(app: &PhotocraftApp, dialog: DialogId) -> bool {
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

/// Uploads a computed mask or records its error, retaining the request key to avoid retrying the same failure.
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

/// Returns a current mask texture, document rectangle and clipped UVs for its owning view only.
/// The docked main view or undocked floating view owns the bounded worker; duplicate views cannot supersede it.
pub(crate) fn overlay(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    host: crate::canvas::document_display::CanvasHost,
    doc_id: photocraft_doc::DocId,
    visible: Rect,
) -> Option<(egui::TextureId, Rect, egui::Rect)> {
    if !host.is_primary(app) {
        return None;
    }
    let d = super::plugin_dialog::top(app)?;
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
    let mut k = crate::proxy::view_factor(doc.size.width.max(doc.size.height).div_ceil(1024).max(1), app.current_zoom(), doc.size.area());
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
        sources: Vec::new(),
        dialog: d.id(),
        open: app.framework.dialogs.open_serial(d.id())?,
        active: Some(layer),
        target: ChannelTarget::Composite,
        command: "skin-smooth-mask".into(),
        params: json!({"values":fields,"__previewRegion":[region.x0,region.y0,region.x1,region.y1]}),
        k,
        tool_colors: (app.session.tools.foreground, app.session.tools.background),
        scratch: crate::scratch_reads::generation(),
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
    // Keep the last mask while colours change, but never reuse it across sources or viewports.
    if !preview.key.as_ref().is_some_and(|key| crate::canvas::filter_preview::session_of(key) == crate::canvas::filter_preview::session_of(&request)) {
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
