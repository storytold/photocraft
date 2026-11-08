//! Histograms for the tone editors (Levels, Curves, Threshold) and the Histogram panel.
//!
//! The adjustment editors themselves live in `adjust_editors` and the panel in `histogram_panel`;
//! this module computes and caches the histograms they draw, in the channels of the adjustment's
//! space (RGB, CMYK ink brightness, Lab), and draws them.

use std::sync::Arc;

use egui::{Color32, Rect};
use photocraft_doc::adjust::ToneSpace;
use photocraft_doc::{Document, Layer, LayerId};

use crate::PhotocraftApp;
use crate::theme::Tokens;

static EMPTY: [u32; 256] = [0; 256];

/// 256-bin histograms: the composite then one per channel of the space (RGB: the channel mean, R, G, B;
/// CMYK: composite, C, M, Y, K as channel brightness; Lab: L, a, b). Native partial selections
/// ([`HistSource::Colors`]) always use combined, then native components, including Lab.
/// `tag` identifies what they were computed from (see [`HistSource`]).
#[derive(Clone, Debug)]
pub struct Histograms {
    pub tag: u64,
    pub ch: Vec<[u32; 256]>,
    /// Luminosity of the composite (0.30 R + 0.59 G + 0.11 B), whatever the space;
    /// empty for a gray plane's.
    pub lum: [u32; 256],
    /// Photoshop's histogram cache level of a composite histogram: 1 when every document pixel
    /// was counted, `n` when a proxy 2^(n-1) times smaller stood in; 0 for a gray plane's.
    pub level: u32,
}

impl Default for Histograms {
    fn default() -> Self {
        Self { tag: 0, ch: Vec::new(), lum: [0; 256], level: 0 }
    }
}

impl std::ops::Index<usize> for Histograms {
    type Output = [u32; 256];
    fn index(&self, i: usize) -> &[u32; 256] {
        self.ch.get(i).unwrap_or(&EMPTY)
    }
}

/// What a tone editor's histogram shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistSource {
    /// The image below an adjustment layer (what the adjustment receives).
    BelowLayer(LayerId),
    /// Native histograms of a partial colour selection, combined then individual.
    Colors { layer: LayerId, colors: photocraft_engine::color_channels::ColorChannels },
    /// One layer on its own (the target of a destructive Image › Adjustments command).
    Layer(LayerId),
    /// The gray plane a Curves target edits: a layer mask, a saved channel or the Quick Mask.
    /// The dialog shows this plane's own levels, not the colour layers beneath (#5335).
    Channel { layer: LayerId, surface: photocraft_engine::channel_cmds::GraySurface },
    /// A layer and everything beneath it, the layers above hidden (the Histogram panel's
    /// Adjustment Composite: what an adjustment layer outputs).
    UpTo(LayerId),
}

/// The whole composited image (the Histogram panel's Entire Image).
pub const IMAGE: HistSource = HistSource::BelowLayer(LayerId(u64::MAX));

/// The highest histogram cache level: a proxy 2^7 = 128 times smaller.
pub const MAX_CACHE_LEVEL: u32 = 8;

pub(crate) fn tag(source: HistSource, space: ToneSpace) -> u64 {
    let (k, id) = match source {
        HistSource::BelowLayer(l) => (1u64, l.0),
        HistSource::Colors { layer, colors } => (8, layer.0 ^ colors.indices().fold(0u64, |m, k| m | (1u64 << k))),
        HistSource::Layer(l) => (2, l.0),
        HistSource::Channel { layer, surface } => match surface {
            photocraft_engine::channel_cmds::GraySurface::LayerMask => (3, layer.0),
            photocraft_engine::channel_cmds::GraySurface::Alpha(i) => (4, i as u64),
            photocraft_engine::channel_cmds::GraySurface::QuickMask => (5, layer.0),
            photocraft_engine::channel_cmds::GraySurface::FilterMask => (6, layer.0),
        },
        HistSource::UpTo(l) => (7, l.0),
    };
    let s = match space {
        ToneSpace::Rgb => 0u64,
        ToneSpace::Cmyk => 1,
        ToneSpace::Lab => 2,
    };
    id.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (k << 60) ^ (s << 56)
}

/// Shows only the branch of the layer tree holding `id`. Returns whether `id` is in `layers`.
fn isolate(layers: &mut [Layer], id: LayerId) -> bool {
    let mut found = false;
    for l in layers.iter_mut() {
        let here = l.id == id || l.children_mut().is_some_and(|c| isolate(c, id));
        l.visible = here;
        found |= here;
    }
    found
}

/// Hides every layer composited above `id` (later siblings, and the later siblings of each group
/// holding it). Returns whether `id` is in `layers`.
fn hide_above(layers: &mut [Layer], id: LayerId) -> bool {
    let mut found = false;
    for l in layers.iter_mut() {
        if found {
            l.visible = false;
        } else if l.id == id || l.children_mut().is_some_and(|c| hide_above(c, id)) {
            found = true;
        }
    }
    found
}

/// Histograms of `doc` as seen by `source`, in the channels of `space`, at whatever cache level
/// the document's size calls for (up to [`MAX_CACHE_LEVEL`]).
pub fn compute_histograms(doc: &Document, source: HistSource, space: ToneSpace) -> Histograms {
    compute_histograms_at(doc, source, space, MAX_CACHE_LEVEL)
}

/// [`compute_histograms`] at cache level `max_level` or below (Preferences › Performance › Cache
/// Levels; 1 counts every pixel, the Histogram panel's Uncached Refresh).
pub fn compute_histograms_at(doc: &Document, source: HistSource, space: ToneSpace, max_level: u32) -> Histograms {
    let mut level = 0;
    let mut lum = [0u32; 256];
    let mut h = vec![
        [0u32; 256];
        match space {
            ToneSpace::Rgb => 4,
            ToneSpace::Cmyk => 5,
            ToneSpace::Lab => 3,
        }
    ];
    match source {
        HistSource::Colors { layer, colors } => {
            h = vec![[0; 256]; 1 + doc.pixel_format().mode.color_channels()];
            if let Some(surface) = doc.layer(layer).and_then(Layer::surface) {
                for k in colors.indices() {
                    {
                        let bins = component_bins(surface, doc.bounds(), k);
                        if let Some(channel) = h.get_mut(k + 1) {
                            *channel = bins;
                        }
                        if let Some(combined) = h.first_mut() {
                            for (a, b) in combined.iter_mut().zip(bins) {
                                *a = a.saturating_add(b);
                            }
                        }
                    }
                }
            }
        }
        // The gray plane itself: its levels are what the curve edits, whatever the document's
        // colour mode (#5335).
        HistSource::Channel { layer, surface } => {
            if let Some(s) = channel_surface(doc, layer, surface)
                && let Some(bin) = h.first_mut()
            {
                *bin = gray_bins(s, doc.bounds());
            }
        }
        HistSource::BelowLayer(_) | HistSource::Layer(_) | HistSource::UpTo(_) => {
            // Luminosity rides along as one more histogram after the space's channels.
            h.push([0; 256]);
            level = composite_bins(doc, source, space, max_level, &mut h);
            if let Some(l) = h.pop() {
                lum = l;
            }
        }
    }
    Histograms { tag: tag(source, space), ch: h, lum, level }
}

/// The largest document counted pixel by pixel at the automatic cache level. The web build counts
/// on the UI thread (it has no worker threads), so it reduces documents a quarter that size.
fn exact_limit() -> u64 {
    if cfg!(target_arch = "wasm32") { photocraft_compose::PROXY_THUMBNAIL_PIXELS / 4 } else { photocraft_compose::PROXY_THUMBNAIL_PIXELS }
}

/// The cache level a histogram of `doc` is counted at: 1 (every pixel) up to the size a proxy
/// would stand in for, else the level whose 2^(n-1) reduction leaves about 768 pixels on the
/// longest side, at most `max_level` (clamped to 1..=[`MAX_CACHE_LEVEL`]).
pub fn cache_level(doc: &Document, max_level: u32) -> u32 {
    let longest = doc.size.width.max(doc.size.height);
    let k = longest / 768;
    let auto = if k >= 2 && doc.size.area() > exact_limit() && photocraft_compose::proxy::proxy_faithful(doc) { k.next_power_of_two().ilog2() + 1 } else { 1 };
    auto.min(max_level.clamp(1, MAX_CACHE_LEVEL))
}

/// How many times smaller the proxy counted at `level` (> 1) is: about 768 pixels on the longest
/// side (as at the automatic level), or 2^(level-1) when Cache Levels caps the level below that.
fn proxy_factor(doc: &Document, level: u32) -> u32 {
    let k = doc.size.width.max(doc.size.height) / 768;
    k.min(1 << (level.clamp(1, MAX_CACHE_LEVEL) - 1)).max(1)
}

/// Bins of the composited image below (`BelowLayer`), up to (`UpTo`) or of one layer alone
/// (`Layer`), in `space`, then luminosity in the last of `h`. Returns the cache level counted
/// (see [`cache_level`]): 1 when every document pixel was counted, higher when a downsampled proxy
/// stood in (like Photoshop's histogram cache levels).
///
/// The pixels are counted as they are, never blended: an area-averaged thumbnail invented greys
/// that aren't in the image wherever an edge fell inside one of its cells (#6538). Documents up to
/// [`photocraft_compose::PROXY_THUMBNAIL_PIXELS`] (and those a proxy can't reduce faithfully) are
/// composited in bands and counted exactly, which costs what the thumbnail did (it reduced the
/// same full composite). Larger ones count a nearest-neighbour proxy composite of about the old
/// thumbnail's work: it only holds values the layers have.
fn composite_bins(doc: &Document, source: HistSource, space: ToneSpace, max_level: u32, h: &mut [[u32; 256]]) -> u32 {
    let mut d = doc.clone();
    match source {
        HistSource::BelowLayer(hide) => {
            if let Some(l) = d.layer_mut(hide) {
                l.visible = false;
            }
        }
        HistSource::Layer(id) => {
            isolate(&mut d.layers, id);
        }
        HistSource::UpTo(id) => {
            hide_above(&mut d.layers, id);
        }
        HistSource::Channel { .. } | HistSource::Colors { .. } => {}
    }
    // An adjustment layer's masks limit what the adjustment touches, so only the pixels they reveal
    // (effective value above one half) are counted, read the way the compositor reads them: a
    // feather blurs across the canvas, so a feathered layer reads the compositor's (cached)
    // canvas-wide combined mask; otherwise each band takes the pixel mask (with density) times the
    // vector mask rendered for that band.
    let below = match source {
        HistSource::BelowLayer(id) => doc.layer(id),
        _ => None,
    };
    let level = cache_level(&d, max_level);
    let k = if level > 1 { proxy_factor(&d, level) } else { 1 };
    let d = if k > 1 { photocraft_compose::proxy::proxy_document(&d, k) } else { d };
    // On the proxy every mask is reduced the way its pixels are, so band and mask agree: the
    // combined and pixel masks with the proxy's own nearest-neighbour sampling, and the vector
    // mask (which the proxy leaves as it is) read at exactly the document pixels the proxy sampled.
    // A pixel mask that reveals everything is skipped.
    let combined = below
        .filter(|l| photocraft_compose::masks::has_feather(l))
        .and_then(|l| photocraft_compose::masks::combined_mask(l, doc.bounds()))
        .map(|s| if k > 1 { photocraft_compose::proxy::downsample(&s, k) } else { s });
    let pixel = match (source, &combined) {
        (HistSource::BelowLayer(id), None) => d.layer(id).and_then(|l| l.mask.as_ref()).filter(|m| m.enabled && !reveals_all(m)),
        _ => None,
    };
    let vector = match &combined {
        None => below.and_then(|l| l.vector_mask.as_ref()).filter(|v| v.enabled).map(photocraft_vector::CompiledVectorMask::new),
        Some(_) => None,
    };
    let mut revealed = Vec::new();
    let _ = photocraft_compose::render_bands(&d, d.bounds(), 0, |mut band| -> Result<(), ()> {
        let masked = if let Some(s) = &combined {
            s.read_region_into(band.rect, &mut revealed);
            true
        } else {
            let vector = vector.as_ref().map(|v| sampled_vector_values(v, band.rect, k));
            match (pixel, vector) {
                (Some(m), vector) => {
                    m.values_into(band.rect, &mut revealed);
                    for (a, b) in revealed.iter_mut().zip(vector.iter().flatten()) {
                        *a *= b;
                    }
                    true
                }
                (None, Some(v)) => {
                    revealed = v;
                    true
                }
                (None, None) => false,
            }
        };
        if masked {
            for (p, v) in band.px.iter_mut().zip(&revealed) {
                if v.is_nan() || *v <= 0.5 {
                    p[3] = 0.0;
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let part = {
            use rayon::prelude::*;
            band.px
                .par_chunks(64 * 1024)
                .map(|px| {
                    let mut part = vec![[0u32; 256]; h.len()];
                    count_pixels(px, space, &mut part);
                    part
                })
                .reduce(|| vec![[0u32; 256]; h.len()], add_bins)
        };
        #[cfg(target_arch = "wasm32")]
        let part = {
            let mut part = vec![[0u32; 256]; h.len()];
            count_pixels(&band.px, space, &mut part);
            part
        };
        for (dst, src) in h.iter_mut().zip(&part) {
            for (a, b) in dst.iter_mut().zip(src) {
                *a = a.saturating_add(*b);
            }
        }
        Ok(())
    });
    level
}

/// The vector mask's values for the proxy pixels in `rect`: each is the value at the document
/// pixel `k` times its coordinates, which is the one the nearest-neighbour proxy sampled. Only
/// the sampled document rows are rendered (one row per proxy row), so a mask edge between
/// samples moves no proxy pixel, as it would if the path were scaled and its coverage averaged.
fn sampled_vector_values(mask: &photocraft_vector::CompiledVectorMask, rect: photocraft_geom::Rect, k: u32) -> Vec<f32> {
    if k <= 1 {
        return mask.render(rect);
    }
    let (w, k) = (rect.width() as usize, k as i32);
    let mut out = Vec::with_capacity(w * rect.height() as usize);
    for y in rect.y0..rect.y1 {
        let x0 = rect.x0.saturating_mul(k);
        // The last proxy column samples x0 + (w - 1)·k: render up to it, not k - 1 past it.
        let span = (w.saturating_sub(1) as u32).saturating_mul(k as u32).saturating_add(1);
        let row = mask.render(photocraft_geom::Rect::from_xywh(x0, y.saturating_mul(k), span, 1));
        out.extend(row.iter().step_by(k as usize).copied().chain(std::iter::repeat(0.0)).take(w));
    }
    out
}

/// Whether a pixel mask reveals every pixel: no painted tiles and a default above one half after
/// density (counting it then changes nothing, so the per-band pass is skipped).
fn reveals_all(m: &photocraft_doc::LayerMask) -> bool {
    let d = m.surface.default_pixel().first().copied().unwrap_or(1.0);
    m.surface.tile_count() == 0 && 1.0 - m.density * (1.0 - d) > 0.5
}

/// Bin-wise sum of two sets of histograms (saturating: a hostile canvas can exceed `u32`).
#[cfg(not(target_arch = "wasm32"))]
fn add_bins(mut a: Vec<[u32; 256]>, b: Vec<[u32; 256]>) -> Vec<[u32; 256]> {
    for (dst, src) in a.iter_mut().zip(&b) {
        for (x, y) in dst.iter_mut().zip(src) {
            *x = x.saturating_add(*y);
        }
    }
    a
}

/// Luminosity weights (the Histogram panel's Luminosity channel).
pub const LUMINOSITY: [f32; 3] = [0.30, 0.59, 0.11];

/// Counts straight-alpha composite pixels into `h` (one histogram per channel of `space`, then
/// luminosity when `h` has room for it); transparent pixels are skipped.
fn count_pixels(px: &[[f32; 4]], space: ToneSpace, h: &mut [[u32; 256]]) {
    let bin = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as usize;
    let lum_at = match space {
        ToneSpace::Rgb => 4,
        ToneSpace::Cmyk => 5,
        ToneSpace::Lab => 3,
    };
    let mut add = |c: usize, b: usize| {
        if let Some(n) = h.get_mut(c).and_then(|ch| ch.get_mut(b)) {
            *n = n.saturating_add(1);
        }
    };
    for p in px {
        // Skips NaN alpha too.
        if p[3].is_nan() || p[3] <= 0.0 {
            continue;
        }
        let rgb = [p[0], p[1], p[2]].map(|v| v.clamp(0.0, 1.0));
        add(lum_at, bin(rgb.iter().zip(LUMINOSITY).map(|(v, w)| v * w).sum()));
        match space {
            ToneSpace::Rgb => {
                let q = [p[0], p[1], p[2]].map(bin);
                for (c, v) in q.iter().enumerate() {
                    add(c + 1, *v);
                }
                add(0, (q[0] + q[1] + q[2]) / 3);
            }
            ToneSpace::Cmyk => {
                let ink = photocraft_color::convert::rgb_to_cmyk([p[0], p[1], p[2]].map(|v| v.clamp(0.0, 1.0)));
                for (c, v) in ink.iter().enumerate() {
                    add(c + 1, bin(1.0 - v));
                }
                add(0, bin(1.0 - ink.iter().sum::<f32>() / 4.0));
            }
            ToneSpace::Lab => {
                let l = photocraft_color::convert::srgb_to_lab([p[0], p[1], p[2]].map(|v| v.clamp(0.0, 1.0)));
                add(0, bin(l[0] / 100.0));
                add(1, bin((l[1] + 128.0) / 255.0));
                add(2, bin((l[2] + 128.0) / 255.0));
            }
        }
    }
}

/// 256-bin histogram of a gray plane: its levels in 0–255, sampled like the engine's auto-tone
/// histograms so a big canvas costs no more than about a megapixel here (#5335).
fn component_bins(surface: &photocraft_raster::Surface, area: photocraft_geom::Rect, component: usize) -> [u32; 256] {
    let mut bins = [0u32; 256];
    let format = surface.format();
    let step = ((u64::from(area.width()) * u64::from(area.height())) / 1_000_000).max(1).isqrt().max(1) as usize;
    for y in (area.y0..area.y1).step_by(step) {
        for x in (area.x0..area.x1).step_by(step) {
            if format.alpha {
                let alpha = surface.sample_channel(x, y, format.mode.color_channels());
                if alpha.is_nan() || alpha <= 0.0 {
                    continue;
                }
            }
            let v = surface.sample_channel(x, y, component);
            let v = if surface.format().mode == photocraft_doc::ColorMode::Cmyk { 1.0 - v } else { v };
            let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            if let Some(bin) = bins.get_mut((v * 255.0).round() as usize) {
                *bin = bin.saturating_add(1);
            }
        }
    }
    bins
}

fn gray_bins(surf: &photocraft_raster::Surface, area: photocraft_geom::Rect) -> [u32; 256] {
    let mut h = [0u32; 256];
    let step = ((u64::from(area.width()) * u64::from(area.height())) / 1_000_000).max(1).isqrt().max(1) as i32;
    let mut y = area.y0;
    while y < area.y1 {
        let mut x = area.x0;
        while x < area.x1 {
            let v = (surf.sample_channel(x, y, 0).clamp(0.0, 1.0) * 255.0).round() as usize;
            if let Some(bin) = h.get_mut(v) {
                *bin += 1;
            }
            x += step;
        }
        y += step;
    }
    h
}

/// The tone editors' cached histograms: what they count, the revision they are valid for, and
/// the fingerprint of the masks they read.
pub(crate) struct ToneHist {
    doc: photocraft_doc::DocId,
    layer: LayerId,
    rev: u64,
    hist: Arc<Histograms>,
    masks: u64,
    // The fingerprint holds the tile addresses of the layer mask or gray plane counted. A weak
    // reference to each keeps its allocation (a few bytes, not the pixels) from being reused for
    // another tile, so equal fingerprints mean equal planes, without keeping the pixels alive. (A
    // write to a tile nothing else holds, such as history, then copies it while the counts are
    // cached.)
    _tiles: Vec<std::sync::Weak<photocraft_raster::Tile>>,
}

/// The gray plane a channel source counts (#5335).
fn channel_surface(doc: &Document, layer: LayerId, surface: photocraft_engine::channel_cmds::GraySurface) -> Option<&photocraft_raster::Surface> {
    use photocraft_engine::channel_cmds::GraySurface;
    match surface {
        GraySurface::LayerMask => doc.layer(layer).and_then(|l| l.mask.as_ref()).map(|m| &m.surface),
        GraySurface::Alpha(i) => doc.channels.get(i).map(|c| &c.surface),
        GraySurface::QuickMask => doc.quick_mask.as_ref().map(|c| &c.surface),
        GraySurface::FilterMask => doc.layer(layer).and_then(|l| photocraft_engine::mask_view_cmds::viewed_mask(l, true)).map(|m| &m.surface),
    }
}

/// What a source reads besides the composite: the layer whose masks limit the counts, or the gray
/// plane counted. Composite damage is the only other guard of the cached counts, and it describes
/// the composite, not these masks and planes (it is clipped to the canvas, for one), so every one
/// of them is fingerprinted alike rather than trusting that an edit to it always damages
/// something. Counts of the composite or a layer's own pixels rely on the damage alone.
enum MaskInput<'a> {
    Masks(&'a Layer),
    Plane(&'a photocraft_raster::Surface),
    None,
}

fn mask_input(doc: &Document, source: HistSource) -> MaskInput<'_> {
    let input = match source {
        HistSource::BelowLayer(id) => doc.layer(id).map(MaskInput::Masks),
        HistSource::Channel { layer, surface } => channel_surface(doc, layer, surface).map(MaskInput::Plane),
        HistSource::Layer(_) | HistSource::UpTo(_) | HistSource::Colors { .. } => None,
    };
    input.unwrap_or(MaskInput::None)
}

/// Fingerprint of [`mask_input`] (tile addresses: see [`ToneHist::_tiles`]).
fn mask_fingerprint(doc: &Document, source: HistSource) -> u64 {
    match mask_input(doc, source) {
        MaskInput::Masks(l) => photocraft_compose::masks::mask_key(l, doc.bounds()),
        // The plane is counted over the canvas, so the canvas is part of what was counted.
        MaskInput::Plane(s) => {
            use std::hash::{Hash, Hasher};
            let b = doc.bounds();
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (photocraft_compose::masks::surface_key(s), b.x0, b.y0, b.x1, b.y1).hash(&mut h);
            h.finish()
        }
        MaskInput::None => 0,
    }
}

/// Weak references to the tiles [`mask_fingerprint`] hashed, so their addresses stay unique.
fn fingerprint_tiles(doc: &Document, source: HistSource) -> Vec<std::sync::Weak<photocraft_raster::Tile>> {
    let surface = match mask_input(doc, source) {
        MaskInput::Masks(l) => l.mask.as_ref().map(|m| &m.surface),
        MaskInput::Plane(s) => Some(s),
        MaskInput::None => None,
    };
    surface.map_or_else(Vec::new, |s| s.tiles().map(|(_, t)| Arc::downgrade(t)).collect())
}

/// Cached histograms for a tone editor; recomputed only when something other than the editor's
/// own commits changed the document (see [`keep_after_commit`]).
pub fn histograms(app: &mut PhotocraftApp, source: HistSource, space: ToneSpace) -> Arc<Histograms> {
    let Some(st) = app.session.active() else { return Arc::new(Histograms::default()) };
    let (doc_id, rev) = (st.doc.id, st.revision);
    let want = tag(source, space);
    let layer = match source {
        HistSource::BelowLayer(l) | HistSource::Layer(l) | HistSource::UpTo(l) => l,
        HistSource::Channel { layer, .. } | HistSource::Colors { layer, .. } => layer,
    };
    if let Some(c) = app.tone_hist.as_mut()
        && c.doc == doc_id
        && c.layer == layer
        && c.hist.tag == want
        // Same revision: nothing changed. The next one changed no pixels: the counts hold unless
        // it edited the masks they read (fingerprinted only then, never on every frame).
        && (c.rev == rev || (c.rev.checked_add(1) == Some(rev) && view_only(st) && mask_fingerprint(&st.doc, source) == c.masks))
    {
        c.rev = rev;
        return c.hist.clone();
    }
    let t0 = crate::gpu_canvas::now_ms();
    let hist = Arc::new(compute_histograms(&st.doc, source, space));
    app.perf.span("histogram", crate::gpu_canvas::now_ms() - t0);
    let masks = mask_fingerprint(&st.doc, source);
    let tiles = fingerprint_tiles(&st.doc, source);
    app.tone_hist = Some(ToneHist { doc: doc_id, layer, rev, hist: hist.clone(), masks, _tiles: tiles });
    hist
}

/// Whether the latest revision changed no pixels (selecting a layer, say): cached histograms of
/// the previous revision still hold (#125).
pub(crate) fn view_only(st: &photocraft_engine::DocState) -> bool {
    st.last_damage.is_some_and(|r| r.is_empty())
}

/// After an editor commits its own adjustment layer, the image below it is unchanged: keep the
/// cached histogram valid for the new revision. Call with the revision seen before the commit.
pub fn keep_after_commit(app: &mut PhotocraftApp, layer: LayerId, before: Option<u64>) {
    let now = app.session.active().map(|s| s.revision);
    if let (Some(c), Some(before), Some(now)) = (app.tone_hist.as_mut(), before, now)
        && c.layer == layer
        && c.rev == before
    {
        c.rev = now;
    }
}

/// Display scale for a 256-bin histogram: bar heights are `v / top` clamped to 1, linear
/// to the tallest bin like Photoshop. A percentile of the sorted bins degenerates on
/// sparse histograms (few occupied levels sort 0 into every low index), drawing tiny
/// counts as tall as the real peaks (#6574).
/// Extracted from [`draw_histogram`] so the scaling is unit-testable.
pub fn histogram_scale_top(h: &[u32; 256]) -> f32 {
    h.iter().copied().max().unwrap_or(1).max(1) as f32 * 1.1
}

pub fn draw_histogram(p: &egui::Painter, r: Rect, h: &[u32; 256], color: Color32) {
    let top = histogram_scale_top(h);
    let mut pts = vec![egui::pos2(r.left(), r.bottom())];
    for (i, v) in h.iter().enumerate() {
        let x = r.left() + (i as f32 + 0.5) / 256.0 * r.width();
        let y = r.bottom() - (*v as f32 / top).min(1.0) * r.height();
        pts.push(egui::pos2(x, y));
    }
    pts.push(egui::pos2(r.right(), r.bottom()));
    // One mesh of vertical strips: no anti-aliasing feather, so no seams between bins.
    let mut mesh = egui::Mesh::default();
    for w in pts.windows(2).skip(1).take(256) {
        let (a, b) = (w[0], w[1]);
        let i = mesh.vertices.len() as u32;
        for q in [egui::pos2(a.x, r.bottom()), a, b, egui::pos2(b.x, r.bottom())] {
            mesh.colored_vertex(q, color);
        }
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
    p.add(mesh);
}

/// Display colour of channel `name` (curve, histogram and gradient bars).
pub fn channel_color(name: &str, t: &Tokens) -> Color32 {
    match name {
        "red" => Color32::from_rgb(235, 80, 80),
        "green" => Color32::from_rgb(80, 205, 95),
        "blue" => Color32::from_rgb(90, 140, 255),
        "cyan" => Color32::from_rgb(40, 200, 230),
        "magenta" => Color32::from_rgb(225, 70, 200),
        "yellow" => Color32::from_rgb(235, 210, 40),
        "a" => Color32::from_rgb(220, 90, 140),
        "b" => Color32::from_rgb(200, 180, 60),
        _ => t.text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn subset_histograms_skip_transparent_unstored_and_nan_alpha_pixels() {
        let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
        let area = photocraft_geom::Rect::new(0, 0, 32, 24);
        surface.fill_rect(photocraft_geom::Rect::new(0, 0, 16, 24), &[0.5, 0.4, 0.3, 1.0]);
        let bins = component_bins(&surface, area, 0);
        assert_eq!(bins.iter().sum::<u32>(), 16 * 24);
        assert_eq!(bins[0], 0);
        let nan = photocraft_raster::Surface::with_default(
            photocraft_color::PixelFormat::new(photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F32, true),
            &[0.5, 0.4, 0.3, f32::NAN],
        );
        assert_eq!(component_bins(&nan, area, 0), [0; 256]);
    }

    #[test]
    fn histogram_ignores_the_edited_adjustment() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap(); // white
        s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        let st = s.active().unwrap();
        let h = compute_histograms(&st.doc, HistSource::BelowLayer(st.active_layer.unwrap()), ToneSpace::Rgb);
        assert!(h[0][255] > 0 && h[0][0] == 0, "sees the white image, not the inverted result");
        assert_eq!(h[9], [0; 256], "out-of-range channels are empty, not a panic");
    }

    #[test]
    fn layer_histogram_sees_only_that_layer_and_spaces_have_their_channels() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap(); // white background
        let bg = s.active().unwrap().doc.layers[0].id;
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 32})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        let st = s.active().unwrap();
        let h = compute_histograms(&st.doc, HistSource::Layer(bg), ToneSpace::Rgb);
        assert!(h[0][255] > 0 && h[0][0] == 0, "the black layer above is hidden");
        assert_eq!(compute_histograms(&st.doc, HistSource::Layer(bg), ToneSpace::Cmyk).ch.len(), 5);
        assert_eq!(compute_histograms(&st.doc, HistSource::Layer(bg), ToneSpace::Lab).ch.len(), 3);
    }

    /// #5335: the Curves histogram of a targeted gray plane describes that plane, not the
    /// colour layers beneath it: a channel filled with 25% grey over a black layer shows a
    /// spike at 25% grey, not at black.
    #[test]
    fn channel_histogram_describes_the_gray_plane() {
        use photocraft_engine::channel_cmds::GraySurface;
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap(); // white background
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 32})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("channel.new", json!({"name": "Sky sel", "fill": "white"})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let b = photocraft_geom::Rect::from_xywh(0, 0, 32, 32);
        s.edit("fill-planes", |doc, active| {
            doc.channels[0].surface.fill_rect(b, &[0.25]);
            doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap().surface.fill_rect(b, &[0.0]);
            Ok(())
        })
        .unwrap();
        let st = s.active().unwrap();
        let layer = st.active_layer.unwrap();
        let h = compute_histograms(&st.doc, HistSource::Channel { layer, surface: GraySurface::Alpha(0) }, ToneSpace::Rgb);
        assert!(h[0][64] > 0, "a spike at 25% grey, got {:?}", &h[0][0..80]);
        assert_eq!(h[0][0], 0, "not the black layer beneath");
        let m = compute_histograms(&st.doc, HistSource::Channel { layer, surface: GraySurface::LayerMask }, ToneSpace::Rgb);
        assert!(m[0][0] > 0, "the hidden mask is black");
        // Each plane keeps its own cache entry: switching targets recomputes.
        assert_ne!(
            tag(HistSource::Channel { layer, surface: GraySurface::Alpha(0) }, ToneSpace::Rgb),
            tag(HistSource::Channel { layer, surface: GraySurface::LayerMask }, ToneSpace::Rgb)
        );
        assert_ne!(tag(HistSource::Channel { layer, surface: GraySurface::LayerMask }, ToneSpace::Rgb), tag(HistSource::Layer(layer), ToneSpace::Rgb));
        // Every source of one layer has its own tag (the filter mask and the Adjustment
        // Composite once shared one).
        let sources = [
            HistSource::BelowLayer(layer),
            HistSource::Layer(layer),
            HistSource::UpTo(layer),
            HistSource::Colors { layer, colors: photocraft_engine::color_channels::ColorChannels::from_indices([0, 2], 3).unwrap() },
            HistSource::Channel { layer, surface: GraySurface::LayerMask },
            HistSource::Channel { layer, surface: GraySurface::QuickMask },
            HistSource::Channel { layer, surface: GraySurface::FilterMask },
        ];
        for (i, a) in sources.iter().enumerate() {
            for b in &sources[i + 1..] {
                assert_ne!(tag(*a, ToneSpace::Rgb), tag(*b, ToneSpace::Rgb), "{a:?} and {b:?}");
            }
        }
    }

    /// #6574: bar heights scale linearly to the tallest bin. A 90/10 black/white
    /// histogram draws the small bar at a ninth of the big one, not full height.
    #[test]
    fn histogram_scale_is_proportional_for_two_level_images() {
        let mut h = [0u32; 256];
        h[0] = 100_000;
        h[255] = 900_000;
        let top = histogram_scale_top(&h);
        let small = (h[0] as f32 / top).min(1.0);
        let big = (h[255] as f32 / top).min(1.0);
        assert!((small / big - 1.0 / 9.0).abs() < 0.02, "small/big = {}, top = {top}", small / big);
    }

    /// #6574: a sparse histogram (few occupied levels) must not scale to a bin
    /// index that is 0: the old `sorted.get(250)` degenerated to `top = 1.1`,
    /// drawing a few hundred samples as tall as hundreds of thousands.
    #[test]
    fn histogram_scale_does_not_degenerate_on_sparse_histograms() {
        let mut h = [0u32; 256];
        h[0] = 500_000;
        h[128] = 300;
        h[255] = 500_000;
        let top = histogram_scale_top(&h);
        assert!(top > h[128] as f32, "scale {top} must exceed the tiny middle bin");
        let tiny = (h[128] as f32 / top).min(1.0);
        assert!(tiny < 0.05, "tiny bin draws near-flat, got {tiny}");
    }

    /// A white document with black filled over columns `0..split` (a layer above the background).
    fn split_doc(w: u32, h: u32, split: u32, depth: Option<u32>) -> photocraft_engine::Session {
        let mut s = photocraft_engine::Session::new();
        let mut p = json!({"width": w, "height": h});
        if let Some(bits) = depth {
            p["depth"] = json!(bits);
        }
        s.execute("file.new", p).unwrap(); // white
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": split, "height": h})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    fn panel_histograms(s: &photocraft_engine::Session) -> Histograms {
        compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb)
    }

    fn mean(h: &[u32; 256]) -> f64 {
        let n: u64 = h.iter().map(|v| u64::from(*v)).sum();
        h.iter().enumerate().map(|(i, v)| i as f64 * f64::from(*v)).sum::<f64>() / n as f64
    }

    /// #6538: a black/white split counts only black and white, wherever the edge falls (the old
    /// 384 px area-averaged thumbnail turned the cell holding the edge mid-grey), and counts
    /// every pixel: cache level 1.
    #[test]
    fn split_image_histogram_counts_only_black_and_white() {
        for (w, h, split) in [(1000, 1000, 334), (1000, 1000, 500), (1000, 600, 1), (777, 333, 389), (2000, 50, 1999), (64, 64, 32)] {
            let s = split_doc(w, h, split, None);
            let hist = panel_histograms(&s);
            assert_eq!(hist.level, 1, "{w}x{h}: every pixel is counted");
            for c in 0..4 {
                let stray: Vec<(usize, u32)> = hist[c].iter().copied().enumerate().filter(|&(i, v)| v > 0 && i != 0 && i != 255).collect();
                assert!(stray.is_empty(), "{w}x{h} split {split}, channel {c}: invented values {stray:?}");
                assert_eq!(u64::from(hist[c][0]), u64::from(split) * u64::from(h), "{w}x{h} split {split}: black pixels");
                assert_eq!(u64::from(hist[c][255]), u64::from(w - split) * u64::from(h), "{w}x{h} split {split}: white pixels");
            }
        }
        let half = panel_histograms(&split_doc(1000, 1000, 500, None));
        assert!((mean(&half[0]) - 127.5).abs() < 1e-9, "a 50/50 split has mean 127.5, got {}", mean(&half[0]));
    }

    /// The same at 16 and 32 bits per channel: the count does not go through an 8-bit thumbnail.
    #[test]
    fn split_image_histogram_is_exact_at_high_bit_depths() {
        for bits in [16, 32] {
            let s = split_doc(1000, 400, 334, Some(bits));
            assert_eq!(s.active().unwrap().doc.depth.bits(), bits, "the document is {bits}-bit");
            let hist = panel_histograms(&s);
            for c in 0..4 {
                let stray: Vec<(usize, u32)> = hist[c].iter().copied().enumerate().filter(|&(i, v)| v > 0 && i != 0 && i != 255).collect();
                assert!(stray.is_empty(), "{bits}-bit channel {c}: invented values {stray:?}");
                assert_eq!(hist[c][0], 334 * 400);
                assert_eq!(hist[c][255], 666 * 400);
            }
        }
    }

    /// Above the exact-count size a nearest-neighbour proxy stands in (cache level > 1): it is
    /// smaller, yet still only holds black and white.
    #[test]
    fn large_split_image_counts_a_nearest_proxy_without_inventing_greys() {
        let s = split_doc(6000, 3000, 2001, None);
        let hist = panel_histograms(&s);
        assert!(hist.level > 1, "a proxy stood in, level {}", hist.level);
        let n: u64 = hist[0].iter().map(|v| u64::from(*v)).sum();
        assert!(n > 0 && n < 6000 * 3000, "counted {n} proxy pixels");
        for c in 0..4 {
            let stray: Vec<(usize, u32)> = hist[c].iter().copied().enumerate().filter(|&(i, v)| v > 0 && i != 0 && i != 255).collect();
            assert!(stray.is_empty(), "channel {c}: invented values {stray:?}");
        }
        assert!((mean(&hist[0]) - 255.0 * 3999.0 / 6000.0).abs() < 1.0, "mean {}", mean(&hist[0]));
    }

    /// `split_doc` (black columns `0..split`, white after) plus an Invert adjustment layer whose mask
    /// reveals only columns `split..w`. Returns the session and the adjustment layer.
    fn masked_adjustment(w: u32, h: u32, split: u32, mask_enabled: bool) -> (photocraft_engine::Session, LayerId) {
        let mut s = split_doc(w, h, split, None);
        s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.edit("hide-left", |doc, active| {
            let m = doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap();
            m.surface.fill_rect(photocraft_geom::Rect::new(0, 0, split as i32, h as i32), &[0.0]);
            m.enabled = mask_enabled;
            Ok(())
        })
        .unwrap();
        (s, id)
    }

    /// #12247: the histogram of the image below an adjustment layer counts only what its mask reveals.
    #[test]
    fn masked_adjustment_histogram_counts_only_the_revealed_area() {
        let (s, id) = masked_adjustment(64, 64, 32, true);
        let hist = compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert_eq!(hist.level, 1);
        assert_eq!(hist[0][255], 32 * 64, "the white half");
        assert_eq!(hist[0][0], 0, "the hidden black half is not counted");
    }

    #[test]
    fn disabled_mask_counts_everything() {
        let (s, id) = masked_adjustment(64, 64, 32, false);
        let hist = compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert_eq!((hist[0][0], hist[0][255]), (32 * 64, 32 * 64));
    }

    #[test]
    fn masked_adjustment_histogram_counts_only_the_revealed_area_on_the_proxy_path() {
        let (s, id) = masked_adjustment(6000, 3000, 3000, true);
        let hist = compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert!(hist.level > 1, "a proxy stood in, level {}", hist.level);
        assert!(hist[0][255] > 0, "the revealed white half is counted");
        assert_eq!(hist[0][0], 0, "the hidden black half is not counted");
    }

    #[test]
    fn fully_hidden_mask_gives_empty_bins_and_a_finite_auto() {
        let (mut s, id) = masked_adjustment(64, 64, 32, true);
        s.edit("hide-all", |doc, active| {
            let m = doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap();
            m.surface.fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[0.0]);
            Ok(())
        })
        .unwrap();
        let hist = compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert_eq!(hist[0], [0; 256]);
        let (lo, hi) = crate::dialogs::adjust_editors::auto_levels(&hist[0]);
        assert!(lo.is_finite() && hi.is_finite() && lo < hi, "{lo} {hi}");
    }

    /// An adjustment whose pixel mask reveals all and whose vector mask covers only the white right half.
    fn vector_masked_adjustment(w: u32, h: u32, split: u32) -> (photocraft_engine::Session, LayerId) {
        let (mut s, id) = masked_adjustment(w, h, split, false);
        s.edit("vector-right", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.mask = None;
            let path = photocraft_vector::shapes::rect(f64::from(split), 0.0, f64::from(w - split), f64::from(h));
            l.vector_mask = Some(photocraft_doc::vector::VectorMask::new(path));
            Ok(())
        })
        .unwrap();
        (s, id)
    }

    #[test]
    fn vector_mask_limits_the_counted_area() {
        let (s, id) = vector_masked_adjustment(64, 64, 32);
        let hist = compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert_eq!(hist.level, 1);
        assert_eq!((hist[0][0], hist[0][255]), (0, 32 * 64));
    }

    /// A large document is still counted on the proxy, with the vector mask scaled to it: only the
    /// revealed right half counts (a canvas-wide mask would exceed its budget on huge documents
    /// and fall back to a constant that hides everything).
    #[test]
    fn vector_mask_on_a_large_document_is_scaled_to_the_proxy() {
        let (s, id) = vector_masked_adjustment(6000, 3000, 3000);
        let hist = compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert!(hist.level > 1);
        // Factor 7: an 858 × 429 proxy whose columns 429..=857 sample the white half; the mask is
        // read at the sampled pixels, so it reveals exactly those columns.
        assert_eq!((hist[0][0], hist[0][255]), (0, 429 * 429));
    }

    /// The vector mask lines up with the pixel each proxy pixel samples: an edge just past a
    /// multiple of the factor hides the proxy column whose sampled pixel it hides.
    #[test]
    fn a_proxied_vector_mask_edge_follows_the_sampled_pixels() {
        // Black left of 3000, white right; the mask reveals from 2997, so black 2997..3000 shows
        // at full size, but no proxy column samples it (column 428 samples 2996, 429 samples 3003).
        let (mut s, id) = vector_masked_adjustment(6000, 3000, 3000);
        s.edit("from-2997", |doc, _| {
            let path = photocraft_vector::shapes::rect(2997.0, 0.0, 3003.0, 3000.0);
            doc.layer_mut(id).unwrap().vector_mask = Some(photocraft_doc::vector::VectorMask::new(path));
            Ok(())
        })
        .unwrap();
        let hist = compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert!(hist.level > 1);
        assert_eq!(hist[0][0], 0, "column 428 samples hidden pixel 2996");
        assert_eq!(hist[0][255], 429 * 429);
    }

    /// A feather is read from the compositor's combined mask, reduced onto the proxy: a large
    /// document keeps its cache level and still counts only what is revealed.
    #[test]
    fn a_feathered_mask_on_a_large_document_is_counted_on_the_proxy() {
        let (mut s, id) = masked_adjustment(6000, 3000, 3000, true);
        s.edit("feather", |doc, _| {
            doc.layer_mut(id).unwrap().mask.as_mut().unwrap().feather = 2.0;
            Ok(())
        })
        .unwrap();
        let hist = compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert!(hist.level > 1, "level {}", hist.level);
        assert_eq!(hist[0][0], 0);
        assert!(hist[0][255] > 0);
    }

    /// Editing the mask of a hidden adjustment changes no pixels, yet changes what is counted.
    #[test]
    fn a_hidden_adjustment_does_not_keep_the_histogram_on_no_damage() {
        let (mut s, id) = masked_adjustment(64, 64, 32, true);
        s.edit("hide", |doc, _| {
            doc.layer_mut(id).unwrap().visible = false;
            Ok(())
        })
        .unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let first = histograms(&mut app, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert_eq!((first[0][0], first[0][255]), (0, 32 * 64));
        app.session
            .edit("reveal all", |doc, _| {
                doc.layer_mut(id).unwrap().mask.as_mut().unwrap().surface.fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[1.0]);
                Ok(())
            })
            .unwrap();
        app.session.active_mut().unwrap().last_damage = Some(photocraft_geom::Rect::EMPTY);
        let after = histograms(&mut app, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert_eq!((after[0][0], after[0][255]), (32 * 64, 32 * 64));
    }

    /// The same holds when the counted channel is a hidden layer's mask: editing it changes no
    /// pixels, but the mask's histogram is counted again.
    #[test]
    fn a_hidden_layer_mask_channel_is_recounted_on_no_damage() {
        let (mut s, id) = masked_adjustment(64, 64, 32, true);
        s.edit("hide", |doc, _| {
            doc.layer_mut(id).unwrap().visible = false;
            Ok(())
        })
        .unwrap();
        let source = HistSource::Channel { layer: id, surface: photocraft_engine::channel_cmds::GraySurface::LayerMask };
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let first = histograms(&mut app, source, ToneSpace::Rgb);
        // A revision that changes neither pixels nor the mask keeps the counts.
        app.session
            .edit("rename", |doc, _| {
                doc.layer_mut(id).unwrap().name = "renamed".into();
                Ok(())
            })
            .unwrap();
        app.session.active_mut().unwrap().last_damage = Some(photocraft_geom::Rect::EMPTY);
        let first = {
            let kept = histograms(&mut app, source, ToneSpace::Rgb);
            assert!(Arc::ptr_eq(&first, &kept), "an unchanged mask keeps the cached counts");
            kept
        };
        assert_eq!(first[0][0], 32 * 64, "the hidden half of the mask");
        app.session
            .edit("reveal all", |doc, _| {
                doc.layer_mut(id).unwrap().mask.as_mut().unwrap().surface.fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[1.0]);
                Ok(())
            })
            .unwrap();
        app.session.active_mut().unwrap().last_damage = Some(photocraft_geom::Rect::EMPTY);
        let after = histograms(&mut app, source, ToneSpace::Rgb);
        assert_eq!((after[0][0], after[0][255]), (0, 64 * 64));
        // An unchanged revision keeps the counts without fingerprinting the mask again.
        assert!(Arc::ptr_eq(&after, &histograms(&mut app, source, ToneSpace::Rgb)));
    }

    /// Every counted plane kind is fingerprinted, even an alpha channel or the quick mask, whose
    /// edits today always damage the composite: forced empty damage shows the fingerprint alone
    /// keeps or drops the counts.
    #[test]
    fn an_alpha_channel_edited_with_no_damage_is_recounted() {
        use photocraft_engine::channel_cmds::GraySurface;
        plane_edited_with_no_damage_is_recounted(GraySurface::Alpha(0), |doc| &mut doc.channels[0].surface);
        plane_edited_with_no_damage_is_recounted(GraySurface::QuickMask, |doc| &mut doc.quick_mask.as_mut().unwrap().surface);
    }

    fn plane_edited_with_no_damage_is_recounted(
        surface: photocraft_engine::channel_cmds::GraySurface,
        plane: fn(&mut Document) -> &mut photocraft_raster::Surface,
    ) {
        let (s, id) = masked_adjustment(64, 64, 32, true);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.session
            .edit("planes", |doc, _| {
                let mut sf = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(
                    photocraft_color::ColorMode::Grayscale,
                    photocraft_color::SampleType::U8,
                    false,
                ));
                sf.fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[0.0]);
                doc.channels.push(photocraft_doc::AlphaChannel::new("Alpha 1", sf.clone()));
                doc.quick_mask = Some(photocraft_doc::AlphaChannel::new("Quick Mask", sf));
                Ok(())
            })
            .unwrap();
        let source = HistSource::Channel { layer: id, surface };
        let first = histograms(&mut app, source, ToneSpace::Rgb);
        assert_eq!(first[0][0], 64 * 64);
        app.session
            .edit("rename", |doc, _| {
                doc.layer_mut(id).unwrap().name = "renamed".into();
                Ok(())
            })
            .unwrap();
        app.session.active_mut().unwrap().last_damage = Some(photocraft_geom::Rect::EMPTY);
        assert!(Arc::ptr_eq(&first, &histograms(&mut app, source, ToneSpace::Rgb)), "an unchanged plane keeps the counts");
        app.session
            .edit("fill plane", |doc, _| {
                plane(doc).fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[1.0]);
                Ok(())
            })
            .unwrap();
        app.session.active_mut().unwrap().last_damage = Some(photocraft_geom::Rect::EMPTY);
        let after = histograms(&mut app, source, ToneSpace::Rgb);
        assert_eq!((after[0][0], after[0][255]), (0, 64 * 64));
    }

    /// A feathered mask is counted as the compositor blurs it: a two-pixel revealed stripe is
    /// counted as painted, but a feather spreads it so thin that nothing stays above one half.
    #[test]
    fn feathered_mask_is_counted_as_rendered() {
        let stripe = |feather: f32| {
            let (mut s, id) = masked_adjustment(64, 64, 32, true);
            s.edit("stripe", |doc, active| {
                let m = doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap();
                m.surface.fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[0.0]);
                m.surface.fill_rect(photocraft_geom::Rect::new(40, 0, 42, 64), &[1.0]);
                m.feather = feather;
                Ok(())
            })
            .unwrap();
            compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb)
        };
        let sharp = stripe(0.0);
        assert_eq!((sharp[0][0], sharp[0][255]), (0, 2 * 64));
        let soft = stripe(8.0);
        assert_eq!(soft.level, 1);
        assert_eq!(soft[0][0], 0, "the hidden black half stays hidden");
        assert!(soft[0][255] < 2 * 64, "the feather thins the stripe: {}", soft[0][255]);
    }

    #[test]
    fn a_mask_revealing_everything_counts_everything() {
        let (mut s, id) = masked_adjustment(64, 64, 32, true);
        s.edit("reveal", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().mask = Some(photocraft_doc::LayerMask::reveal_all());
            Ok(())
        })
        .unwrap();
        let hist = compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(id), ToneSpace::Rgb);
        assert_eq!((hist[0][0], hist[0][255]), (32 * 64, 32 * 64));
    }

    #[test]
    fn image_source_counts_every_pixel() {
        let (s, _) = masked_adjustment(64, 64, 32, true);
        let hist = panel_histograms(&s);
        // The whole composite (no such layer to take a mask from): every pixel counted, here all black.
        assert_eq!(hist[0].iter().sum::<u32>(), 64 * 64);
        assert_eq!(hist[0][0], 64 * 64);
    }
}
