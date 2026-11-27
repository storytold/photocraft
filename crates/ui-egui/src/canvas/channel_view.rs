//! Channel display on the canvas: alpha / Quick Mask colour overlays, spot inks printed over the
//! composite, single colour channel views (grayscale) and hidden colour channels.
//!
//! The document composite (GPU or CPU) is untouched. When the Channels panel shows anything other
//! than the plain composite, one extra picture is drawn over it: transparent overlay pixels when
//! the composite stays visible, an opaque picture with hidden colour channels. Visible spot
//! channels of an RGB or CMYK document print over the sampled composite where they have ink
//! (opaque, through the canvas display transform); elsewhere the picture stays transparent, so the
//! normal canvas shows. This is a screen approximation, not a press proof: each ink is its stored
//! display colour blended by the Multichannel overprint formula (multiply at solidity 0, cover at
//! solidity 1) over the sampled composite. A solo spot (all colour channels hidden) shows as its
//! grayscale values instead. The canvas draws it as a raster source (`raster_source::ChannelOverlay`),
//! which redraws only the damage of each change.

use egui::Color32;
use photocraft_color::PixelFormat;
use photocraft_doc::{AlphaChannel, ColorMode, Document};
use photocraft_engine::channel_cmds::{ChannelView, color_count};
use photocraft_engine::color_cmds::ColorExt;
use photocraft_engine::mask_view_cmds::MaskViewMode;
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba, to_rgba};

use crate::PhotocraftApp;

struct CompositeSource<'a> {
    /// What printed spot inks go through on their way to the screen.
    screen: &'a Screen,
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

/// What the canvas does to composite values on their way to the screen: the sRGB encoding of
/// linear composites, then the monitor transform (with the soft proof under View › Proof Colors).
/// Spot inks printed over the composite go through it, so they match the canvas around them.
#[derive(Clone, Default)]
pub struct Screen {
    pub encode_srgb: bool,
    pub transform: Option<std::sync::Arc<craft::cms::Transform>>,
}

impl Screen {
    /// How the canvas shows `doc` on `display` (`None`: the main window's).
    pub fn of(app: &PhotocraftApp, doc: &Document, display: Option<u32>) -> Screen {
        let color = app.session.color();
        let display = display.or(color.main_display);
        let Ok(d) = color.canvas_display_for(doc, display) else { return Screen::default() };
        let proof = color.proof_ref(doc.id).is_some_and(|p| p.enabled);
        let transform = if proof || d.transform.is_some() { color.display_transform_for(doc, display).ok() } else { None };
        Screen { encode_srgb: d.encode_srgb, transform }
    }

    /// A key that changes with [`Screen::of`].
    pub fn key(app: &PhotocraftApp, doc: &Document, display: Option<u32>) -> u64 {
        let color = app.session.color();
        color.display_signature_for(doc, display.or(color.main_display))
    }

    /// Opaque composite pixels `px` as monitor values.
    fn apply(&self, px: &mut [[f32; 4]]) {
        if self.encode_srgb {
            for p in px.iter_mut() {
                for v in &mut p[..3] {
                    *v = photocraft_color::convert::linear_to_srgb(v.clamp(0.0, 1.0));
                }
            }
        }
        if let Some(t) = &self.transform {
            // The display transform is RGB → RGB; were it refused, the pixels stay unconverted.
            let _ = t.apply(px.as_flattened_mut(), 4);
        }
    }
}

/// The proxy [`composite`] and [`inks`] sample the whole canvas from at `factor` (#10249): it
/// picks the same pixels (`downsample` keeps every factor-th one) for ~1/factor² of the
/// compositing. `None` unless `r` is the whole canvas and a proxy composites the same.
fn proxy(doc: &Document, r: Rect, factor: u32) -> Option<Document> {
    if factor <= 1 || r != doc.bounds() || !proxy_matches(doc) {
        return None;
    }
    // Only Multichannel composites read the extra channels; the selection and Quick Mask never
    // show in a composite. Left out, they cost nothing to shrink.
    let mut layers_only = doc.clone();
    if doc.mode != ColorMode::Multichannel {
        layers_only.channels.clear();
    }
    layers_only.selection = None;
    layers_only.quick_mask = None;
    Some(photocraft_compose::proxy::proxy_document(&layers_only, factor))
}

/// Composite RGBA (over white, like Photoshop's channel views) at `factor` stride.
/// Render in bands instead of allocating a full-resolution float composite to sample a preview.
fn composite(doc: &Document, r: Rect, factor: u32) -> Vec<[f32; 4]> {
    let (w, h) = (r.width().div_ceil(factor), r.height().div_ceil(factor));
    if let Some(p) = proxy(doc, r, factor) {
        let mut out = Vec::with_capacity((w * h) as usize);
        let _ = photocraft_compose::render_bands(&p, Rect::new(0, 0, w as i32, h as i32), 0, |band| -> Result<(), ()> {
            out.extend(band.px.iter().map(|p| {
                let a = p[3].clamp(0.0, 1.0);
                [p[0] * a + 1.0 - a, p[1] * a + 1.0 - a, p[2] * a + 1.0 - a, 1.0]
            }));
            Ok(())
        });
        if out.len() == (w * h) as usize {
            return out;
        }
    }
    composite_sampled(doc, r, factor)
}

/// A CMYK document's inks (see [`ink_samples`]) of `r` at `factor` stride, from the [`proxy`]
/// when there is one.
fn inks(doc: &Document, r: Rect, factor: u32) -> Vec<[f32; 4]> {
    if let Some(p) = proxy(doc, r, factor) {
        let (w, h) = (r.width().div_ceil(factor) as i32, r.height().div_ceil(factor) as i32);
        let (xs, ys): (Vec<i32>, Vec<i32>) = ((0..w).collect(), (0..h).collect());
        return ink_samples(&p, &xs, &ys);
    }
    let f = factor as usize;
    let xs: Vec<i32> = (r.x0..r.x1).step_by(f).collect();
    let ys: Vec<i32> = (r.y0..r.y1).step_by(f).collect();
    ink_samples(doc, &xs, &ys)
}

/// The C, M, Y and K inks of `doc` flattened onto white paper (no ink) at document pixels `xs` ×
/// `ys` (both ascending), row by row. These are the document's own inks, as Photoshop's channel
/// views show them, not the composite separated again from RGB: a 100/100/100/0 black has no K
/// (#8359). Composited in bands, so no full-size ink composite is held.
pub(crate) fn ink_samples(doc: &Document, xs: &[i32], ys: &[i32]) -> Vec<[f32; 4]> {
    let mut out = Vec::with_capacity(xs.len() * ys.len());
    let (Some(&x0), Some(&x1)) = (xs.first(), xs.last()) else {
        return out;
    };
    let fmt = PixelFormat { alpha: false, ..doc.pixel_format() };
    let rows = photocraft_compose::band_rows_for((x1 - x0 + 1) as u32, 0);
    let mut row = Vec::new();
    let mut i = 0;
    while i < ys.len() {
        let y0 = ys[i];
        let end = i + ys[i..].iter().take_while(|&&y| y < y0.saturating_add(rows)).count();
        let band = Rect::new(x0, y0, x1 + 1, ys[end - 1] + 1);
        let inks = photocraft_compose::composite_to_surface(doc, band, fmt, Some([1.0; 3]));
        for &y in &ys[i..end] {
            inks.read_region_into(Rect::new(x0, y, x1 + 1, y + 1), &mut row);
            out.extend(xs.iter().map(|&x| {
                let p = &row[(x - x0) as usize * 4..];
                [p[0], p[1], p[2], p[3]]
            }));
        }
        i = end;
    }
    out
}

/// Whether `v` shows some but not all colour channels of a CMYK document: that view reads the
/// document's inks (see [`inks`]), every other one the composite.
fn shows_inks(doc: &Document, v: &ChannelView) -> bool {
    let colors = color_count(doc);
    let vis = v.visible_colors(colors);
    doc.mode == ColorMode::Cmyk && vis > 0 && vis < colors
}

/// [`composite`] sampled from the full-resolution composite, rendered in bands.
fn composite_sampled(doc: &Document, r: Rect, factor: u32) -> Vec<[f32; 4]> {
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

/// The samples of `r` (see [`composite`], or [`inks`] when `ink`).
fn composite_px(doc: &Document, r: Rect, factor: u32, ink: bool) -> Vec<[f32; 4]> {
    if ink { inks(doc, r, factor) } else { composite(doc, r, factor) }
}

/// The samples of `r` (see [`composite`]) at the grid indices `inked` (ascending), in that order.
/// A few inked pixels of a big document render just their bounding box; when that costs no less
/// than the whole composite (a proxy renders it at about 1/factor² of the pixels), the whole one
/// is used.
fn composite_at(doc: &Document, r: Rect, factor: u32, inked: &[usize]) -> Vec<[f32; 4]> {
    let w = r.width().div_ceil(factor) as usize;
    let (Some(&first), Some(&last)) = (inked.first(), inked.last()) else { return Vec::new() };
    if w == 0 {
        return Vec::new();
    }
    // The inked samples' bounding box, in samples, then in document pixels (on the same grid).
    let (bx0, bx1) = inked.iter().fold((usize::MAX, 0), |(lo, hi), &k| (lo.min(k % w), hi.max(k % w + 1)));
    let (by0, by1) = (first / w, last / w + 1);
    let f = factor as usize;
    let at = |o: i32, s: usize| o.saturating_add((s * f) as i32);
    let area = Rect::new(at(r.x0, bx0), at(r.y0, by0), at(r.x0, bx1 - 1).saturating_add(1).min(r.x1), at(r.y0, by1 - 1).saturating_add(1).min(r.y1));
    let proxy = factor > 1 && r == doc.bounds() && proxy_matches(doc);
    let whole_cost = r.size().area() / if proxy { u64::from(factor) * u64::from(factor) } else { 1 };
    if area.size().area() < whole_cost {
        let (px, bw) = (composite_sampled(doc, area, factor), bx1 - bx0);
        return inked.iter().filter_map(|&k| px.get((k / w - by0) * bw + k % w - bx0).copied()).collect();
    }
    let px = composite_px(doc, r, factor, false);
    inked.iter().filter_map(|&k| px.get(k).copied()).collect()
}

/// Whether `v` prints spot inks of `doc` over its composite, through the [`Screen`].
fn prints_inks(doc: &Document, v: &ChannelView) -> bool {
    let colors = color_count(doc);
    v.visible_colors(colors) == colors && (0..doc.channels.len()).any(|i| v.alpha_shown(i) && photocraft_compose::multichannel::is_overprint_spot(doc, i))
}

/// Whether a proxy composite picks the full composite's pixels: no layer effects or vector masks
/// ([`proxy_faithful`](photocraft_compose::proxy::proxy_faithful)), no feathered masks, whose
/// radius is in document pixels, and no pattern fills, whose tiles keep their size on a proxy
/// unless a cache of the same fill is shrunk with it. The ink thumbnails use it too.
pub(crate) fn proxy_matches(doc: &Document) -> bool {
    use photocraft_doc::{Fill, LayerContent};
    // A stack, not recursion: groups can nest deeply. Hidden layers (and what they hold) are not
    // composited, so they don't count.
    let mut stack: Vec<&photocraft_doc::Layer> = doc.layers.iter().filter(|l| l.visible).collect();
    while let Some(l) = stack.pop() {
        if photocraft_compose::masks::has_feather(l) {
            return false;
        }
        if let LayerContent::Fill(fill @ Fill::Pattern { .. }) = &l.content
            && l.fill_cache.as_ref().is_none_or(|c| c.fill != *fill)
        {
            return false;
        }
        if let LayerContent::Group(g) = &l.content {
            stack.extend(g.children.iter().filter(|c| c.visible));
        }
    }
    photocraft_compose::proxy::proxy_faithful(doc)
}

/// The channel view of `r` (document pixels, sampled every `factor` px) as premultiplied pixels,
/// or `None` when the plain composite is all there is to show.
pub fn render(doc: &Document, v: &ChannelView, r: Rect, factor: u32, look: Look, active: Option<photocraft_doc::LayerId>) -> Option<Vec<Color32>> {
    render_with_source(doc, v, r, factor, look, active, CompositeSource { screen: &Screen::default() })
}

fn render_with_source(
    doc: &Document,
    v: &ChannelView,
    r: Rect,
    factor: u32,
    look: Look,
    active: Option<photocraft_doc::LayerId>,
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
    let feathered_mask = v
        .layer_mask
        .and_then(|view| doc.layer(view.layer))
        .filter(|layer| layer.mask.as_ref().is_some_and(|mask| mask.enabled && photocraft_compose::masks::feather_sigma(mask.feather) > 0.0))
        .and_then(|layer| {
            let mut pixel_mask_layer = layer.clone();
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
    let mut overlays: Vec<usize> = (0..doc.channels.len()).filter(|i| v.alpha_shown(*i)).collect();
    let gray = |s: &Surface| sample(s, r, factor).into_iter().map(|g| [g, g, g, 1.0]).collect();
    // Spot inks preview with Color Settings › Spot's dot gain, as Photoshop shows them.
    let gain = photocraft_compose::multichannel::spot_dot_gain();
    // Visible spot channels of an RGB or CMYK document print over the picture as inks, in panel
    // order, before the alpha channel and Quick Mask tints.
    let spots: Vec<(photocraft_compose::multichannel::Ink, Vec<f32>)> = if vis > 0 {
        let spot_ids: Vec<usize> = overlays.iter().copied().filter(|&i| photocraft_compose::multichannel::is_overprint_spot(doc, i)).collect();
        overlays.retain(|i| !spot_ids.contains(i));
        // The ink colours are converted once per render, and only when there are inks to print.
        let space = (!spot_ids.is_empty()).then(|| photocraft_compose::multichannel::InkSpace::of(doc));
        space.iter().flat_map(|space| spot_ids.iter().filter_map(|&i| doc.channels.get(i)).map(|ch| (space.ink(ch), sample(&ch.surface, r, factor)))).collect()
    } else {
        Vec::new()
    };
    // Over the whole composite, spot inks print (#10249) only where they have ink.
    let inked: Vec<usize> = if vis == colors && !spots.is_empty() {
        (0..n).filter(|&k| spots.iter().any(|(_, vals)| vals.get(k).is_some_and(|v| *v > 0.0))).collect()
    } else {
        Vec::new()
    };
    // No ink, and no alpha channel, Quick Mask or layer mask shown over the composite: nothing to
    // draw, so no all-transparent texture either.
    if vis == colors && inked.is_empty() && overlays.is_empty() && (doc.quick_mask.is_none() || v.quick_mask_hidden) && shown_mask.is_none() {
        return None;
    }
    // Premultiplied RGBA accumulator. With no colour channel left, the channel being edited
    // (alpha, spot, Quick Mask, or layer mask) is the picture — grayscale, not a rubylith over nothing.
    let mut solo_quick = false;
    let mut solo_mask = false;
    let mut out: Vec<[f32; 4]> = if vis == 0 {
        if !overlays.is_empty() {
            let first = overlays.remove(0);
            gray(&doc.channels.get(first)?.surface)
        } else if let Some(q) = doc.quick_mask.as_ref().filter(|_| !v.quick_mask_hidden) {
            solo_quick = true;
            gray(&q.surface)
        } else if let Some((m, _)) = shown_mask {
            solo_mask = true;
            gray(m)
        } else {
            let m = active.and_then(|id| doc.layer(id)).and_then(|l| l.mask.as_ref())?;
            solo_mask = true;
            gray(&m.surface)
        }
    } else if vis < colors {
        let fmt = doc.pixel_format();
        let mode = fmt.mode;
        let shown: Vec<usize> = (0..colors).filter(|k| v.color_visible(*k)).collect();
        let ink = shows_inks(doc, v);
        let pixels = composite_px(doc, r, factor, ink);
        // The inks show through the document's own CMYK profile, as in its composite.
        let space = photocraft_compose::cmyk_space(doc);
        photocraft_color::convert::with_cmyk_space(space.as_ref(), || {
            pixels
                .iter()
                .map(|&px| {
                    // CMYK samples are inks already (opaque: flattened onto paper).
                    let mut native = if ink { px.into_iter().chain(fmt.alpha.then_some(1.0)).collect() } else { from_rgba(&fmt, px) };
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
                            // tints over mid gray instead of black, as the reference editor does).
                            *x = if mode == ColorMode::Lab && (k > 0 || in_color) { 0.5 } else { 0.0 };
                        }
                    }
                    let c = to_rgba(&fmt, &native);
                    [c[0], c[1], c[2], 1.0]
                })
                .collect()
        })
    } else {
        vec![[0.0; 4]; n]
    };
    if !inked.is_empty() {
        // The inked pixels become the opaque printed picture, the rest stay transparent so the
        // normal canvas (display LUT, proofing, full resolution) shows. Only the inked area of the
        // composite is rendered.
        let mut printed = composite_at(doc, r, factor, &inked);
        if printed.len() == inked.len() {
            for (ink, vals) in &spots {
                ink.apply_with_gain(&mut printed, inked.iter().map(|&k| vals.get(k).copied().unwrap_or(0.0)), gain);
            }
            source.screen.apply(&mut printed);
            for (&k, p) in inked.iter().zip(printed) {
                if let Some(o) = out.get_mut(k) {
                    *o = p;
                }
            }
        }
    } else if vis < colors {
        for (ink, vals) in &spots {
            ink.apply_with_gain(&mut out, vals.iter().copied(), gain);
        }
    }
    let overlays: Vec<&AlphaChannel> = overlays.into_iter().filter_map(|i| doc.channels.get(i)).collect();
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
        // Spot channels reach here only in Multichannel documents, whose composite already prints
        // them: they show more solid as solidity rises.
        let (color, opacity) = match ch.spot {
            Some((ink, solidity)) => (ink, 0.5 + 0.5 * solidity),
            None => (ch.color, ch.opacity),
        };
        if ch.spot.is_some() {
            tint(sample(&ch.surface, r, factor), color, opacity, &|x| photocraft_compose::multichannel::dot_gain(ch.overlay_coverage(x), gain));
        } else {
            tint(sample(&ch.surface, r, factor), color, opacity, &|x| ch.overlay_coverage(x));
        }
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
pub(crate) fn mask_px(s: &Surface, r: Rect, factor: u32, color: impl Fn(f32) -> Color32 + Sync) -> Vec<Color32> {
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
    // Spot overlays follow Color Settings › Spot: a new dot gain redraws them.
    let seed = (look.in_color as u64 + 1) ^ u64::from(photocraft_compose::multichannel::spot_dot_gain().to_bits()) << 8;
    s.bytes().fold(seed, |h, b| h.wrapping_mul(1_099_511_628_211).wrapping_add(b as u64))
}

/// A channel or mask view as the canvas's raster source draws it over the composite, for any region at any factor.
pub(crate) struct Overlay {
    view: ChannelView,
    look: Look,
    active: Option<photocraft_doc::LayerId>,
    screen: Screen,
    /// Changes with the view, its look and, for printed spot inks, the display transform.
    pub(crate) key: u64,
}

impl Overlay {
    /// The view document `idx` shows of `doc` (its committed or previewed composite) on `display` (`None`: the main
    /// window's); `None` while the plain composite shows.
    pub(crate) fn of(app: &PhotocraftApp, idx: usize, doc: &Document, display: Option<u32>) -> Option<Overlay> {
        let st = app.session.documents().get(idx)?;
        let view = st.channel_view.clone();
        if view.is_plain(&st.doc) {
            return None;
        }
        let look = Look::of(&app.session);
        let inks = prints_inks(doc, &view);
        let (screen, screen_key) = if inks { (Screen::of(app, doc, display), Screen::key(app, doc, display)) } else { (Screen::default(), 0) };
        let key = view_key(&view, look) ^ screen_key.rotate_left(17);
        Some(Overlay { view, look, active: st.active_layer, screen, key })
    }

    /// The view's pixels over `r` of `doc`, one sample every `factor` pixels from `r`'s origin, row by row
    /// (`r`'s size divided by `factor`, rounded up); `None` where it draws nothing.
    pub(crate) fn render(&self, doc: &Document, r: Rect, factor: u32) -> Option<Vec<Color32>> {
        render_with_source(doc, &self.view, r, factor, self.look, self.active, CompositeSource { screen: &self.screen })
    }
}

#[cfg(test)]
mod tests;
