//! In-dialog preview window for the generic schema filter dialogs (#3769).
//!
//! Photoshop shows a preview of the filtered result *inside* its filter dialogs. This draws one
//! from the same proxy preview the canvas already shows (`filter_dialog::preview_document`): a
//! cropped region of the filtered proxy with − / + zoom and a percentage readout, drag to pan, a
//! click to centre the view on a document point, and press-and-hold to compare with the unfiltered
//! image. The proxy is drawn scaled by its reduction factor, so 100% is one *document* pixel per
//! point whatever the proxy factor. The Preview checkbox keeps toggling the on-canvas live result
//! (`canvas::ensure_filter_preview`), unchanged. While such a dialog is on top, the image under it
//! is the preview's locator: the pointer is a small square there, and a click centres the window
//! on that document point ([`center_at`], #9648).

use egui::{Color32, Rect, Sense, Stroke, Vec2, pos2, vec2};
use photocraft_doc::Document;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::filter_dialog::FilterPreviewKey;
use crate::theme::Tokens;

/// Zoom bounds, as a fraction of document scale: 100% shows one document pixel per point.
const MIN_ZOOM: f32 = 0.125;
const MAX_ZOOM: f32 = 8.0;
const DEFAULT_ZOOM: f32 = 1.0;
/// The preview region's height in points; its width follows the dialog.
const PREVIEW_H: f32 = 150.0;
/// Private dialog fields holding the preview's view (stripped from the command's params).
const ZOOM_FIELD: &str = "__pvZoom";
const CENTER_X_FIELD: &str = "__pvCx";
const CENTER_Y_FIELD: &str = "__pvCy";
/// Whether a full-window preview's zoom follows Fit in View (until − / + picks a level).
const FIT_FIELD: &str = "__pvFit";
/// The height of the zoom row under the preview image, in points.
pub(crate) const ZOOM_ROW_H: f32 = 28.0;

/// Filters whose dialog fills the app window: the preview takes most of it and the parameters
/// sit in a column on its right, as in Photoshop's Lens Blur, so the effect of each setting
/// can be judged on the whole image rather than a strip.
const FULL_WINDOW: &[&str] = &["filter.blur.lensBlur"];
/// The parameter column's width in a full-window filter dialog.
pub(crate) const SIDE_W: f32 = 300.0;

/// Whether the dialog with fields `f` is a full-window filter dialog ([`FULL_WINDOW`]).
pub(crate) fn full_window(f: &Map<String, Value>) -> bool {
    shows(f) && f.get("__command").and_then(Value::as_str).is_some_and(|c| FULL_WINDOW.contains(&c))
}

/// Draw the filter dialog's in-dialog preview window below its parameters (its `fields` `f`). A
/// no-op for dialogs without a preview and for Select and Mask, which has its own view.
pub(crate) fn window(app: &PhotocraftApp, dialog: u64, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let size = vec2(ui.available_width().max(140.0), PREVIEW_H);
    window_sized(app, dialog, ui, f, size, false);
}

/// The full-window dialog's preview: an image region of `size` points (the zoom row goes under
/// it), opening at Fit in View.
pub(crate) fn large_window(app: &PhotocraftApp, dialog: u64, ui: &mut egui::Ui, f: &mut Map<String, Value>, size: Vec2) {
    window_sized(app, dialog, ui, f, size.max(vec2(140.0, 100.0)), true);
}

/// The zoom at which a `doc`-sized image fits a `region` (within the zoom bounds).
fn fit_zoom(region: Vec2, doc: Vec2) -> f32 {
    let z = (region.x / doc.x).min(region.y / doc.y);
    if z.is_finite() && z > 0.0 { z.clamp(MIN_ZOOM, MAX_ZOOM) } else { DEFAULT_ZOOM }
}

fn window_sized(app: &PhotocraftApp, dialog: u64, ui: &mut egui::Ui, f: &mut Map<String, Value>, size: Vec2, fits: bool) {
    if !shows(f) {
        return;
    }
    let Some(cmd) = f.get("__command").and_then(Value::as_str).map(str::to_owned) else { return };
    let Some(idx) = app.session.active_index() else { return };
    let Some(st) = app.session.documents().get(idx) else { return };
    let (doc_id, revision) = (st.doc.id, st.revision);

    // The filtered proxy the canvas already computed for this dialog, when there is one. Matching
    // the dialog keeps a stacked dialog from showing another's preview; the Preview checkbox still
    // decides whether the filtered result (rather than the original) is shown.
    let preview_on = f.get("__preview").and_then(Value::as_bool) == Some(true);
    let filtered: Option<(FilterPreviewKey, std::sync::Arc<Document>)> = app.filter_preview.as_ref().and_then(|p| {
        // An edited smart filter previews as `layer.smartFilter.setParams` (#6324).
        let previewed = crate::smart_ui::preview_command(f, &cmd);
        (preview_on && p.key.dialog == dialog && p.key.doc == doc_id && p.key.command == previewed)
            .then(|| p.result.clone().map(|r| (p.key.clone(), r)))
            .flatten()
    });
    // The proxy factor the preview runs at (the filtered key's when it exists, else the request's).
    // The fallback must be the factor the canvas will ask for, not the document's own: since #6326
    // the canvas caps it with the view's zoom, and a window that sized its textures from the
    // uncapped factor built a proxy the canvas never asked for, then rebuilt it (a full-size copy
    // and upload on the UI thread at 100 %) the moment the first result arrived, shifting this
    // window's 100 % mapping and pan clamp for a frame.
    let base_k = filtered.as_ref().map(|(k, _)| k.k).unwrap_or_else(|| {
        let params = app.with_mask_target(&cmd, crate::filter_dialog::params_of(f));
        crate::canvas::filter_preview_factor(app, &st.doc, &cmd, &params)
    });

    let t = Tokens::get(ui.ctx());
    let cache_id = egui::Id::unique(("filter-preview-ui", dialog));
    let mut cache: Cache = ui.ctx().data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<Cache>(cache_id)));
    // Channel visibility and the active mask can change without a document revision. Both the
    // original (press-and-hold) and filtered textures must follow the canvas's current view.
    let view = (st.channel_view.clone(), app.session.prefs().interface.show_channels_in_color, st.active_layer);
    if cache.view.as_ref() != Some(&view) {
        cache = Cache { view: Some(view), ..Default::default() };
    }

    // Unfiltered proxy: built once per document revision and factor (cheap at the proxy factor).
    let orig_key = TexKey::Original(doc_id, revision, base_k);
    if cache.original.as_ref().map(|c| &c.key) != Some(&orig_key) {
        let proxy = crate::proxy::proxy_document(&st.doc, base_k);
        let tex = upload(ui.ctx(), app, "filter-preview-original", &proxy);
        cache.original = Some(CachedTex { key: orig_key, tex, size: [proxy.size.width, proxy.size.height] });
    }
    // Filtered proxy: rebuilt whenever the worker publishes a new result.
    match &filtered {
        Some((key, filtered_doc)) => {
            let fkey = TexKey::Filtered(key.clone());
            if cache.filtered.as_ref().map(|c| &c.key) != Some(&fkey) {
                let tex = upload(ui.ctx(), app, "filter-preview-filtered", filtered_doc);
                cache.filtered = Some(CachedTex { key: fkey, tex, size: [filtered_doc.size.width, filtered_doc.size.height] });
            }
        }
        None => cache.filtered = None,
    }
    let orig = cache.original.as_ref().map(|c| (c.tex.clone(), c.size));
    let filt = cache.filtered.as_ref().map(|c| (c.tex.clone(), c.size));
    ui.ctx().data_mut(|d| d.insert_temp(cache_id, cache));

    let Some([img_w, img_h]) = filt.as_ref().map(|(_, s)| *s).or_else(|| orig.as_ref().map(|(_, s)| *s)) else { return };
    if img_w == 0 || img_h == 0 {
        return;
    }
    // The proxy is the document reduced by `base_k`, so the view works in *document* pixels: at
    // 100% one document pixel covers one point, whatever the proxy factor (#3769).
    let doc = document_size([img_w, img_h], base_k);

    // View state, sanitised: a corrupt field must not divide by zero or draw off the texture.
    let mut zoom = f.get(ZOOM_FIELD).and_then(Value::as_f64).map(|v| v as f32).filter(|z| z.is_finite() && *z > 0.0).unwrap_or(DEFAULT_ZOOM);
    zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    // A full-window preview follows Fit in View until − / + picks a level, so it fits again
    // when the app window (and so the dialog) is resized.
    let mut fit = fits && f.get(FIT_FIELD).and_then(Value::as_bool) != Some(false);
    if fit {
        zoom = fit_zoom(size, doc);
    }
    let mut cx = f.get(CENTER_X_FIELD).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite()).unwrap_or(doc.x / 2.0);
    let mut cy = f.get(CENTER_Y_FIELD).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite()).unwrap_or(doc.y / 2.0);
    (cx, cy) = clamp_center(cx, cy, doc.x, doc.y);

    if fit {
        (cx, cy) = (doc.x / 2.0, doc.y / 2.0);
    }
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let (x0, y0, sw, sh) = source_rect(cx, cy, zoom, rect.size(), doc);
    let (dest, uv) = placement(rect, (x0, y0, sw, sh), zoom, doc);

    // Press-and-hold shows the unfiltered image, as Photoshop's preview does.
    let holding = response.is_pointer_button_down_on();
    let shown = if holding { orig.as_ref() } else { filt.as_ref().or(orig.as_ref()) };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.radius_sm, t.canvas);
    if let Some((tex, _)) = shown {
        crate::widgets::checker(&painter, dest, 8.0);
        painter.image(tex.id(), dest, uv, Color32::WHITE);
    }
    painter.rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);

    // Drag to pan (the image follows the pointer); a plain click centres on the clicked point.
    if response.dragged() && !fit {
        let d = response.drag_delta();
        cx -= d.x / zoom;
        cy -= d.y / zoom;
    }
    if !fit
        && response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        (cx, cy) = point_to_source(p - rect.min, x0, y0, zoom, doc);
    }
    (cx, cy) = clamp_center(cx, cy, doc.x, doc.y);

    ui.horizontal(|ui| {
        if crate::icons::button(ui, "minus", 22.0, false, tl!("Zoom out")).clicked() {
            zoom = step_zoom(zoom, false);
            fit = false;
        }
        ui.label(egui::RichText::new(format!("{}%", crate::widgets::fmt_num(zoom as f64 * 100.0))).color(t.text_dim));
        if crate::icons::button(ui, "plus", 22.0, false, tl!("Zoom in")).clicked() {
            zoom = step_zoom(zoom, true);
            fit = false;
        }
        if fits && !fit && crate::widgets::secondary_button(ui, tl!("Fit in View"), 84.0).clicked() {
            fit = true;
        }
    });

    if fits {
        f.insert(FIT_FIELD.into(), json!(fit));
    }
    f.insert(ZOOM_FIELD.into(), json!(zoom.clamp(MIN_ZOOM, MAX_ZOOM)));
    f.insert(CENTER_X_FIELD.into(), json!(cx));
    f.insert(CENTER_Y_FIELD.into(), json!(cy));
}

/// The top dialog, when it has the preview window: the image under it is then the preview's
/// locator (another dialog opened over it takes the input). Not while the image is something else
/// of the dialog's: the Blur Gallery's pins (`blur_pins`) or Replace Color's armed eyedropper.
pub(crate) fn locator(app: &PhotocraftApp) -> Option<u64> {
    if crate::filter_dialog::color_eyedropper_armed(app) {
        return None;
    }
    app.ui.dialogs.last().filter(|d| shows(&d.fields) && !crate::blur_pins::owns(&d.fields)).map(|d| d.id)
}

/// Centre the top dialog's preview window on document point (x, y), clamped to the image, as a
/// click on the image does in Photoshop's filter dialogs (#9648). Only the dialog's private view
/// fields change: no command runs, the document and its history are untouched. `false` when no
/// dialog with a preview window is on top or the point is not finite.
pub(crate) fn center_at(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    let Some(id) = locator(app) else { return false };
    if !(x.is_finite() && y.is_finite()) {
        return false;
    }
    let Some(size) = app.session.active().map(|st| st.doc.size) else { return false };
    // Clamped before narrowing: a huge finite point (1e40 from `ui.pointer`) would overflow `f32`
    // to infinity, which `clamp_center` reads as corrupt and puts at 0, not at the far edge.
    let (cx, cy) = (x.clamp(0.0, f64::from(size.width)) as f32, y.clamp(0.0, f64::from(size.height)) as f32);
    let Some(d) = app.ui.dialog_mut(id) else { return false };
    d.fields.insert(CENTER_X_FIELD.into(), json!(cx));
    d.fields.insert(CENTER_Y_FIELD.into(), json!(cy));
    true
}

/// The zoom levels the − / + buttons step through, like the Filter Gallery's.
const ZOOM_STEPS: [f32; 7] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];

/// The vertical space the preview window takes below the parameters: the image, the zoom row and
/// their spacing. `0` when the dialog has no preview (so the caller reserves nothing).
pub(crate) fn reserved_height(f: &Map<String, Value>) -> f32 {
    if shows(f) { PREVIEW_H + 32.0 } else { 0.0 }
}

/// Whether the dialog with fields `f` gets the preview window: Filter menu dialogs with a Preview
/// checkbox, as in Photoshop. Image › Rotation › Arbitrary and the adjustment dialogs keep only
/// the on-canvas preview, and Select and Mask has its own view.
fn shows(f: &Map<String, Value>) -> bool {
    f.contains_key("__preview") && !crate::select_mask_view::owns(f) && f.get("__command").and_then(Value::as_str).is_some_and(|c| c.starts_with("filter."))
}

/// The next zoom level up or down from `zoom`, clamped to [`ZOOM_STEPS`]' ends.
fn step_zoom(zoom: f32, up: bool) -> f32 {
    if up {
        ZOOM_STEPS.iter().copied().find(|s| *s > zoom + 1e-3).unwrap_or(MAX_ZOOM)
    } else {
        ZOOM_STEPS.iter().rev().copied().find(|s| *s < zoom - 1e-3).unwrap_or(MIN_ZOOM)
    }
}

/// Keep the view centre on the image (so panning never runs off it entirely). A non-finite centre
/// (corrupt dialog state) falls back to the image's origin.
fn clamp_center(cx: f32, cy: f32, w: f32, h: f32) -> (f32, f32) {
    let axis = |v: f32, max: f32| if v.is_finite() { v.clamp(0.0, max.max(0.0)) } else { 0.0 };
    (axis(cx, w), axis(cy, h))
}

/// The image rectangle shown in a `region` of points at `zoom`, centred on `(cx, cy)`:
/// `(x0, y0, width, height)` in document pixels. A region wider than the image is centred on it.
fn source_rect(cx: f32, cy: f32, zoom: f32, region: Vec2, doc: Vec2) -> (f32, f32, f32, f32) {
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { DEFAULT_ZOOM };
    let sw = (region.x / zoom).max(0.0);
    let sh = (region.y / zoom).max(0.0);
    (origin(cx, sw, doc.x), origin(cy, sh, doc.y), sw, sh)
}

fn origin(centre: f32, size: f32, total: f32) -> f32 {
    if size >= total { (total - size) / 2.0 } else { (centre - size / 2.0).clamp(0.0, total - size) }
}

/// Where the image is drawn in `rect` and which part of the texture fills it, for the `source`
/// rectangle `(x0, y0, width, height)` in document pixels shown at `zoom`. Only the part of the
/// source that lies on the image is drawn, at its own place in the region: stretching the clamped
/// texture over the whole region made the pixels non-square whenever the region overhung the image
/// on one axis (#4640).
fn placement(rect: Rect, source: (f32, f32, f32, f32), zoom: f32, doc: Vec2) -> (Rect, Rect) {
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { DEFAULT_ZOOM };
    let (x0, y0, sw, sh) = source;
    let (vx0, vy0) = (x0.max(0.0), y0.max(0.0));
    let (vx1, vy1) = ((x0 + sw).min(doc.x).max(vx0), (y0 + sh).min(doc.y).max(vy0));
    let at = |x: f32, y: f32| rect.min + vec2((x - x0) * zoom, (y - y0) * zoom);
    let uv = Rect::from_min_max(pos2(vx0 / doc.x, vy0 / doc.y), pos2(vx1 / doc.x, vy1 / doc.y));
    (Rect::from_min_max(at(vx0, vy0), at(vx1, vy1)), uv)
}

/// The document pixel under a pointer at `local` (relative to the region's top-left) in a view whose
/// source rectangle starts at `(x0, y0)` and is shown at `zoom`.
fn point_to_source(local: Vec2, x0: f32, y0: f32, zoom: f32, doc: Vec2) -> (f32, f32) {
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { DEFAULT_ZOOM };
    ((x0 + local.x / zoom).clamp(0.0, doc.x), (y0 + local.y / zoom).clamp(0.0, doc.y))
}

/// The document-pixel size of a proxy with `k`-reduced dimensions. A hostile/overflowing size falls
/// back to the proxy's own dimensions (factor 1), which stays finite and positive.
fn document_size(proxy: [u32; 2], k: u32) -> Vec2 {
    let scale = k.max(1) as f32;
    let doc = vec2(proxy[0] as f32 * scale, proxy[1] as f32 * scale);
    if doc.x.is_finite() && doc.y.is_finite() && doc.x > 0.0 && doc.y > 0.0 { doc } else { vec2(proxy[0] as f32, proxy[1] as f32) }
}

/// A composite as monitor pixels, matching the canvas (`canvas::display_image`).
fn upload(ctx: &egui::Context, app: &PhotocraftApp, name: &str, doc: &Document) -> egui::TextureHandle {
    // Reduce only the display texture when a full-resolution result exceeds the device limit.
    // The cached document and the window's document-pixel zoom coordinates stay at full size.
    let factor = texture_factor(doc.size.width, doc.size.height, ctx.input(|i| i.max_texture_side));
    let (width, height) = (doc.size.width.div_ceil(factor), doc.size.height.div_ceil(factor));
    if let Some(st) = app.session.active()
        && let Some(pixels) =
            crate::channel_view::render(doc, &st.channel_view, doc.bounds(), factor, crate::channel_view::Look::of(&app.session), st.active_layer)
    {
        let image = egui::ColorImage::new([width as usize, height as usize], pixels);
        return ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
    }
    let buf = if factor == 1 { photocraft_compose::flatten(doc) } else { photocraft_compose::render_reduced(doc, width, height) };
    let (display, _) = crate::canvas::canvas_display(app, doc, None);
    let img = crate::canvas::display_image(display.as_deref(), &buf);
    ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
}

/// The reduction that fits a `w`×`h` image inside a `max_side` texture (1: it already fits).
pub(crate) fn texture_factor(w: u32, h: u32, max_side: usize) -> u32 {
    let max_side = u32::try_from(max_side).unwrap_or(u32::MAX).max(1);
    w.max(h).div_ceil(max_side).max(1)
}

/// What a cached texture was built from, so it is rebuilt only when its source changes.
#[derive(Clone, PartialEq)]
enum TexKey {
    Filtered(FilterPreviewKey),
    Original(photocraft_doc::DocId, u64, u32),
}

#[derive(Clone)]
struct CachedTex {
    key: TexKey,
    tex: egui::TextureHandle,
    /// The image's size in pixels.
    size: [u32; 2],
}

/// The dialog's live textures: the filtered proxy and the unfiltered proxy.
#[derive(Clone, Default)]
struct Cache {
    filtered: Option<CachedTex>,
    original: Option<CachedTex>,
    view: Option<(photocraft_engine::channel_cmds::ChannelView, bool, Option<photocraft_doc::LayerId>)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_resolution_preview_textures_respect_the_device_limit() {
        let ctx = egui::Context::default();
        let limit = ctx.input(|i| i.max_texture_side) as u32;
        let width = limit + 2;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": width, "height": 2})).unwrap();
        for alpha in [false, true] {
            if alpha {
                app.run("channel.new", json!({"name": "Alpha", "fill": "black"})).unwrap();
                app.session
                    .edit("Fixture", |doc, _| {
                        doc.channels[0].surface.fill_rect(photocraft_geom::Rect::new(0, 0, (width / 2) as i32, 2), &[1.0]);
                        Ok(())
                    })
                    .unwrap();
            }
            let doc = &app.session.active().unwrap().doc;
            let texture = upload(&ctx, &app, "full-resolution-preview", doc);
            let mut delta = ctx.tex_manager().write().take_delta();
            let egui::ImageData::Color(image) = &delta.set.get(&texture.id()).unwrap()[0].image;
            let pixels = image.pixels.clone();
            delta.clear();
            assert_eq!(texture.size(), [width.div_ceil(2) as usize, 1]);
            assert!(texture.size().into_iter().all(|side| side <= limit as usize));
            assert_eq!(doc.size.width, width, "display reduction must not resize the filter result");
            assert_eq!(pixels[0], Color32::WHITE);
            assert_eq!(*pixels.last().unwrap(), if alpha { Color32::BLACK } else { Color32::WHITE });
        }
    }

    #[test]
    fn uploaded_preview_shows_the_filtered_alpha_at_every_depth_and_proxy_scale() {
        use photocraft_engine::channel_cmds::ChannelTarget;
        for depth in [8, 16, 32] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
            app.run("channel.new", json!({"name": "Alpha", "fill": "black"})).unwrap();
            app.session
                .edit("Fixture", |doc, _| {
                    doc.channels[0].surface.fill_rect(photocraft_geom::Rect::new(0, 0, 8, 16), &[1.0]);
                    Ok(())
                })
                .unwrap();
            let st = app.session.active().unwrap();
            for k in [1, 2, 4] {
                let preview = crate::filter_dialog::preview_document_for_target(
                    &st.doc,
                    st.active_layer,
                    "filter.blur.gaussianBlur",
                    &json!({"radius": 2.0}),
                    k,
                    None,
                    ChannelTarget::Alpha(0),
                    &[],
                    &Default::default(),
                    Some((app.session.tools.foreground, app.session.tools.background)),
                )
                .unwrap();
                let ctx = egui::Context::default();
                let texture = upload(&ctx, &app, "alpha-preview", &preview);
                let mut delta = ctx.tex_manager().write().take_delta();
                let egui::ImageData::Color(image) = &delta.set.get(&texture.id()).unwrap()[0].image;
                let pixels = image.pixels.clone();
                delta.clear();
                let expected =
                    crate::channel_view::render(&preview, &st.channel_view, preview.bounds(), 1, crate::channel_view::Look::default(), st.active_layer)
                        .unwrap();
                assert_eq!(pixels, expected, "depth {depth}, factor {k}");
                assert_ne!(pixels[0], pixels[pixels.len() - 1], "the alpha edge is visible, not the white RGB layer");
                assert!(pixels.iter().any(|p| p.r() > 0 && p.r() < 255), "the blurred edge is visible");
            }
        }
    }

    #[test]
    fn original_texture_refreshes_when_the_channel_view_changes_without_an_edit() {
        use photocraft_engine::channel_cmds::ChannelTarget;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("channel.new", json!({"name": "White", "fill": "white"})).unwrap();
        app.run("channel.new", json!({"name": "Black", "fill": "black"})).unwrap();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(0);
        let revision = app.session.active().unwrap().revision;
        let id = crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let mut fields = app.ui.dialogs.last().unwrap().fields.clone();
        let ctx = egui::Context::default();
        let cache_id = egui::Id::unique(("filter-preview-ui", id));
        ctx.run_ui(Default::default(), |ui| window(&app, id, ui, &mut fields)).textures_delta.clear();
        let before = ctx.data(|d| d.get_temp::<Cache>(cache_id).unwrap().original.unwrap().tex.id());
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(1);
        assert_eq!(app.session.active().unwrap().revision, revision);
        let mut output = ctx.run_ui(Default::default(), |ui| window(&app, id, ui, &mut fields));
        let after = ctx.data(|d| d.get_temp::<Cache>(cache_id).unwrap().original.unwrap().tex.id());
        let pixels = output.textures_delta.set.get(&after).map(|delta| {
            let egui::ImageData::Color(image) = &delta[0].image;
            image.pixels.clone()
        });
        output.textures_delta.clear();
        assert_ne!(before, after, "press-and-hold must not reuse the old channel");
        assert!(pixels.unwrap().iter().all(|p| *p == Color32::BLACK));
    }

    #[test]
    fn zoom_steps_walk_the_levels_and_stay_in_bounds() {
        assert_eq!(step_zoom(1.0, true), 2.0);
        assert_eq!(step_zoom(1.0, false), 0.5);
        assert_eq!(step_zoom(0.9, true), 1.0);
        assert_eq!(step_zoom(1.1, false), 1.0);
        assert_eq!(step_zoom(MAX_ZOOM, true), MAX_ZOOM, "clamped at the top");
        assert_eq!(step_zoom(MIN_ZOOM, false), MIN_ZOOM, "clamped at the bottom");
        assert_eq!(step_zoom(0.0, true), MIN_ZOOM, "a corrupt zoom recovers upward");
        assert_eq!(step_zoom(f32::NAN, false), MIN_ZOOM);
    }

    #[test]
    fn only_filter_dialogs_get_the_preview_window() {
        let fields = |cmd: &str| json!({"__preview": true, "__command": cmd}).as_object().cloned().unwrap();
        assert!(shows(&fields("filter.blur.gaussianBlur")));
        assert!(reserved_height(&fields("filter.blur.gaussianBlur")) > 0.0);
        // Photoshop's Rotate Canvas and adjustment dialogs have no preview window.
        assert!(!shows(&fields("image.rotation.arbitrary")));
        assert_eq!(reserved_height(&fields("image.rotation.arbitrary")), 0.0);
        assert!(!shows(&fields("adjust.levels")));
        // No Preview checkbox or no command: no window.
        assert!(!shows(json!({"__command": "filter.blur.gaussianBlur"}).as_object().unwrap()));
        assert!(!shows(json!({"__preview": true}).as_object().unwrap()));
    }

    #[test]
    fn textures_are_reduced_to_fit_the_gpu() {
        assert_eq!(texture_factor(3000, 2000, 2048), 2);
        assert_eq!(texture_factor(2048, 100, 2048), 1);
        assert_eq!(texture_factor(9000, 9000, 2048), 5);
        // A zero or absurd limit never divides by zero.
        assert_eq!(texture_factor(3000, 2000, 0), 3000);
        assert_eq!(texture_factor(0, 0, 2048), 1);
    }

    #[test]
    fn the_centre_is_clamped_to_the_document() {
        assert_eq!(clamp_center(-5.0, 99.0, 100.0, 50.0), (0.0, 50.0));
        assert_eq!(clamp_center(40.0, 25.0, 100.0, 50.0), (40.0, 25.0));
        // A corrupt (non-finite) centre falls back to the origin, not a panic or a NaN.
        assert_eq!(clamp_center(f32::NAN, f32::INFINITY, 100.0, 50.0), (0.0, 0.0));
    }

    #[test]
    fn the_source_rect_follows_the_centre_and_zoom() {
        let doc = vec2(1000.0, 500.0);
        let region = vec2(200.0, 100.0);
        // 100%: a 200×100 document-pixel crop centred at (500, 250) sits in the middle.
        assert_eq!(source_rect(500.0, 250.0, 1.0, region, doc), (400.0, 200.0, 200.0, 100.0));
        // 200%: the crop is half as large, still centred.
        assert_eq!(source_rect(500.0, 250.0, 2.0, region, doc), (450.0, 225.0, 100.0, 50.0));
        // Near an edge the crop stops at it instead of running off.
        let (x0, _, sw, _) = source_rect(0.0, 250.0, 1.0, region, doc);
        assert_eq!((x0, sw), (0.0, 200.0));
        let (x0, _, sw, _) = source_rect(1000.0, 250.0, 1.0, region, doc);
        assert_eq!((x0, sw), (800.0, 200.0));
    }

    #[test]
    fn a_zoomed_out_region_centres_the_whole_image() {
        let doc = vec2(100.0, 50.0);
        let region = vec2(200.0, 100.0);
        let (x0, y0, sw, sh) = source_rect(50.0, 25.0, 0.5, region, doc);
        // The region is wider than the image: it is centred (a negative origin) and covers all of
        // it, so the UV clamps to 0..1 rather than reading outside the texture.
        assert_eq!((x0, y0, sw, sh), (-150.0, -75.0, 400.0, 200.0));
        assert!(x0 / doc.x < 0.0 && (x0 + sw) / doc.x > 1.0);
    }

    #[test]
    fn the_image_keeps_square_pixels_when_the_region_overhangs_it() {
        // #4640: a 370×240 document in a 650×150 region at 100% is wider than the image but shorter
        // than it, and at 50% it fits inside on both axes. Either way one document pixel must cover
        // the same number of points across and down, and the image is centred, not stretched.
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(650.0, 150.0));
        let doc = vec2(370.0, 240.0);
        for zoom in [1.0, 0.5] {
            let (dest, uv) = placement(rect, source_rect(185.0, 120.0, zoom, rect.size(), doc), zoom, doc);
            let across = dest.width() / (uv.width() * doc.x);
            let down = dest.height() / (uv.height() * doc.y);
            assert!((across - zoom).abs() < 1e-4 && (down - zoom).abs() < 1e-4, "zoom {zoom}: {across} × {down} points per pixel");
            assert!((dest.center().x - rect.center().x).abs() < 1e-3, "zoom {zoom}: centred across");
            assert!(rect.contains_rect(dest), "zoom {zoom}: drawn inside the region");
            assert!(Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)).contains_rect(uv), "zoom {zoom}: {uv:?} stays on the texture");
        }
    }

    #[test]
    fn a_click_maps_to_the_document_point_under_it() {
        let doc = vec2(1000.0, 500.0);
        // The top-left of a crop starting at (400, 200) shown at 100% is that point.
        assert_eq!(point_to_source(vec2(0.0, 0.0), 400.0, 200.0, 1.0, doc), (400.0, 200.0));
        // At 200%, 50 points in is 25 document pixels in.
        assert_eq!(point_to_source(vec2(50.0, 50.0), 400.0, 200.0, 2.0, doc), (425.0, 225.0));
        // Clicking outside the image clamps to it.
        assert_eq!(point_to_source(vec2(-10_000.0, 0.0), 400.0, 200.0, 1.0, doc), (0.0, 200.0));
    }

    #[test]
    fn a_hundred_percent_is_one_document_pixel_per_point() {
        // A 4×-reduced proxy of a 4000×3000 document: 100% must span 200 document pixels across a
        // 200-point region, not 200 proxy pixels (which would be a quarter of the document scale).
        let doc = document_size([1000, 750], 4);
        assert_eq!(doc, vec2(4000.0, 3000.0));
        let (_, _, sw, sh) = source_rect(2000.0, 1500.0, 1.0, vec2(200.0, 100.0), doc);
        assert_eq!((sw, sh), (200.0, 100.0), "one document pixel per point at 100%");
        // The same region covers 50 proxy pixels: the stored UV is a 200/4000 fraction.
        let (x0, _, _, _) = source_rect(2000.0, 1500.0, 1.0, vec2(200.0, 100.0), doc);
        assert_eq!(x0 / doc.x, 0.475);
        assert!(document_size([0, 0], 4) == vec2(0.0, 0.0), "degenerate size stays finite");
        assert_eq!(document_size([10, 20], 0), vec2(10.0, 20.0), "a zero factor means 1×");
    }

    #[test]
    fn a_point_on_the_image_centres_the_top_preview_window() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        // No dialog, or one without a preview window: nothing to centre.
        assert!(!center_at(&mut app, 10.0, 10.0));
        crate::dialogs::open_command_dialog(&mut app, "image.adjustments.brightnessContrast", "Brightness/Contrast…");
        assert_eq!(locator(&app), None, "an adjustment dialog has no preview window");
        app.ui.dialogs.clear();
        // Every filter dialog with the window shares the locator, Unsharp Mask and Reduce Noise too.
        for command in ["filter.sharpen.unsharpMask", "filter.noise.reduceNoise"] {
            let id = crate::filter_dialog::open(&mut app, command).expect("the filter opens a dialog");
            assert_eq!(locator(&app), Some(id), "{command}");
            app.ui.dialogs.clear();
        }
        let id = crate::filter_dialog::open(&mut app, "filter.sharpen.smartSharpen").expect("Smart Sharpen opens a dialog");
        assert_eq!(locator(&app), Some(id), "Smart Sharpen has the preview window");
        let (revision, history) = (app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len());
        let centre = |app: &PhotocraftApp| {
            let f = &app.ui.dialogs.last().unwrap().fields;
            (f.get(CENTER_X_FIELD).cloned(), f.get(CENTER_Y_FIELD).cloned())
        };
        assert!(center_at(&mut app, 10.5, 40.0));
        assert_eq!(centre(&app), (Some(json!(10.5)), Some(json!(40.0))));
        // Off the image it clamps to the nearest edge; a non-finite point changes nothing.
        assert!(center_at(&mut app, -30.0, 999.0));
        assert_eq!(centre(&app), (Some(json!(0.0)), Some(json!(48.0))));
        assert!(!center_at(&mut app, f64::NAN, 5.0));
        assert!(!center_at(&mut app, 5.0, f64::INFINITY));
        assert_eq!(centre(&app), (Some(json!(0.0)), Some(json!(48.0))));
        // A huge but finite point (past `f32`'s range) clamps to the far edge, not to 0.
        assert!(center_at(&mut app, 1e40, -1e300));
        assert_eq!(centre(&app), (Some(json!(64.0)), Some(json!(0.0))));
        assert!(center_at(&mut app, -1e300, 1e40));
        assert_eq!(centre(&app), (Some(json!(0.0)), Some(json!(48.0))));
        let st = app.session.active().unwrap();
        assert_eq!((st.revision, st.history.past_len()), (revision, history), "the document and its history are untouched");
        // The view fields are private: the command's params don't carry them.
        let params = crate::filter_dialog::params_of(&app.ui.dialogs.last().unwrap().fields);
        assert!(params.get(CENTER_X_FIELD).is_none() && params.get(CENTER_Y_FIELD).is_none(), "{params}");
        // A dialog opened over it takes the input.
        crate::color_picker_ui::open(&mut app, "foreground");
        assert_eq!(locator(&app), None);
        assert!(!center_at(&mut app, 20.0, 20.0));
    }

    #[test]
    fn only_lens_blur_fills_the_window() {
        let fields = |cmd: &str| json!({"__command": cmd, "__preview": true}).as_object().cloned().unwrap();
        assert!(full_window(&fields("filter.blur.lensBlur")));
        assert!(!full_window(&fields("filter.blur.gaussianBlur")));
        // Without a preview (a dialog that isn't a filter's) there is nothing to fill it with.
        assert!(!full_window(json!({"__command": "filter.blur.lensBlur"}).as_object().unwrap()));
    }

    #[test]
    fn fit_zoom_fits_the_whole_image_within_the_zoom_bounds() {
        assert_eq!(fit_zoom(vec2(1000.0, 500.0), vec2(4000.0, 1000.0)), 0.25, "the wider side limits it");
        assert_eq!(fit_zoom(vec2(1000.0, 500.0), vec2(1000.0, 2000.0)), 0.25, "the taller side limits it");
        assert_eq!(fit_zoom(vec2(1000.0, 1000.0), vec2(10.0, 10.0)), MAX_ZOOM, "a tiny image stops at the top level");
        assert_eq!(fit_zoom(vec2(100.0, 100.0), vec2(100_000.0, 100_000.0)), MIN_ZOOM);
        assert_eq!(fit_zoom(vec2(100.0, 100.0), vec2(0.0, 0.0)), DEFAULT_ZOOM, "a degenerate size never yields inf or NaN");
    }

    #[test]
    fn the_full_window_preview_opens_at_fit_and_keeps_a_chosen_level() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
        let id = crate::filter_dialog::open(&mut app, "filter.blur.lensBlur").expect("Lens Blur opens a dialog");
        let mut f = app.ui.dialogs.iter().find(|d| d.id == id).expect("the dialog is open").fields.clone();
        assert!(full_window(&f));
        let draw = |f: &mut Map<String, Value>| {
            egui::Context::default().run_ui(Default::default(), |ui| large_window(&app, id, ui, f, vec2(400.0, 400.0))).textures_delta.clear();
        };
        draw(&mut f);
        assert_eq!(f.get(FIT_FIELD), Some(&json!(true)));
        assert_eq!(f.get(ZOOM_FIELD), Some(&json!(0.5)), "800 px wide in 400 points");
        assert_eq!((f.get(CENTER_X_FIELD), f.get(CENTER_Y_FIELD)), (Some(&json!(400.0)), Some(&json!(200.0))));
        // − / + leave Fit: the level they picked stays, and so does a pan.
        f.insert(FIT_FIELD.into(), json!(false));
        f.insert(ZOOM_FIELD.into(), json!(2.0));
        f.insert(CENTER_X_FIELD.into(), json!(100.0));
        draw(&mut f);
        assert_eq!(f.get(ZOOM_FIELD), Some(&json!(2.0)));
        assert_eq!(f.get(CENTER_X_FIELD), Some(&json!(100.0)));
        // The strip preview of other dialogs never writes the fit flag.
        let mut strip = f.clone();
        strip.remove(FIT_FIELD);
        egui::Context::default().run_ui(Default::default(), |ui| window(&app, id, ui, &mut strip)).textures_delta.clear();
        assert!(!strip.contains_key(FIT_FIELD));
    }

    #[test]
    fn the_window_defaults_its_view_and_never_panics() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let id = crate::filter_dialog::open(&mut app, "filter.stylize.emboss").expect("Emboss opens a dialog");
        let mut f = app.ui.dialogs.iter().find(|d| d.id == id).expect("the dialog is open").fields.clone();
        assert!(f.contains_key("__preview"), "Emboss previews");
        egui::Context::default().run_ui(Default::default(), |ui| window(&app, id, ui, &mut f)).textures_delta.clear();
        assert_eq!(f.get(ZOOM_FIELD), Some(&json!(1.0)), "100% by default");
        assert_eq!(f.get(CENTER_X_FIELD), Some(&json!(32.0)), "centred on the document");
        assert_eq!(f.get(CENTER_Y_FIELD), Some(&json!(24.0)));
        // Corrupt state is recovered, not panicked on.
        f.insert(ZOOM_FIELD.into(), json!(0.0));
        f.insert(CENTER_X_FIELD.into(), json!(f64::NAN));
        f.insert(CENTER_Y_FIELD.into(), json!(-999.0));
        egui::Context::default().run_ui(Default::default(), |ui| window(&app, id, ui, &mut f)).textures_delta.clear();
        assert_eq!(f.get(ZOOM_FIELD), Some(&json!(1.0)));
        assert_eq!(f.get(CENTER_Y_FIELD), Some(&json!(0.0)));
    }
}
