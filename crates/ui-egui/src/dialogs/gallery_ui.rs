//! Filter › Filter Gallery…: the framework's Filter Gallery workspace on the app's dialog host.
//!
//! The workspace owns the layout, the effect-layer stack, its history and the zoom. This module
//! gives it what only the app knows: the filter catalog ([`PcCatalog`]), the preview and
//! thumbnail renderers ([`PcGallerySource`]) and what OK runs ([`take`]). The preview shows what
//! OK applies at the preview's zoom: the stack runs at full resolution on the visible part of
//! the layer (plus the filters' reach), and is then averaged down to the zoom (#2502). OK runs
//! `filter.filterGallery {effects:[...]}`, one history step (`layer.smartFilter.setParams` for a
//! smart filter on a smart object). Automation drives the open workspace through
//! `control::dialogs::workspace_ui`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft::behavior::PixelSize;
use craft::imaging::ui::filter_gallery::{
    EffectLayer, FilterCatalog, FilterCategory, FilterGalleryEvent, FilterGalleryOpen, FilterGallerySession, FilterGallerySource, FilterGalleryWorkspace,
    GalleryFilter as CatalogFilter, GalleryThumb,
};
use craft::imaging::ui::wide_angle::PreviewDone;
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::dialog::HostedWorkspace;
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::patterns::form_dialogs::{Param, ParamKind};
use craft::ui::primitives::textures::{Revision, ThumbRender, ThumbRequest, ThumbSource};
use craft::ui::runtime::pending::Job;
use egui::Color32;
use photocraft_algo::{FilterParams, GALLERY_CATEGORIES, GalleryFilter};
use photocraft_doc::{DocId, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const THUMB: [usize; 2] = [80, 56];
const KEY: &str = "filter.filterGallery";

/// A picture a job writes and the workspace reads as a [`RasterSource`]. Clones share one picture.
#[derive(Clone)]
pub(crate) struct SharedPicture(Arc<PictureInner>);

struct PictureInner {
    size: PixelSize,
    state: Mutex<PictureState>,
    revision: AtomicU64,
}

/// The pixels last written: `image` shows the document rectangle starting at `origin`, one texel per `factor` pixels.
struct PictureState {
    image: egui::ColorImage,
    factor: u32,
    origin: [i32; 2],
}

impl SharedPicture {
    /// An empty picture of the document's `size`.
    pub(crate) fn new(size: PixelSize) -> Self {
        let state = PictureState { image: egui::ColorImage::new([0, 0], Vec::new()), factor: 1, origin: [0, 0] };
        SharedPicture(Arc::new(PictureInner { size, state: Mutex::new(state), revision: AtomicU64::new(0) }))
    }

    /// Replaces the picture: `image` covers the document pixels from `origin`, `factor` pixels per texel.
    pub(crate) fn replace(&self, image: egui::ColorImage, factor: u32, origin: [i32; 2]) {
        if let Ok(mut state) = self.0.state.lock() {
            *state = PictureState { image, factor: factor.max(1), origin };
        }
        self.0.revision.fetch_add(1, Ordering::AcqRel);
    }
}

impl RasterSource for SharedPicture {
    fn size(&self) -> PixelSize {
        self.0.size
    }

    fn revision(&self) -> Revision {
        Revision::new(self.0.revision.load(Ordering::Acquire))
    }

    /// A new picture replaces everything, so the display reads it all.
    fn damage(&self, _since: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut egui::ColorImage) {
        let Ok(state) = self.0.state.lock() else { return };
        let [ow, oh] = out.size;
        let [iw, ih] = state.image.size;
        let (sf, factor) = (i64::from(state.factor), i64::from(factor.max(1)));
        for y in 0..oh {
            let sy = (i64::from(region.y0) + y as i64 * factor - i64::from(state.origin[1])).div_euclid(sf);
            for x in 0..ow {
                let sx = (i64::from(region.x0) + x as i64 * factor - i64::from(state.origin[0])).div_euclid(sf);
                let inside = sx >= 0 && sy >= 0 && (sx as usize) < iw && (sy as usize) < ih;
                out.pixels[y * ow + x] = if inside { state.image.pixels[sy as usize * iw + sx as usize] } else { Color32::TRANSPARENT };
            }
        }
    }
}

/// The hosted Filter Gallery dialog.
pub(crate) type Hosted = HostedWorkspace<FilterGalleryWorkspace>;

/// The engine's gallery filters as the workspace's catalog.
pub(crate) struct PcCatalog;

impl FilterCatalog for PcCatalog {
    fn categories(&self) -> Vec<FilterCategory> {
        GALLERY_CATEGORIES
            .iter()
            .map(|cat| {
                let keys = GalleryFilter::ALL.iter().filter(|f| f.category() == *cat).map(|f| f.key().to_string()).collect();
                FilterCategory::new(*cat, tl!(*cat).to_string(), keys)
            })
            .collect()
    }

    fn filter(&self, key: &str) -> Option<CatalogFilter> {
        let f = GalleryFilter::from_key(key)?;
        let mut params = Param::parse_list(f.params_doc());
        // The registry notation leaves colour values as `json`, which the parser drops; the effect's own colour is a
        // swatch row (the foreground and background are the session's, set by `uses_colors`).
        if f.params_doc().contains("\"glowColor\":json") {
            params.push(Param::new("glowColor", tl!("Glow Color").to_string(), ParamKind::color(None)));
        }
        Some(CatalogFilter::new(f.key(), tl!(f.name()).to_string(), f.category(), params).uses_colors(f.uses_colours()))
    }
}

/// The layer under the effects: the pixels the preview and thumbnails render from, and the
/// picture the render job writes.
pub(crate) struct PcGallerySource {
    src: Arc<Surface>,
    canvas: Rect,
    target: Value,
    foreground: [f32; 4],
    background: [f32; 4],
    /// The middle of the layer, about 4× the thumbnail size, sampled down to the thumbnail size.
    thumb_src: Surface,
    catalog: PcCatalog,
    preview: SharedPicture,
}

impl PcGallerySource {
    fn new(src: Surface, canvas: Rect, target: Value, colours: [[f32; 4]; 2]) -> Self {
        let cb = src.content_bounds().intersect(&canvas);
        let cb = if cb.is_empty() { canvas } else { cb };
        let k = ((cb.width() as usize / (THUMB[0] * 4)).min(cb.height() as usize / (THUMB[1] * 4))).max(1) as u32;
        let (tw, th) = (THUMB[0] as i32 * k as i32, THUMB[1] as i32 * k as i32);
        let (cx, cy) = ((cb.x0 + cb.x1) / 2, (cb.y0 + cb.y1) / 2);
        let thumb_src = sample(&src, Rect::new(cx - tw / 2, cy - th / 2, cx - tw / 2 + tw, cy - th / 2 + th), k);
        let preview = SharedPicture::new(PixelSize::new(canvas.width().max(1), canvas.height().max(1)));
        PcGallerySource { src: Arc::new(src), canvas, target, foreground: colours[0], background: colours[1], thumb_src, catalog: PcCatalog, preview }
    }

    /// The command params for `effects`: what OK runs and what the preview renders.
    pub(crate) fn command_params(&self, effects: &[EffectLayer]) -> Value {
        let mut effects = serde_json::to_value(effects).unwrap_or(Value::Null);
        map_glow_colors(&mut effects, egui::ecolor::linear_from_gamma);
        let p = json!({
            "effects": effects,
            "foreground": self.foreground,
            "background": self.background,
        });
        crate::distort_ui::with_filter_target(p, &self.target)
    }
}

/// Renders `vis` at 1/`k` into `picture` and returns the time it took; the render job's body.
fn paint_preview(src: &Surface, canvas: Rect, params: &Value, vis: Rect, k: u32, picture: &SharedPicture, ctl: &Interrupt) -> Result<Duration, String> {
    let t0 = crate::gpu_canvas::now_ms();
    let out = preview_pixels(src, canvas, params, vis, k, ctl).ok_or_else(|| "the Filter Gallery preview was cancelled".to_string())?;
    picture.replace(image(&out, reduced(vis.width(), k), reduced(vis.height(), k)), k, [vis.x0, vis.y0]);
    Ok(Duration::from_secs_f64(((crate::gpu_canvas::now_ms() - t0) / 1000.0).max(0.0)))
}

/// The factor the preview is averaged down by at `zoom` (screen px per document px): at least one
/// texture pixel per screen pixel; the display's linear filter does the rest.
pub(crate) fn preview_factor(zoom: f64) -> u32 {
    if zoom.is_finite() && zoom > 0.0 { (1.0 / zoom + 1e-3).floor().clamp(1.0, 4096.0) as u32 } else { 1 }
}

impl FilterGallerySource for PcGallerySource {
    fn catalog(&self) -> &dyn FilterCatalog {
        &self.catalog
    }

    fn preview(&self) -> &dyn RasterSource {
        &self.preview
    }

    fn render(&self, effects: &[EffectLayer], visible: PixelRect, zoom: f64) -> Job<PreviewDone, String> {
        let vis = Rect::new(visible.x0, visible.y0, visible.x1, visible.y1).intersect(&self.canvas);
        if vis.is_empty() {
            return Job::new(|_, _| Ok(PreviewDone::new(Duration::ZERO)));
        }
        let k = preview_factor(zoom);
        let params = self.command_params(effects);
        let (src, canvas, picture) = (self.src.clone(), self.canvas, self.preview.clone());
        Job::new(move |progress, cancel| {
            let cancelled = || cancel.is_cancelled();
            let report = |fraction: f32| progress.report(fraction);
            let ctl = Interrupt::new(&cancelled, &report);
            paint_preview(&src, canvas, &params, vis, k, &picture, &ctl).map(PreviewDone::new)
        })
    }

    fn thumb_source(&self) -> &dyn ThumbSource<GalleryThumb> {
        self
    }
}

impl ThumbSource<GalleryThumb> for PcGallerySource {
    /// The crop and colours are fixed while the gallery is open, so a filter's thumbnail never changes.
    fn revision(&self, key: &GalleryThumb, _request: &ThumbRequest) -> Revision {
        Revision::of(&key.filter)
    }

    fn render(&self, key: &GalleryThumb, _request: &ThumbRequest) -> ThumbRender {
        let p = crate::distort_ui::with_filter_target(
            json!({"effects": [{"filter": key.filter}], "foreground": self.foreground, "background": self.background}),
            &self.target,
        );
        let out = render(&self.thumb_src, &p);
        let b = self.thumb_src.content_bounds();
        ThumbRender::Ready(image(&out, b.x1.max(1) as usize, b.y1.max(1) as usize))
    }
}

/// What the app keeps of the open Filter Gallery: the dialog's id and what OK needs to run.
pub(crate) struct GalleryApp {
    id: DialogId,
    document: DocId,
    layer: LayerId,
    /// Editing smart filter `index` of the layer: OK updates it instead of adding a filter.
    smart_filter: Option<usize>,
    source: Arc<PcGallerySource>,
}

/// The open gallery's fields for automation, with the document, layer and smart filter it edits.
pub(crate) fn describe(app: &PhotocraftApp) -> Option<Value> {
    let g = app.gallery.as_ref()?;
    let mut v: Value = crate::distort_ui::workspace_fields(app, KEY).unwrap_or_else(|| json!({}));
    if let Some(o) = v.as_object_mut() {
        o.insert("document".into(), json!(g.document.0));
        o.insert("layer".into(), json!(g.layer.0));
        o.insert("smartFilter".into(), json!(g.smart_filter));
    }
    Some(v)
}

/// Acts on what the gallery reported this frame: opens the colour picker for a swatch, hands the
/// picked colour back, and runs OK's stack. Takes only what is queued now, so a second pass in a
/// frame finds nothing left to repeat.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(id) = app.gallery.as_ref().map(|g| g.id) else { return };

    let events = app.framework.dialogs.get_mut_by_id::<Hosted>(id).map(|h| h.take_events()).unwrap_or_default();
    for event in events {
        if let FilterGalleryEvent::PickColor(request) = event {
            crate::color_picker_ui::open_for_request(app, id, request);
        }
    }
    if let Some((key, color)) = crate::color_picker_ui::answer_for(app, id)
        && let Some(h) = app.framework.dialogs.get_mut_by_id::<Hosted>(id)
    {
        h.workspace_mut().set_color(&key, color);
    }
    let Some(result) = app.framework.dialogs.take_by_id::<Hosted>(id) else { return };
    let Some(g) = app.gallery.take() else { return };
    let DialogResult::Ok(session) = result else { return };
    if !session.effects.iter().any(|e| e.visible) {
        return;
    }
    let params = g.source.command_params(&session.effects);
    let run = if let Some(index) = g.smart_filter {
        app.run("layer.smartFilter.setParams", json!({"layer": g.layer.0, "index": index, "params": params}))
    } else {
        let mut p = params;
        p["layer"] = json!(g.layer.0);
        app.run("filter.filterGallery", p)
    };
    if let Err(e) = run {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// A nearest-neighbour copy of `r` of `surf`, every `k`-th pixel, placed at the origin.
fn sample(surf: &Surface, r: Rect, k: u32) -> Surface {
    let k = k.max(1) as i32;
    let (w, h) = ((r.width() as i32 + k - 1) / k, (r.height() as i32 + k - 1) / k);
    let mut out = Surface::new(surf.format());
    if w <= 0 || h <= 0 {
        return out;
    }
    let n = surf.channels();
    let mut data = Vec::with_capacity((w * h) as usize * n);
    for j in 0..h {
        let y = r.y0 + j * k;
        let row = surf.read_region(Rect::new(r.x0, y, r.x1, y + 1));
        for i in 0..w {
            let o = (i * k) as usize * n;
            data.extend_from_slice(&row[o..o + n]);
        }
    }
    out.write_region(Rect::new(0, 0, w, h), &data);
    out
}

/// Runs a stack on a small surface (the thumbnails).
fn render(src: &Surface, params: &Value) -> Surface {
    let effects = photocraft_engine::gallery_cmds::effects_from_json(params).unwrap_or_default();
    let b = src.content_bounds().union(&Rect::new(0, 0, 1, 1));
    let b = Rect::new(0, 0, b.x1, b.y1);
    let mut out = photocraft_algo::apply_in(src, &FilterParams::FilterGallery { effects }, b, b, None, b);
    if let Ok(Some(colors)) = photocraft_engine::color_channels::explicit(params, src.format().mode.color_channels())
        && photocraft_engine::color_channels::restrict_surface(src, &mut out, colors).is_err()
    {
        return src.clone();
    }
    out
}

/// The preview of `vis` at 1/`k` (#2502): the stack runs at full resolution exactly as OK runs it
/// (same bounds, same canvas-edge handling, textures and noise seeded by document position), so
/// the pixels equal the same crop of the applied result; then `k`×`k` blocks are averaged.
/// `None` when cancelled.
fn preview_pixels(src: &Surface, canvas: Rect, params: &Value, vis: Rect, k: u32, ctl: &Interrupt) -> Option<Surface> {
    let effects = photocraft_engine::gallery_cmds::effects_from_json(params).unwrap_or_default();
    let fp = FilterParams::FilterGallery { effects };
    let content = src.content_bounds();
    let area = photocraft_algo::output_area(&fp, content, canvas, None).intersect(&vis);
    let mut full = if area.is_empty() { src.clone() } else { photocraft_algo::apply_in_with(src, &fp, area, canvas, None, canvas.union(&content), ctl)? };
    if let Some(colors) = photocraft_engine::color_channels::explicit(params, src.format().mode.color_channels()).ok()? {
        photocraft_engine::color_channels::restrict_region(src, &mut full, colors, area).ok()?;
    }
    Some(reduce(&full, vis, k))
}

/// `r` of `surf` scaled down by `k`, each pixel the average of a `k`×`k` block (colour weighted by
/// alpha), placed at the origin.
pub(crate) fn reduce(surf: &Surface, r: Rect, k: u32) -> Surface {
    let k = k.clamp(1, 1 << 16) as usize;
    let (bw, bh) = (r.width() as usize, r.height() as usize);
    let (w, h) = (bw.div_ceil(k), bh.div_ceil(k));
    let mut out = Surface::new(surf.format());
    if w == 0 || h == 0 {
        return out;
    }
    let n = surf.channels().max(1);
    let alpha = surf.format().alpha;
    let colours = if alpha { n - 1 } else { n };
    // One output row from a band of `k` source rows.
    let row = |j: usize, out: &mut [f32]| {
        let y0 = r.y0.saturating_add((j * k) as i32);
        let band = surf.read_region(Rect::new(r.x0, y0, r.x1, y0.saturating_add(k as i32).min(r.y1)));
        let mut acc = vec![0.0f64; w * n];
        let mut count = vec![0u32; w];
        for (i, px) in band.chunks_exact(n).enumerate() {
            let x = (i % bw) / k;
            let (Some(sum), Some(c)) = (acc.get_mut(x * n..x * n + n), count.get_mut(x)) else { continue };
            let a = if alpha { px[n - 1] as f64 } else { 1.0 };
            for (s, v) in sum.iter_mut().zip(&px[..colours]) {
                *s += *v as f64 * a;
            }
            if alpha {
                sum[n - 1] += a;
            }
            *c += 1;
        }
        for ((sum, c), o) in acc.chunks_exact(n).zip(&count).zip(out.chunks_exact_mut(n)) {
            let a = if alpha { sum[n - 1] } else { *c as f64 };
            for (o, v) in o.iter_mut().zip(&sum[..colours]) {
                *o = if a > 0.0 { (v / a) as f32 } else { 0.0 };
            }
            if alpha {
                o[n - 1] = (a / (*c).max(1) as f64) as f32;
            }
        }
    };
    let mut data = vec![0.0f32; w * h * n];
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        data.par_chunks_mut(w * n).enumerate().for_each(|(j, out)| row(j, out));
    }
    #[cfg(target_arch = "wasm32")]
    for (j, out) in data.chunks_mut(w * n).enumerate() {
        row(j, out);
    }
    out.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    out
}

/// A length of `n` document pixels reduced by `k` (partial blocks count).
fn reduced(n: u32, k: u32) -> usize {
    (n as usize).div_ceil(k.max(1) as usize)
}

/// The top-left `w` × `h` pixels of `surf` as a display image (also the Wide Angle preview's).
pub(crate) fn image(surf: &Surface, w: usize, h: usize) -> egui::ColorImage {
    let mut px = vec![[0u8; 4]; w * h];
    for y in 0..h {
        surf.read_rgba8_into(Rect::new(0, y as i32, w as i32, y as i32 + 1), &mut px[y * w..(y + 1) * w]);
    }
    let alpha = surf.format().alpha;
    egui::ColorImage::new([w, h], px.iter().map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], if alpha { p[3] } else { 255 })).collect())
}

/// The effect layers of a `filter.filterGallery` params object: unknown filters are skipped and
/// every effect starts from its filter's defaults under the recorded values.
fn effects_from(p: &Value, catalog: &PcCatalog) -> Vec<EffectLayer> {
    let mut effects = p.get("effects").cloned().unwrap_or(Value::Null);
    map_glow_colors(&mut effects, egui::ecolor::gamma_from_linear);
    effects
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let mut e = e.clone();
            e.as_object_mut()?.entry("visible").or_insert(Value::Bool(true));
            let mut layer = serde_json::from_value::<EffectLayer>(e).ok()?;
            let filter = catalog.filter(&layer.filter)?;
            let mut fields = filter.defaults();
            for (k, v) in layer.fields.iter() {
                fields.set(k, v.clone());
            }
            layer.fields = fields;
            Some(layer)
        })
        .collect()
}

/// Applies `convert` to the red, green and blue of every effect's `glowColor`. The command records it as
/// linear RGB; the dialog's swatch row holds encoded sRGB.
fn map_glow_colors(effects: &mut Value, convert: fn(f32) -> f32) {
    let colors = effects.as_array_mut().into_iter().flatten().filter_map(|e| e.get_mut("params")?.get_mut("glowColor")?.as_array_mut());
    for color in colors {
        for channel in color.iter_mut().take(3) {
            if let Some(v) = channel.as_f64() {
                *channel = json!(f64::from(convert(v as f32)));
            }
        }
    }
}

/// A recorded `[r, g, b, a]` colour param.
fn colour_param(p: &Value, key: &str) -> Option<[f32; 4]> {
    let a = p.get(key)?.as_array()?;
    let c = |j: usize, d: f32| a.get(j).and_then(Value::as_f64).map_or(Some(d), |v| Some(v as f32).filter(|v| v.is_finite()));
    Some([c(0, 0.0)?, c(1, 0.0)?, c(2, 0.0)?, c(3, 1.0)?])
}

/// Opens the dialog on the active layer, starting from the last gallery stack used.
pub fn open(app: &mut PhotocraftApp) -> Result<(), String> {
    let (layer, src, _) = crate::distort_ui::active_filter_target(app)?;
    let last = app.session.journal.iter().rev().find(|(id, _)| id == KEY).map(|(_, p)| p.clone());
    let effects = last.as_ref().map(|p| effects_from(p, &PcCatalog)).unwrap_or_default();
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    open_on(app, layer, src, effects, [fg, bg], None)
}

/// Re-opens Filter Gallery smart filter `index` of smart object `layer` with its recorded stack,
/// previewing on the pixels below it; OK updates that filter (`layer.smartFilter.setParams`).
pub fn open_smart_filter(app: &mut PhotocraftApp, layer: LayerId, index: usize) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else {
        return Err("the layer is not a smart object".into());
    };
    let filter = sm.smart_filters.get(index).ok_or("no such smart filter")?;
    if filter.command != KEY {
        return Err("the smart filter is not a Filter Gallery filter".into());
    }
    let p = filter.params.clone();
    let src = photocraft_engine::smart_cmds::render_below_filter(&st.doc, sm, index)
        .map_err(|e| e.to_string())?
        .ok_or("the smart object's contents are unavailable (missing linked file?)")?;
    let fg = colour_param(&p, "foreground").unwrap_or(app.session.tools.foreground);
    let bg = colour_param(&p, "background").unwrap_or(app.session.tools.background);
    open_on(app, layer, src, effects_from(&p, &PcCatalog), [fg, bg], Some(index))
}

fn open_on(
    app: &mut PhotocraftApp,
    layer: LayerId,
    src: Surface,
    effects: Vec<EffectLayer>,
    colours: [[f32; 4]; 2],
    smart_filter: Option<usize>,
) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let canvas = st.doc.bounds();
    let document = st.doc.id;
    let layer_name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let target = if smart_filter.is_some() { json!("pixels") } else { crate::distort_ui::colour_set_target(app) };
    let mut session = FilterGallerySession::new(GalleryFilter::ColoredPencil.key(), &PcCatalog);
    if !effects.is_empty() {
        session.effects = effects;
    }
    let size = PixelSize::new(canvas.width().max(1), canvas.height().max(1));
    let source = Arc::new(PcGallerySource::new(src, canvas, target, colours));
    let workspace = FilterGalleryWorkspace::new(FilterGalleryOpen::new(size, layer_name, session));

    let shared: Arc<dyn FilterGallerySource> = source.clone();
    let id = app.framework.dialogs.open(HostedWorkspace::new(workspace, shared));
    app.gallery = Some(GalleryApp { id, document, layer, smart_filter, source });
    Ok(())
}

#[cfg(test)]
mod tests;
