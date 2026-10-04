//! Document canvas: display texture cache, view transform, tool input, extra document windows.
//!
//! Rendering is CPU (`photocraft-compose`) for now, uploaded into an egui texture. Brush strokes
//! update only their damage rectangle (`set_partial`). The wgpu compositor (M5) will replace this
//! with direct GPU rendering behind the same `CanvasCache` interface.

use egui::{Color32, Pos2, Rect, Sense, Stroke, TextureOptions, Vec2, pos2, vec2};
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Rect as DRect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, View};

/// Largest texture side we upload; bigger documents display downsampled until the GPU path lands.
pub const MAX_TEXTURE: u32 = 4096;

pub struct CanvasCache {
    pub revision: u64,
    pub texture: Option<egui::TextureHandle>,
    /// Display pixels per document pixel of the cached image (≤ 1).
    pub scale: f32,
    /// Hash of the live-adjust preview used for this render (0 = none).
    pub preview_key: u64,
    /// The current image lives in the GPU canvas (`gpu_canvas`) rather than `texture`.
    pub on_gpu: bool,
    /// Revision and preview key `texture` was rendered at. Tracked apart from the GPU canvas's
    /// `revision`, so the Navigator never shows a stale CPU image after GPU-path edits.
    pub tex_revision: u64,
    pub tex_preview_key: u64,
}

/// An in-progress pointer gesture on the canvas.
#[derive(Clone, Debug)]
pub struct Drag {
    pub tool: Tool,
    pub start: [f64; 2],
    pub points: Vec<[f64; 3]>,
    pub modifiers: egui::Modifiers,
}

/// Abstract tool event, produced by the mouse or by automation (`ui.pointer`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolEvent {
    Down { x: f64, y: f64, pressure: f32 },
    Move { x: f64, y: f64, pressure: f32 },
    Up { x: f64, y: f64 },
}

/// Document ↔ screen mapping for a canvas rect and a view.
#[derive(Clone, Copy, Debug)]
pub struct ViewXform {
    pub rect: Rect,
    pub zoom: f32,
    pub center: [f32; 2],
    /// View › Flip Horizontal: the view is mirrored left-right (the document is not).
    pub flip: bool,
}

impl ViewXform {
    pub fn to_screen(&self, x: f32, y: f32) -> Pos2 {
        let sx = if self.flip { -1.0 } else { 1.0 };
        self.rect.center() + vec2((x - self.center[0]) * self.zoom * sx, (y - self.center[1]) * self.zoom)
    }
    pub fn to_doc(&self, p: Pos2) -> [f64; 2] {
        let d = (p - self.rect.center()) / self.zoom;
        let dx = if self.flip { -d.x } else { d.x };
        [(dx + self.center[0]) as f64, (d.y + self.center[1]) as f64]
    }
    pub fn doc_rect(&self, r: DRect) -> Rect {
        Rect::from_two_pos(self.to_screen(r.x0 as f32, r.y0 as f32), self.to_screen(r.x1 as f32, r.y1 as f32))
    }
}

pub fn fit_view(view: &mut View, doc: &Document, area: Vec2) {
    let (w, h) = (doc.size.width as f32, doc.size.height as f32);
    let zoom = ((area.x - 40.0) / w).min((area.y - 40.0) / h).clamp(0.01, 1.0);
    view.zoom = zoom;
    view.center = [w / 2.0, h / 2.0];
    view.fit_pending = false;
}

/// Zoom steps like Photoshop's (⌘+ / ⌘−).
pub fn zoom_step(z: f32, dir: i32) -> f32 {
    const STEPS: [f32; 22] =
        [0.01, 0.02, 0.03, 0.05, 0.0667, 0.1, 0.125, 0.1667, 0.25, 0.333, 0.5, 0.6667, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 12.0, 16.0, 32.0];
    if dir > 0 { STEPS.iter().copied().find(|s| *s > z * 1.001).unwrap_or(32.0) } else { STEPS.iter().rev().copied().find(|s| *s < z * 0.999).unwrap_or(0.01) }
}

fn checker(app: &mut PhotocraftApp, ctx: &egui::Context) -> egui::TextureId {
    // Preferences › Transparency & Gamut colours (the texture is rebuilt when they change).
    let [ca, cb] = app.session.prefs().transparency_and_gamut.colors();
    if app.prefs_rt.checker_key != Some([ca, cb]) {
        app.prefs_rt.checker_key = Some([ca, cb]);
        app.checker = None;
    }
    app.checker
        .get_or_insert_with(|| {
            let (a, b) = (Color32::from_rgb(ca[0], ca[1], ca[2]), Color32::from_rgb(cb[0], cb[1], cb[2]));
            let img = egui::ColorImage::new([2, 2], vec![a, b, b, a]);
            let opts = TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Nearest,
                wrap_mode: egui::TextureWrapMode::Repeat,
                mipmap_mode: None,
            };
            ctx.load_texture("checker", img, opts)
        })
        .id()
}

fn buffer_to_image(buf: &photocraft_compose::Buffer) -> egui::ColorImage {
    let img = buf.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels)
}

/// Box-downsample a composite for display.
fn downsample(buf: &photocraft_compose::Buffer, factor: u32) -> photocraft_compose::Buffer {
    let (w, h) = (buf.rect.width(), buf.rect.height());
    let (nw, nh) = ((w / factor).max(1), (h / factor).max(1));
    let mut out = photocraft_compose::Buffer::transparent(DRect::from_xywh(0, 0, nw, nh));
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0.0f32; 4];
            let mut n = 0.0;
            for dy in 0..factor {
                for dx in 0..factor {
                    let (sx, sy) = (x * factor + dx, y * factor + dy);
                    if sx < w && sy < h {
                        let p = buf.px[(sy * w + sx) as usize];
                        // premultiplied average
                        acc[0] += p[0] * p[3];
                        acc[1] += p[1] * p[3];
                        acc[2] += p[2] * p[3];
                        acc[3] += p[3];
                        n += 1.0;
                    }
                }
            }
            let a = acc[3] / n;
            let px = if acc[3] > 0.0 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], a] } else { [0.0; 4] };
            out.px[(y * nw + x) as usize] = px;
        }
    }
    out
}

/// The document to render: the committed one, or a clone with the live adjustment preview applied.
fn display_doc(app: &mut PhotocraftApp, idx: usize) -> (std::sync::Arc<Document>, u64) {
    let st = &app.session.documents()[idx];
    // Puppet / Perspective Warp previews hide the layer they draw on a mesh.
    if let Some(shown) = crate::distort_ui::display_doc(app, idx) {
        return shown;
    }
    if let (Some(t), Some(pv)) = (&app.ui.transform, &app.transform_preview)
        && app.session.active_index() == Some(idx)
        && st.doc.layer(photocraft_doc::LayerId(t.layer)).is_some()
    {
        return (pv.doc.clone(), (1 << 40) + pv.session);
    }
    // Layer Style dialog: show its effects live (Cancel just drops the preview).
    let style = app.ui.dialogs.iter().find(|d| d.kind == crate::state::DialogKind::LayerStyle);
    if style.is_none() {
        app.style_preview = None;
    }
    if let Some(d) = style
        && app.session.active_index() == Some(idx)
    {
        let key = (crate::layer_style::preview_hash(&d.fields) ^ st.revision.wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 1 << 63;
        if app.style_preview.as_ref().map(|p| p.0) != Some(key) {
            let shown = crate::layer_style::preview_document(&st.doc, &app.session.patterns, &d.fields).map(std::sync::Arc::new);
            app.style_preview = Some((key, shown));
        }
        if let Some((_, Some(doc))) = &app.style_preview {
            return (doc.clone(), key);
        }
    }
    if let Some((layer, params)) = &app.live_adjust
        && let Some(l) = st.doc.layer(*layer)
        && let LayerContent::Adjustment(a) = &l.content
    {
        let kind = photocraft_engine::commands::adjustment_kind(a);
        let mut doc = (*st.doc).clone();
        if let Some(lm) = doc.layer_mut(*layer) {
            lm.content = LayerContent::Adjustment(photocraft_engine::commands::adjustment_from_params(kind, params));
        }
        let key = 1 + params.to_string().bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
        return (std::sync::Arc::new(doc), key);
    }
    // Duotone documents display through their inks.
    if let Some(shown) = photocraft_engine::mode_cmds::display_document(&st.doc) {
        return (std::sync::Arc::new(shown), 1 << 41);
    }
    (st.doc.clone(), 0)
}

/// Make sure the canvas texture for document `idx` is current; returns (texture id, scale).
pub fn ensure_texture(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize) -> Option<(egui::TextureId, f32)> {
    let (revision, last_damage, id) = {
        let st = app.session.documents().get(idx)?;
        (st.revision, st.last_damage.map(|r| if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) }), st.doc.id)
    };
    let (doc, preview_key) = display_doc(app, idx);
    let cache = app.canvases.entry(id).or_insert(CanvasCache {
        revision: 0,
        texture: None,
        scale: 1.0,
        preview_key: 0,
        on_gpu: false,
        tex_revision: 0,
        tex_preview_key: 0,
    });
    if cache.tex_revision != revision || cache.texture.is_none() || cache.tex_preview_key != preview_key {
        let partial = cache.texture.is_some()
            && cache.tex_preview_key == preview_key
            && cache.tex_revision + 1 == revision
            && cache.scale == 1.0
            && last_damage.is_some();
        let t0 = crate::gpu_canvas::now_ms();
        if partial {
            let r = last_damage.unwrap().intersect(&doc.bounds());
            if !r.is_empty() {
                let buf = photocraft_compose::render(&doc, r);
                let t1 = crate::gpu_canvas::now_ms();
                if let Some(t) = cache.texture.as_mut() {
                    t.set_partial([r.x0 as usize, r.y0 as usize], buf_image(&buf), TextureOptions::LINEAR);
                }
                app.perf.record("rect", r.width() as u64 * r.height() as u64, t1 - t0, crate::gpu_canvas::now_ms() - t1);
            }
        } else {
            let full = photocraft_compose::flatten(&doc);
            let t1 = crate::gpu_canvas::now_ms();
            let longest = doc.size.width.max(doc.size.height);
            let factor = longest.div_ceil(MAX_TEXTURE).max(1);
            let (img, scale) = if factor > 1 { (buffer_to_image(&downsample(&full, factor)), 1.0 / factor as f32) } else { (buffer_to_image(&full), 1.0) };
            match cache.texture.as_mut() {
                Some(t) if t.size() == img.size => t.set(img, TextureOptions::LINEAR),
                _ => cache.texture = Some(ctx.load_texture(format!("canvas-{}", id.0), img, TextureOptions::LINEAR)),
            }
            cache.scale = scale;
            app.perf.record("full", doc.size.width as u64 * doc.size.height as u64, t1 - t0, crate::gpu_canvas::now_ms() - t1);
        }
        cache.tex_revision = revision;
        cache.tex_preview_key = preview_key;
    }
    Some((cache.texture.as_ref()?.id(), cache.scale))
}

/// How far beyond an edit's damage rect the composite can change: layer effects (shadows, glows,
/// strokes, …) on the edited layer and on the groups around it reach that far.
fn effect_reach(layers: &[photocraft_doc::Layer]) -> i32 {
    layers
        .iter()
        .filter(|l| l.visible)
        .map(|l| {
            let own = if photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l) } else { 0 };
            let inner = match &l.content {
                LayerContent::Group(g) => effect_reach(&g.children),
                _ => 0,
            };
            own + inner
        })
        .max()
        .unwrap_or(0)
}

/// GPU path: make sure document `idx` is current in the GPU canvas. Brush strokes re-composite
/// and upload only their damage rect; everything else re-composites the whole document.
/// Returns false if there is no GPU canvas.
fn ensure_gpu(app: &mut PhotocraftApp, idx: usize) -> bool {
    let Some(gpu) = app.gpu.clone() else { return false };
    let Some((revision, last_damage, id)) = app
        .session
        .documents()
        .get(idx)
        .map(|st| (st.revision, st.last_damage.map(|r| if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) }), st.doc.id))
    else {
        return false;
    };
    let (doc, preview_key) = display_doc(app, idx);
    let size = [doc.size.width, doc.size.height];
    let cache = app.canvases.entry(id).or_insert(CanvasCache {
        revision: 0,
        texture: None,
        scale: 1.0,
        preview_key: 0,
        on_gpu: false,
        tex_revision: 0,
        tex_preview_key: 0,
    });
    let present = cache.on_gpu && gpu.has(id.0, size);
    if present && cache.revision == revision && cache.preview_key == preview_key {
        return true;
    }
    let t0 = crate::gpu_canvas::now_ms();
    let partial = present && cache.preview_key == preview_key && cache.revision + 1 == revision && last_damage.is_some();
    let mut done = false;
    // Preferred: the wgpu compositor renders straight into the display texture (damage rect for
    // strokes, everything otherwise). Falls back to the CPU compositor below when unsupported.
    let region = if partial { last_damage.unwrap_or(doc.bounds()).intersect(&doc.bounds()) } else { doc.bounds() };
    match gpu.composite(&doc, region) {
        Ok(stats) => {
            done = true;
            let kind = if partial { "gpu-rect" } else { "gpu-full" };
            app.perf.record(kind, region.width() as u64 * region.height() as u64, crate::gpu_canvas::now_ms() - t0, 0.0);
            app.perf.gpu_uploads = stats.tiles_uploaded as u64;
        }
        Err(e) => {
            if app.perf.gpu_fallback.as_deref() != Some(e.0.as_str()) {
                log::info!("{e}; using the CPU compositor");
            }
            app.perf.gpu_fallback = Some(e.0);
        }
    }
    if partial && !done {
        let r = last_damage.unwrap().intersect(&doc.bounds());
        if r.is_empty() {
            done = true;
        } else {
            let buf = photocraft_compose::render(&doc, r);
            let t1 = crate::gpu_canvas::now_ms();
            done = gpu.upload_buffer_rect(id.0, &buf);
            app.perf.record("rect", r.width() as u64 * r.height() as u64, t1 - t0, crate::gpu_canvas::now_ms() - t1);
        }
    }
    if !done {
        // CPU fallback (documents the GPU compositor doesn't cover yet, e.g. layer effects).
        // TODO: render only the visible region at display resolution when zoomed out.
        let full = photocraft_compose::flatten(&doc);
        let t1 = crate::gpu_canvas::now_ms();
        gpu.upload_buffer_full(id.0, &full);
        app.perf.record("full", size[0] as u64 * size[1] as u64, t1 - t0, crate::gpu_canvas::now_ms() - t1);
    }
    let cache = app.canvases.get_mut(&id).expect("inserted above");
    cache.revision = revision;
    cache.preview_key = preview_key;
    cache.on_gpu = true;
    cache.texture = None;
    true
}

/// Live preview for an open filter dialog: run the filter on the proxy and upload it.
fn ensure_filter_preview(app: &mut PhotocraftApp, idx: usize) -> Option<(u32, u64)> {
    let d = app.ui.dialogs.iter().find(|d| d.fields.contains_key("__filter"))?;
    if d.fields.get("__preview").and_then(serde_json::Value::as_bool) != Some(true) {
        return None;
    }
    let cmd = d.fields.get("__command")?.as_str()?.to_string();
    let params = crate::filter_dialog::params_of(&d.fields);
    let (doc_id, revision, doc, active) = {
        let st = app.session.documents().get(idx)?;
        (st.doc.id, st.revision, st.doc.clone(), st.active_layer)
    };
    let k = crate::proxy::factor(&doc);
    let hash = format!("{cmd}{params}").bytes().fold(k as u64 ^ revision.wrapping_mul(0x9e37), |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    let key = doc_id.0 ^ (1u64 << 61);
    let fresh = matches!(&app.filter_preview, Some(p) if p.doc == doc_id && p.hash == hash);
    if !fresh {
        let t0 = crate::gpu_canvas::now_ms();
        let result = crate::filter_dialog::preview_document(&doc, active, &cmd, &params, k).map(std::sync::Arc::new);
        if let Some(r) = &result {
            let buf = photocraft_compose::flatten(r);
            let t1 = crate::gpu_canvas::now_ms();
            app.gpu.as_ref()?.upload_buffer_full(key, &buf);
            app.perf.record("filter-preview", r.size.area(), t1 - t0, crate::gpu_canvas::now_ms() - t1);
        }
        app.filter_preview = Some(crate::filter_dialog::FilterPreview { doc: doc_id, revision, hash, k, result });
    }
    app.filter_preview.as_ref().filter(|p| p.result.is_some()).map(|p| (p.k, key))
}

/// If a live adjustment preview is active on a large document, composite it on the proxy and upload
/// it under its own GPU key. Returns (factor, gpu key) when the proxy should be drawn.
fn ensure_proxy_preview(app: &mut PhotocraftApp, idx: usize) -> Option<(u32, u64)> {
    let (layer, params) = app.live_adjust.clone()?;
    let (doc_id, revision, doc) = {
        let st = app.session.documents().get(idx)?;
        (st.doc.id, st.revision, st.doc.clone())
    };
    let k = crate::proxy::factor(&doc);
    if k <= 1 {
        return None;
    }
    let fresh = matches!(&app.proxy, Some((d, r, kk, _)) if *d == doc_id && *r == revision && *kk == k);
    if !fresh {
        app.proxy = Some((doc_id, revision, k, std::sync::Arc::new(crate::proxy::proxy_document(&doc, k))));
    }
    let proxy = app.proxy.as_ref()?.3.clone();
    let key = doc_id.0 ^ (1u64 << 62);
    let hash = params.to_string().bytes().fold(k as u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    if app.proxy_uploaded != Some((doc_id, hash)) {
        let l = proxy.layer(layer)?;
        let LayerContent::Adjustment(a) = &l.content else { return None };
        let kind = photocraft_engine::commands::adjustment_kind(a);
        let mut p = (*proxy).clone();
        if let Some(lm) = p.layer_mut(layer) {
            lm.content = LayerContent::Adjustment(photocraft_engine::commands::adjustment_from_params(kind, &params));
        }
        let t0 = crate::gpu_canvas::now_ms();
        let buf = photocraft_compose::flatten(&p);
        let t1 = crate::gpu_canvas::now_ms();
        app.gpu.as_ref()?.upload_buffer_full(key, &buf);
        app.perf.record("proxy", p.size.area(), t1 - t0, crate::gpu_canvas::now_ms() - t1);
        app.proxy_uploaded = Some((doc_id, hash));
    }
    Some((k, key))
}

fn buf_image(buf: &photocraft_compose::Buffer) -> egui::ColorImage {
    buffer_to_image(buf)
}

/// Tabs + canvas for the active document, or the start screen.
pub fn document_area(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if let Some(gpu) = &app.gpu {
        // Keep each document's texture plus its preview textures (filter preview, adjustment proxy);
        // retaining only document ids freed the previews every frame (blank canvas while previewing).
        let live: Vec<u64> = app.session.documents().iter().flat_map(|st| [st.doc.id.0, st.doc.id.0 ^ (1u64 << 61), st.doc.id.0 ^ (1u64 << 62)]).collect();
        gpu.retain(&live);
    }
    if app.ui.chrome.shows_home(app.session.documents().len()) {
        start_screen(app, ui);
        return;
    }
    if !app.ui.view.hides_tabs() {
        tabs(app, ui);
    }
    let Some(idx) = app.session.active_index() else { return };
    let rect = ui.available_rect_before_wrap();
    app.last_canvas_rect = rect;
    let n = app.session.documents().len();
    // Window › Arrange: tiled / n-up layouts show several documents side by side; the active one
    // takes input, a click elsewhere activates that document.
    if let Some(cells) = crate::view_cmds::cells(&app.ui.view.arrange, rect, n) {
        let order: Vec<usize> = (0..n).map(|k| (idx + k) % n).collect();
        let mut shown: Vec<(usize, Rect)> = order.into_iter().zip(cells).collect();
        shown.sort_by_key(|(d, _)| *d);
        let t = crate::theme::Tokens::get(ui.ctx());
        for (d, cell) in shown {
            let cell = cell.shrink(1.0);
            let view = app.ui.views[d].clone();
            let v = canvas_view(app, ui, d, cell, view, d == idx);
            if d != idx {
                app.ui.views[d] = v;
                if ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|p| cell.contains(p))) {
                    app.session.set_active(d);
                }
            }
            let stroke = if d == idx { Stroke::new(1.5, t.accent) } else { Stroke::new(1.0, Color32::from_black_alpha(160)) };
            ui.painter().rect_stroke(cell, 0, stroke, egui::StrokeKind::Inside);
        }
        return;
    }
    let view = app.ui.views[idx].clone();
    canvas_view(app, ui, idx, rect, view, true);
}

fn tabs(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    if t.pro {
        return pro_tabs(app, ui);
    }
    let active = app.session.active_index();
    let mut activate = None;
    let mut close = None;
    egui::Frame::NONE.fill(t.canvas).inner_margin(egui::Margin { left: 8, right: 8, top: 6, bottom: 4 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (i, st) in app.session.documents().iter().enumerate() {
                let sel = Some(i) == active;
                let name = format!("{}{}", st.doc.name, if st.is_dirty() { " *" } else { "" });
                let meta = format!("{}/{}", mode_label(&st.doc), st.doc.depth.bits());
                let name_g = ui.painter().layout_no_wrap(name, crate::theme::medium(12.5), t.text);
                let meta_g = ui.painter().layout_no_wrap(meta, egui::FontId::proportional(10.5), t.text_faint);
                let w = name_g.size().x + meta_g.size().x + 44.0;
                let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 26.0), Sense::click());
                if sel {
                    ui.painter().rect_filled(r, t.radius_sm, t.card);
                    ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, t.radius_sm, t.hover.gamma_multiply(0.5));
                }
                let color = if sel { t.text } else { t.text_dim };
                let ny = r.center().y - name_g.size().y / 2.0;
                ui.painter().galley_with_override_text_color(egui::pos2(r.left() + 10.0, ny), name_g.clone(), color);
                ui.painter().galley(egui::pos2(r.left() + 16.0 + name_g.size().x, r.center().y - meta_g.size().y / 2.0), meta_g, t.text_faint);
                let xr = Rect::from_center_size(egui::pos2(r.right() - 12.0, r.center().y), egui::vec2(16.0, 16.0));
                let xresp = ui.interact(xr, ui.id().with(("tabx", i)), Sense::click());
                if xresp.hovered() {
                    ui.painter().rect_filled(xr, 4.0, t.hover);
                }
                crate::icons::paint(ui, xr, "x", 11.0, if xresp.hovered() { t.text } else { t.text_faint });
                if xresp.clicked() {
                    close = Some(i);
                } else if resp.clicked() {
                    activate = Some(i);
                }
            }
        });
    });
    if let Some(i) = activate {
        app.session.set_active(i);
    }
    if let Some(i) = close {
        let _ = crate::menus::invoke(app, ui.ctx(), "file.close", json!({"document": i}));
    }
}

/// Photoshop document tabs: "name @ 33.3% (RGB/8)" on a dark strip; active tab matches panels.
fn pro_tabs(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let active = app.session.active_index();
    let (mut activate, mut close) = (None, None);
    let (strip, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), Sense::hover());
    ui.painter().rect_filled(strip, 0.0, t.tab_strip);
    let mut x = strip.left();
    for (i, st) in app.session.documents().iter().enumerate() {
        let zoom = app.ui.views.get(i).map_or(100.0, |v| v.zoom * 100.0);
        // Photoshop: "name @ 50% (Layer 1, RGB/8)", "(Layer 1, Layer Mask/8)" when the mask is targeted;
        // the Background layer's name is omitted.
        let active_layer = st.active_layer.and_then(|id| st.doc.layer(id)).filter(|l| !(l.name == "Background" && l.locks.transparency));
        let is_active_doc = app.session.active_index() == Some(i);
        let mask = is_active_doc && app.ui.mask_target && active_layer.is_some_and(|l| l.mask.is_some());
        let model = if mask { "Layer Mask".to_string() } else { mode_label(&st.doc).to_string() };
        let layer = active_layer.map(|l| format!("{}, ", l.name)).unwrap_or_default();
        let title = format!("{} @ {}% ({layer}{model}/{}){}", st.doc.name, fmt_zoom(zoom), st.doc.depth.bits(), if st.is_dirty() { "*" } else { "" });
        let g = ui.painter().layout_no_wrap(title, egui::FontId::proportional(11.5), t.text);
        let r = Rect::from_min_size(egui::pos2(x, strip.top()), egui::vec2(g.size().x + 42.0, strip.height()));
        let resp = ui.interact(r, ui.id().with(("ptab", i)), Sense::click());
        let sel = Some(i) == active;
        if sel {
            ui.painter().rect_filled(r, 0.0, t.chrome);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.35));
        }
        ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.0, t.separator));
        let xr = Rect::from_center_size(egui::pos2(r.left() + 13.0, r.center().y), egui::vec2(14.0, 14.0));
        let xresp = ui.interact(xr, ui.id().with(("ptabx", i)), Sense::click());
        crate::icons::paint(ui, xr, "x", 10.0, if xresp.hovered() { t.text } else { t.text_faint });
        ui.painter().galley_with_override_text_color(egui::pos2(r.left() + 26.0, r.center().y - g.size().y / 2.0), g, if sel { t.text } else { t.text_faint });
        if xresp.clicked() {
            close = Some(i);
        } else if resp.clicked() {
            activate = Some(i);
        }
        x = r.right();
    }
    if let Some(i) = activate {
        app.session.set_active(i);
    }
    if let Some(i) = close {
        let _ = crate::menus::invoke(app, ui.ctx(), "file.close", json!({"document": i}));
    }
}

/// Photoshop-style zoom label: "33.3", "100", "12.5".
pub fn fmt_zoom(pct: f32) -> String {
    if (pct - pct.round()).abs() < 0.05 { format!("{}", pct.round() as i64) } else { format!("{pct:.1}") }
}

/// Repeating dot-grid texture for the canvas surround.
fn dots(ctx: &egui::Context, t: &crate::theme::Tokens) -> Option<egui::TextureId> {
    if t.bevel {
        return None;
    }
    let key = egui::Id::new(("canvas-dots", format!("{:?}", t.kind)));
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(tex.id());
    }
    let n = 22usize;
    let mut px = vec![Color32::TRANSPARENT; n * n];
    for (dx, dy, a) in [(0usize, 0usize, 255u8), (1, 0, 110), (0, 1, 110), (1, 1, 60)] {
        let c = t.canvas_dot;
        px[(n / 2 + dy) * n + n / 2 + dx] = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
    }
    let opts = TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::Repeat,
        mipmap_mode: None,
    };
    let tex = ctx.load_texture("canvas-dots", egui::ColorImage::new([n, n], px), opts);
    let id = tex.id();
    ctx.data_mut(|d| d.insert_temp(key, tex));
    Some(id)
}

fn paint_dots(ui: &egui::Ui, rect: Rect) {
    let t = crate::theme::Tokens::get(ui.ctx());
    if let Some(id) = dots(ui.ctx(), &t) {
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(rect.width() / 22.0, rect.height() / 22.0));
        ui.painter_at(rect).image(id, rect, uv, Color32::WHITE);
    }
}

pub fn mode_label(doc: &Document) -> &'static str {
    match doc.mode {
        photocraft_doc::ColorMode::Rgb => "RGB",
        photocraft_doc::ColorMode::Grayscale => "Gray",
        photocraft_doc::ColorMode::Cmyk => "CMYK",
        photocraft_doc::ColorMode::Lab => "Lab",
        photocraft_doc::ColorMode::Indexed => "Indexed",
        photocraft_doc::ColorMode::Bitmap => "Bitmap",
        photocraft_doc::ColorMode::Duotone => "Duotone",
        photocraft_doc::ColorMode::Multichannel => "Multichannel",
    }
}

fn start_screen(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let area = ui.available_rect_before_wrap();
    paint_dots(ui, area);
    let card = Rect::from_center_size(area.center(), egui::vec2(460.0, 330.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
        ui.vertical_centered(|ui| {
            ui.horizontal(|ui| {
                let title = ui.painter().layout_no_wrap("PhotoCraft".into(), crate::theme::semibold(38.0), t.text);
                let by = ui.painter().layout_no_wrap("open source".into(), egui::FontId::proportional(13.0), t.text_faint);
                let total = title.size().x + by.size().x + 10.0;
                ui.add_space(((card.width() - total) / 2.0).max(0.0));
                let (r, _) = ui.allocate_exact_size(title.size(), Sense::hover());
                ui.painter().galley(r.min, title, t.text);
                let (r2, _) = ui.allocate_exact_size(egui::vec2(by.size().x + 10.0, r.height()), Sense::hover());
                ui.painter().galley(egui::pos2(r2.left() + 10.0, r.bottom() - by.size().y - 8.0), by, t.text_faint);
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Create a new document or open an existing file.").color(t.text_dim).size(14.0));
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                ui.add_space(((card.width() - 2.0 * 190.0 - 12.0) / 2.0).max(0.0));
                ui.spacing_mut().item_spacing.x = 12.0;
                if crate::widgets::primary_button(ui, "New document…     ⌘N", 190.0).clicked() {
                    app.ui.open_dialog(crate::state::DialogKind::NewDocument, crate::state::UiState::new_document_fields());
                }
                if crate::widgets::secondary_button(ui, "Open…     ⌘O", 190.0).clicked() {
                    app.open_dialog_file();
                }
            });
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                let msg = "Drop an image or PSD anywhere to open it.";
                let g = ui.painter().layout_no_wrap(msg.into(), egui::FontId::proportional(12.5), t.text_faint);
                ui.add_space(((card.width() - g.size().x - 24.0) / 2.0).max(0.0));
                let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
                crate::icons::paint(ui, r, "image", 15.0, t.text_faint);
                ui.label(egui::RichText::new(msg).color(t.text_faint));
            });
            ui.add_space(26.0);
            crate::links::discord_button(app, ui, 190.0);
            ui.add_space(10.0);
            crate::links::link_row(app, ui);
        });
    });
}

/// Lattice size of the canvas display LUT (Proof Colors / Gamut Warning).
const DISPLAY_LUT: usize = 33;

/// Keep the GPU display LUT of `doc` in step with its View › Proof Colors / Gamut Warning state.
/// Returns the canvas `display` mode (0 none, 1 proof, 2 proof + gamut warning).
fn sync_display_lut(app: &mut PhotocraftApp, ctx: &egui::Context, doc: &photocraft_doc::Document) -> u8 {
    let Some(gpu) = app.gpu.as_ref() else { return 0 };
    let pv = app.session.color.proof(doc.id);
    let mode = if pv.gamut_warning { 2 } else { u8::from(pv.enabled) };
    // Rebuild only when anything feeding the LUT changes.
    let sig = if mode == 0 {
        String::new()
    } else {
        format!(
            "{mode} {} {:?} {} {} {} {:?} {:?}",
            pv.setup.profile.content_hash(),
            pv.setup.intent,
            pv.setup.bpc,
            pv.setup.simulate_paper,
            pv.gamut_threshold,
            doc.mode,
            doc.icc_profile.as_ref().map(|p| p.len())
        )
    };
    let key = egui::Id::new(("pc-display-lut", doc.id.0));
    if ctx.data(|d| d.get_temp::<String>(key)).as_deref() == Some(sig.as_str()) {
        return mode;
    }
    let lut = if mode == 0 { Ok(None) } else { app.session.color.canvas_lut(doc, DISPLAY_LUT) };
    match lut {
        Ok(bytes) => gpu.set_display_lut(doc.id.0, DISPLAY_LUT as u32, bytes.as_deref()),
        Err(e) => {
            app.ui.status = format!("Proof Colors: {e}");
            gpu.set_display_lut(doc.id.0, DISPLAY_LUT as u32, None);
        }
    }
    ctx.data_mut(|d| d.insert_temp(key, sig));
    mode
}

/// Draw one canvas view and handle its input. `primary` = main window (tools active).
pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect: Rect, mut view: View, primary: bool) -> View {
    let ctx = ui.ctx().clone();
    let full = rect;
    let rect = if primary { crate::rulers::content_rect(app, rect) } else { rect };
    let doc = app.session.documents()[idx].doc.clone();
    let size = [doc.size.width, doc.size.height];
    if view.doc_size != size {
        // Like Photoshop: keep the zoom level, re-centre the resized document.
        if view.doc_size != [0, 0] {
            view.center = [size[0] as f32 / 2.0, size[1] as f32 / 2.0];
        }
        view.doc_size = size;
    }
    if view.fit_pending && rect.width() > 50.0 {
        fit_view(&mut view, &doc, rect.size());
    }
    let flip = app.ui.view.flip_horizontal;
    let xf = ViewXform { rect, zoom: view.zoom, center: view.center, flip };
    let pixel_grid = app.ui.view.shows(app.ui.view.show.pixel_grid);
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    match crate::prefs_ui::pasteboard_color(app) {
        Some(c) => {
            ui.painter_at(rect).rect_filled(rect, 0.0, c);
        }
        None => paint_dots(ui, rect),
    }
    let border = app.session.prefs().interface.canvas_border;
    let drop_shadow = border == photocraft_engine::prefs::CanvasBorder::DropShadow;
    // Drop shadow, checkerboard, document image.
    let img_rect = xf.doc_rect(doc.bounds());
    // Live adjustment previews on big documents use a downsampled proxy (see proxy.rs).
    let mut on_gpu = false;
    // A flipped view draws through the CPU path (the GPU canvas shader has no mirroring).
    if app.gpu.is_some()
        && !flip
        && let Some((k, key)) = ensure_filter_preview(app, idx).or_else(|| ensure_proxy_preview(app, idx))
    {
        on_gpu = true;
        let params = crate::gpu_canvas::ViewParams {
            doc: key,
            doc_size: [doc.size.width.div_ceil(k), doc.size.height.div_ceil(k)],
            zoom: view.zoom * k as f32,
            center: [view.center[0] / k as f32, view.center[1] / k as f32],
            shadow: {
                let t = crate::theme::Tokens::get(&ctx);
                !t.bevel && !t.pro && drop_shadow
            },
            pixel_grid: false,
            view_key: egui::Id::new(("pc-canvas-proxy", ctx.viewport_id(), idx)).value(),
            display: 0,
        };
        crate::gpu_canvas::GpuCanvas::paint(&painter, rect, params);
    } else if !flip && ensure_gpu(app, idx) {
        on_gpu = true;
        app.perf.gpu = true;
        // Shadow, checkerboard, document and pixel grid in one custom shader (gpu_canvas.rs).
        let params = crate::gpu_canvas::ViewParams {
            doc: doc.id.0,
            doc_size: [doc.size.width, doc.size.height],
            zoom: view.zoom,
            center: view.center,
            shadow: {
                let t = crate::theme::Tokens::get(&ctx);
                !t.bevel && !t.pro && drop_shadow
            },
            pixel_grid,
            view_key: egui::Id::new(("pc-canvas", ctx.viewport_id(), idx)).value(),
            display: sync_display_lut(app, &ctx, &doc),
        };
        crate::gpu_canvas::GpuCanvas::paint(&painter, rect, params);
    } else {
        if !crate::theme::Tokens::get(&ctx).bevel && drop_shadow {
            painter.add(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(150) }.as_shape(img_rect, 0));
        }
        match app.session.prefs().transparency_and_gamut.square() {
            Some(square) => {
                let checker_id = checker(app, &ctx);
                let tiles = img_rect.size() / (2.0 * square);
                painter.image(checker_id, img_rect, Rect::from_min_max(Pos2::ZERO, pos2(tiles.x, tiles.y)), Color32::WHITE);
            }
            None => {
                painter.rect_filled(img_rect, 0.0, Color32::WHITE);
            }
        }
        if let Some((tex, _scale)) = ensure_texture(app, &ctx, idx) {
            let uv = if flip { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)) };
            painter.image(tex, img_rect, uv, Color32::WHITE);
        }
    }
    // Channels panel: alpha / Quick Mask overlays and single-channel views (channel_view.rs).
    if let Some(tex) = crate::channel_view::ensure(app, &ctx, idx) {
        let uv = if flip { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)) };
        painter.image(tex, img_rect, uv, Color32::WHITE);
    }
    // Artboards: pasteboard between the boards, outlines and names (artboard_ui.rs).
    if doc.has_artboards() {
        let t = crate::theme::Tokens::get(&ctx);
        let pasteboard = crate::prefs_ui::pasteboard_color(app);
        let dot_tex = dots(&ctx, &t);
        for r in crate::artboard_ui::pasteboard_rects(&xf, &doc) {
            let r = r.intersect(rect);
            if !r.is_positive() {
                continue;
            }
            match pasteboard {
                Some(c) => {
                    painter.rect_filled(r, 0.0, c);
                }
                None => {
                    painter.rect_filled(r, 0.0, t.canvas);
                    if let Some(id) = dot_tex {
                        // Same phase as the dots around the document.
                        let uv = Rect::from_min_max(((r.min - rect.min) / 22.0).to_pos2(), ((r.max - rect.min) / 22.0).to_pos2());
                        painter.image(id, r, uv, Color32::WHITE);
                    }
                }
            }
        }
        if primary {
            crate::artboard_ui::draw_frames(app, &painter, &xf);
        }
    }

    // Pixel grid at high zoom (the GPU path draws its own).
    if !on_gpu && pixel_grid && view.zoom >= 12.0 {
        let vis = img_rect.intersect(rect);
        let a = xf.to_doc(vis.min);
        let b = xf.to_doc(vis.max);
        let grid = Stroke::new(1.0, Color32::from_white_alpha(40));
        for x in (a[0].floor() as i32)..=(b[0].ceil() as i32) {
            let sx = xf.to_screen(x as f32, 0.0).x;
            painter.line_segment([pos2(sx, vis.top()), pos2(sx, vis.bottom())], grid);
        }
        for y in (a[1].floor() as i32)..=(b[1].ceil() as i32) {
            let sy = xf.to_screen(0.0, y as f32).y;
            painter.line_segment([pos2(vis.left(), sy), pos2(vis.right(), sy)], grid);
        }
    }

    // View › Show › Layer Edges: the active layer's content bounds.
    if app.ui.view.shows(app.ui.view.show.layer_edges)
        && let Some(st) = app.session.documents().get(idx)
        && let Some(b) = st.active_layer.and_then(|id| st.doc.layer(id)).and_then(|l| l.surface()).map(|s| s.content_bounds())
        && !b.is_empty()
    {
        painter.rect_stroke(xf.doc_rect(b), 0, Stroke::new(1.0, Color32::from_rgb(0x2d, 0x8c, 0xeb)), egui::StrokeKind::Outside);
    }

    // Selection outline: true boundary, animated marching ants (cached per revision).
    if let Some(sel) = doc.selection.as_ref().filter(|_| app.ui.view.shows(app.ui.view.show.selection_edges)) {
        // Trace at display resolution over the visible part only; key by the mask's tile identity
        // (not the document revision) so unrelated edits don't re-trace it.
        let step = (1.0 / view.zoom.max(1e-3)).log2().floor().exp2().clamp(1.0, 64.0) as u32;
        let tl = xf.to_doc(rect.min);
        let br = xf.to_doc(rect.max);
        let q = 256 * step as i32; // quantise the region so small pans reuse the cache
        let vis = photocraft_geom::Rect::new(
            (tl[0].floor() as i32).div_euclid(q) * q - q,
            (tl[1].floor() as i32).div_euclid(q) * q - q,
            ((br[0].ceil() as i32).div_euclid(q) + 2) * q,
            ((br[1].ceil() as i32).div_euclid(q) + 2) * q,
        );
        let key = crate::surface_fingerprint(sel)
            ^ (step as u64) << 56
            ^ doc.id.0.rotate_left(17)
            ^ (vis.x0 as u64) << 8
            ^ (vis.y0 as u64) << 24
            ^ (vis.x1 as u64) << 36
            ^ (vis.y1 as u64) << 48;
        let fresh = matches!(&app.outline_cache, Some((d, k, _)) if *d == doc.id && *k == key);
        if !fresh {
            let t0 = crate::gpu_canvas::now_ms();
            let b = app.cached_bounds(u64::MAX - doc.id.0, sel).intersect(&vis);
            let segs = crate::outline::outline_scaled(sel, b, step);
            app.perf.span("outline", crate::gpu_canvas::now_ms() - t0);
            app.outline_cache = Some((doc.id, key, std::sync::Arc::new(segs)));
        }
        if let Some((_, _, segs)) = &app.outline_cache {
            let time = ui.input(|i| i.time);
            marching_ants_segments(&painter, &xf, segs, time);
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    // Navigation: scroll pans, pinch / ⌘-scroll zooms around the pointer.
    if response.hovered() {
        let (scroll, zoom_delta, pointer) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
        if zoom_delta != 1.0
            && let Some(p) = pointer
        {
            let nz = (view.zoom * zoom_delta).clamp(0.01, 64.0);
            zoom_about(&mut view, &xf, p, nz);
        } else if scroll.y != 0.0
            && app.session.prefs().general.zoom_with_scroll_wheel
            && let Some(p) = pointer
        {
            // Preferences › General › Zoom with Scroll Wheel.
            let nz = (view.zoom * (scroll.y / 200.0).exp()).clamp(0.01, 64.0);
            zoom_about(&mut view, &xf, p, nz);
        } else if scroll != Vec2::ZERO {
            view.center[0] -= scroll.x / view.zoom * if flip { -1.0 } else { 1.0 };
            view.center[1] -= scroll.y / view.zoom;
        }
    }

    let space_pan = ui.input(|i| i.key_down(egui::Key::Space));
    let middle = ui.input(|i| i.pointer.middle_down());
    let tool = if space_pan || middle { Tool::Hand } else { app.ui.tool };

    if tool == Tool::Hand && response.dragged() {
        let d = response.drag_delta();
        view.center[0] -= d.x / view.zoom * if flip { -1.0 } else { 1.0 };
        view.center[1] -= d.y / view.zoom;
    } else if primary {
        let mods = ui.input(|i| i.modifiers);
        if response.drag_started()
            && let Some(p) = response.interact_pointer_pos()
        {
            let d = xf.to_doc(p);
            tool_event(app, ToolEvent::Down { x: d[0], y: d[1], pressure: 1.0 }, mods);
        }
        if response.dragged()
            && let Some(p) = response.interact_pointer_pos()
        {
            let d = xf.to_doc(p);
            tool_event(app, ToolEvent::Move { x: d[0], y: d[1], pressure: 1.0 }, mods);
        }
        if response.drag_stopped() {
            let p = response.interact_pointer_pos().map(|p| xf.to_doc(p)).or_else(|| app.drag.as_ref().and_then(|d| d.points.last().map(|q| [q[0], q[1]])));
            if let Some(d) = p {
                tool_event(app, ToolEvent::Up { x: d[0], y: d[1] }, mods);
            }
        }
        if response.clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            let d = xf.to_doc(p);
            match tool {
                Tool::Zoom => {
                    let nz = zoom_step(view.zoom, if mods.alt { -1 } else { 1 });
                    zoom_about(&mut view, &xf, p, nz);
                }
                _ => {
                    tool_event(app, ToolEvent::Down { x: d[0], y: d[1], pressure: 1.0 }, mods);
                    tool_event(app, ToolEvent::Up { x: d[0], y: d[1] }, mods);
                }
            }
        }
        if app.ui.transform.is_some() && response.double_clicked() {
            crate::transform_tool::commit(app);
        }
        if tool == Tool::Type && response.double_clicked() {
            crate::type_tool::select_word(app);
        }
        if app.ui.extras.grid && app.ui.view.extras {
            crate::rulers::draw_grid(app, &painter, &xf, &doc);
        }
        if app.ui.view.shows(app.ui.view.show.canvas_guides) {
            crate::rulers::draw_guides(app, &painter, &xf, &doc);
        }
        draw_drag_preview(app, &painter, &xf);
        draw_transform_controls(app, &painter, &xf);
        crate::snap_ui::draw(app, &painter, &xf);
        if border == photocraft_engine::prefs::CanvasBorder::Line {
            painter.rect_stroke(img_rect, 0.0, Stroke::new(1.0, Color32::from_gray(20)), egui::StrokeKind::Outside);
        }
        crate::type_tool::draw_overlay(app, &painter, &xf);
        crate::transform_tool::draw_overlay(app, &painter, &xf);
        crate::distort_ui::draw_overlay(app, &painter, &xf);
        crate::retouch_ui::draw_source_marker(app, &painter, &xf);
        crate::vector_ui::draw_overlay(app, &painter, &xf, &doc);
        crate::analysis_ui::draw_overlay(app, &painter, &xf);
        crate::slice_ui::draw_overlay(app, &painter, &xf);
        // Tool cursors (Photoshop-style).
        let guide_hover = response.hover_pos().filter(|_| tool == Tool::Move).and_then(|p| {
            let d = xf.to_doc(p);
            crate::rulers::guide_at(app, d[0], d[1])
        });
        if let Some((vertical, _)) = guide_hover {
            ui.ctx().set_cursor_icon(if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
        } else if let Some(c) = response.hover_pos().and_then(|p| crate::transform_tool::cursor(app, xf.to_doc(p))) {
            ui.ctx().set_cursor_icon(c);
        } else if let Some(p) = response.hover_pos() {
            let alt = ui.input(|i| i.modifiers.alt);
            let icon = match tool {
                t if t.is_brushlike() || t == Tool::QuickSelection => {
                    // Preferences › Cursors: brush tip outline (normal = the 50% contour, or
                    // full size), precise crosshair, or the standard pointer.
                    use photocraft_engine::prefs::PaintingCursor;
                    let cur = app.session.prefs().cursors.clone();
                    let painting = app.drag.is_some();
                    let brush = &app.session.tools.brush;
                    let full = (brush.size / 2.0 * view.zoom).max(1.0);
                    let r = if cur.painting == PaintingCursor::NormalTip { (full * (0.5 + 0.5 * brush.hardness.clamp(0.0, 1.0))).max(1.0) } else { full };
                    let crosshair = |len: f32| {
                        for (w, c) in [(2.5, Color32::from_black_alpha(140)), (1.0, Color32::from_white_alpha(220))] {
                            painter.line_segment([p - vec2(len, 0.0), p + vec2(len, 0.0)], Stroke::new(w, c));
                            painter.line_segment([p - vec2(0.0, len), p + vec2(0.0, len)], Stroke::new(w, c));
                        }
                    };
                    match cur.painting {
                        PaintingCursor::Standard => egui::CursorIcon::Default,
                        PaintingCursor::Precise => {
                            crosshair(6.0);
                            egui::CursorIcon::None
                        }
                        _ if painting && cur.show_only_crosshair_while_painting => {
                            crosshair(5.0);
                            egui::CursorIcon::None
                        }
                        _ => {
                            painter.circle_stroke(p, r + 0.5, Stroke::new(1.0, Color32::from_black_alpha(140)));
                            painter.circle_stroke(p, r, Stroke::new(1.0, Color32::from_white_alpha(220)));
                            if cur.show_crosshair_in_brush_tip || r > 6.0 {
                                crosshair(3.0);
                            }
                            egui::CursorIcon::None
                        }
                    }
                }
                // Preferences › Cursors › Other Cursors: Precise shows a crosshair for every tool.
                Tool::Move | Tool::Type | Tool::Eyedropper if app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise => {
                    egui::CursorIcon::Crosshair
                }
                Tool::Move => egui::CursorIcon::Move,
                Tool::Hand => {
                    if response.dragged() {
                        egui::CursorIcon::Grabbing
                    } else {
                        egui::CursorIcon::Grab
                    }
                }
                Tool::Zoom => {
                    if alt {
                        egui::CursorIcon::ZoomOut
                    } else {
                        egui::CursorIcon::ZoomIn
                    }
                }
                Tool::Type => egui::CursorIcon::Text,
                _ => egui::CursorIcon::Crosshair,
            };
            ui.ctx().set_cursor_icon(icon);
        }
    }
    if primary {
        app.hover_doc = response.hover_pos().map(|p| xf.to_doc(p));
    }
    if primary && app.ui.extras.rulers {
        crate::rulers::draw_rulers(app, ui, full, &xf);
    }
    if primary {
        app.ui.views[idx] = view.clone();
    }
    view
}

fn zoom_about(view: &mut View, xf: &ViewXform, p: Pos2, new_zoom: f32) {
    let before = xf.to_doc(p);
    view.zoom = new_zoom;
    let d = (p - xf.rect.center()) / new_zoom;
    let dx = if xf.flip { -d.x } else { d.x };
    view.center = [before[0] as f32 - dx, before[1] as f32 - d.y];
}

/// Draw boundary segments as marching ants: white base, black dashes phased along x + y.
fn marching_ants_segments(painter: &egui::Painter, xf: &ViewXform, segs: &[crate::outline::Segment], time: f64) {
    let dash = 4.0f32;
    let phase = ((time * 10.0) % (dash as f64 * 2.0)) as f32;
    let white = Stroke::new(1.0, Color32::WHITE);
    let black = Stroke::new(1.0, Color32::BLACK);
    let clip = painter.clip_rect();
    for (a, b) in segs {
        let pa = xf.to_screen(a[0] as f32, a[1] as f32);
        let pb = xf.to_screen(b[0] as f32, b[1] as f32);
        if !clip.intersects(Rect::from_two_pos(pa, pb).expand(1.0)) {
            continue;
        }
        let pa = pos2(pa.x.round() + 0.5, pa.y.round() + 0.5);
        let pb = pos2(pb.x.round() + 0.5, pb.y.round() + 0.5);
        painter.line_segment([pa, pb], white);
        let len = pa.distance(pb);
        if len < 0.5 {
            continue;
        }
        let dir = (pb - pa) / len;
        // Phase by screen position so dashes line up across joined segments.
        let start = (pa.x + pa.y + phase).rem_euclid(dash * 2.0);
        let mut t = -start;
        while t < len {
            let s0 = t.max(0.0);
            let s1 = (t + dash).min(len);
            if s1 > s0 {
                painter.line_segment([pa + dir * s0, pa + dir * s1], black);
            }
            t += dash * 2.0;
        }
    }
}

#[allow(dead_code)]
fn marching_ants(painter: &egui::Painter, r: Rect, time: f64) {
    let dash = 4.0;
    let offset = ((time * 8.0) % (dash as f64 * 2.0)) as f32;
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = a.distance(b);
        let dir = (b - a) / len.max(1e-3);
        let mut t = -offset;
        while t < len {
            let s = t.max(0.0);
            let e = (t + dash).min(len);
            if e > s {
                painter.line_segment([a + dir * s, a + dir * e], Stroke::new(1.0, Color32::BLACK));
            }
            t += dash * 2.0;
        }
    }
}

/// Photoshop crop overlay: dimmed outside, bright frame, rule-of-thirds grid, corner handles.
fn crop_overlay(painter: &egui::Painter, r: Rect) {
    let clip = painter.clip_rect();
    let dim = Color32::from_black_alpha(130);
    for band in [
        Rect::from_min_max(clip.min, egui::pos2(clip.max.x, r.min.y)),
        Rect::from_min_max(egui::pos2(clip.min.x, r.max.y), clip.max),
        Rect::from_min_max(egui::pos2(clip.min.x, r.min.y), egui::pos2(r.min.x, r.max.y)),
        Rect::from_min_max(egui::pos2(r.max.x, r.min.y), egui::pos2(clip.max.x, r.max.y)),
    ] {
        painter.rect_filled(band, 0.0, dim);
    }
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
    let thin = Stroke::new(1.0, Color32::from_white_alpha(90));
    for i in 1..3 {
        let fx = r.left() + r.width() * i as f32 / 3.0;
        let fy = r.top() + r.height() * i as f32 / 3.0;
        painter.line_segment([egui::pos2(fx, r.top()), egui::pos2(fx, r.bottom())], thin);
        painter.line_segment([egui::pos2(r.left(), fy), egui::pos2(r.right(), fy)], thin);
    }
    let h = Stroke::new(3.0, Color32::WHITE);
    let l = 14.0f32.min(r.width() / 3.0).min(r.height() / 3.0);
    for (c, dx, dy) in [(r.left_top(), 1.0, 1.0), (r.right_top(), -1.0, 1.0), (r.left_bottom(), 1.0, -1.0), (r.right_bottom(), -1.0, -1.0)] {
        painter.line_segment([c, c + vec2(l * dx, 0.0)], h);
        painter.line_segment([c, c + vec2(0.0, l * dy)], h);
    }
}

/// Overlays that persist between gestures: polygonal lasso in progress, pending crop box.
fn draw_tool_state(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, hover: Option<Pos2>) {
    if !app.ui.polygon.is_empty() {
        let mut pts: Vec<Pos2> = app.ui.polygon.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
        if let Some(h) = hover {
            pts.push(h);
        }
        painter.add(egui::Shape::line(pts.clone(), Stroke::new(1.0, Color32::WHITE)));
        painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, Color32::BLACK), 4.0, 4.0));
        for p in &pts[..app.ui.polygon.len()] {
            painter.rect_filled(Rect::from_center_size(*p, vec2(5.0, 5.0)), 0.0, Color32::WHITE);
        }
    }
    if let Some(c) = app.ui.crop_rect
        && app.drag.is_none()
    {
        let r = Rect::from_two_pos(xf.to_screen(c[0] as f32, c[1] as f32), xf.to_screen(c[2] as f32, c[3] as f32));
        crop_overlay(painter, r);
    }
}

/// Move tool › Show Transform Controls: the active layer's bounding box with its eight handles.
fn draw_transform_controls(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if app.ui.tool != Tool::Move || !app.ui.tool_options.move_show_transform || app.ui.transform.is_some() || app.drag.is_some() {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(l) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return };
    if crate::doc_props_ui::is_background(&st.doc, l) {
        return;
    }
    let (id, Some(surf)) = (l.id.0, l.surface().cloned()) else { return };
    let b = app.cached_bounds(id, &surf);
    if b.is_empty() {
        return;
    }
    let r = Rect::from_two_pos(xf.to_screen(b.x0 as f32, b.y0 as f32), xf.to_screen(b.x1 as f32, b.y1 as f32));
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Middle);
    for p in [r.left_top(), r.center_top(), r.right_top(), r.right_center(), r.right_bottom(), r.center_bottom(), r.left_bottom(), r.left_center()] {
        let h = Rect::from_center_size(p, vec2(7.0, 7.0));
        painter.rect_filled(h, 0.0, Color32::WHITE);
        painter.rect_stroke(h, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    }
}

fn draw_drag_preview(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    draw_tool_state(app, painter, xf, painter.ctx().input(|i| i.pointer.hover_pos()));
    let Some(d) = &app.drag else { return };
    let mut last = d.points.last().map(|p| [p[0], p[1]]).unwrap_or(d.start);
    if matches!(d.tool, Tool::RectMarquee | Tool::EllipseMarquee) {
        let o = &app.ui.tool_options;
        last = crate::chrome_ui::marquee_end(&o.marquee_style, o.marquee_width as f64, o.marquee_height as f64, false, d.start, last);
    }
    if d.tool == Tool::Crop {
        last = crop_end(app, d.start, last);
    }
    match d.tool {
        Tool::Brush | Tool::Eraser => {
            let c = if d.tool == Tool::Eraser { [1.0, 1.0, 1.0, 0.6] } else { app.session.tools.foreground };
            let color = Color32::from_rgba_unmultiplied((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8);
            let pts: Vec<Pos2> = d.points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
            let w = (app.session.tools.brush.size * xf.zoom).max(1.0);
            if pts.len() == 1 {
                painter.circle_filled(pts[0], w / 2.0, color);
            } else {
                painter.add(egui::Shape::line(pts, Stroke::new(w, color)));
            }
        }
        t if t.is_brushlike() || t == Tool::QuickSelection => {
            // Retouching strokes preview as a translucent trail of the brush footprint.
            let pts: Vec<Pos2> = d.points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
            let w = (app.session.tools.brush.size * xf.zoom).max(1.0);
            let col = Color32::from_white_alpha(if t == Tool::QuickSelection { 40 } else { 60 });
            if pts.len() == 1 {
                painter.circle_filled(pts[0], w / 2.0, col);
            } else {
                painter.add(egui::Shape::line(pts, Stroke::new(w, col)));
            }
        }
        Tool::Line => {
            painter.line_segment(
                [xf.to_screen(d.start[0] as f32, d.start[1] as f32), xf.to_screen(last[0] as f32, last[1] as f32)],
                Stroke::new(1.0, crate::theme::Tokens::get(painter.ctx()).accent),
            );
        }
        Tool::RectMarquee
        | Tool::EllipseMarquee
        | Tool::ObjectSelection
        | Tool::Rectangle
        | Tool::EllipseShape
        | Tool::Triangle
        | Tool::Polygon
        | Tool::CustomShape => {
            let r = Rect::from_two_pos(xf.to_screen(d.start[0] as f32, d.start[1] as f32), xf.to_screen(last[0] as f32, last[1] as f32));
            if matches!(d.tool, Tool::EllipseMarquee | Tool::EllipseShape) {
                painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2.0, Stroke::new(1.0, Color32::WHITE)));
            } else {
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
            }
        }
        Tool::Lasso => {
            let pts: Vec<Pos2> = d.points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect();
            if pts.len() > 1 {
                painter.add(egui::Shape::line(pts.clone(), Stroke::new(1.0, Color32::WHITE)));
                painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, Color32::BLACK), 4.0, 4.0));
            }
        }
        Tool::Gradient => {
            let a = xf.to_screen(d.start[0] as f32, d.start[1] as f32);
            let b = xf.to_screen(last[0] as f32, last[1] as f32);
            painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(140)));
            painter.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
            painter.circle_filled(a, 3.0, Color32::WHITE);
            painter.circle_filled(b, 3.0, Color32::WHITE);
        }
        Tool::Crop => {
            let r = Rect::from_two_pos(xf.to_screen(d.start[0] as f32, d.start[1] as f32), xf.to_screen(last[0] as f32, last[1] as f32));
            crop_overlay(painter, r);
        }
        Tool::Type => {
            let r = Rect::from_two_pos(xf.to_screen(d.start[0] as f32, d.start[1] as f32), xf.to_screen(last[0] as f32, last[1] as f32));
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            painter.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.0, Color32::WHITE)));
            painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, Color32::BLACK), 3.0, 3.0));
        }
        Tool::Move => {
            let off = vec2(((last[0] - d.start[0]) as f32) * xf.zoom, ((last[1] - d.start[1]) as f32) * xf.zoom);
            painter.arrow(xf.to_screen(d.start[0] as f32, d.start[1] as f32), off, Stroke::new(2.0, crate::theme::Tokens::get(painter.ctx()).accent));
        }
        _ => {}
    }
}

/// Tool state machine. Shared by mouse input and automation.
pub fn tool_event(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    // View › Snap / Snap To and smart guides (snap_ui.rs).
    let ev = crate::snap_ui::filter_event(app, ev, mods);
    if crate::transform_tool::pointer(app, ev, mods) {
        return;
    }
    if crate::distort_ui::pointer(app, ev, mods) {
        return;
    }
    // Window › Modifier Keys: sticky Shift/⌘/⌥ act as held keys.
    let mods = crate::workspace_ui::sticky_mods(app, mods);
    // Ruler, Count and Note tools.
    if crate::analysis_ui::pointer(app, ev, mods) {
        return;
    }
    // Slice and Slice Select tools.
    if crate::slice_ui::pointer(app, ev, mods) {
        return;
    }
    let tool = app.ui.tool;
    // Move tool over a guide drags the guide (off the canvas deletes it).
    match ev {
        ToolEvent::Down { x, y, .. } if tool == Tool::Move => {
            if let Some((vertical, i)) = crate::rulers::guide_at(app, x, y) {
                app.guide_drag = Some(crate::rulers::GuideDrag { vertical, index: Some(i), pos: if vertical { x } else { y } });
                return;
            }
            // Auto-Select (or ⌘-click while it is off) picks the layer under the pointer first.
            if app.ui.tool_options.move_auto_select != mods.command {
                let target = app.ui.tool_options.move_target.clone();
                let mode = if mods.shift { "add" } else { "replace" };
                let _ = app.run("layer.pickAt", json!({"x": x, "y": y, "target": target, "mode": mode}));
            }
        }
        ToolEvent::Move { x, y, .. } => {
            if let Some(d) = app.guide_drag.as_mut().filter(|d| d.index.is_some()) {
                d.pos = if d.vertical { x } else { y };
                return;
            }
        }
        ToolEvent::Up { x, y } => {
            if let Some(mut d) = app.guide_drag.filter(|d| d.index.is_some()) {
                app.guide_drag = None;
                d.pos = if d.vertical { x } else { y };
                crate::rulers::finish_drag(app, d);
                return;
            }
        }
        _ => {}
    }
    match ev {
        ToolEvent::Down { x, y, pressure } => {
            if tool == Tool::Eyedropper {
                if let Ok(v) = app.run("document.pixel", json!({"x": x.floor(), "y": y.floor()})) {
                    let c: Vec<f32> = serde_json::from_value(v).unwrap_or_default();
                    if c.len() == 4 && c[3] > 0.0 {
                        let key = if mods.alt { "background" } else { "foreground" };
                        let _ = app.run("tools.setColors", json!({ key: [c[0], c[1], c[2], 1.0] }));
                    }
                }
                return;
            }
            match tool {
                Tool::Pen => {
                    crate::vector_ui::pen_down(app, x, y);
                    return;
                }
                Tool::CloneStamp | Tool::Healing if mods.alt => {
                    crate::retouch_ui::set_source(app, x, y);
                    return;
                }
                Tool::MagicWand => {
                    let o = app.ui.tool_options.clone();
                    let mode = selection_mode(app, mods);
                    let _ = app.run("select.magicWand", json!({"x": x.floor(), "y": y.floor(), "tolerance": o.tolerance, "contiguous": o.contiguous, "antiAlias": o.anti_alias, "sampleAllLayers": o.sample_all_layers, "mode": mode}));
                    return;
                }
                Tool::PaintBucket => {
                    let o = app.ui.tool_options.clone();
                    let contents = if o.bucket_fill_pattern { "pattern" } else { "foreground" };
                    let _ = app.run("paint.bucket", json!({"x": x.floor(), "y": y.floor(), "tolerance": o.tolerance, "contiguous": o.contiguous, "antiAlias": o.anti_alias, "opacity": o.fill_opacity, "contents": contents, "target": paint_target(app)}));
                    return;
                }
                Tool::PolygonLasso => {
                    // Click adds a vertex; clicking near the first vertex closes the polygon.
                    let close = app.ui.polygon.first().is_some_and(|p0| {
                        app.ui.polygon.len() >= 3 && ((p0[0] - x).powi(2) + (p0[1] - y).powi(2)).sqrt() < 8.0 / app.current_zoom().max(0.01) as f64
                    });
                    if close {
                        commit_polygon(app, mods);
                    } else {
                        app.ui.polygon.push([x, y]);
                    }
                    return;
                }
                Tool::Type if crate::type_tool::pointer_down(app, x, y, mods.shift) => return,
                _ => {}
            }
            app.drag = Some(Drag { tool, start: [x, y], points: vec![[x, y, pressure as f64]], modifiers: mods });
        }
        ToolEvent::Move { x, y, pressure } => {
            if tool == Tool::Type && app.drag.is_none() {
                crate::type_tool::pointer_move(app, x, y);
            }
            if tool == Tool::Pen {
                crate::vector_ui::pen_move(app, x, y);
            }
            if let Some(d) = &mut app.drag
                && d.points.last().is_none_or(|p| (p[0] - x).abs() + (p[1] - y).abs() > 0.25)
            {
                d.points.push([x, y, pressure as f64]);
            }
        }
        ToolEvent::Up { x, y } => {
            if tool == Tool::Type
                && let Some(e) = app.ui.text_edit.as_mut()
            {
                e.dragging = false;
            }
            if tool == Tool::Pen {
                crate::vector_ui::pen_up(app);
            }
            let Some(mut d) = app.drag.take() else { return };
            if d.points.last().is_none_or(|p| p[0] != x || p[1] != y) {
                d.points.push([x, y, d.points.last().map_or(1.0, |p| p[2])]);
            }
            finish_gesture(app, d);
        }
    }
}

fn finish_gesture(app: &mut PhotocraftApp, d: Drag) {
    let end = d.points.last().copied().unwrap_or([d.start[0], d.start[1], 1.0]);
    if crate::retouch_ui::finish_stroke(app, d.tool, &d.points, d.modifiers) {
        return;
    }
    match d.tool {
        Tool::ObjectSelection => crate::retouch_ui::finish_object_selection(app, d.start, [end[0], end[1]], d.modifiers),
        t if crate::vector_ui::is_shape_tool(t) => crate::vector_ui::finish_shape(app, t, d.start, [end[0], end[1]], d.modifiers),
        Tool::PathSelection => crate::vector_ui::path_selection_finish(app, d.start, [end[0], end[1]]),
        Tool::Type => crate::type_tool::pointer_up(app, d.start, [end[0], end[1]]),
        Tool::Brush | Tool::Eraser => {
            let pts: Vec<[f64; 3]> = d.points.clone();
            let _ = app.run("paint.stroke", json!({ "points": pts, "erase": d.tool == Tool::Eraser, "smoothing": 0.3, "target": paint_target(app) }));
        }
        Tool::RectMarquee | Tool::EllipseMarquee => {
            let o = &app.ui.tool_options;
            let e = crate::chrome_ui::marquee_end(&o.marquee_style, o.marquee_width as f64, o.marquee_height as f64, false, d.start, [end[0], end[1]]);
            let end = [e[0], e[1], 1.0];
            let (x0, y0) = (d.start[0].min(end[0]).floor(), d.start[1].min(end[1]).floor());
            let (x1, y1) = (d.start[0].max(end[0]).ceil(), d.start[1].max(end[1]).ceil());
            if x1 - x0 < 2.0 || y1 - y0 < 2.0 {
                if app.session.is_enabled("select.deselect") {
                    let _ = app.run("select.deselect", json!({}));
                }
                return;
            }
            let bar = ["replace", "add", "subtract", "intersect"][app.ui.selection_mode.min(3) as usize];
            let mode = if !d.modifiers.shift && !d.modifiers.alt {
                bar
            } else if d.modifiers.shift && d.modifiers.alt {
                "intersect"
            } else if d.modifiers.shift {
                "add"
            } else if d.modifiers.alt {
                "subtract"
            } else {
                "replace"
            };
            let (aa, feather) = (app.ui.tool_options.anti_alias, app.ui.tool_options.feather);
            let _ = app.run("select.rect", json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0, "mode": mode, "ellipse": d.tool == Tool::EllipseMarquee, "antiAlias": aa, "feather": feather}));
        }
        Tool::Lasso => {
            let pts: Vec<[f64; 2]> = d.points.iter().map(|p| [p[0], p[1]]).collect();
            if pts.len() >= 3 {
                let mode = selection_mode(app, d.modifiers);
                let _ = app.run("select.lasso", json!({"points": pts, "mode": mode, "antiAlias": app.ui.tool_options.anti_alias}));
            } else if app.session.is_enabled("select.deselect") {
                let _ = app.run("select.deselect", json!({}));
            }
        }
        Tool::Gradient => {
            if (end[0] - d.start[0]).abs() + (end[1] - d.start[1]).abs() >= 2.0 {
                let o = app.ui.tool_options.clone();
                let fg = app.session.tools.foreground;
                let bg = app.session.tools.background;
                let _ = app.run(
                    "paint.gradient",
                    json!({"from": [d.start[0], d.start[1]], "to": [end[0], end[1]], "style": o.gradient_style, "reverse": o.gradient_reverse, "dither": o.gradient_dither, "colors": [hex(fg), hex(bg)], "opacity": o.fill_opacity, "target": paint_target(app)}),
                );
            }
        }
        Tool::Crop => {
            let end = crop_end(app, d.start, [end[0], end[1]]);
            let r = [d.start[0].min(end[0]), d.start[1].min(end[1]), d.start[0].max(end[0]), d.start[1].max(end[1])];
            if r[2] - r[0] >= 2.0 && r[3] - r[1] >= 2.0 {
                app.ui.crop_rect = Some(r);
            }
        }
        Tool::Move => {
            let (dx, dy) = ((end[0] - d.start[0]).round(), (end[1] - d.start[1]).round());
            if dx != 0.0 || dy != 0.0 {
                let _ = app.run("layer.translate", json!({"dx": dx, "dy": dy}));
            }
        }
        _ => {}
    }
}

/// Extra OS windows showing documents (multi-window / multi-monitor).
pub fn extra_windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let wins = app.ui.windows.clone();
    for w in wins.into_iter().filter(|w| w.open) {
        let Some(st) = app.session.documents().get(w.document) else { continue };
        let title = format!("{} — window {}", st.doc.name, w.id);
        let vid = egui::ViewportId::from_hash_of(("docwin", w.id));
        let builder = egui::ViewportBuilder::default().with_title(title).with_inner_size([800.0, 600.0]);
        let mut view = w.view.clone();
        let mut close = false;
        ctx.show_viewport_immediate(vid, builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(crate::theme::Tokens::get(ui.ctx()).canvas)).show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                view = canvas_view(app, ui, w.document, rect, view.clone(), false);
            });
        });
        if let Some(win) = app.ui.windows.iter_mut().find(|x| x.id == w.id) {
            win.view = view;
            if close {
                win.open = false;
            }
        }
    }
    app.ui.windows.retain(|w| w.open);
}

/// Selection mode from the options bar, overridden by modifier keys (⇧ add, ⌥ subtract, ⇧⌥ intersect).
fn selection_mode(app: &PhotocraftApp, m: egui::Modifiers) -> &'static str {
    if m.shift && m.alt {
        "intersect"
    } else if m.shift {
        "add"
    } else if m.alt {
        "subtract"
    } else {
        ["replace", "add", "subtract", "intersect"][app.ui.selection_mode.min(3) as usize]
    }
}

/// Close the polygonal lasso and make the selection.
pub fn commit_polygon(app: &mut PhotocraftApp, mods: egui::Modifiers) {
    let pts = std::mem::take(&mut app.ui.polygon);
    if pts.len() >= 3 {
        let mode = selection_mode(app, mods);
        let _ = app.run("select.lasso", json!({"points": pts, "mode": mode, "antiAlias": app.ui.tool_options.anti_alias}));
    }
}

/// Apply the crop tool's rectangle.
/// Crop drag end under the options-bar aspect ratio.
fn crop_end(app: &PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> [f64; 2] {
    let size = app.session.active().map_or((1.0, 1.0), |s| (s.doc.size.width as f64, s.doc.size.height as f64));
    match crate::chrome_ui::crop_ratio(&app.ui.tool_options.crop_ratio, size.0, size.1) {
        Some((w, h)) => crate::chrome_ui::marquee_end("fixedRatio", w, h, false, start, end),
        None => end,
    }
}

pub fn commit_crop(app: &mut PhotocraftApp) {
    let Some(r) = app.ui.crop_rect.take() else { return };
    let (x, y) = (r[0].round(), r[1].round());
    let (w, h) = ((r[2] - r[0]).round().max(1.0), (r[3] - r[1]).round().max(1.0));
    let delete = app.ui.tool_options.crop_delete;
    if app.run("image.crop", json!({"x": x, "y": y, "width": w, "height": h, "deleteCroppedPixels": delete})).is_ok()
        && let Some(i) = app.session.active_index()
    {
        app.ui.views[i].fit_pending = true;
    }
}

/// "mask" when the Layers panel targets the active layer's mask, else "pixels".
pub fn paint_target(app: &PhotocraftApp) -> serde_json::Value {
    use photocraft_engine::channel_cmds::ChannelTarget;
    let Some(st) = app.session.active() else { return json!("pixels") };
    // A targeted alpha channel (Channels panel) or Quick Mask mode wins over the layer.
    match st.channel_view.target {
        ChannelTarget::Alpha(i) if i < st.doc.channels.len() => return json!({ "channel": i }),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => return json!("quickMask"),
        _ => {}
    }
    let has_mask = st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.mask.is_some());
    json!(if app.ui.mask_target && has_mask { "mask" } else { "pixels" })
}

/// `#rrggbb` for an sRGB colour (the engine's colour parameter notation).
fn hex(c: [f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_transform_roundtrip() {
        for flip in [false, true] {
            let xf = ViewXform { rect: Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0)), zoom: 2.5, center: [320.0, 240.0], flip };
            let s = xf.to_screen(10.0, 20.0);
            let d = xf.to_doc(s);
            assert!((d[0] - 10.0).abs() < 1e-3 && (d[1] - 20.0).abs() < 1e-3);
            assert_eq!(xf.to_screen(320.0, 240.0), xf.rect.center());
            // Flipped, document x grows to the left.
            assert_eq!(xf.to_screen(330.0, 240.0).x > xf.rect.center().x, !flip);
            let r = xf.doc_rect(DRect::new(0, 0, 10, 10));
            assert!(r.width() > 0.0 && r.height() > 0.0);
        }
    }

    #[test]
    fn layer_style_dialog_previews_live_and_cancel_restores() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let fx = |d: &Document| d.layers.iter().map(|l| l.effects.items.len()).sum::<usize>();
        let id = crate::layer_style::open(&mut app, Some("colorOverlay")).unwrap();
        let (shown, key) = display_doc(&mut app, 0);
        assert_eq!((fx(&shown), fx(&app.session.documents()[0].doc)), (1, 0), "previewed, not committed");
        app.ui.dialog_mut(id).unwrap().fields.insert("on:stroke".into(), json!(true));
        let (shown, key2) = display_doc(&mut app, 0);
        assert_eq!(fx(&shown), 2);
        assert_ne!(key, key2, "an edit re-renders the canvas");
        app.ui.close_dialog(id);
        let (shown, key) = display_doc(&mut app, 0);
        assert_eq!((fx(&shown), key), (0, 0));
        assert!(app.style_preview.is_none());
    }

    #[test]
    fn zoom_steps_monotone() {
        assert_eq!(zoom_step(1.0, 1), 2.0);
        assert_eq!(zoom_step(1.0, -1), 0.6667);
        assert_eq!(zoom_step(0.4, 1), 0.5);
        assert_eq!(zoom_step(32.0, 1), 32.0);
    }

    #[test]
    fn downsample_averages_premultiplied() {
        let mut b = photocraft_compose::Buffer::transparent(DRect::new(0, 0, 2, 2));
        b.px[0] = [1.0, 0.0, 0.0, 1.0];
        let d = downsample(&b, 2);
        assert_eq!(d.px.len(), 1);
        let p = d.px[0];
        assert!((p[0] - 1.0).abs() < 1e-6 && (p[3] - 0.25).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn damage_grows_by_nested_effect_reach() {
        use photocraft_doc::{Effect, Layer};
        let fmt = photocraft_color::PixelFormat::RGBA8;
        assert_eq!(effect_reach(&[Layer::raster("plain", fmt)]), 0);
        let mut inner = Layer::raster("inner", fmt);
        inner.effects.items.push(Effect::default_drop_shadow());
        let m = photocraft_compose::effects::margin(&inner);
        let mut group = Layer::group("g", vec![inner.clone()]);
        group.effects.items.push(Effect::default_drop_shadow());
        assert_eq!(effect_reach(std::slice::from_ref(&inner)), m);
        // A child's edit moves the group's shape, whose effects reach further.
        assert_eq!(effect_reach(&[group.clone()]), 2 * m);
        group.visible = false;
        assert_eq!(effect_reach(&[group]), 0);
    }
}
