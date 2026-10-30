//! Channel display on the canvas: alpha / spot / Quick Mask colour overlays, single colour
//! channel views (grayscale) and hidden colour channels.
//!
//! The document composite (GPU or CPU) is untouched. When the Channels panel shows anything other
//! than the plain composite, one extra texture is drawn over it: transparent overlay pixels when
//! the composite stays visible, an opaque picture otherwise. The texture is keyed by document
//! revision and view state and updated per damage rectangle while painting, so nothing is
//! recomposited per frame. Output textures are per monitor; sampled composite pixels are shared.

use std::collections::HashMap;

use egui::{Color32, TextureOptions};
use photocraft_doc::{AlphaChannel, ColorMode, Document};
use photocraft_engine::channel_cmds::{ChannelView, color_count};
use photocraft_engine::mask_view_cmds::MaskViewMode;
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba, to_rgba};

use crate::PhotocraftApp;

/// Largest overlay texture side; bigger documents show a downsampled overlay.
const MAX_SIDE: u32 = 4096;
/// Limit the sampled float composite to 128 MiB across open documents.
const MAX_COMPOSITE_CACHE_PIXELS: usize = 8 * 1024 * 1024;

struct SampledComposite {
    revision: u64,
    /// `revision` minus the view-only part: what the pixels depend on.
    content: u64,
    preview_key: u64,
    factor: u32,
    pixels: Vec<[f32; 4]>,
}

struct CompositeSource<'a> {
    cached: Option<&'a [[f32; 4]]>,
    fresh: &'a mut Option<Vec<[f32; 4]>>,
    display: Option<&'a photocraft_engine::display_color::CanvasDisplay>,
}

pub struct Cache {
    source: Option<SampledComposite>,
    outputs: HashMap<Option<u32>, OutputCache>,
}

struct OutputCache {
    revision: u64,
    key: u64,
    display_key: u64,
    /// The canvas document shown by a live stroke (0 is the committed document).
    preview_key: u64,
    factor: u32,
    /// False while the plain composite shows: the texture is kept (so toggling a view back on
    /// reuses its allocation) but its pixels are stale.
    valid: bool,
    tex: egui::TextureHandle,
}

impl Cache {
    pub(crate) fn shown_as_document(&mut self, was_preview: impl Fn(u64) -> bool) {
        for output in self.outputs.values_mut() {
            if was_preview(output.preview_key) {
                output.preview_key = 0;
            }
        }
    }
}

fn sample(s: &Surface, r: Rect, factor: u32) -> Vec<f32> {
    let (w, h) = (r.width().div_ceil(factor), r.height().div_ceil(factor));
    if factor == 1 {
        let mut v = Vec::new();
        s.read_region_into(r, &mut v);
        let ch = s.channels();
        return if ch > 1 { v.chunks_exact(ch).map(|p| p[0]).collect() } else { v };
    }
    let mut out = Vec::with_capacity((w * h) as usize);
    let mut row = vec![0.0f32; w as usize];
    for y in 0..h {
        s.sample_row_strided(r.y0 + (y * factor) as i32, r.x0, factor as i32, 0, &mut row);
        out.extend_from_slice(&row);
    }
    out
}

/// How the view draws what it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Single colour channels tinted instead of grayscale (Photoshop's "Show Channels in Color").
    pub in_color: bool,
    /// The layer mask rubylith's colour and opacity (0..1): Layer Mask Display Options.
    pub mask_rgb: [f32; 3],
    pub mask_opacity: f32,
}

impl Default for Look {
    /// Photoshop's: grayscale channels, a red rubylith at 50 %.
    fn default() -> Self {
        Self { in_color: false, mask_rgb: AlphaChannel::DEFAULT_COLOR.to_rgb(), mask_opacity: 0.5 }
    }
}

impl Look {
    /// The current preferences of `s`.
    pub fn of(s: &photocraft_engine::Session) -> Self {
        let p = s.prefs();
        Self { in_color: p.interface.show_channels_in_color, mask_rgb: p.layer_mask_overlay.rgb(), mask_opacity: p.layer_mask_overlay.alpha() }
    }
}

/// Composite RGBA (over white, like Photoshop's channel views) at `factor` stride.
/// Render in bands instead of allocating a full-resolution float composite to sample a preview.
fn composite(doc: &Document, r: Rect, factor: u32) -> Vec<[f32; 4]> {
    let (w, h) = (r.width().div_ceil(factor), r.height().div_ceil(factor));
    let mut out = Vec::with_capacity((w * h) as usize);
    let f = factor as i32;
    let _ = photocraft_compose::render_bands(doc, r, 0, |band| -> Result<(), ()> {
        let first = band.rect.y0 + (r.y0 - band.rect.y0).rem_euclid(f);
        for y in (first..band.rect.y1).step_by(factor as usize) {
            for x in (r.x0..r.x1).step_by(factor as usize) {
                let p = band.get(x, y);
                let a = p[3].clamp(0.0, 1.0);
                out.push([p[0] * a + 1.0 - a, p[1] * a + 1.0 - a, p[2] * a + 1.0 - a, 1.0]);
            }
        }
        Ok(())
    });
    out
}

/// The channel view of `r` (document pixels, sampled every `factor` px) as premultiplied pixels,
/// or `None` when the plain composite is all there is to show.
pub fn render(doc: &Document, v: &ChannelView, r: Rect, factor: u32, look: Look, active: Option<photocraft_doc::LayerId>) -> Option<Vec<Color32>> {
    let mut unused = None;
    render_with_source(doc, v, r, factor, look, active, CompositeSource { cached: None, fresh: &mut unused, display: None })
}

/// As [`render`], but colour-manage the opaque colour-component base before screen-space alpha,
/// Quick Mask and layer-mask overlays are composited onto it.
pub(crate) fn render_for_display(
    doc: &Document,
    v: &ChannelView,
    r: Rect,
    factor: u32,
    look: Look,
    active: Option<photocraft_doc::LayerId>,
    display: Option<&photocraft_engine::display_color::CanvasDisplay>,
) -> Option<Vec<Color32>> {
    let mut unused = None;
    render_with_source(doc, v, r, factor, look, active, CompositeSource { cached: None, fresh: &mut unused, display })
}

fn render_with_source(
    doc: &Document,
    v: &ChannelView,
    r: Rect,
    factor: u32,
    look: Look,
    _active: Option<photocraft_doc::LayerId>,
    source: CompositeSource<'_>,
) -> Option<Vec<Color32>> {
    if v.is_plain(doc) || r.is_empty() {
        return None;
    }
    let in_color = look.in_color;
    let (mask_rgb, mask_opacity) = (look.mask_rgb, look.mask_opacity.clamp(0.0, 1.0));
    let layer_mask = v.shown_layer_mask(doc);
    // ⌥-click mask view (#196): the pixel mask alone, in grayscale (white reveals); ⇧⌥ with
    // nothing else shown: just the rubylith. Use the compositor's cached feather for the pixel
    // mask, but leave vector masks out: this view targets the pixel mask channel by itself.
    let feathered_mask = v.layer_mask.and_then(|view| {
        let layer = doc.layer(view.layer)?;
        let mask = photocraft_engine::mask_view_cmds::viewed_mask(layer, view.filter)?;
        if !mask.enabled || photocraft_compose::masks::feather_sigma(mask.feather) <= 0.0 {
            return None;
        }
        let mut pixel_mask_layer = layer.clone();
        pixel_mask_layer.mask = Some(mask.clone());
        pixel_mask_layer.vector_mask = None;
        photocraft_compose::masks::combined_mask(&pixel_mask_layer, doc.bounds())
    });
    let shown_mask = layer_mask.map(|(m, mode)| (feathered_mask.as_ref().unwrap_or(&m.surface), mode));
    let only_mask = ChannelView { layer_mask: None, ..v.clone() }.is_plain(doc);
    match shown_mask {
        Some((m, MaskViewMode::Gray)) => return Some(mask_px(m, r, factor, |g| Color32::from_gray(q(g)))),
        Some((m, MaskViewMode::Overlay)) if only_mask => {
            let c = mask_rgb;
            return Some(mask_px(m, r, factor, |g| {
                let a = mask_opacity * (1.0 - g.clamp(0.0, 1.0));
                Color32::from_rgba_premultiplied(q(c[0] * a), q(c[1] * a), q(c[2] * a), q(a))
            }));
        }
        _ => {}
    }
    let colors = color_count(doc);
    let vis = v.visible_colors(colors);
    let (w, h) = (r.width().div_ceil(factor), r.height().div_ceil(factor));
    let n = (w * h) as usize;
    let mut overlays: Vec<&AlphaChannel> = (0..doc.channels.len()).filter(|i| v.alpha_shown(*i)).map(|i| &doc.channels[i]).collect();
    let gray = |s: &Surface| sample(s, r, factor).into_iter().map(|g| [g, g, g, 1.0]).collect();
    // Premultiplied RGBA accumulator. With no colour channel left, the channel being edited
    // (alpha, Quick Mask, or layer mask) is the picture — grayscale, not a rubylith over nothing.
    let mut solo_quick = false;
    let mut solo_mask = false;
    let mut out: Vec<[f32; 4]> = if vis == 0 {
        if !overlays.is_empty() {
            let first = overlays.remove(0);
            gray(&first.surface)
        } else if let Some(q) = doc.quick_mask.as_ref().filter(|_| !v.quick_mask_hidden) {
            solo_quick = true;
            gray(&q.surface)
        } else if let Some((m, _)) = shown_mask {
            solo_mask = true;
            gray(m)
        } else {
            // Every eye may be off during a channel transform and after commit/cancel.
            // Cover the composite so hidden channels never leak through.
            vec![[1.0; 4]; n]
        }
    } else if vis < colors {
        let fmt = doc.pixel_format();
        let mode = fmt.mode;
        let shown: Vec<usize> = (0..colors).filter(|k| v.color_visible(*k)).collect();
        let computed = if source.cached.is_some_and(|px| px.len() == n) { None } else { Some(composite(doc, r, factor)) };
        let pixels = source.cached.filter(|px| px.len() == n).or(computed.as_deref())?;
        let converted = pixels
            .iter()
            .copied()
            .map(|rgba| {
                let mut native = from_rgba(&fmt, rgba);
                if shown.len() == 1 && !in_color {
                    let x = native[shown[0]];
                    // Ink channels read dark where there is ink.
                    let g = if mode == ColorMode::Cmyk { 1.0 - x } else { x };
                    return [g, g, g, 1.0];
                }
                for (k, x) in native[..colors].iter_mut().enumerate() {
                    if !v.color_visible(k) {
                        // Hidden channels contribute nothing (Lab: neutral a/b, and a neutral
                        // Lightness under Show Channels in Color, so an isolated a/b channel
                        // tints over mid gray instead of black, as Photoshop does).
                        *x = if mode == ColorMode::Lab && (k > 0 || in_color) { 0.5 } else { 0.0 };
                    }
                }
                let c = to_rgba(&fmt, &native);
                [c[0], c[1], c[2], 1.0]
            })
            .collect();
        if n <= MAX_COMPOSITE_CACHE_PIXELS {
            *source.fresh = computed;
        }
        converted
    } else {
        vec![[0.0; 4]; n]
    };
    if vis > 0
        && vis < colors
        && let Some(display) = source.display.filter(|display| !display.is_identity())
    {
        // Partial colour views are in the document's composite profile. Convert their base
        // before adding channel/mask overlays, whose UI colours are screen-space values.
        let converted = display.composite_pixels_to_rgba8(&out);
        for (pixel, converted) in out.iter_mut().zip(converted) {
            for (component, value) in pixel.iter_mut().zip(converted) {
                *component = f32::from(value) / 255.0;
            }
        }
    }
    // Rubylith: `color` at `opacity` × coverage over the accumulated pixels.
    let mut tint = |values: Vec<f32>, color: photocraft_color::Color, opacity: f32, coverage: &dyn Fn(f32) -> f32| {
        let c = color.to_rgb();
        for (o, val) in out.iter_mut().zip(values) {
            let a = (coverage(val.clamp(0.0, 1.0)) * opacity).clamp(0.0, 1.0);
            for j in 0..3 {
                o[j] = c[j] * a + o[j] * (1.0 - a);
            }
            o[3] = a + o[3] * (1.0 - a);
        }
    };
    let quick = doc.quick_mask.as_ref().filter(|_| !v.quick_mask_hidden && !solo_quick);
    for ch in overlays.into_iter().chain(quick) {
        let (color, opacity) = match ch.spot {
            // Spot inks show more solid as solidity rises.
            Some((ink, solidity)) => (ink, 0.5 + 0.5 * solidity),
            None => (ch.color, ch.opacity),
        };
        tint(sample(&ch.surface, r, factor), color, opacity, &|x| ch.overlay_coverage(x));
    }
    // ⇧⌥-click mask view (#196): the layer mask tinted over the hidden areas in the Layer Mask
    // Display Options colour and opacity (red at 50 % unless set).
    if !solo_mask && let Some((m, MaskViewMode::Overlay)) = shown_mask {
        let [cr, cg, cb] = mask_rgb;
        tint(sample(m, r, factor), photocraft_color::Color::rgb(cr, cg, cb), mask_opacity, &|x| 1.0 - x);
    }
    Some(out.into_iter().map(|p| Color32::from_rgba_premultiplied(q(p[0]), q(p[1]), q(p[2]), q(p[3]))).collect())
}

/// `r` of a one-channel surface sampled every `factor` px, mapped to colours row by row (in
/// parallel off the web).
fn mask_px(s: &Surface, r: Rect, factor: u32, color: impl Fn(f32) -> Color32 + Sync) -> Vec<Color32> {
    let (w, h) = (r.width().div_ceil(factor) as usize, r.height().div_ceil(factor) as usize);
    let mut px = vec![Color32::TRANSPARENT; w * h];
    let row = |y: usize, out: &mut [Color32]| {
        let mut vals = vec![0.0f32; out.len()];
        s.sample_row_strided(r.y0 + (y as u32 * factor) as i32, r.x0, factor as i32, 0, &mut vals);
        for (o, v) in out.iter_mut().zip(vals) {
            *o = color(v);
        }
    };
    if w == 0 {
        return px;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        px.par_chunks_mut(w).enumerate().for_each(|(y, out)| row(y, out));
    }
    #[cfg(target_arch = "wasm32")]
    for (y, out) in px.chunks_mut(w).enumerate() {
        row(y, out);
    }
    px
}

/// A float in 0..=1 as an 8-bit channel value.
fn q(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn view_key(v: &ChannelView, look: Look) -> u64 {
    let s = serde_json::to_string(&(v, look.mask_rgb, look.mask_opacity)).unwrap_or_default();
    s.bytes().fold(look.in_color as u64 + 1, |h, b| h.wrapping_mul(1_099_511_628_211).wrapping_add(b as u64))
}

/// Make the channel-view texture of document `idx` current on `output`.
pub fn ensure_for_display(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, output: Option<u32>) -> Option<egui::TextureId> {
    let st = app.session.documents().get(idx)?;
    let view = st.channel_view.clone();
    let active = st.active_layer;
    let (revision, last_damage) = (st.revision, st.last_damage);
    let content = revision.wrapping_sub(st.view_revision);
    let id = st.doc.id.0;
    if view.is_plain(&st.doc) {
        if let Some(c) = app.channel_views.get_mut(&id) {
            for output in c.outputs.values_mut() {
                output.valid = false;
            }
        }
        return None;
    }
    // Mask and channel views are drawn over the canvas: show the same live document,
    // otherwise an opaque grayscale mask hides the stroke until it is committed (#2078).
    // The plain-composite path above needs no preview work.
    let (doc, preview_key) = crate::canvas::filter_channel_preview(app, idx, ctx).unwrap_or_else(|| crate::canvas::display_doc(app, idx));
    let (display, display_key) = crate::canvas::canvas_display(app, &doc, output);
    // Interface ▸ Show Channels in Color tints single colour channel views on the canvas too,
    // not just the panel thumbnails (#2385). The view key carries the flag and the Layer Mask
    // Display Options, so changing them rebuilds the texture.
    let look = Look::of(&app.session);
    let key = view_key(&view, look);
    // Never exceed what the GPU accepts (egui panics on oversized textures).
    let max_side = (ctx.input(|i| i.max_texture_side) as u32).clamp(256, MAX_SIDE);
    let factor = doc.size.width.max(doc.size.height).div_ceil(max_side).max(1);
    // A live preview can mutate the same Arc in place. Content revision and preview_key, not
    // pointer identity, determine whether these flattened pixels are still current.
    if let Some(c) = app.channel_views.get_mut(&id) {
        if let Some(s) = c.source.as_mut().filter(|s| s.factor == factor && s.content == content && s.preview_key == preview_key) {
            s.revision = revision;
        } else {
            c.source = None;
        }
    }
    if let Some(c) = app.channel_views.get(&id).and_then(|c| c.outputs.get(&output))
        && c.key == key
        && c.display_key == display_key
        && c.factor == factor
        && c.valid
        && c.revision == revision
        && c.preview_key == preview_key
    {
        return Some(c.tex.id());
    }
    let damage = app
        .channel_views
        .get(&id)
        .and_then(|c| c.outputs.get(&output))
        .and_then(|c| crate::canvas::damage_since(app, idx, (c.revision, c.preview_key), (revision, preview_key), 0, last_damage));
    if let Some(c) = app.channel_views.get_mut(&id).and_then(|c| c.outputs.get_mut(&output))
        && c.key == key
        && c.display_key == display_key
        && c.factor == factor
        && c.valid
        && let Some(d) = damage
    {
        // Re-render just the damage, snapped to the sampling grid so a downsampled texture
        // (big documents) updates in place too.
        let f = factor as i32;
        let d = d.intersect(&doc.bounds());
        let r = Rect::new(d.x0.div_euclid(f) * f, d.y0.div_euclid(f) * f, d.x1, d.y1).intersect(&doc.bounds());
        if !r.is_empty()
            && let Some(px) = render_for_display(&doc, &view, r, factor, look, active, display.as_deref())
        {
            let img = egui::ColorImage::new([r.width().div_ceil(factor) as usize, r.height().div_ceil(factor) as usize], px);
            c.tex.set_partial([(r.x0 / f) as usize, (r.y0 / f) as usize], img, TextureOptions::LINEAR);
        }
        c.revision = revision;
        c.preview_key = preview_key;
        return Some(c.tex.id());
    }
    let b = doc.bounds();
    let cached = app.channel_views.get(&id).and_then(|c| c.source.as_ref()).map(|s| s.pixels.as_slice());
    let mut fresh = None;
    let px = render_with_source(&doc, &view, b, factor, look, active, CompositeSource { cached, fresh: &mut fresh, display: display.as_deref() })?;
    let new_source = fresh.is_some();
    let size = [b.width().div_ceil(factor) as usize, b.height().div_ceil(factor) as usize];
    let img = egui::ColorImage::new(size, px);
    let cache = app.channel_views.entry(id).or_insert_with(|| Cache { source: None, outputs: HashMap::new() });
    match cache.outputs.get_mut(&output) {
        Some(c) if c.tex.size() == size => {
            c.tex.set(img, TextureOptions::LINEAR);
            c.revision = revision;
            c.preview_key = preview_key;
            c.key = key;
            c.display_key = display_key;
            c.factor = factor;
            c.valid = true;
        }
        _ => {
            let tex = ctx.load_texture(format!("channel-view-{id}-{output:?}"), img, TextureOptions::LINEAR);
            cache.outputs.insert(output, OutputCache { revision, key, display_key, preview_key, factor, valid: true, tex });
        }
    }
    if let Some(pixels) = fresh.take() {
        cache.source = Some(SampledComposite { revision, content, preview_key, factor, pixels });
    }
    if new_source {
        for (other_id, c) in &mut app.channel_views {
            if *other_id != id {
                c.source = None;
            }
        }
    }
    app.channel_views.get(&id).and_then(|c| c.outputs.get(&output)).map(|c| c.tex.id())
}

#[cfg(test)]
pub(crate) fn ensure(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize) -> Option<egui::TextureId> {
    ensure_for_display(app, ctx, idx, None)
}

#[cfg(test)]
mod tests;
