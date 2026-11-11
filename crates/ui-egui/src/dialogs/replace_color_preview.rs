//! Image › Adjustments › Replace Color's Selection / Image thumbnail (#6281), as in Photoshop:
//! Selection shows the pixels the dialog's settings change (white = fully, grey = partly, black =
//! not at all), Image shows the picture to sample from. Clicking the thumbnail samples like a click
//! on the canvas (Shift adds, Alt subtracts). The mask comes from the command's own matcher
//! (`ReplaceColorMatch`) on a reduced copy of the document, so it shows what OK would change.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::{DocId, Document};
use photocraft_engine::adjust_cmds::ReplaceColorMatch;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The thumbnail's frame, in points.
const THUMB: egui::Vec2 = egui::vec2(200.0, 150.0);
/// Longest side of the reduced document the mask is judged on, in pixels.
const PREVIEW: u32 = 400;
/// Which of the two views the dialog shows (a dialog-only field, never a command param).
const VIEW: &str = "__view";

/// The cached reduced document and the texture last drawn from it.
#[derive(Clone)]
struct Cache {
    doc: DocId,
    revision: u64,
    /// One proxy pixel stands for `k` document pixels.
    k: u32,
    proxy: Arc<Document>,
    key: u64,
    shown: Option<(egui::TextureHandle, [usize; 2])>,
    error: Option<String>,
}

fn cache_id() -> egui::Id {
    egui::Id::unique("replace-color-preview")
}

/// The pixels Replace Color edits, on the proxy: the active layer's own, or the composite when the
/// active layer has no pixels of its own.
fn layer_pixels(app: &PhotocraftApp, proxy: &Document) -> Vec<[f32; 4]> {
    let area = proxy.bounds();
    let layer = app.session.active().and_then(|d| d.active_layer).and_then(|id| proxy.layer(id)).and_then(|l| l.surface());
    match layer {
        Some(surf) => {
            let mut px = vec![[0.0f32; 4]; area.width() as usize * area.height() as usize];
            surf.read_rgba_into(area, &mut px);
            px
        }
        None => photocraft_compose::render(proxy, area).into_px(),
    }
}

/// The reduced document the thumbnail is judged on, and how many document pixels each of its
/// pixels stands for.
fn proxy_of(doc: &Document) -> (u32, Arc<Document>) {
    let k = doc.size.width.max(doc.size.height).max(1).div_ceil(PREVIEW).max(1);
    (k, Arc::new(crate::proxy::proxy_document(doc, k)))
}

/// The Selection view: how much (0..1) OK would change each proxy pixel, from the command's own
/// matcher, or why it can't say.
fn selection_mask(app: &PhotocraftApp, proxy: &Document, k: u32, params: &Value) -> Result<Vec<f32>, String> {
    let m = ReplaceColorMatch::new(&app.session, params).map_err(|e| e.to_string())?;
    let px = layer_pixels(app, proxy);
    Ok(m.coverage(&px, proxy.size.width as usize, k as f32, [0.0, 0.0]).into_iter().map(|v| v.clamp(0.0, 1.0)).collect())
}

/// The Selection view's values for the dialog's fields and the proxy's size, as the thumbnail
/// draws them.
#[cfg(test)]
fn mask_values(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<(Vec<f32>, [usize; 2]), String> {
    let doc = app.session.active().ok_or("no document")?.doc.clone();
    let (k, proxy) = proxy_of(&doc);
    let mask = selection_mask(app, &proxy, k, &crate::filter_dialog::params_of(f))?;
    Ok((mask, [proxy.size.width as usize, proxy.size.height as usize]))
}

fn key_of(app: &PhotocraftApp, params: &Value, image: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    params.to_string().hash(&mut h);
    image.hash(&mut h);
    app.session.tools.foreground.map(f32::to_bits).hash(&mut h);
    app.session.active().and_then(|d| d.active_layer).hash(&mut h);
    h.finish()
}

/// What the thumbnail shows for the dialog's fields: the texture and its size in proxy pixels, or
/// why it can't be drawn.
fn refresh(app: &PhotocraftApp, ctx: &egui::Context, f: &Map<String, Value>, image: bool) -> Option<Cache> {
    let st = app.session.active()?;
    let (doc_id, revision) = (st.doc.id, st.revision);
    let mut cache = ctx.data(|d| d.get_temp::<Cache>(cache_id())).filter(|c| c.doc == doc_id && c.revision == revision).unwrap_or_else(|| {
        let (k, proxy) = proxy_of(&st.doc);
        Cache { doc: doc_id, revision, k, proxy, key: 0, shown: None, error: None }
    });
    let params = crate::filter_dialog::params_of(f);
    let key = key_of(app, &params, image);
    if cache.key != key || (cache.shown.is_none() && cache.error.is_none()) {
        let area = cache.proxy.bounds();
        let size = [area.width() as usize, area.height() as usize];
        let drawn = if image {
            let px = photocraft_compose::render(&cache.proxy, area).into_px();
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            Ok(px.iter().map(|p| Color32::from_rgba_unmultiplied(q(p[0]), q(p[1]), q(p[2]), q(p[3]))).collect::<Vec<_>>())
        } else {
            selection_mask(app, &cache.proxy, cache.k, &params).map(|m| m.into_iter().map(|v| Color32::from_gray((v * 255.0 + 0.5) as u8)).collect())
        };
        match drawn {
            Ok(pixels) => {
                let img = egui::ColorImage::new(size, pixels);
                let tex = match cache.shown.take() {
                    Some((mut t, _)) => {
                        t.set(img, egui::TextureOptions::LINEAR);
                        t
                    }
                    None => ctx.load_texture("replace-color-preview", img, egui::TextureOptions::LINEAR),
                };
                cache.shown = Some((tex, size));
                cache.error = None;
            }
            Err(e) => {
                log::warn!("Replace Color preview: {e}");
                cache.shown = None;
                cache.error = Some(e);
            }
        }
        cache.key = key;
    }
    ctx.data_mut(|d| d.insert_temp(cache_id(), cache.clone()));
    Some(cache)
}

/// The thumbnail and its Selection / Image choice, drawn in the dialog between the Selection
/// settings and the Replacement sliders.
pub(crate) fn thumbnail(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let image = f.get(VIEW).and_then(Value::as_str) == Some("image");
    ui.add_space(4.0);
    // Centred in the dialog, with the view choice under it, as in Photoshop.
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width().max(THUMB.x), THUMB.y), Sense::hover());
    let frame = Rect::from_center_size(row.center(), THUMB);
    let resp = ui.interact(frame, ui.scope_id().with("replace-color-thumbnail"), Sense::click());
    ui.painter().rect_filled(frame, 0.0, t.canvas);
    let cache = refresh(app, ui.ctx(), f, image);
    let doc_size = app.session.active().map(|st| [f64::from(st.doc.size.width.max(1)), f64::from(st.doc.size.height.max(1))]);
    if let Some((tex, [w, h])) = cache.as_ref().and_then(|c| c.shown.clone()) {
        let scale = (THUMB.x / w.max(1) as f32).min(THUMB.y / h.max(1) as f32);
        let img = Rect::from_center_size(frame.center(), vec2(w as f32 * scale, h as f32 * scale));
        ui.painter().image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        // Response::clicked may take an accessibility input lock: ask before reading the pointer.
        let clicked = resp.clicked();
        if let (Some(p), Some([dw, dh])) = (resp.interact_pointer_pos().filter(|p| clicked && img.contains(*p)), doc_size) {
            let x = (f64::from((p.x - img.left()) / img.width()) * dw).floor().clamp(0.0, dw - 1.0);
            let y = (f64::from((p.y - img.top()) / img.height()) * dh).floor().clamp(0.0, dh - 1.0);
            let mods = ui.input(|i| i.modifiers);
            crate::filter_dialog::sample_into(app, f, x, y, mods);
        }
    }
    if let Some(e) = cache.as_ref().and_then(|c| c.error.as_deref()) {
        let g = ui.painter().layout(e.to_string(), egui::FontId::proportional(11.0), t.text_faint, frame.width() - 16.0);
        ui.painter().galley(frame.center() - g.size() / 2.0, g, t.text_faint);
    }
    ui.painter().rect_stroke(frame, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), tl!("Preview")));
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - THUMB.x).max(0.0) / 2.0);
        if crate::color_range_ui::radio(ui, !image, "Selection").clicked() {
            f.insert(VIEW.into(), json!("selection"));
        }
        ui.add_space(8.0);
        if crate::color_range_ui::radio(ui, image, "Image").clicked() {
            f.insert(VIEW.into(), json!("image"));
        }
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests;
