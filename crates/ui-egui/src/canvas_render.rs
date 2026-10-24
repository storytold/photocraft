//! Canvas texture rendering, caching, and preview updates.

use super::{live_stroke, ViewXform};
use crate::PhotocraftApp;
use egui::{Color32, Rect, TextureOptions, pos2};
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Rect as DRect;

/// Largest texture side we upload; bigger documents are uploaded downsampled, with the part in
/// view added at the screen's resolution when zoomed in (`detail_plan`).
pub const MAX_TEXTURE: u32 = 4096;

/// The largest texture side to upload on this device: [`MAX_TEXTURE`], or less where the GPU
/// allows less (2048 on some older or low-end GPUs and WebGL contexts). Past the device's limit
/// egui asserts and the backend fails to create the texture.
pub(super) fn texture_side(ctx: &egui::Context) -> u32 {
    clamp_texture_side(ctx.input(|i| i.max_texture_side))
}

/// [`texture_side`] for a device whose largest texture side is `max_texture_side`.
fn clamp_texture_side(max_texture_side: usize) -> u32 {
    u32::try_from(max_texture_side).unwrap_or(MAX_TEXTURE).clamp(1, MAX_TEXTURE)
}

/// How many document pixels per side one texel of the CPU canvas texture of a `size` document
/// holds when textures are at most `side` texels per side: 1 (full resolution) when it fits.
fn texture_factor(size: [u32; 2], side: u32) -> u32 {
    size[0].max(size[1]).div_ceil(side.max(1)).max(1)
}

/// What the CPU canvas draws over its reduced texture so a large document shows its own pixels
/// at 100 % (#7568): the part of the document in view (`region`, document pixels) composited at
/// one texel per `factor` pixels, a power of two no coarser than the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DetailPlan {
    pub factor: u32,
    pub region: DRect,
}

/// The detail region snaps outward to multiples of this many texels, so a small pan reuses it.
const DETAIL_QUANTUM: i32 = 256;

/// The detail texture a CPU canvas view of a `size` document needs, at `zoom` physical pixels
/// per document pixel over `visible` (document pixels), with textures at most `side` texels per
/// side: `None` when the whole-document texture is already as fine as the screen (a document
/// that fits, or a view zoomed out far enough), or nothing of the document is in view.
///
/// A document over `side` (4096 px, or the device's smaller limit) is uploaded reduced, so at
/// 100 % it was drawn magnified and blurred. The detail costs what the window shows, not what the
/// document holds: its side is capped by `side` like any texture, coarsening by powers of two.
fn detail_plan(size: [u32; 2], side: u32, zoom: f32, visible: DRect) -> Option<DetailPlan> {
    if !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    let base = texture_factor(size, side);
    let mut factor = crate::adjust_preview::proxy_factor(zoom);
    let bounds = DRect::new(0, 0, i32::try_from(size[0]).ok()?, i32::try_from(size[1]).ok()?);
    let v = visible.intersect(&bounds);
    if v.is_empty() {
        return None;
    }
    while factor < base {
        let q = DETAIL_QUANTUM.saturating_mul(i32::try_from(factor).unwrap_or(i32::MAX));
        let up = |x: i32| x.div_euclid(q).saturating_add(i32::from(x.rem_euclid(q) != 0)).saturating_mul(q);
        let region = DRect::new(v.x0.div_euclid(q).saturating_mul(q), v.y0.div_euclid(q).saturating_mul(q), up(v.x1), up(v.y1)).intersect(&bounds);
        // A reduced region's texel count can round one up (`render_reduced_damage`'s cells).
        let fits = |n: u32| n.div_ceil(factor).saturating_add(1) <= side;
        // Snapped when that fits, else just what is in view (a window near the device's limit).
        for region in [region, v] {
            if !region.is_empty() && fits(region.width()) && fits(region.height()) {
                return Some(DetailPlan { factor, region });
            }
        }
        factor = factor.saturating_mul(2);
    }
    None
}

pub(crate) struct ClonePreviewCache {
    pub(super) key: String,
    pub(super) texture: egui::TextureHandle,
}

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
    /// All uploaded texels are opaque. Screen coverage is checked separately before omitting
    /// the checker. Updated with uploads, never by scanning during an unchanged frame.
    pub texture_opaque: bool,
    /// The CPU path's pixel grid for the part of the document in view (pixel_grid.rs).
    pub grid: Option<crate::pixel_grid::GridLines>,
    /// The CPU path's sharper textures of the part in view, over a reduced `texture` (#7568),
    /// by `InView::key` (two windows of a document can show different parts).
    pub detail: Vec<(u64, DetailTexture)>,
}

/// [`CanvasCache::detail`]: the part of the document `DetailPlan` chose, uploaded at its factor.
pub struct DetailTexture {
    pub(super) texture: egui::TextureHandle,
    plan: DetailPlan,
    /// The texels it holds, in the plan's reduced pixels (`render_reduced_damage`'s output rect).
    out: DRect,
    /// Revision and preview key (display key folded in) it was rendered at.
    revision: u64,
    preview_key: u64,
    /// Pixels were composited for it this call (for the perf span).
    rendered: bool,
}

/// The CPU canvas textures to draw: the whole document, and over it the sharper part in view.
pub(crate) struct CpuTextures {
    pub texture: egui::TextureId,
    pub scale: f32,
    /// The detail texture and the document area it covers (x0, y0, x1, y1 in document pixels).
    pub detail: Option<(egui::TextureId, [f32; 4])>,
}

/// The part of a CPU canvas view in sight: its document area and zoom (physical pixels per
/// document pixel), for [`ensure_texture_in_view`]'s detail texture.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InView {
    pub visible: DRect,
    pub zoom: f32,
    /// Which view this is, so each keeps its own detail texture.
    pub key: u64,
}

/// Lattice size of the canvas display LUT (colour management, Proof Colors, Gamut Warning).
const DISPLAY_LUT: usize = 33;

/// Keep the GPU display LUT under `key` (the document's texture or a preview of it) in step with
/// `doc`'s colour management: document → monitor profile and View › Proof Colors / Gamut
/// Warning (the 32-bit preview is applied by the canvas shader, see the canvas HDR preview). Returns the canvas `display` mode (0 none — the identity, e.g. sRGB on
/// an sRGB monitor —, 1 LUT, 2 LUT + gamut warning, 3 matrix/shaper).
pub(super) fn sync_display_lut(app: &mut PhotocraftApp, doc: &photocraft_doc::Document, key: u64, display: Option<u32>) -> u8 {
    let Some(gpu) = app.gpu.clone() else { return 0 };
    let output = display.unwrap_or(0);
    // Rebuild only when anything feeding the LUT changes.
    let sig = app.session.color.display_signature_for(doc, display);
    if let Some((s, mode)) = gpu.display_lut_signature(key, output)
        && s == sig
    {
        return mode;
    }
    // Keep the curve/matrix/curve stages separate: interpolating a finished display
    // transform on a 33³ lattice crushes shadows when the monitor curve is steep near zero.
    if let Ok(Some(shaper)) = app.session.color.gpu_canvas_matrix_shaper_for(doc, display) {
        gpu.set_display_matrix_shaper(key, output, &shaper);
        gpu.cache_display_lut_signature(key, output, sig, 3);
        return 3;
    }
    let gamut = app.session.color.proof(doc.id).gamut_warning;
    let mode = match app.session.color.gpu_canvas_lut_for(doc, DISPLAY_LUT, display) {
        Ok(Some(bytes)) => {
            gpu.set_display_lut(key, output, DISPLAY_LUT as u32, Some(&bytes));
            if gamut { 2 } else { 1 }
        }
        Ok(None) => {
            gpu.set_display_lut(key, output, DISPLAY_LUT as u32, None);
            0
        }
        Err(e) => {
            let template = "Color management: {error}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e.to_string())]);
            gpu.set_display_lut(key, output, DISPLAY_LUT as u32, None);
            0
        }
    };
    gpu.cache_display_lut_signature(key, output, sig, mode);
    mode
}

pub(super) fn checker(app: &mut PhotocraftApp, ctx: &egui::Context) -> egui::TextureId {
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
    // `buf.to_rgba8()` then `color_image`, fused: one parallel pass, no intermediate image.
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let mut pixels = vec![egui::Color32::TRANSPARENT; buf.px.len()];
    let convert = |out: &mut [egui::Color32], src: &[[f32; 4]]| {
        for (o, p) in out.iter_mut().zip(src) {
            *o = egui::Color32::from_rgba_unmultiplied(q(p[0]), q(p[1]), q(p[2]), q(p[3]));
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        pixels.par_chunks_mut(16 * 1024).zip(buf.px.par_chunks(16 * 1024)).for_each(|(o, s)| convert(o, s));
    }
    #[cfg(target_arch = "wasm32")]
    convert(&mut pixels, &buf.px);
    egui::ColorImage::new([buf.rect.width() as usize, buf.rect.height() as usize], pixels)
}

/// `img` as an egui image: `ColorImage::from_rgba_unmultiplied`, in parallel off wasm. egui's own
/// conversion runs on one thread and took 37 ms of a 24 MP canvas or preview update, three times
/// the quantize before it.
fn color_image(img: &photocraft_raster::Rgba8Image) -> egui::ColorImage {
    let size = [img.width as usize, img.height as usize];
    let src = img.pixels.as_chunks::<4>().0;
    let mut pixels = vec![egui::Color32::TRANSPARENT; src.len()];
    let convert = |out: &mut [egui::Color32], src: &[[u8; 4]]| {
        for (o, &[r, g, b, a]) in out.iter_mut().zip(src) {
            *o = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        pixels.par_chunks_mut(16 * 1024).zip(src.par_chunks(16 * 1024)).for_each(|(o, s)| convert(o, s));
    }
    #[cfg(target_arch = "wasm32")]
    convert(&mut pixels, src);
    egui::ColorImage::new(size, pixels)
}

/// A composite as monitor pixels: document profile → monitor profile (nothing to do in the
/// common sRGB-on-sRGB case).
pub(crate) fn display_image(display: Option<&photocraft_engine::display_color::CanvasDisplay>, buf: &photocraft_compose::Buffer) -> egui::ColorImage {
    match display {
        Some(d) if !d.is_identity() => color_image(&d.to_rgba8(buf)),
        _ => buffer_to_image(buf),
    }
}

/// The canvas display of `doc` on `display` (`None`: the main window's) and a key that changes
/// with it (folded into the canvas caches' preview keys, so a monitor or profile change
/// re-renders).
pub(crate) fn canvas_display(
    app: &PhotocraftApp,
    doc: &Document,
    display: Option<u32>,
) -> (Option<std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>>, u64) {
    match app.session.color.canvas_display_for(doc, display.or(app.session.color.main_display)) {
        Ok(d) => {
            let k = d.key;
            (Some(d), k)
        }
        Err(_) => (None, 0),
    }
}

/// [`canvas_display`]'s key for the GPU canvas texture: what the texture stores, the same on
/// every display (the display LUT holds the monitor profile).
fn texture_key(display: Option<&photocraft_engine::display_color::CanvasDisplay>) -> u64 {
    display.map_or(0, |d| d.texture_key)
}

/// `app.canvases` key: CPU canvas textures are per document and display (they hold monitor
/// values); the GPU canvas state uses [`GPU_OUTPUT`].
pub(crate) fn cache_key(doc: photocraft_doc::DocId, display: Option<u32>) -> (photocraft_doc::DocId, u32) {
    (doc, display.unwrap_or(0))
}

/// `app.canvases` output of the GPU canvas state (one texture per document for all displays).
pub(crate) const GPU_OUTPUT: u32 = u32::MAX;

/// The document to render: the committed one, or a clone with the live adjustment preview applied.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> (std::sync::Arc<Document>, u64) {
    if let Some(shown) = crate::type_panels_ui::color_picker::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::solid_fill_ui::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::pattern_fill_ui::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::shape_fill_ui::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::type_transform::display_doc(app, idx) {
        return shown;
    }
    if let Some(shown) = crate::type_font_preview::display_doc(app, idx) {
        return shown;
    }
    // Puppet / Perspective Warp previews hide the layer they draw on a mesh.
    if let Some(shown) = crate::distort_ui::display_doc(app, idx) {
        return shown;
    }
    // Image › Adjustments dialog: a temporary adjustment layer clipped to the target.
    if let Some(shown) = crate::adjust_preview::display_doc(app, idx) {
        return shown;
    }
    // Gradient tool (live) drag, or a stop dragged in the Properties panel.
    if let Some(shown) = crate::gradient_ui::display_doc(app, idx) {
        return shown;
    }
    // Move tool drag: the moving layers at the pointer.
    if let Some(shown) = crate::move_ui::display_doc(app, idx) {
        return shown;
    }
    // Patch Tool drag: the selection healed from where the pointer is.
    if let Some(shown) = crate::patch_preview::display_doc(app, idx) {
        return shown;
    }
    // Content-Aware Move drag or box: the result, once computed in the background. It comes
    // before the reorder preview, which is built from the session document and would drop it.
    if let Some(shown) = crate::cam_preview::display_doc(app, idx) {
        return shown;
    }
    // Live Layers-panel reorder previews the actual composite before mouse release.
    if let Some(shown) = crate::layers_panel_ui::display_reorder(app, idx) {
        return shown;
    }
    // A blend mode hovered in the Layers panel.
    if let Some(shown) = crate::blend_preview::display_doc(app, idx) {
        return shown;
    }
    crate::transform_tool::follow_edits(app);
    let st = &app.session.documents()[idx];
    if let Some(l) = live_stroke(app, idx) {
        return (l.stroke.doc().clone(), l.display_key());
    }
    if let (Some(t), Some(pv)) = (&app.ui.transform, &app.transform_preview)
        && app.session.active_index() == Some(idx)
        && pv.doc.id == st.doc.id
        && st.doc.layer(photocraft_doc::LayerId(t.layer)).is_some()
    {
        return (pv.doc.clone(), (1 << 40) + pv.session);
    }
    // Layer Style dialog: show its effects live (Cancel just drops the preview).
    let style = app.ui.dialogs.iter().find(|d| d.kind == crate::state::DialogKind::LayerStyle);
    // The Preview checkbox off shows the document as it was when the dialog opened;
    // edits still land in the dialog state and OK applies them whatever it says.
    let preview_on = style.and_then(|d| d.fields.get("preview")).and_then(serde_json::Value::as_bool).unwrap_or(true);
    if style.is_none() || !preview_on {
        app.style_preview = None;
    }
    if let Some(d) = style.filter(|_| preview_on)
        && app.session.active_index() == Some(idx)
    {
        let fields = crate::layer_style::color_picker::preview_fields(app, d.id, &d.fields);
        let key = (crate::layer_style::preview_hash(&fields) ^ st.revision.wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 1 << 63;
        if app.style_preview.as_ref().map(|p| p.0) != Some(key) {
            let shown = crate::layer_style::preview_document(&st.doc, &app.session.patterns, &fields).map(std::sync::Arc::new);
            if let Err(error) = &shown {
                log::warn!("Layer Style preview: {error}");
            }
            app.style_preview = Some((key, shown));
        }
        if let Some((_, Ok(doc))) = &app.style_preview {
            return (doc.clone(), key);
        }
    }
    let revision = st.revision;
    let snapshot = st.doc.clone();
    if let Some((layer, params)) = app.live_adjust.clone()
        && let Some(l) = snapshot.layer(layer)
        && let LayerContent::Adjustment(a) = &l.content
    {
        let key = crate::adjust_preview::live_key(app, &snapshot, revision, layer, &params);
        let shown = memo_display(app, &snapshot, key, || {
            let kind = photocraft_engine::commands::adjustment_kind(a);
            let preview = photocraft_engine::adjust_params::from_params(kind, &params, Some(a), snapshot.mode).unwrap_or_else(|_| a.clone());
            let mut doc = (*snapshot).clone();
            if let Some(lm) = doc.layer_mut(layer) {
                lm.content = LayerContent::Adjustment(preview);
            }
            Some(doc)
        });
        if let Some(doc) = shown {
            return (doc, key);
        }
    }
    // Duotone documents display through their inks.
    if let Some(shown) = memo_display(app, &snapshot, 1 << 41, || photocraft_engine::mode_cmds::display_document(&snapshot)) {
        return (shown, 1 << 41);
    }
    (snapshot, 0)
}

/// `display_doc`'s live-adjust and Duotone documents, built once per document snapshot and key
/// rather than on each of its 3–4 calls a frame (#4527): each build clones the whole document.
fn memo_display(
    app: &mut PhotocraftApp,
    snapshot: &std::sync::Arc<Document>,
    key: u64,
    build: impl FnOnce() -> Option<Document>,
) -> Option<std::sync::Arc<Document>> {
    let id = snapshot.id;
    if let Some((doc, k, shown)) = app.display_memo.get(&id)
        && *k == key
        && doc.ptr_eq(&std::sync::Arc::downgrade(snapshot))
    {
        return Some(shown.clone());
    }
    let Some(shown) = build().map(std::sync::Arc::new) else {
        app.display_memo.remove(&id);
        return None;
    };
    // Entries of closed documents go when another is stored.
    app.display_memo.retain(|_, (doc, _, _)| doc.strong_count() > 0);
    app.display_memo.insert(id, (std::sync::Arc::downgrade(snapshot), key, shown.clone()));
    Some(shown)
}

/// Longest side of the Navigator panel's image (about twice the panel's width, for HiDPI).
pub const NAVIGATOR_SIDE: u32 = 512;
/// The navigator image of a document: its snapshot, preview key and revision, and the texture.
pub(crate) type NavigatorCache = (std::sync::Weak<Document>, u64, u64, egui::TextureHandle);
/// How long adjustment-dialog settings must stay unchanged before the navigator shows them.
const NAVIGATOR_SETTLE_MS: f64 = 200.0;

/// While a Move or Patch drag or a live paint stroke is under way, the navigator keeps its image
/// of document `idx` and catches up on release: each step of the drag is a new preview, and a
/// thumbnail composite of the whole document per step makes painting lag on large images.
pub(super) fn navigator_waits_for_drag(app: &PhotocraftApp, idx: usize) -> bool {
    crate::move_ui::showing(app) || crate::patch_preview::showing(app) || crate::cam_preview::showing(app) || live_stroke(app, idx).is_some()
}

/// The Navigator panel's image of document `idx`. With the CPU canvas that is the canvas texture
/// itself; with the GPU canvas a thumbnail cached per revision, so an edit to a huge document
/// doesn't also pay a full-resolution CPU composite for the navigator.
/// The Navigator's preview from the GPU canvas (#4518): the main canvas's texture of the
/// document, drawn at `rect` with its own mip chain, so an edit costs no CPU composite. `None`
/// while the canvas isn't showing the plain document on the GPU (the CPU thumbnail then).
pub fn navigator_gpu_view(app: &PhotocraftApp, idx: usize, rect: Rect, ctx: &egui::Context) -> Option<crate::gpu_canvas::ViewParams> {
    app.gpu.as_ref()?;
    let st = app.session.documents().get(idx)?;
    let params = *app.navigator_gpu.get(&st.doc.id)?;
    let [w, h] = params.doc_size;
    if w == 0 || h == 0 || !rect.is_positive() || [st.doc.size.width, st.doc.size.height] != params.doc_size {
        return None;
    }
    Some(crate::gpu_canvas::ViewParams {
        zoom: rect.width() / w as f32,
        center: [w as f32 / 2.0, h as f32 / 2.0],
        rotation: 0.0,
        shadow: false,
        pixel_grid: false,
        view_key: egui::Id::new(("pc-navigator", ctx.viewport_id(), idx)).value(),
        offset: [0.0, 0.0],
        ..params
    })
}

/// Frames the Navigator waits for the GPU canvas to draw a document it hasn't drawn yet, rather
/// than build a CPU thumbnail it would only use once (a full composite on a large document).
const NAVIGATOR_WAIT_FRAMES: u32 = 3;

/// Whether the Navigator should wait a frame for the GPU canvas (#4518): the GPU canvas is on but
/// hasn't drawn document `idx` yet (the Navigator can draw before the canvas in a frame). After
/// [`NAVIGATOR_WAIT_FRAMES`] it stops waiting, so it never stays empty.
pub fn navigator_waits_for_gpu(app: &PhotocraftApp, idx: usize, ctx: &egui::Context) -> bool {
    if app.gpu.is_none() {
        return false;
    }
    let Some(id) = app.session.documents().get(idx).map(|st| st.doc.id) else { return false };
    if app.canvases.keys().any(|(d, _)| *d == id) {
        return false;
    }
    let key = egui::Id::new(("navigator-waits", id.0));
    let waited = ctx.data(|d| d.get_temp::<u32>(key)).unwrap_or(0);
    if waited >= NAVIGATOR_WAIT_FRAMES {
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(key, waited + 1));
    ctx.request_repaint();
    true
}

pub fn navigator_texture(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize) -> Option<egui::TextureId> {
    if app.gpu.is_none() {
        return ensure_texture(app, ctx, idx, app.session.color.main_display).map(|(t, _)| t);
    }
    // Keyed by the document snapshot, not its revision: selecting a layer changes no pixels.
    let (snapshot, id, revision, last_damage) =
        app.session.documents().get(idx).map(|st| (std::sync::Arc::downgrade(&st.doc), st.doc.id, st.revision, st.last_damage))?;
    let (doc, preview_key) = display_doc(app, idx);
    let (display, display_key) = canvas_display(app, &doc, None);
    let preview_key = preview_key ^ display_key;
    let cached = app.navigator_textures.get(&id).cloned();
    if let Some((r, p, _, t)) = &cached
        && r.ptr_eq(&snapshot)
        && *p == preview_key
    {
        return Some(t.id());
    }
    if let Some((_, _, _, t)) = &cached
        && navigator_waits_for_drag(app, idx)
    {
        return Some(t.id());
    }
    // While an adjustment dialog's settings are changing, the navigator keeps its image and
    // catches up once they settle (a thumbnail composite per change would cost more than the canvas).
    if let Some((r, _, _, t)) = &cached
        && r.ptr_eq(&snapshot)
        && crate::adjust_preview::settling(app, NAVIGATOR_SETTLE_MS)
    {
        ctx.request_repaint_after(std::time::Duration::from_millis(NAVIGATOR_SETTLE_MS as u64));
        return Some(t.id());
    }
    let t0 = crate::gpu_canvas::now_ms();
    // One edit since the cached image, with the area it changed: update just that part (nothing
    // for a rename, a selection or a new empty layer). A thumbnail composite of the whole
    // document on every edit cost a full-resolution CPU composite on the UI thread.
    let size = photocraft_compose::thumbnail_size(&doc, NAVIGATOR_SIDE);
    if let Some((_, p, rev, t)) = &cached
        && *p == preview_key
        && rev.checked_add(1) == Some(revision)
        && let Some(d) = last_damage
        && t.size() == [size.0 as usize, size.1 as usize]
    {
        let mut t = t.clone();
        if !d.is_empty() {
            let buf = photocraft_compose::render_reduced_damage(&doc, size.0, size.1, d.inflate(effect_reach(&doc.layers)));
            let r = buf.rect;
            if !r.is_empty() {
                t.set_partial([r.x0 as usize, r.y0 as usize], display_image(display.as_deref(), &buf), TextureOptions::LINEAR);
            }
        }
        app.perf.span("navigator", crate::gpu_canvas::now_ms() - t0);
        app.navigator_textures.insert(id, (snapshot, preview_key, revision, t.clone()));
        return Some(t.id());
    }
    let image = display_image(display.as_deref(), &photocraft_compose::thumbnail_buffer(&doc, NAVIGATOR_SIDE));
    let tex = match cached {
        Some((_, _, _, mut t)) => {
            t.set(image, TextureOptions::LINEAR);
            t
        }
        None => ctx.load_texture(format!("navigator-{}", id.0), image, TextureOptions::LINEAR),
    };
    app.perf.span("navigator", crate::gpu_canvas::now_ms() - t0);
    app.navigator_textures.insert(id, (snapshot, preview_key, revision, tex.clone()));
    Some(tex.id())
}

/// The CPU canvas texture's sampling (#4810). Zoomed in, a full-resolution texture shows each
/// document pixel as a crisp square, as the GPU canvas's sharp nearest filter and Photoshop do;
/// zoomed out it is still filtered. A reduced texture (`factor` > 1) stays bilinear: its texels
/// are averages of several pixels, and squares of them would be blocks the image doesn't have.
pub(super) fn canvas_texture_options(factor: u32) -> TextureOptions {
    if factor > 1 { TextureOptions::LINEAR } else { TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR } }
}

/// Make sure the canvas texture for document `idx` on `display` is current; returns (texture
/// id, scale).
pub fn ensure_texture(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, display: Option<u32>) -> Option<(egui::TextureId, f32)> {
    ensure_texture_in_view(app, ctx, idx, display, None).map(|t| (t.texture, t.scale))
}

/// [`ensure_texture`], and for a canvas showing `view` of a document too large for one
/// full-resolution texture, the detail texture of the part in view ([`detail_plan`], #7568).
pub(crate) fn ensure_texture_in_view(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    idx: usize,
    display: Option<u32>,
    view: Option<InView>,
) -> Option<CpuTextures> {
    let output = display;
    let (revision, last_damage, id) = {
        let st = app.session.documents().get(idx)?;
        (st.revision, st.last_damage.map(|r| if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) }), st.doc.id)
    };
    let (mut doc, mut preview_key) = display_doc(app, idx);
    // A flipped view draws a GPU machine's canvas through here: an adjustment dialog's preview of
    // a large document (taken by the GPU path) would cost a full-size CPU composite per change.
    if crate::adjust_preview::shown_key(app) == Some(preview_key)
        && let Some(st) = app.session.documents().get(idx)
        && crate::proxy::preview_factor(&st.doc, crate::proxy::reduced_previews(app)) > 1
    {
        (doc, preview_key) = (st.doc.clone(), 0);
    }
    let (display, display_key) = canvas_display(app, &doc, output);
    let seen = app.canvases.get(&cache_key(id, output)).map(|c| (c.tex_revision, c.tex_preview_key));
    let damage = seen.and_then(|seen| damage_since(app, idx, seen, (revision, preview_key), display_key, last_damage));
    // The detail texture can lag the whole-document one (made or moved later): its own damage.
    let detail_seen = view.and_then(|v| {
        let c = app.canvases.get(&cache_key(id, output))?;
        c.detail.iter().find(|(k, _)| *k == v.key).map(|(_, d)| (d.revision, d.preview_key))
    });
    let detail_damage = match detail_seen {
        Some(s) if Some(s) == seen => damage,
        Some(s) => damage_since(app, idx, s, (revision, preview_key), display_key, last_damage),
        None => None,
    };
    let preview_key = preview_key ^ display_key;
    let cache = app.canvases.entry(cache_key(id, output)).or_insert(CanvasCache {
        revision: 0,
        texture: None,
        scale: 1.0,
        preview_key: 0,
        on_gpu: false,
        tex_revision: 0,
        tex_preview_key: 0,
        texture_opaque: false,
        grid: None,
        detail: Vec::new(),
    });
    if cache.tex_revision != revision || cache.texture.is_none() || cache.tex_preview_key != preview_key {
        let t0 = crate::gpu_canvas::now_ms();
        let factor = texture_factor([doc.size.width, doc.size.height], texture_side(ctx));
        let (w, h) = ((doc.size.width / factor).max(1), (doc.size.height / factor).max(1));
        let current = cache.texture.as_mut().filter(|t| t.size() == [w as usize, h as usize]);
        if let (Some(d), Some(t)) = (damage, current) {
            // Only what the edit or stroke touched: the reduced texture's pixels over it, with the
            // values the whole reduction gives them (factor 1 is the plain composite of `d`).
            let buf = photocraft_compose::render_reduced_damage(&doc, w, h, d);
            let r = buf.rect;
            if !r.is_empty() {
                let t1 = crate::gpu_canvas::now_ms();
                let image = display_image(display.as_deref(), &buf);
                // Partial uploads can disprove opacity. Once transparent, conservatively keep
                // the checker until a full upload proves the whole texture opaque again.
                cache.texture_opaque &= image.pixels.iter().all(|p| p.is_opaque());
                t.set_partial([r.x0 as usize, r.y0 as usize], image, canvas_texture_options(factor));
                let px = (r.width() as u64 * r.height() as u64).saturating_mul(u64::from(factor).pow(2));
                app.perf.record("rect", px, t1 - t0, crate::gpu_canvas::now_ms() - t1);
            }
        } else {
            // Reduced in bands straight from the compositor: no full-size composite in memory.
            let full = photocraft_compose::render_reduced(&doc, w, h);
            let t1 = crate::gpu_canvas::now_ms();
            let (img, scale) = (display_image(display.as_deref(), &full), 1.0 / factor as f32);
            cache.texture_opaque = img.pixels.iter().all(|p| p.is_opaque());
            match cache.texture.as_mut() {
                Some(t) if t.size() == img.size => t.set(img, canvas_texture_options(factor)),
                _ => cache.texture = Some(ctx.load_texture(format!("canvas-{}-{}", id.0, cache_key(id, output).1), img, canvas_texture_options(factor))),
            }
            cache.scale = scale;
            app.perf.record("full", doc.size.width as u64 * doc.size.height as u64, t1 - t0, crate::gpu_canvas::now_ms() - t1);
        }
        cache.tex_revision = revision;
        cache.tex_preview_key = preview_key;
    }
    let texture = cache.texture.as_ref()?.id();
    let scale = cache.scale;
    // Without a view (the Navigator's call) the views' detail textures stay as they are.
    let Some(view) = view else { return Some(CpuTextures { texture, scale, detail: None }) };
    let old = cache.detail.iter().position(|(k, _)| *k == view.key).map(|i| cache.detail.remove(i).1);
    let Some(plan) = detail_plan([doc.size.width, doc.size.height], texture_side(ctx), view.zoom, view.visible) else {
        return Some(CpuTextures { texture, scale, detail: None });
    };
    let t0 = crate::gpu_canvas::now_ms();
    let name = format!("canvas-detail-{}-{}", id.0, cache_key(id, output).1);
    let detail = update_detail(ctx, &doc, display.as_deref(), name, plan, old, detail_damage, (revision, preview_key));
    if detail.as_ref().is_some_and(|d| d.rendered) {
        app.perf.span("canvas detail", crate::gpu_canvas::now_ms() - t0);
    }
    let Some(mut detail) = detail else { return Some(CpuTextures { texture, scale, detail: None }) };
    detail.rendered = false;
    // Where its texels lie in the document: reduced texel t covers [t·W/w, (t+1)·W/w).
    let (w, h) = ((doc.size.width / plan.factor).max(1), (doc.size.height / plan.factor).max(1));
    let (sx, sy) = (doc.size.width as f32 / w as f32, doc.size.height as f32 / h as f32);
    let out = detail.out;
    let shown = (detail.texture.id(), [out.x0 as f32 * sx, out.y0 as f32 * sy, out.x1 as f32 * sx, out.y1 as f32 * sy]);
    cache.detail.push((view.key, detail));
    // Views that stopped asking (a closed window, a zoom out) give theirs up past a few.
    if cache.detail.len() > MAX_DETAIL_VIEWS {
        cache.detail.remove(0);
    }
    Some(CpuTextures { texture, scale, detail: Some(shown) })
}

/// Most CPU canvas views of one document on one display that keep a detail texture.
const MAX_DETAIL_VIEWS: usize = 4;

/// The detail texture for `plan` at document state `now` (revision, preview key with the display
/// key folded in): `old` as it is when current, updated over `damage` when only that changed
/// within the same region, or rendered again. `None` when the region holds no texels.
#[allow(clippy::too_many_arguments)]
fn update_detail(
    ctx: &egui::Context,
    doc: &Document,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    name: String,
    plan: DetailPlan,
    old: Option<DetailTexture>,
    damage: Option<DRect>,
    now: (u64, u64),
) -> Option<DetailTexture> {
    let (w, h) = ((doc.size.width / plan.factor).max(1), (doc.size.height / plan.factor).max(1));
    let options = canvas_texture_options(plan.factor);
    let (kept, spare) = match old {
        Some(d) if d.plan == plan => (Some(d), None),
        d => (None, d.map(|d| d.texture)),
    };
    if let Some(mut d) = kept {
        if (d.revision, d.preview_key) == now {
            return Some(d);
        }
        // An edit or a stroke step in view: only the texels it touched.
        if let Some(dmg) = damage {
            let dmg = dmg.intersect(&plan.region);
            let buf = (!dmg.is_empty()).then(|| photocraft_compose::render_reduced_damage(doc, w, h, dmg));
            let fits = buf.as_ref().is_none_or(|b| d.out.contains_rect(&b.rect));
            if fits {
                if let Some(b) = buf.filter(|b| !b.rect.is_empty()) {
                    let at = [b.rect.x0.abs_diff(d.out.x0) as usize, b.rect.y0.abs_diff(d.out.y0) as usize];
                    d.texture.set_partial(at, display_image(display, &b), options);
                    d.rendered = true;
                }
                (d.revision, d.preview_key) = now;
                return Some(d);
            }
        }
        return render_detail(ctx, doc, display, name, plan, Some(d.texture), now);
    }
    render_detail(ctx, doc, display, name, plan, spare, now)
}

/// [`update_detail`]'s whole render of `plan`'s region, into `texture` when it is the same size.
fn render_detail(
    ctx: &egui::Context,
    doc: &Document,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    name: String,
    plan: DetailPlan,
    texture: Option<egui::TextureHandle>,
    now: (u64, u64),
) -> Option<DetailTexture> {
    let (w, h) = ((doc.size.width / plan.factor).max(1), (doc.size.height / plan.factor).max(1));
    let options = canvas_texture_options(plan.factor);
    let buf = photocraft_compose::render_reduced_damage(doc, w, h, plan.region);
    let out = buf.rect;
    if out.is_empty() {
        return None;
    }
    let img = display_image(display, &buf);
    let texture = match texture {
        Some(mut t) if t.size() == img.size => {
            t.set(img, options);
            t
        }
        _ => ctx.load_texture(name, img, options),
    };
    Some(DetailTexture { texture, plan, out, revision: now.0, preview_key: now.1, rendered: true })
}

/// What changed since a canvas cache showed (`revision`, `preview key`) `seen`, when only a
/// rectangle did: the last edit's damage, or the live stroke's dabs since then. Cached keys have
/// the colour display's key folded in (`^ display_key`); `now`'s is still raw.
pub(crate) fn damage_since(app: &PhotocraftApp, idx: usize, seen: (u64, u64), now: (u64, u64), display_key: u64, last_damage: Option<DRect>) -> Option<DRect> {
    // A brush stroke committed with the pixels its live preview already showed: only the steps
    // this cache hadn't shown yet (usually nothing), not the stroke's whole bounding box.
    if seen.0 + 1 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(c) = app.committed_stroke.as_ref().filter(|c| c.doc == st.doc.id && c.revision == now.0)
        && let Some(r) = c.since(seen.1 ^ display_key)
    {
        let r = if c.relayout.is_empty() {
            r
        } else if r.is_empty() {
            c.relayout
        } else {
            r.union(&c.relayout)
        };
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    if seen.1 == now.1 ^ display_key && seen.0 + 1 == now.0 {
        return last_damage;
    }
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::type_transform::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between an adjustment dialog's previews (and the document): the target's area.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::adjust_preview::switch_region(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(r);
    }
    // Between a Properties-panel adjustment-layer drag's previews (and the document): the
    // layer's area. Without this every parameter value recomposited the whole document (#6839).
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::adjust_preview::live_switch_region(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(r);
    }
    // Between a Move drag's offsets: where the moving layers were and are (Auto-Select's pick
    // at the press changes no pixels).
    if (seen.0 == now.0 || (seen.0 + 1 == now.0 && last_damage.is_some_and(|r| r.is_empty())))
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::move_ui::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between a Patch drag's previews: the areas they healed.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::patch_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between hovered blend modes (and the document): where the layer's blending shows.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::blend_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    // Between hovered fonts (and the document): the text layer's old/new ink bounds.
    if seen.0 == now.0
        && let Some(st) = app.session.documents().get(idx)
        && let Some(r) = crate::type_font_preview::damage(app, st.doc.id, now.0, seen.1 ^ display_key, now.1)
    {
        return Some(if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) });
    }
    let l = live_stroke(app, idx).filter(|l| seen.0 == now.0 && l.display_key() == now.1)?;
    let r = l.since(seen.1 ^ display_key)?;
    Some(if r.is_empty() { r } else { r.inflate(effect_reach(&l.stroke.doc().layers)) })
}

/// Document `doc`'s canvas caches showed a preview that the edit just committed reproduces
/// (`was_preview` tells its keys): count it as the document itself, so the commit's damage rect
/// refreshes only that area instead of everything.
pub(crate) fn shown_as_document(app: &mut PhotocraftApp, doc: photocraft_doc::DocId, was_preview: impl Fn(u64) -> bool) {
    let Some(d) = app.session.documents().iter().find(|st| st.doc.id == doc).map(|st| st.doc.clone()) else { return };
    if let Some(c) = app.channel_views.get_mut(&doc.0)
        && was_preview(c.preview_key)
    {
        c.preview_key = 0;
    }
    // Every display's cache of the document: the GPU state folds in the texture key, CPU
    // textures their display's key.
    let outputs: Vec<u32> = app.canvases.keys().filter(|k| k.0 == doc).map(|k| k.1).collect();
    for out in outputs {
        let (display, key) = canvas_display(app, &d, (out != 0 && out != GPU_OUTPUT).then_some(out));
        let gpu_key = texture_key(display.as_deref());
        let Some(c) = app.canvases.get_mut(&(doc, out)) else { continue };
        if was_preview(c.preview_key ^ gpu_key) {
            c.preview_key = gpu_key;
        }
        if was_preview(c.tex_preview_key ^ key) {
            c.tex_preview_key = key;
        }
    }
}

/// How far beyond an edit's damage rect the composite can change: layer effects (shadows, glows,
/// strokes, …) on the edited layer and on the groups around it reach that far.
pub(crate) fn effect_reach(layers: &[photocraft_doc::Layer]) -> i32 {
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

/// GPU path: make sure document `idx` is current in the GPU canvas. Edits draw their damage;
/// unchanged lower-stack composites can be reused by the compositor inside that region.
/// Returns false if there is no GPU canvas.
pub(super) fn ensure_gpu(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, visible: DRect) -> bool {
    let Some(gpu) = app.gpu.clone() else { return false };
    let Some((revision, last_damage, id)) = app
        .session
        .documents()
        .get(idx)
        .map(|st| (st.revision, st.last_damage.map(|r| if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) }), st.doc.id))
    else {
        return false;
    };
    let (doc, raw_key) = display_doc(app, idx);
    let (display, _) = canvas_display(app, &doc, None);
    // Repeated pattern tiles sample the same texture, including pixels outside the unwrapped
    // viewport. Refresh the affected document region while Pattern Preview is enabled.
    let blend_visible = if app.ui.view.pattern_preview { doc.bounds() } else { visible };
    // The texture is shared by every display: only what it stores counts, not the monitor.
    let display_key = texture_key(display.as_deref());
    let seen = app.canvases.get(&(id, GPU_OUTPUT)).map(|c| (c.revision, c.preview_key));
    let mut damage = seen.and_then(|seen| damage_since(app, idx, seen, (revision, raw_key), display_key, last_damage));
    if let Some(region) = damage {
        damage = Some(crate::blend_preview::clip_damage(app, id, revision, raw_key, region, blend_visible));
    }
    // To an adjustment dialog's preview: only what the view shows now (the rest as it moves there).
    if damage.is_some()
        && let Some(r) = seen.filter(|s| s.0 == revision).and_then(|s| crate::adjust_preview::switch_region(app, id, revision, s.1 ^ display_key, raw_key))
    {
        damage = Some(crate::adjust_preview::switch_damage(app, raw_key, r, visible));
    }
    // The same for a Properties-panel drag's previews.
    if damage.is_some()
        && let Some(r) = seen.filter(|s| s.0 == revision).and_then(|s| crate::adjust_preview::live_switch_region(app, id, revision, s.1 ^ display_key, raw_key))
    {
        damage = Some(crate::adjust_preview::live_switch_damage(app, raw_key, r, visible));
    }
    let preview_key = raw_key ^ display_key;
    // The Navigator may draw this texture while it holds the plain document or a drag preview
    // that updates the full texture (not an adjustment preview composited only where the view is).
    if raw_key == 0 || navigator_waits_for_drag(app, idx) {
        app.navigator_plain.insert(id);
    } else {
        app.navigator_plain.remove(&id);
        app.navigator_gpu.remove(&id);
    }
    let size = [doc.size.width, doc.size.height];
    // A full CPU redraw running in the background: show the blocks it has finished.
    crate::background_redraw::poll(app, &gpu, id);
    let cache = app.canvases.entry((id, GPU_OUTPUT)).or_insert(CanvasCache {
        revision: 0,
        texture: None,
        scale: 1.0,
        preview_key: 0,
        on_gpu: false,
        tex_revision: 0,
        tex_preview_key: 0,
        texture_opaque: false,
        grid: None,
        detail: Vec::new(),
    });
    let present = cache.on_gpu && gpu.has(id.0, size);
    let current = cache.revision == revision && cache.preview_key == preview_key;
    // GPU Memory or Memory Usage changed with nothing to redraw: the new budget, and whether the
    // layers fit it, show at once (the status bar's GPU Memory Full).
    if present && app.perf.gpu_budget_allowance != budget_allowance(app) {
        gpu_budget(app, &gpu, idx, false, visible);
    }
    // Switched to a document whose canvas is already current: size the budget for it and measure
    // whether its layers fit, so GPU Memory Full shows (or clears) at once rather than at its
    // next full refresh. Once per switch: the fit then names this document.
    if present && app.perf.gpu_layer_fit.is_none_or(|f| f.doc != id.0) {
        gpu_budget(app, &gpu, idx, false, visible);
        let fit = gpu.measure_fit(&doc).unwrap_or(crate::gpu_canvas::LayerFit { doc: id.0, needed: 0, budget: 0 });
        app.perf.gpu_layer_fit = Some(fit);
    }
    if present && current {
        // An adjustment preview composited where the view was: catch up where it moved to.
        if let Some(r) = crate::adjust_preview::uncovered(app, id, raw_key, visible) {
            let r = gpu.refresh(id.0, &doc, Some(r), display.as_deref());
            app.perf.record_refresh(&r);
        }
        if let Some(r) = crate::adjust_preview::live_uncovered(app, id, raw_key, visible) {
            let r = gpu.refresh(id.0, &doc, Some(r), display.as_deref());
            app.perf.record_refresh(&r);
        }
        // Blend previews keep offscreen pixels untouched until the view reaches them.
        // Use the normal refresh path so CPU fallback and pending redraws stay synchronized.
        damage = crate::blend_preview::uncovered(app, id, revision, raw_key, blend_visible);
        if damage.is_none() {
            return true;
        }
    }
    // Display textures for a canvas this large would not fit the GPU: the reduced CPU canvas
    // draws it (checked before the memory budget, which walks the history, every frame).
    if !gpu.fits_canvas(&doc) {
        crate::background_redraw::cancel(app, id);
        // Drawn on the CPU canvas: no layer budget applies to it.
        if app.perf.gpu_layer_fit.is_some_and(|f| f.doc == id.0) {
            app.perf.gpu_layer_fit = None;
        }
        return false;
    }
    let partial = present && damage.is_some();
    gpu_budget(app, &gpu, idx, partial, visible);
    let defer = crate::background_redraw::enabled(app);
    let r = gpu.refresh_with(id.0, &doc, if partial { damage } else { None }, display.as_deref(), defer);
    app.perf.gpu_layer_fit = gpu.layer_fit();
    if r.kind == "lost" || r.kind == crate::gpu_canvas::REFRESH_OVER_BUDGET {
        // The device is gone (`gpu_status` then drops the GPU canvas) or the canvas is over the
        // texture budget: draw this frame on the CPU path.
        crate::background_redraw::cancel(app, id);
        return false;
    }
    if r.kind == crate::gpu_canvas::REFRESH_DEFERRED {
        // Composited off the UI thread, the visible part first (#4024).
        let region = if partial { damage.unwrap_or(doc.bounds()) } else { doc.bounds() };
        crate::background_redraw::start(app, ctx, &gpu, doc.clone(), display.clone(), visible, region, r.fallback.clone());
    } else if partial {
        // The edit drew its damage; blocks of a running redraw still to come follow the edit.
        crate::background_redraw::restart(app, ctx, &gpu, doc.clone(), display.clone(), visible);
    } else {
        crate::background_redraw::cancel(app, id);
    }
    if let Some(e) = &r.fallback
        && app.perf.gpu_fallback.as_deref() != Some(e.as_str())
    {
        log::info!("{e}; using the CPU compositor");
    }
    if r.kind != crate::gpu_canvas::REFRESH_DEFERRED {
        app.perf.record_refresh(&r);
    }
    if r.kind == "full" {
        crate::notices::slow_cpu_refresh(app, id.0, r.composite_ms + r.upload_ms, r.fallback.as_deref());
    }
    if r.kind.starts_with("gpu") {
        app.perf.gpu_uploads = r.uploads;
    } else {
        app.perf.gpu_fallback = r.fallback;
    }
    let Some(cache) = app.canvases.get_mut(&(id, GPU_OUTPUT)) else { return true };
    cache.revision = revision;
    cache.preview_key = preview_key;
    cache.on_gpu = true;
    cache.texture = None;
    true
}

/// Point the wgpu compositor at what the view shows, and size its memory budget from GPU Memory,
/// or Memory Usage, the document's pixels and History (charged where the GPU shares the system's
/// memory), physical memory and the GPU's own: on full refreshes (pixels counted once per
/// structural change, not per brush dab) and whenever either preference changes.
fn gpu_budget(app: &mut PhotocraftApp, gpu: &crate::gpu_canvas::GpuCanvas, idx: usize, partial: bool, visible: DRect) {
    gpu.set_focus(Some(visible));
    let allowance = budget_allowance(app);
    let changed = app.perf.gpu_budget_allowance != allowance;
    if partial && !changed && gpu.memory_budget().is_some() {
        return;
    }
    let Some(st) = app.session.documents().get(idx) else { return };
    let pixels = st.memory_bytes() as u64;
    let budget = crate::gpu_canvas::memory_budget(allowance.0, pixels, crate::gpu_canvas::physical_memory(), gpu.video_memory(), allowance.1);
    gpu.set_memory_budget(budget);
    app.perf.gpu_budget_mb = budget >> 20;
    app.perf.gpu_budget_allowance = allowance;
    if changed && let Some(fit) = gpu.measure_fit(&st.doc) {
        app.perf.gpu_layer_fit = Some(fit);
    }
}

/// Memory Usage and GPU Memory (Preferences › Performance), in bytes.
fn budget_allowance(app: &PhotocraftApp) -> (u64, u64) {
    let performance = &app.session.prefs().performance;
    (u64::from(performance.memory_usage_mb).saturating_mul(1 << 20), u64::from(performance.gpu_memory_mb).saturating_mul(1 << 20))
}

/// The proxy factor of a live preview of filter `cmd` with `params` on `doc`, the active document:
/// reduced on a large document (Preferences › Performance › Low Resolution Previews), except for
/// filters whose features don't scale with a proxy (#2063), and never coarser than the active view,
/// so at 100 % the preview shows every pixel instead of k-pixel blocks (#6326, as adjustment
/// previews have since #5764). Zoomed out, the proxy keeps previews of large documents fast.
///
/// The in-dialog preview window ([`crate::filter_preview_ui`]) sizes its own textures from this
/// too, so the canvas and the window never disagree about the factor.
///
/// Full resolution costs a whole filter per parameter change, which only the background worker
/// absorbs. The web build has no worker and computes the preview inline on the UI thread (#1676),
/// where that would freeze the page: there the document's proxy stands. The test and
/// `PHOTOCRAFT_INLINE_JOBS` inline paths keep the cap deliberately, because `background_jobs` is
/// flipped mid-session (a shell save during `actions.play`): a factor that followed it would change
/// [`crate::filter_dialog::FilterPreviewKey`] under a finished preview and recompute it inline,
/// the very stall this avoids.
pub(crate) fn filter_preview_factor(app: &PhotocraftApp, doc: &Document, cmd: &str, params: &serde_json::Value) -> u32 {
    // A smart filter edit re-renders the whole stack: every stage constrains the factor.
    let k = crate::filter_dialog::smart_preview_factor(doc, cmd, params, crate::proxy::preview_factor(doc, crate::proxy::reduced_previews(app)));
    if cfg!(target_arch = "wasm32") { k } else { crate::proxy::view_factor(k, app.current_zoom()) }
}

/// Live preview for an open filter dialog: run the filter on the proxy and upload it. With
/// background jobs (the desktop app) it computes on one background worker
/// (`filter_preview_worker`); otherwise (web, tests, `PHOTOCRAFT_INLINE_JOBS`) it keeps the
/// deterministic inline path (#1676), which also lets headless/CPU tests inspect previews
/// without a GPU texture.
pub(crate) fn ensure_filter_preview(app: &mut PhotocraftApp, idx: usize, ctx: &egui::Context) -> Option<(u32, u64, [u32; 2])> {
    if let Some(held) = committed_filter_preview(app, idx) {
        return held;
    }
    // Command dialogs always edit the active document. Never show their preview in another tab.
    if app.session.active_index() != Some(idx) {
        return None;
    }
    if crate::adjust_preview::on_layer(app, idx) {
        return None;
    }
    let d = app.ui.dialogs.iter().find(|d| d.fields.contains_key("__filter"))?;
    if d.fields.get("__preview").and_then(serde_json::Value::as_bool) != Some(true) {
        return None;
    }
    let cmd = d.fields.get("__command")?.as_str()?.to_string();
    // Previews edit what the command will: a targeted layer mask included (#780).
    let (cmd, mut params) = if d.fields.contains_key("__smartFilter") {
        crate::smart_filter_dialog::edit_command(app, &d.fields).ok()?
    } else {
        (cmd.clone(), app.with_mask_target(&cmd, crate::filter_dialog::params_of(&d.fields)))
    };
    // Select and Mask previews in its view mode; holding X shows the image without one.
    if crate::select_mask_view::owns(&d.fields) {
        if crate::select_mask_view::hides_view(&d.fields) {
            return None;
        }
        params = crate::select_mask_view::request_params(&d.fields, params);
    }
    let st = app.session.documents().get(idx)?;
    let k = filter_preview_factor(app, &st.doc, &cmd, &params);
    let request = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        dialog: d.id,
        active: st.active_layer,
        target: st.channel_view.target,
        command: cmd,
        params,
        k,
    };
    let key = request.doc.0 ^ (1u64 << 61);
    // Every filter's preview computes off the UI thread in the desktop app: Gaussian Blur at
    // 1000 px takes long enough to freeze the window. Once the slider settles, a preview still
    // running for an older value is cancelled.
    #[cfg(not(target_arch = "wasm32"))]
    if app.background_jobs {
        app.filter_preview_worker.supersede(&request, crate::gpu_canvas::now_ms());
        if let Some((finished, computed)) = app.filter_preview_worker.poll() {
            match computed {
                // The dialog still holds these exact parameters. Publish as before, including an
                // empty result: that is how a filter which no longer applies clears the canvas.
                Ok(computed) if finished == request => publish_filter_preview(app, idx, finished, computed.result, computed.ms),
                // Mid-drag, a frame for the value the pointer has just left is what makes the
                // canvas follow the drag. Three things have to hold for it: it belongs to this
                // preview, it has a frame to show (an empty one would blank a good preview), and
                // it must not displace a frame that is already exact for the dialog, which is
                // what a slider dragged out and back would otherwise make the canvas do.
                Ok(computed)
                    if finished.same_target(&request) && computed.result.is_some() && !app.filter_preview.as_ref().is_some_and(|p| p.key == request) =>
                {
                    publish_filter_preview(app, idx, finished, computed.result, computed.ms);
                }
                Ok(_) => {}
                // Errors stay on the exact request: a value the pointer has already dragged past
                // must not raise a notice, and a still one will raise it when it is recomputed.
                Err(error) if finished == request => {
                    app.filter_preview = Some(crate::filter_dialog::FilterPreview { key: finished, result: None, committing: false });
                    crate::notices::error(app, error);
                }
                Err(_) => {}
            }
        }
        let fresh = app.filter_preview.as_ref().is_some_and(|p| p.key == request);
        if !fresh && !app.filter_preview_worker.busy() {
            let doc = app.session.documents().get(idx)?.doc.clone();
            let documents: Vec<_> = app.session.documents().iter().map(|d| d.doc.clone()).collect();
            let color = app.session.color.settings.clone();
            let task = request.clone();
            if let Err(error) = app.filter_preview_worker.start(request.clone(), ctx.clone(), move |cancel| {
                let t0 = crate::gpu_canvas::now_ms();
                let result = crate::filter_dialog::preview_document_for_target(
                    &doc,
                    task.active,
                    &task.command,
                    &task.params,
                    task.k,
                    Some(cancel),
                    task.target,
                    &documents,
                    &color,
                )
                .map(|doc| {
                    let buffer = photocraft_compose::flatten(&doc);
                    (std::sync::Arc::new(doc), buffer)
                });
                crate::filter_preview_worker::Computed { result, ms: crate::gpu_canvas::now_ms() - t0 }
            }) {
                app.filter_preview = Some(crate::filter_dialog::FilterPreview { key: request.clone(), result: None, committing: false });
                crate::notices::error(app, error);
            }
        }
        if app.filter_preview_worker.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let preview = app.filter_preview.as_ref()?;
        // Keep the last preview during a drag, but never one from another source or dialog.
        if !preview.key.same_target(&request) {
            return None;
        }
        let result = preview.result.as_ref()?;
        return Some((preview.key.k, key, [result.size.width, result.size.height]));
    }
    // Web and explicitly inline sessions keep the deterministic synchronous path (#1676):
    // computed inline, uploaded when a GPU texture exists, still inspectable without one.
    // Flattened once, as the background worker does; `publish_filter_preview` uploads it (#7490).
    let _ = ctx;
    if app.filter_preview.as_ref().is_none_or(|p| p.key != request) {
        let doc = app.session.documents().get(idx)?.doc.clone();
        let documents: Vec<_> = app.session.documents().iter().map(|d| d.doc.clone()).collect();
        let t0 = crate::gpu_canvas::now_ms();
        let computed = crate::filter_dialog::preview_document_for_target(
            &doc,
            request.active,
            &request.command,
            &request.params,
            request.k,
            None,
            request.target,
            &documents,
            &app.session.color.settings,
        )
        .map(|doc| {
            let buffer = photocraft_compose::flatten(&doc);
            (std::sync::Arc::new(doc), buffer)
        });
        publish_filter_preview(app, idx, request, computed, crate::gpu_canvas::now_ms() - t0);
    }
    let preview = app.filter_preview.as_ref()?;
    let result = preview.result.as_ref()?;
    Some((preview.key.k, key, [result.size.width, result.size.height]))
}

/// Channel overlays must use the filter's current proxy too, including on the CPU canvas.
/// The cache key changes with the completed request; unchanged frames reuse the overlay texture.
pub(crate) fn filter_channel_preview(app: &mut PhotocraftApp, idx: usize, ctx: &egui::Context) -> Option<(std::sync::Arc<Document>, u64)> {
    use std::hash::{Hash, Hasher};
    ensure_filter_preview(app, idx, ctx)?;
    let preview = app.filter_preview.as_ref()?;
    let doc = preview.result.as_ref()?.clone();
    let mut key = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", preview.key).hash(&mut key);
    Some((doc, key.finish() | (1 << 59)))
}

fn publish_filter_preview(
    app: &mut PhotocraftApp,
    idx: usize,
    request: crate::filter_dialog::FilterPreviewKey,
    computed: Option<(std::sync::Arc<photocraft_doc::Document>, photocraft_compose::Buffer)>,
    ms: f64,
) {
    let result = computed.and_then(|(result, buffer)| {
        let t0 = crate::gpu_canvas::now_ms();
        let doc = app.session.documents().get(idx)?.doc.clone();
        // Shown through the preview's own colour management, as the result will be: Assign and
        // Convert to Profile change the profile (and Convert the mode), not just the pixels.
        let (display, _) = canvas_display(app, &result, None);
        // A GPU texture is an optimisation: headless/CPU tests must still see the preview.
        if let Some(gpu) = app.gpu.as_ref() {
            gpu.upload_buffer_full(request.doc.0 ^ (1u64 << 61), &texture_buffer(display.as_deref(), &buffer), doc.depth);
        }
        app.perf.record("filter-preview", result.size.area(), ms, crate::gpu_canvas::now_ms() - t0);
        Some(result)
    });
    app.filter_preview = Some(crate::filter_dialog::FilterPreview { key: request, result, committing: false });
}

/// The document whose colour management the canvas texture under `key` is shown through: a filter
/// preview's own result (it keeps the document's id, so Proof Colors still apply), else `doc`.
/// Assign Profile changes only the profile, so through `doc`'s its preview looked unchanged.
pub(super) fn filter_preview_display_doc(app: &PhotocraftApp, doc: &std::sync::Arc<Document>, key: u64) -> std::sync::Arc<Document> {
    if key != doc.id.0 ^ (1u64 << 61) {
        return doc.clone();
    }
    app.filter_preview.as_ref().and_then(|p| p.result.clone()).filter(|r| r.id == doc.id).unwrap_or_else(|| doc.clone())
}

/// After a filter dialog's OK: `Some(shown)` while its preview stands in for the background job
/// that commits it (the document unchanged and the job still running, for this document's views),
/// `None` otherwise (other documents' views included). A job that lands, fails or is cancelled
/// drops it.
pub(crate) fn committed_filter_preview(app: &mut PhotocraftApp, idx: usize) -> Option<Option<(u32, u64, [u32; 2])>> {
    let p = app.filter_preview.as_ref().filter(|p| p.committing)?;
    let st = app.session.documents().get(idx)?;
    if st.doc.id != p.key.doc {
        return None;
    }
    let size = p.result.as_ref().map(|r| [r.size.width, r.size.height]);
    if let Some(size) = size
        && st.revision == p.key.revision
        && app.session.job_on(p.key.doc).is_some()
    {
        return Some(Some((p.key.k, p.key.doc.0 ^ (1u64 << 61), size)));
    }
    app.filter_preview = None;
    Some(None)
}

/// The document pixels a view shows, with a margin for filtering. Uses all four canvas corners
/// so a rotated camera still covers the visible tiles.
pub(super) fn visible_doc_rect(xf: &ViewXform) -> DRect {
    let r = xf.rect;
    let docs = [xf.to_doc(r.min), xf.to_doc(pos2(r.max.x, r.min.y)), xf.to_doc(r.max), xf.to_doc(pos2(r.min.x, r.max.y))];
    let mut x0 = docs[0][0];
    let mut y0 = docs[0][1];
    let mut x1 = x0;
    let mut y1 = y0;
    for d in docs {
        x0 = x0.min(d[0]);
        y0 = y0.min(d[1]);
        x1 = x1.max(d[0]);
        y1 = y1.max(d[1]);
    }
    let c = |v: f64| v.clamp(-1e9, 1e9) as i32;
    DRect::new(c(x0.floor()) - 2, c(y0.floor()) - 2, c(x1.ceil()) + 2, c(y1.ceil()) + 2)
}

/// Prioritize deferred redraw blocks visible at either the target or the animated display camera.
/// The displayed camera may still show old texture contents outside the target's visible area.
pub(super) fn redraw_focus(target: &ViewXform, displayed: &ViewXform) -> DRect {
    visible_doc_rect(target).union(&visible_doc_rect(displayed))
}

/// Zoomed-out Image › Adjustments preview on a large document: the wgpu compositor renders the
/// reduced preview document (`adjust_preview::gpu_proxy`) into its own texture, over the target's
/// area after the first frame. Returns (factor, gpu key, dimensions) to draw.
pub(super) fn ensure_adjust_proxy(app: &mut PhotocraftApp, idx: usize, zoom: f32) -> Option<(u32, u64, [u32; 2])> {
    let frame = crate::adjust_preview::gpu_proxy(app, idx, zoom)?;
    let key = frame.doc.id.0;
    let size = [frame.doc.size.width, frame.doc.size.height];
    let gpu = app.gpu.clone()?;
    if frame.stale || !gpu.has(key, size) {
        let doc = app.session.documents().get(idx)?.doc.clone();
        let (display, _) = canvas_display(app, &doc, None);
        let r = gpu.refresh(key, &frame.doc, frame.damage, display.as_deref());
        app.perf.record(if r.kind.starts_with("gpu") { "gpu-adjust-proxy" } else { "adjust-proxy" }, r.px, r.composite_ms, r.upload_ms);
        if let Some(t) = r.gpu {
            app.perf.record_gpu(t);
        }
        crate::adjust_preview::proxy_drawn(app, &frame);
    }
    Some((frame.k, key, size))
}

/// If a live adjustment preview is active on a large document, composite it on the proxy and upload
/// it under its own GPU key. Returns (factor, gpu key, dimensions) when the proxy should be drawn.
/// `zoom` is the view's (physical screen pixels per document pixel).
pub(super) fn ensure_proxy_preview(app: &mut PhotocraftApp, idx: usize, zoom: f32) -> Option<(u32, u64, [u32; 2])> {
    let (layer, params) = app.live_adjust.clone()?;
    let (doc_id, revision, doc) = {
        let st = app.session.documents().get(idx)?;
        (st.doc.id, st.revision, st.doc.clone())
    };
    // Low-resolution previews off: k is 1 and the canvas shows `display_doc` (full size).
    let mut k = crate::proxy::preview_factor(&doc, crate::proxy::reduced_previews(app));
    // Zoomed in, a proxy coarser than the screen showed the drag in k-pixel blocks (#5764): when
    // the wgpu compositor draws the full-size document, use a proxy only as coarse as the screen
    // (none at 50 % and closer). Otherwise the full size would be a CPU composite per change.
    let fine = crate::proxy::view_factor(k, zoom);
    if fine < k && app.gpu.as_ref().is_some_and(|g| g.supports(&doc)) {
        k = fine;
    }
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
        let preview = photocraft_engine::adjust_params::from_params(kind, &params, Some(a), proxy.mode).unwrap_or_else(|_| a.clone());
        let mut p = (*proxy).clone();
        if let Some(lm) = p.layer_mut(layer) {
            lm.content = LayerContent::Adjustment(preview);
        }
        let t0 = crate::gpu_canvas::now_ms();
        let buf = photocraft_compose::flatten(&p);
        let t1 = crate::gpu_canvas::now_ms();
        let (display, _) = canvas_display(app, &doc, None);
        app.gpu.as_ref()?.upload_buffer_full(key, &texture_buffer(display.as_deref(), &buf), doc.depth);
        app.perf.record("proxy", p.size.area(), t1 - t0, crate::gpu_canvas::now_ms() - t1);
        app.proxy_uploaded = Some((doc_id, hash));
    }
    Some((k, key, [proxy.size.width, proxy.size.height]))
}

/// A CPU composite as the GPU canvas texture stores it (sRGB-encoded for linear documents).
fn texture_buffer<'a>(
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
    buf: &'a photocraft_compose::Buffer,
) -> std::borrow::Cow<'a, photocraft_compose::Buffer> {
    match display {
        Some(d) => d.texture_buffer(buf),
        None => std::borrow::Cow::Borrowed(buf),
    }
}

pub(crate) fn retain_gpu_documents(app: &mut PhotocraftApp) {
    // Remove a closed adjustment owner's proxy before collecting its live GPU keys.
    crate::adjust_preview::retain_documents(app);
    if let Some(gpu) = &app.gpu {
        // Keep each document's texture plus its preview textures (filter preview, adjustment proxy);
        // retaining only document ids freed the previews every frame (blank canvas while previewing).
        let mut live: Vec<u64> = app.session.documents().iter().flat_map(|st| [st.doc.id.0, st.doc.id.0 ^ (1u64 << 61), st.doc.id.0 ^ (1u64 << 62)]).collect();
        live.extend(crate::adjust_preview::gpu_keys(app));
        gpu.retain(&live);
    }
    crate::background_redraw::retain(app);
    // Upload markers must not outlive the resources they describe: native reopen can reuse
    // both the document ID and revision before the next frame while a preview dialog stays open.
    let documents = app.session.documents();
    if app.filter_preview.as_ref().is_some_and(|preview| !documents.iter().any(|st| st.doc.id == preview.key.doc)) {
        app.filter_preview = None;
    }
    if app.proxy_uploaded.is_some_and(|(doc, _)| !documents.iter().any(|st| st.doc.id == doc)) {
        app.proxy_uploaded = None;
        app.proxy = None;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_on_two_displays() -> PhotocraftApp {
        use photocraft_engine::color_cmds::resolve_profile;
        use photocraft_engine::display_color::Display;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("edit.fill", json!({"color": "#cc8040"})).unwrap();
        let icc = |id: &str| Some(resolve_profile(id, None, None).unwrap().to_bytes());
        app.session.color.set_displays(Ok(vec![
            Display { id: 1, name: "A".into(), frame: [0.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("srgb") },
            Display { id: 4, name: "B".into(), frame: [100.0, 0.0, 100.0, 100.0], profile_name: None, icc: icc("display-p3") },
        ]));
        app
    }

    #[test]
    #[ignore = "manual texture-retention measurement on six 6 MP documents"]
    fn closed_canvas_memory_measurement() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.max_texture_side = 8192);
        for cycle in 1..=6 {
            app.run("file.new", json!({"width":3000,"height":2000})).unwrap();
            ensure_texture(&mut app, &ctx, 0, None).unwrap();
            // Drain uploaded image deltas just as a renderer does; count live texture owners.
            ctx.tex_manager().write().take_delta().clear();
            app.run("file.close", json!({})).unwrap();
            let bytes: usize = ctx.tex_manager().read().allocated().map(|(_, m)| m.bytes_used()).sum();
            println!("closed cycle={cycle} texture_bytes={bytes} rss={:?}", photocraft_testkit::perf::current_rss_bytes());
        }
    }

    #[test]
    fn closed_documents_release_navigator_but_keep_open_document_textures() {
        let mut app = app_on_two_displays();
        let ctx = egui::Context::default();
        let first = app.session.active().unwrap().doc.clone();
        let a = ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0;
        // The same handle ownership as navigator_texture, without requiring a GPU adapter.
        let tex = ctx.load_texture("navigator-test", egui::ColorImage::filled([64, 64], egui::Color32::WHITE), TextureOptions::LINEAR);
        let nav = tex.id();
        app.navigator_textures.insert(first.id, (std::sync::Arc::downgrade(&first), 0, 0, tex));
        app.run("file.new", json!({"width":32,"height":32})).unwrap();
        let other = app.session.active().unwrap().doc.id;
        let b = ensure_texture(&mut app, &ctx, 1, Some(1)).unwrap().0;
        drop(app.session.close(0));
        app.sync_views();
        assert!(ctx.tex_manager().read().meta(a).is_none());
        assert!(ctx.tex_manager().read().meta(nav).is_none());
        assert!(ctx.tex_manager().read().meta(b).is_some());
        assert!(app.canvases.keys().all(|(id, _)| *id == other));
        assert!(app.navigator_textures.is_empty());
        app.session.add_document((*first).clone(), None);
        app.sync_views();
        assert_ne!(ensure_texture(&mut app, &ctx, 1, Some(1)).unwrap().0, a, "reopening a preserved ID creates a fresh texture");
    }

    #[test]
    fn closed_documents_release_cpu_canvas_textures() {
        let mut app = app_on_two_displays();
        let ctx = egui::Context::default();
        let original = app.session.active().unwrap().doc.clone();
        for cycle in 0..12 {
            if cycle > 0 {
                app.session.add_document((*original).clone(), None);
                app.sync_views();
            }
            let a = ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0;
            let b = ensure_texture(&mut app, &ctx, 0, Some(4)).unwrap().0;
            assert_ne!(a, b);
            app.run("file.close", json!({})).unwrap();
            assert!(app.canvases.is_empty(), "closed canvas metadata retained at cycle {cycle}");
            assert!(ctx.tex_manager().read().meta(a).is_none(), "first display texture retained");
            assert!(ctx.tex_manager().read().meta(b).is_none(), "second display texture retained");
        }
    }

    #[test]
    fn cpu_canvas_textures_are_per_display() {
        // Two windows of one document on different displays: each has its own texture of
        // monitor values, and drawing them in turn re-renders neither.
        let mut app = app_on_two_displays();
        let ctx = egui::Context::default();
        let id = app.session.documents()[0].doc.id;
        let a = ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0;
        let b = ensure_texture(&mut app, &ctx, 0, Some(4)).unwrap().0;
        assert_ne!(a, b);
        app.perf.last_refresh = "";
        for _ in 0..3 {
            assert_eq!(ensure_texture(&mut app, &ctx, 0, Some(1)).unwrap().0, a);
            assert_eq!(ensure_texture(&mut app, &ctx, 0, Some(4)).unwrap().0, b);
        }
        assert_eq!(app.perf.last_refresh, "", "no re-render");
        let doc = app.session.documents()[0].doc.clone();
        assert!(canvas_display(&app, &doc, Some(1)).0.unwrap().is_identity());
        assert!(!canvas_display(&app, &doc, Some(4)).0.unwrap().is_identity());
        assert_ne!(app.canvases[&(id, 1)].tex_preview_key, app.canvases[&(id, 4)].tex_preview_key);
        // An edit updates both.
        app.run("edit.fill", json!({"color": "#2060c0"})).unwrap();
        ensure_texture(&mut app, &ctx, 0, Some(1));
        ensure_texture(&mut app, &ctx, 0, Some(4));
        let rev = app.session.documents()[0].revision;
        assert_eq!((app.canvases[&(id, 1)].tex_revision, app.canvases[&(id, 4)].tex_revision), (rev, rev));
    }

    #[test]
    fn cpu_canvas_shows_crisp_pixels_when_zoomed_in() {
        // #4810: above 100% the CPU canvas shows each pixel as a square, like the GPU canvas's
        // sharp nearest filter; below 100% it still filters. Both the first upload and a
        // damage-rect update carry the sampling, since the renderer takes it from each delta.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 2, "height": 2})).unwrap();
        let ctx = egui::Context::default();
        let tex = ensure_texture(&mut app, &ctx, 0, None).unwrap().0;
        let uploads = |ctx: &egui::Context| {
            let mut delta = ctx.tex_manager().write().take_delta();
            let seen = delta.set.get(&tex).into_iter().flatten().map(|d| (d.pos.is_some(), d.options)).collect::<Vec<_>>();
            delta.clear();
            seen
        };
        let crisp = TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR };
        assert_eq!(uploads(&ctx), vec![(false, crisp)]);
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap();
        ensure_texture(&mut app, &ctx, 0, None).unwrap();
        uploads(&ctx);
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        assert_eq!(ensure_texture(&mut app, &ctx, 0, None).unwrap().0, tex);
        assert_eq!(uploads(&ctx), vec![(true, crisp)], "a partial update keeps the crisp sampling");
    }

    /// A device that allows smaller textures than MAX_TEXTURE (2048 on some older GPUs and WebGL
    /// contexts, and on the fallback adapter the GPU tests sometimes get under load): the canvas
    /// texture is reduced to fit it. A 2600 px document was uploaded at full size, which egui
    /// asserts on and the backend can't create.
    #[test]
    fn cpu_canvas_texture_fits_the_device_limit() {
        for (side, (w, h), want) in [(2048, (2600, 1700), [1300, 850]), (2048, (5000, 2000), [1666, 666]), (8192, (5000, 2000), [2500, 1000])] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": w, "height": h})).unwrap();
            let ctx = egui::Context::default();
            ctx.input_mut(|i| i.max_texture_side = side);
            let (tex, _) = ensure_texture(&mut app, &ctx, 0, None).unwrap();
            let mut delta = ctx.tex_manager().write().take_delta();
            let sizes: Vec<_> = delta.set.get(&tex).into_iter().flatten().map(|d| d.image.size()).collect();
            delta.clear();
            assert_eq!(sizes, vec![want], "{w}x{h} on a {side} px device");
        }
    }

    #[test]
    fn cpu_canvas_of_a_reduced_document_stays_bilinear() {
        // Over MAX_TEXTURE the texture holds one texel per `factor` pixels: a texel is not a
        // document pixel, so showing it as a square would draw blocks that aren't in the image.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": MAX_TEXTURE + 1, "height": 2})).unwrap();
        let ctx = egui::Context::default();
        let (tex, scale) = ensure_texture(&mut app, &ctx, 0, None).unwrap();
        assert!(scale < 1.0);
        let mut delta = ctx.tex_manager().write().take_delta();
        let uploads: Vec<_> = delta.set.get(&tex).into_iter().flatten().map(|d| d.options).collect();
        delta.clear();
        assert_eq!(uploads, vec![TextureOptions::LINEAR]);
    }

    /// #7568: a document over the texture limit was drawn from one reduced texture, so at 100 %
    /// it was magnified and blurred. The view now gets the part in sight at one texel per
    /// document pixel, about the window's size, whatever the device's limit.
    #[test]
    fn cpu_canvas_detail_plan_shows_large_documents_at_full_resolution() {
        let size = [12000, 8000];
        // A 1920×1080 view in the middle (`visible_doc_rect` adds 2 px around it).
        let screen = DRect::new(4998, 2998, 6922, 4082);
        for limit in [16384, 8192, 4096, 2048] {
            let side = clamp_texture_side(limit);
            assert!(texture_factor(size, side) > 1, "the whole document is reduced on a {limit} px device");
            for zoom in [1.0, 2.0, 8.0] {
                let plan = detail_plan(size, side, zoom, screen).unwrap_or_else(|| panic!("{limit} px device at {zoom}"));
                assert_eq!(plan.factor, 1, "one texel per document pixel at {zoom} on a {limit} px device");
                assert!(plan.region.contains_rect(&screen), "{plan:?}");
                assert!(plan.region.width() <= 1924 + 2 * 256 && plan.region.height() <= 1084 + 2 * 256, "about the view's size: {plan:?}");
                assert!(plan.region.width() < side && plan.region.height() < side, "{plan:?}");
            }
        }
        // A small pan reuses the region (no new composite).
        assert_eq!(detail_plan(size, 4096, 1.0, screen), detail_plan(size, 4096, 1.0, screen.translate(8, -8)));
        // A window wider than a small device's limit coarsens rather than exceed it.
        let wide = DRect::new(0, 0, 3000, 2000);
        let plan = detail_plan(size, 2048, 1.0, wide).unwrap();
        assert_eq!(plan.factor, 2);
        assert!(plan.region.width() / 2 < 2048 && plan.region.contains_rect(&wide), "{plan:?}");
        // Zoomed out a little: finer than the reduced whole, as fine as the screen.
        let plan = detail_plan(size, 4096, 0.4, DRect::new(0, 0, 4800, 2700)).unwrap();
        assert_eq!(plan.factor, 2);
        // Zoomed out far enough, the reduced whole is as fine as the screen: nothing to add.
        let whole = DRect::new(0, 0, 12000, 8000);
        assert_eq!(detail_plan(size, 4096, 0.25, whole), None);
        assert_eq!(detail_plan(size, 4096, 1.0 / 3.0, whole), None, "a detail as large as the document would not fit");
        // A document that fits is uploaded whole at full resolution.
        assert_eq!(detail_plan([4000, 3000], 4096, 1.0, screen), None);
        // Nothing of the document in view, or a broken zoom.
        assert_eq!(detail_plan(size, 4096, 1.0, DRect::new(-500, -500, -10, -10)), None);
        for zoom in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(detail_plan(size, 4096, zoom, screen), None, "{zoom}");
        }
        assert_eq!(detail_plan([0, 0], 4096, 1.0, screen), None);
        assert_eq!(detail_plan([u32::MAX, 8], 4096, 1.0, screen), None);
    }

    #[test]
    fn cpu_canvas_detail_shows_a_large_documents_own_pixels_at_100_percent() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 3 * MAX_TEXTURE, "height": 8})).unwrap();
        app.run("select.rect", json!({"x": 101, "y": 0, "width": 1, "height": 8})).unwrap();
        app.run("edit.fill", json!({"color": "#000000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let ctx = egui::Context::default();
        ctx.input_mut(|i| i.max_texture_side = 16384);
        let uploads = |ctx: &egui::Context, tex: egui::TextureId| {
            let mut delta = ctx.tex_manager().write().take_delta();
            let seen: Vec<_> = delta.set.get(&tex).into_iter().flatten().map(|d| (d.pos, d.options, d.image.clone())).collect();
            delta.clear();
            seen
        };
        let row = |image: &egui::ImageData| {
            let egui::ImageData::Color(image) = image;
            (image.size, image.pixels.clone())
        };
        let view = InView { visible: DRect::new(0, 0, 1000, 8), zoom: 1.0, key: 1 };
        let t = ensure_texture_in_view(&mut app, &ctx, 0, None, Some(view)).unwrap();
        assert!(t.scale < 1.0, "the whole document is reduced");
        let (detail, area) = t.detail.expect("a detail texture at 100 %");
        assert_eq!(area, [0.0, 0.0, 1024.0, 8.0]);
        let crisp = TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR };
        let up = uploads(&ctx, detail);
        assert_eq!(up.len(), 1);
        let (pos, options, image) = &up[0];
        assert_eq!((*pos, *options), (None, crisp));
        let (size, px) = row(image);
        assert_eq!(size, [1024, 8]);
        assert_eq!((px[100], px[101], px[102]), (Color32::WHITE, Color32::BLACK, Color32::WHITE), "the 1 px line stays 1 px");
        // Unchanged: nothing is composited again, and a small pan keeps the region.
        let panned = InView { visible: DRect::new(4, 0, 1004, 8), ..view };
        assert_eq!(ensure_texture_in_view(&mut app, &ctx, 0, None, Some(panned)).unwrap().detail, Some((detail, area)));
        assert!(uploads(&ctx, detail).is_empty());
        // The Navigator's call (no view) leaves the view's detail alone.
        ensure_texture(&mut app, &ctx, 0, None).unwrap();
        assert_eq!(app.canvases.values().map(|c| c.detail.len()).sum::<usize>(), 1);
        // An edit in view updates just its texels.
        app.run("select.rect", json!({"x": 300, "y": 0, "width": 1, "height": 8})).unwrap();
        ensure_texture_in_view(&mut app, &ctx, 0, None, Some(view)).unwrap();
        uploads(&ctx, detail);
        app.run("edit.fill", json!({"color": "#000000"})).unwrap();
        assert_eq!(ensure_texture_in_view(&mut app, &ctx, 0, None, Some(view)).unwrap().detail.map(|d| d.0), Some(detail));
        let up = uploads(&ctx, detail);
        assert_eq!(up.len(), 1);
        let (pos, options, image) = &up[0];
        assert_eq!(*options, crisp);
        let [x, y] = pos.expect("a partial update");
        let (size, px) = row(image);
        assert!(y == 0 && x < 300 && x + size[0] > 300, "the edit's texels: {x} {size:?}");
        assert_eq!((px[299 - x], px[300 - x]), (Color32::WHITE, Color32::BLACK));
        // Zoomed out, the reduced texture is enough: the detail goes.
        let far = InView { zoom: 0.25, ..view };
        assert!(ensure_texture_in_view(&mut app, &ctx, 0, None, Some(far)).unwrap().detail.is_none());
        assert!(app.canvases.values().all(|c| c.detail.is_empty()));
    }

    #[test]
    fn gpu_display_luts_are_per_display() {
        let _gpu = photocraft_testkit::gpu_lock();
        let Ok(rs) = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let mut app = app_on_two_displays();
        app.set_wgpu(rs);
        let doc = app.session.documents()[0].doc.clone();
        let key = doc.id.0;
        // sRGB on the sRGB display needs no LUT; on the P3 display it does.
        assert_eq!(sync_display_lut(&mut app, &doc, key, Some(1)), 0);
        assert_eq!(sync_display_lut(&mut app, &doc, key, Some(4)), 3);
        let gpu = app.gpu.clone().unwrap();
        let (s1, s4) = (gpu.display_lut_signature(key, 1).unwrap(), gpu.display_lut_signature(key, 4).unwrap());
        assert_ne!(s1.0, s4.0);
        assert!(gpu.has_display_lut(key, 4) && !gpu.has_display_lut(key, 1));
        // Drawing one window leaves the other's LUT alone.
        for _ in 0..3 {
            assert_eq!(sync_display_lut(&mut app, &doc, key, Some(1)), 0);
            assert_eq!(sync_display_lut(&mut app, &doc, key, Some(4)), 3);
        }
        assert_eq!((gpu.display_lut_signature(key, 1), gpu.display_lut_signature(key, 4)), (Some(s1), Some(s4)));
        assert!(gpu.has_display_lut(key, 4));
        // The shared texture's key is the same for both displays.
        assert_eq!(texture_key(canvas_display(&app, &doc, Some(1)).0.as_deref()), texture_key(canvas_display(&app, &doc, Some(4)).0.as_deref()));
    }


}
