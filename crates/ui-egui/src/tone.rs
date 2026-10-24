//! Histograms for the tone editors (Levels, Curves, Threshold) and the Histogram panel.
//!
//! The adjustment editors themselves live in `adjust_editors`; this module computes and caches
//! the histograms they draw behind their controls, in the channels of the adjustment's space
//! (RGB, CMYK ink brightness, Lab), and draws them. Photoshop keeps the two audiences apart
//! (see [`HistScope`]): the panel follows the selection, the editors' dialogs do not.

use std::sync::Arc;

use egui::{Color32, Rect, Sense, vec2};
use photocraft_doc::adjust::ToneSpace;
use photocraft_doc::{Document, Layer, LayerId};

use crate::PhotocraftApp;
use crate::theme::Tokens;

const CHANNELS: [(&str, &str); 4] = [("rgb", "RGB"), ("red", "Red"), ("green", "Green"), ("blue", "Blue")];

static EMPTY: [u32; 256] = [0; 256];

/// 256-bin histograms: the composite then one per channel of the space (RGB: luminosity, R, G, B;
/// CMYK: composite, C, M, Y, K as channel brightness; Lab: L, a, b). `tag` identifies what they
/// were computed from (see [`HistSource`]).
#[derive(Clone, Debug, Default)]
pub struct Histograms {
    pub tag: u64,
    pub ch: Vec<[u32; 256]>,
    /// Photoshop's histogram cache level of a composite histogram: 1 when every document pixel
    /// was counted, `n` when a proxy 2^(n-1) times smaller stood in; 0 for a gray plane's.
    pub level: u32,
    /// Every pixel the count kept — with a selection, those the mask covers ≥ ½ — regardless of
    /// transparency, at the counted resolution (exact at level 1). This is what the panel's
    /// Pixels statistic reports; the bins hold only the visible pixels.
    pub pixel_count: u64,
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
    /// One layer on its own (the target of a destructive Image › Adjustments command).
    Layer(LayerId),
    /// The gray plane a Curves target edits: a layer mask, a saved channel or the Quick Mask.
    /// The dialog shows this plane's own levels, not the colour layers beneath (#5335).
    Channel { layer: LayerId, surface: photocraft_engine::channel_cmds::GraySurface },
}

/// Whether a histogram counts the whole input or only the selected pixels. Photoshop's Histogram
/// panel follows the selection ("to display histogram data for a portion of the image, first
/// select that portion" — Adobe), while its Levels/Curves/Threshold dialogs show the
/// adjustment's whole input: with a selection active the histogram behind the curve is of the
/// entire image, not just the selected pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistScope {
    /// The whole input, unmasked by any selection (the tone editors' histograms).
    WholeInput,
    /// Only the selected pixels, partial coverage counting from ½ up (the Histogram panel).
    Selection,
}

fn tag(source: HistSource, space: ToneSpace, scope: HistScope, sel: u64) -> u64 {
    let (k, id) = match source {
        HistSource::BelowLayer(l) => (1u64, l.0),
        HistSource::Layer(l) => (2, l.0),
        HistSource::Channel { layer, surface } => match surface {
            photocraft_engine::channel_cmds::GraySurface::LayerMask => (3, layer.0),
            photocraft_engine::channel_cmds::GraySurface::Alpha(i) => (4, i as u64),
            photocraft_engine::channel_cmds::GraySurface::QuickMask => (5, layer.0),
        },
    };
    let s = match space {
        ToneSpace::Rgb => 0u64,
        ToneSpace::Cmyk => 1,
        ToneSpace::Lab => 2,
    };
    let m = match scope {
        HistScope::WholeInput => 0u64,
        HistScope::Selection => 1,
    };
    id.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (k << 60) ^ (s << 56) ^ (m << 52) ^ sel.rotate_left(31)
}

/// Cache identity of the current selection: the mask's default (what tile-less pixels read as),
/// its bounds, and which tiles it is built from. A clone shares its tiles, so an untouched
/// selection keeps its key across edits (the #125 fast path stays); any command that writes a
/// new mask replaces at least the tiles it touched, or the default (e.g. `select.inverse`'s
/// `prune` leaves a tile-less mask), which changes the key. 0 when nothing is selected.
///
/// Tile identities are pointers, and an allocator can hand a freed tile's address to a new tile
/// (ABA), so two different selections can in principle hash alike. A stale hit needs that
/// collision inside the single view-only revision the caches promote across, and selection
/// commands build the new mask before dropping the old one (no free-then-allocate), so the
/// addresses cannot meet within one command; the default and bounds in the key keep the
/// pointer-less masks (all vs. none) apart regardless.
fn selection_key(doc: &Document) -> u64 {
    use std::hash::{Hash, Hasher};
    let Some(m) = doc.selection.as_ref() else { return 0 };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    1u8.hash(&mut h);
    m.default_bytes().hash(&mut h);
    let b = m.tile_bounds();
    (b.x0, b.y0, b.x1, b.y1).hash(&mut h);
    for (c, t) in m.tiles() {
        (c.tx, c.ty).hash(&mut h);
        std::ptr::hash(Arc::as_ptr(t), &mut h);
    }
    h.finish()
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

/// Histograms of `doc` as seen by `source`, in the channels of `space`. [`HistScope::Selection`]
/// counts only the selected pixels (the Histogram panel; partial coverage counts from ½ up);
/// [`HistScope::WholeInput`] ignores the selection, like Photoshop's Levels/Curves/Threshold
/// dialogs, which show the adjustment's whole input.
pub fn compute_histograms(doc: &Document, source: HistSource, space: ToneSpace, scope: HistScope) -> Histograms {
    let (mut level, mut pixels) = (0, 0u64);
    let mut h = vec![
        [0u32; 256];
        match space {
            ToneSpace::Rgb => 4,
            ToneSpace::Cmyk => 5,
            ToneSpace::Lab => 3,
        }
    ];
    match source {
        // The gray plane itself: its levels are what the curve edits, whatever the document's
        // colour mode (#5335). A selection does not mask a plane the dialog edits directly.
        HistSource::Channel { layer, surface } => {
            let surf = match surface {
                photocraft_engine::channel_cmds::GraySurface::LayerMask => doc.layer(layer).and_then(|l| l.mask.as_ref()).map(|m| &m.surface),
                photocraft_engine::channel_cmds::GraySurface::Alpha(i) => doc.channels.get(i).map(|c| &c.surface),
                photocraft_engine::channel_cmds::GraySurface::QuickMask => doc.quick_mask.as_ref().map(|c| &c.surface),
            };
            if let Some(s) = surf
                && let Some(bin) = h.first_mut()
            {
                *bin = gray_bins(s, doc.bounds());
            }
        }
        HistSource::BelowLayer(_) | HistSource::Layer(_) => {
            level = composite_bins(doc, source, space, scope, &mut h, &mut pixels);
        }
    }
    let sel_tag = if scope == HistScope::Selection { selection_key(doc) } else { 0 };
    Histograms { tag: tag(source, space, scope, sel_tag), ch: h, level, pixel_count: pixels }
}

/// Bins of the composited image below (`BelowLayer`) or of one layer alone (`Layer`), in `space`,
/// counting the whole input or only the selected pixels per `scope` into `h`; `pixels` counts
/// every pixel the count kept, transparent ones included. Returns the cache level counted: 1
/// when every document pixel was counted, higher when a downsampled proxy stood in (like
/// Photoshop's histogram cache levels). The proxy only stands in while there is no selection
/// mask to sample (the editors always, the panel while nothing is selected): a masked count
/// samples the mask in document space and stays exact (level 1).
///
/// The pixels are counted as they are, never blended: an area-averaged thumbnail invented greys
/// that aren't in the image wherever an edge fell inside one of its cells (#6538). Documents up to
/// [`photocraft_compose::PROXY_THUMBNAIL_PIXELS`] (and those a proxy can't reduce faithfully) are
/// composited in bands and counted exactly, which costs what the thumbnail did (it reduced the
/// same full composite). Larger ones count a nearest-neighbour proxy composite of about the old
/// thumbnail's work: it only holds values the layers have.
fn composite_bins(doc: &Document, source: HistSource, space: ToneSpace, scope: HistScope, h: &mut [[u32; 256]], pixels: &mut u64) -> u32 {
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
        HistSource::Channel { .. } => {}
    }
    let sel = match scope {
        HistScope::WholeInput => None,
        HistScope::Selection => d.selection.clone(),
    };
    let longest = d.size.width.max(d.size.height);
    let k = longest / 768;
    // The proxy stands in only while there is no selection mask to sample: the editors never
    // have one, and the panel has none while nothing is selected — either way the count is the
    // whole input. A masked count samples the mask in document space and stays exact (level 1).
    let (d, level) = if k >= 2 && sel.is_none() && d.size.area() > photocraft_compose::PROXY_THUMBNAIL_PIXELS && photocraft_compose::proxy::proxy_faithful(&d) {
        (photocraft_compose::proxy::proxy_document(&d, k), k.next_power_of_two().ilog2() + 1)
    } else {
        (d, 1)
    };
    // A selection whose default reads as unselected (marquee/lasso masks) can only pass the
    // ≥ ½ threshold inside its content bounds, so composite just that region: a fraction of
    // the canvas for a small selection. An inverted selection (default selected) still scans
    // the whole canvas; an empty selection counts nothing.
    let region = match &sel {
        Some(m) if m.default_pixel().iter().all(|&v| v < 0.5) => m.content_bounds().intersect(&d.bounds()),
        _ => d.bounds(),
    };
    if region.is_empty() {
        return level;
    }
    let _ = photocraft_compose::render_bands(&d, region, 0, |band| -> Result<(), ()> {
        // Selection coverage per band pixel (≥ ½ counts), sampled from the mask in document
        // coordinates: each band pixel is one document pixel (or proxy cell) at `rect` + index.
        let sel_pass: Option<Vec<bool>> = sel.as_ref().map(|m| {
            let w = band.rect.width().max(1) as usize;
            band.px.iter().enumerate().map(|(i, _)| m.sample_channel(band.rect.x0 + (i % w) as i32, band.rect.y0 + (i / w) as i32, 0) >= 0.5).collect()
        });
        #[cfg(not(target_arch = "wasm32"))]
        let (part, n) = {
            use rayon::prelude::*;
            let count = |px: &[[f32; 4]], mask: Option<&[bool]>| {
                let mut part = vec![[0u32; 256]; h.len()];
                let mut n = 0u64;
                count_pixels(px, mask, space, &mut part, &mut n);
                (part, n)
            };
            match sel_pass.as_deref() {
                Some(mask) => band
                    .px
                    .par_chunks(64 * 1024)
                    .zip(mask.par_chunks(64 * 1024))
                    .map(|(px, m)| count(px, Some(m)))
                    .reduce(|| (vec![[0u32; 256]; h.len()], 0), add_bins),
                None => band.px.par_chunks(64 * 1024).map(|px| count(px, None)).reduce(|| (vec![[0u32; 256]; h.len()], 0), add_bins),
            }
        };
        #[cfg(target_arch = "wasm32")]
        let (part, n) = {
            let mut part = vec![[0u32; 256]; h.len()];
            let mut n = 0u64;
            count_pixels(&band.px, sel_pass.as_deref(), space, &mut part, &mut n);
            (part, n)
        };
        *pixels = pixels.saturating_add(n);
        for (dst, src) in h.iter_mut().zip(&part) {
            for (a, b) in dst.iter_mut().zip(src) {
                *a = a.saturating_add(*b);
            }
        }
        Ok(())
    });
    level
}

/// Bin- and pixel-wise sum of two partial counts (saturating: a hostile canvas can exceed
/// `u32`).
#[cfg(not(target_arch = "wasm32"))]
fn add_bins(mut a: (Vec<[u32; 256]>, u64), b: (Vec<[u32; 256]>, u64)) -> (Vec<[u32; 256]>, u64) {
    for (dst, src) in a.0.iter_mut().zip(&b.0) {
        for (x, y) in dst.iter_mut().zip(src) {
            *x = x.saturating_add(*y);
        }
    }
    a.1 = a.1.saturating_add(b.1);
    a
}

/// Counts straight-alpha composite pixels into `h` (one histogram per channel of `space`);
/// transparent pixels stay out of the bins. `sel`, when given, keeps only the pixels whose mask
/// coverage is ≥ ½ (the Histogram panel's [`HistScope::Selection`]); `n` counts every kept
/// pixel — the transparent ones too, which is what the panel's Pixels statistic reports.
fn count_pixels(px: &[[f32; 4]], sel: Option<&[bool]>, space: ToneSpace, h: &mut [[u32; 256]], n: &mut u64) {
    let bin = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as usize;
    let mut add = |c: usize, b: usize| {
        if let Some(n) = h.get_mut(c).and_then(|ch| ch.get_mut(b)) {
            *n = n.saturating_add(1);
        }
    };
    for (i, p) in px.iter().enumerate() {
        if sel.is_some_and(|s| !s.get(i).copied().unwrap_or(true)) {
            continue;
        }
        *n = n.saturating_add(1);
        // Skips NaN alpha too.
        if p[3].is_nan() || p[3] <= 0.0 {
            continue;
        }
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

/// Cached histograms for a tone editor; recomputed only when something other than the editor's
/// own commits changed the document (see [`keep_after_commit`]). The editors' histograms show the
/// whole input ([`HistScope::WholeInput`]), so a selection change — view-only, no pixels moved —
/// keeps the cache (#125); only the panel's masked histogram tracks the selection.
pub fn histograms(app: &mut PhotocraftApp, source: HistSource, space: ToneSpace) -> Arc<Histograms> {
    let Some(st) = app.session.active() else { return Arc::new(Histograms::default()) };
    let (doc_id, rev) = (st.doc.id, st.revision);
    let want = tag(source, space, HistScope::WholeInput, 0);
    let layer = match source {
        HistSource::BelowLayer(l) | HistSource::Layer(l) => l,
        HistSource::Channel { layer, .. } => layer,
    };
    let pixels_kept = view_only(st);
    if let Some((d, l, r, h)) = app.tone_hist.as_mut()
        && *d == doc_id
        && *l == layer
        && (*r == rev || (*r + 1 == rev && pixels_kept))
        && h.tag == want
    {
        *r = rev;
        return h.clone();
    }
    let t0 = crate::gpu_canvas::now_ms();
    let h = Arc::new(compute_histograms(&st.doc, source, space, HistScope::WholeInput));
    app.perf.span("histogram", crate::gpu_canvas::now_ms() - t0);
    app.tone_hist = Some((doc_id, layer, rev, h.clone()));
    h
}

/// Whether the latest revision changed no pixels (selecting a layer, say): cached histograms of
/// the previous revision still hold (#125).
fn view_only(st: &photocraft_engine::DocState) -> bool {
    st.last_damage.is_some_and(|r| r.is_empty())
}

/// After an editor commits its own adjustment layer, the image below it is unchanged: keep the
/// cached histogram valid for the new revision. Call with the revision seen before the commit.
pub fn keep_after_commit(app: &mut PhotocraftApp, layer: LayerId, before: Option<u64>) {
    let now = app.session.active().map(|s| s.revision);
    if let (Some((_, l, r, _)), Some(before), Some(now)) = (app.tone_hist.as_mut(), before, now)
        && *l == layer
        && *r == before
    {
        *r = now;
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

fn channel_picker(ui: &mut egui::Ui, id: egui::Id, label: &str) -> usize {
    let mut ch: usize = ui.data(|d| d.get_temp(id)).unwrap_or(0);
    ui.horizontal(|ui| {
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!(&label)).color(t.text_dim).size(12.0));
        let opts: Vec<(usize, &str)> = CHANNELS.iter().enumerate().map(|(i, (_, l))| (i, *l)).collect();
        crate::widgets::dropdown(ui, &format!("{id:?}-ch"), &mut ch, &opts, 110.0);
    });
    ui.data_mut(|d| d.insert_temp(id, ch));
    ch
}

// ---------------------------------------------------------------------------------------------
// Histogram panel

/// Histogram panel (Window › Histogram): whole-image histogram with Photoshop's statistics.
/// Recomputed at most every 250 ms while the document keeps changing (e.g. during painting).
pub fn histogram_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        ui.label(egui::RichText::new(tl!("No document")).color(t.text_faint));
        return;
    };
    let (doc_id, rev) = (st.doc.id, st.revision);
    let want = tag(HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::Selection, selection_key(&st.doc));
    let now = crate::gpu_canvas::now_ms();
    // A selection change moves no pixels but does change what the histogram counts, so only a
    // cache computed for the very same selection may follow a view-only revision (#125).
    if view_only(st)
        && let Some((d, r, _, h)) = app.doc_hist.as_mut()
        && *d == doc_id
        && *r + 1 == rev
        && h.tag == want
    {
        *r = rev;
    }
    let stale = !matches!(&app.doc_hist, Some((d, r, _, h)) if *d == doc_id && *r == rev && h.tag == want);
    let due = app.doc_hist.as_ref().is_none_or(|(d, _, at, _)| *d != doc_id || now - at > 250.0);
    if stale && due {
        // Computed off the UI thread (doc clone + proxy + pixel counts cost tens of ms on
        // large documents): the panel keeps showing the previous histogram with the existing
        // "cached data" warning until the worker delivers.
        let pending = app.hist_job.as_ref().is_some_and(|(d, r, _)| *d == doc_id && *r == rev);
        if !pending {
            let (tx, rx) = std::sync::mpsc::channel();
            // No threads on the web (and a spawn can fail natively): compute inline then, and
            // deliver through the same channel so one receive path lands both.
            #[cfg(not(target_arch = "wasm32"))]
            let spawned = {
                let doc = st.doc.clone();
                let tx = tx.clone();
                let ctx = ui.ctx().clone();
                std::thread::Builder::new()
                    .name("histogram".into())
                    .spawn(move || {
                        let t0 = crate::gpu_canvas::now_ms();
                        let h = Arc::new(compute_histograms(&doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::Selection));
                        let _ = tx.send((doc.id, crate::gpu_canvas::now_ms() - t0, h));
                        ctx.request_repaint();
                    })
                    .is_ok()
            };
            #[cfg(target_arch = "wasm32")]
            let spawned = false;
            if !spawned {
                let t0 = crate::gpu_canvas::now_ms();
                let h = Arc::new(compute_histograms(&st.doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::Selection));
                let _ = tx.send((doc_id, crate::gpu_canvas::now_ms() - t0, h));
            }
            app.hist_job = Some((doc_id, rev, rx));
        }
    } else if stale {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(260));
    }
    // A finished worker lands its histogram, but only a result computed for the very revision
    // and selection on screen is cached: the selection can change while the worker runs, and a
    // stale landing would reset the panel's 250 ms throttle without ever being shown fresh. A
    // worker that died without sending clears the job so the next frame schedules a new one.
    if let Some((jd, jr, rx)) = &app.hist_job {
        match rx.try_recv() {
            Ok((_, ms, h)) => {
                app.perf.span("histogram", ms);
                if *jd == doc_id && *jr == rev && h.tag == want {
                    app.doc_hist = Some((doc_id, *jr, crate::gpu_canvas::now_ms(), h));
                }
                app.hist_job = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => app.hist_job = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
    let Some((_, _, _, h)) = app.doc_hist.clone() else { return };
    // 1 when the statistics count every pixel; higher when a downsampled proxy stood in.
    let level = h.level.max(1);
    let ch = channel_picker(ui, egui::Id::new("histogram-panel-ch"), "Channel:");
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 100.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect_filled(r, 0.0, t.field);
    let color = if ch == 0 { Color32::from_gray(if t.pro { 190 } else { 70 }) } else { channel_color(CHANNELS[ch.min(3)].0, &t) };
    draw_histogram(&p, r, &h[ch], color);
    if stale {
        // Photoshop's "cached data" warning triangle while the histogram lags the image.
        crate::icons::paint(ui, Rect::from_min_size(r.right_top() + vec2(-18.0, 2.0), vec2(16.0, 16.0)), "triangle-alert", 12.0, t.warning);
    }
    let hist = &h[ch];
    let n: u64 = hist.iter().map(|v| *v as u64).sum();
    // A selection of only transparent pixels has no bins to draw statistics from, but its
    // pixels are still counted.
    if n == 0 && h.pixel_count == 0 {
        return;
    }
    let mean = hist.iter().enumerate().map(|(i, v)| i as f64 * *v as f64).sum::<f64>() / n.max(1) as f64;
    let var = hist.iter().enumerate().map(|(i, v)| (i as f64 - mean).powi(2) * *v as f64).sum::<f64>() / n.max(1) as f64;
    let mut acc = 0u64;
    let median = hist.iter().position(|v| {
        acc += *v as u64;
        acc * 2 >= n
    });
    ui.add_space(4.0);
    egui::Grid::new("hist-stats").num_columns(2).spacing(vec2(12.0, 2.0)).show(ui, |ui| {
        for (k, v) in [
            (tl!("Mean:"), format!("{mean:.2}")),
            (tl!("Std Dev:"), format!("{:.2}", var.sqrt())),
            (tl!("Median:"), median.unwrap_or(0).to_string()),
            (tl!("Pixels:"), h.pixel_count.to_string()),
            (tl!("Cache Level:"), level.to_string()),
        ] {
            ui.label(egui::RichText::new(k).color(t.text_dim).size(11.5));
            ui.label(egui::RichText::new(v).font(crate::theme::mono(11.5)).color(t.text));
            ui.end_row();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn histogram_ignores_the_edited_adjustment() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap(); // white
        s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        let st = s.active().unwrap();
        let h = compute_histograms(&st.doc, HistSource::BelowLayer(st.active_layer.unwrap()), ToneSpace::Rgb, HistScope::WholeInput);
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
        let h = compute_histograms(&st.doc, HistSource::Layer(bg), ToneSpace::Rgb, HistScope::WholeInput);
        assert!(h[0][255] > 0 && h[0][0] == 0, "the black layer above is hidden");
        assert_eq!(compute_histograms(&st.doc, HistSource::Layer(bg), ToneSpace::Cmyk, HistScope::WholeInput).ch.len(), 5);
        assert_eq!(compute_histograms(&st.doc, HistSource::Layer(bg), ToneSpace::Lab, HistScope::WholeInput).ch.len(), 3);
    }

    #[test]
    fn a_selection_restricts_the_panel_histogram_but_not_the_editors() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap(); // white background
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 32})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap(); // left half black
        s.execute("select.deselect", json!({})).unwrap();
        let st = s.active().unwrap();
        let whole = compute_histograms(&st.doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::WholeInput);
        assert!(whole[0][0] > 0 && whole[0][255] > 0, "black and white both count");
        assert_eq!(whole[1].iter().map(|v| *v as u64).sum::<u64>(), 32 * 32, "no selection: the whole canvas");
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 32})).unwrap();
        let st = s.active().unwrap();
        let left = compute_histograms(&st.doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::Selection);
        assert!(left[0][0] > 0 && left[0][255] == 0, "the panel counts only the selected black half");
        assert_eq!(left.pixel_count, 16 * 32, "Pixels counts the selection, transparent or not");
        let editor = compute_histograms(&st.doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::WholeInput);
        assert!(editor[0][255] > 0, "the editors show the adjustment's whole input (Photoshop)");
        assert_eq!(left[1].iter().map(|v| *v as u64).sum::<u64>(), 16 * 32, "Pixels counts the selection");
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 12, "y": 0, "width": 8, "height": 32, "feather": 2})).unwrap();
        let st = s.active().unwrap();
        let soft = compute_histograms(&st.doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::Selection);
        assert!(soft[0][0] > 0 && soft[0][255] > 0, "a feathered mask keeps its ≥ ½ core on both sides");
    }

    #[test]
    fn the_selection_cache_key_tracks_the_selection() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        assert_eq!(selection_key(&s.active().unwrap().doc), 0, "no selection");
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
        let a = s.active().unwrap().doc.clone();
        let k = selection_key(&a);
        assert_ne!(k, 0);
        assert_eq!(selection_key(&a), k, "a clone keeps the key");
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 8})).unwrap();
        assert_ne!(selection_key(&s.active().unwrap().doc), k, "a new mask is a new key");
    }

    /// Two masks with no tiles at all differ only in their default (what every pixel reads as):
    /// the key must tell an all-selected mask from a none-selected one, or the panel's histogram
    /// goes stale between them.
    #[test]
    fn tile_less_selection_masks_differ_by_their_default() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        let mut doc = s.active().unwrap().doc.as_ref().clone();
        doc.selection = Some(photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]));
        let all = selection_key(&doc);
        doc.selection = Some(photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[0.0]));
        let none = selection_key(&doc);
        assert_ne!(all, 0);
        assert_ne!(none, 0, "a mask selecting nothing is still a selection, not `None`");
        assert_ne!(all, none, "tile-less masks differ by their default");
    }

    /// A tile-less mask that selects nothing (default 0; the engine's marquee deselects instead,
    /// but Quick Mask and scripted documents can leave one) composites over an empty region:
    /// every bin stays 0. A default-selected mask (inverse of a selection) counts everything.
    #[test]
    fn a_selection_outside_the_canvas_counts_nothing() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        let mut doc = s.active().unwrap().doc.as_ref().clone();
        doc.selection = Some(photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[0.0]));
        let h = compute_histograms(&doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::Selection);
        assert!(h.ch.iter().all(|b| b.iter().all(|&v| v == 0)), "nothing passes the threshold");
        assert_eq!(h.level, 1);
        doc.selection = Some(photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]));
        let all = compute_histograms(&doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::Selection);
        assert!(all.ch.iter().all(|b| b.iter().sum::<u32>() > 0), "a default-selected mask scans the whole canvas");
        assert_eq!(all.level, 1);
    }

    /// Photoshop's Pixels statistic counts the pixels the selection covers, transparent ones
    /// included, while the histogram bins keep skipping them: a selection over a transparent
    /// canvas must report the selection's size with an empty histogram.
    #[test]
    fn the_pixels_count_keeps_transparent_selected_pixels() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32, "background": "transparent"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 32})).unwrap();
        let st = s.active().unwrap();
        let h = compute_histograms(&st.doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::Selection);
        assert!(h.ch.iter().all(|b| b.iter().all(|&v| v == 0)), "nothing visible is selected");
        assert_eq!(h.pixel_count, 16 * 32, "the transparent selected pixels still count");
    }

    /// The key's tile identities are pointers, so two masks may only hash alike if a write kept a
    /// tile's pointer while changing its bytes. Commands edit a staged clone whose tiles are
    /// shared until written, and a COW write allocates a new tile (`raster` pins that). This test
    /// pins the consequence the cache relies on: masks differing only in tile *content* — same
    /// default, same bounds, same tile coordinates — get different keys.
    #[test]
    fn selection_key_distinguishes_content_only_changes() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        let mut doc = s.active().unwrap().doc.as_ref().clone();
        let mut mask = photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[0.0]);
        mask.try_write_region(photocraft_geom::Rect::new(0, 0, 64, 64), &vec![1.0; 64 * 64]).unwrap();
        doc.selection = Some(mask.clone());
        let before = selection_key(&doc);
        // A staged edit writes through the shared tiles: same default, same bounds, same tile
        // coordinates, different content in the first quarter only.
        let mut staged = doc.clone();
        staged.selection.as_mut().unwrap().try_write_region(photocraft_geom::Rect::new(0, 0, 16, 64), &vec![0.5; 16 * 64]).unwrap();
        let after = selection_key(&staged);
        assert_ne!(before, 0);
        assert_ne!(after, 0);
        assert_ne!(before, after, "a content-only mask change must change the key");
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
        let h = compute_histograms(&st.doc, HistSource::Channel { layer, surface: GraySurface::Alpha(0) }, ToneSpace::Rgb, HistScope::WholeInput);
        assert!(h[0][64] > 0, "a spike at 25% grey, got {:?}", &h[0][0..80]);
        assert_eq!(h[0][0], 0, "not the black layer beneath");
        let m = compute_histograms(&st.doc, HistSource::Channel { layer, surface: GraySurface::LayerMask }, ToneSpace::Rgb, HistScope::WholeInput);
        assert!(m[0][0] > 0, "the hidden mask is black");
        // Each plane keeps its own cache entry: switching targets recomputes.
        assert_ne!(
            tag(HistSource::Channel { layer, surface: GraySurface::Alpha(0) }, ToneSpace::Rgb, HistScope::WholeInput, 0),
            tag(HistSource::Channel { layer, surface: GraySurface::LayerMask }, ToneSpace::Rgb, HistScope::WholeInput, 0)
        );
        assert_ne!(
            tag(HistSource::Channel { layer, surface: GraySurface::LayerMask }, ToneSpace::Rgb, HistScope::WholeInput, 0),
            tag(HistSource::Layer(layer), ToneSpace::Rgb, HistScope::WholeInput, 0)
        );
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
        compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, HistScope::Selection)
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
    /// smaller, yet still only holds black and white. It stands in whenever no selection mask is
    /// sampled — the editors always, the panel while nothing is selected — and a
    /// selection-masked count stays exact (level 1).
    #[test]
    fn large_split_image_counts_a_nearest_proxy_without_inventing_greys() {
        let s = split_doc(6000, 3000, 2001, None);
        let doc = &s.active().unwrap().doc;
        for scope in [HistScope::WholeInput, HistScope::Selection] {
            let hist = compute_histograms(doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb, scope);
            assert!(hist.level > 1, "{scope:?}: a proxy stood in, level {}", hist.level);
            let n: u64 = hist[0].iter().map(|v| u64::from(*v)).sum();
            assert!(n > 0 && n < 6000 * 3000, "{scope:?}: counted {n} proxy pixels");
            for c in 0..4 {
                let stray: Vec<(usize, u32)> = hist[c].iter().copied().enumerate().filter(|&(i, v)| v > 0 && i != 0 && i != 255).collect();
                assert!(stray.is_empty(), "{scope:?} channel {c}: invented values {stray:?}");
            }
            assert!((mean(&hist[0]) - 255.0 * 3999.0 / 6000.0).abs() < 1.0, "{scope:?} mean {}", mean(&hist[0]));
        }
    }
}

#[cfg(test)]
mod hist_async_tests {
    use super::*;
    use crate::PhotocraftApp;
    use serde_json::json;

    /// The panel schedules the histogram on a worker and keeps showing the previous data
    /// until it lands: right after the trigger the job exists (nothing computed in this
    /// frame), the receiver delivers, and a stale result cannot overwrite a newer revision.
    #[test]
    fn panel_histogram_is_computed_off_the_ui_thread() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let _ = &ctx;
        app.run("file.new", json!({"width": 640, "height": 480})).unwrap();
        let (doc_id, rev) = {
            let st = app.session.active().unwrap();
            (st.doc.id, st.revision)
        };
        // Drive one panel frame through a minimal egui pass.
        let ctx = egui::Context::default();
        ctx.begin_pass(Default::default());
        let panel_ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("hist-test"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))),
        );
        let mut panel_ui = panel_ui;
        histogram_panel(&mut app, &mut panel_ui);
        // A fast worker (or the inline fallback) may already have landed the result this frame.
        if let Some(job) = app.hist_job.take() {
            assert_eq!(job.0, doc_id);
            assert_eq!(job.1, rev);
            // The worker delivers (its repaint request is a no-op outside a frame).
            let (did, _, h) = job.2.recv().expect("the worker delivers the histogram");
            assert_eq!(did, doc_id);
            assert!(!h.ch.is_empty(), "the histogram has data");
            assert_eq!(h[1].iter().map(|v| *v as u64).sum::<u64>(), 640 * 480, "no selection: every canvas pixel is counted");
            // Delivering it for the revision it was computed from lands in the cache.
            app.doc_hist = Some((doc_id, rev, crate::gpu_canvas::now_ms(), h));
        }
        assert_eq!(app.doc_hist.as_ref().map(|h| (h.0, h.1)), Some((doc_id, rev)));
        // …and a newer revision keeps the old entry stale instead of being overwritten by it.
        app.run("layer.new.layer", json!({})).unwrap();
        let rev2 = app.session.active().unwrap().revision;
        assert_ne!(rev, rev2);
        assert_ne!(app.doc_hist.as_ref().unwrap().1, rev2, "stale after an edit");
    }

    /// A worker that died without sending (its sender dropped) clears the job, so the next
    /// frame schedules a new one instead of waiting forever.
    #[test]
    fn a_dead_histogram_worker_clears_the_job() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let (doc_id, rev) = {
            let st = app.session.active().unwrap();
            (st.doc.id, st.revision)
        };
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        app.hist_job = Some((doc_id, rev, rx));
        let ctx = egui::Context::default();
        ctx.begin_pass(Default::default());
        let mut ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("hist-dead"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))),
        );
        histogram_panel(&mut app, &mut ui);
        assert!(app.hist_job.is_none(), "a disconnected job is dropped");
        assert!(app.doc_hist.is_none(), "nothing landed from the dead worker");
    }

    /// Drives one panel frame and delivers whatever worker it scheduled (like the real panel
    /// receiving a repaint after the worker finishes).
    fn frame(app: &mut PhotocraftApp, id: &str) {
        let ctx = egui::Context::default();
        ctx.begin_pass(Default::default());
        let mut ui =
            egui::Ui::new(ctx, egui::Id::new(id), egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 400.0))));
        histogram_panel(app, &mut ui);
        if let Some((d, r, rx)) = app.hist_job.take() {
            let (did, _, h) = rx.recv().expect("the worker delivers");
            if did == d {
                app.doc_hist = Some((did, r, crate::gpu_canvas::now_ms(), h));
            }
        }
    }

    fn age_cache(app: &mut PhotocraftApp) {
        if let Some((_, _, at, _)) = app.doc_hist.as_mut() {
            *at -= 1000.0; // let the 250 ms throttle pass so the next stale frame recomputes
        }
    }

    /// A selection changes no pixels, but the histogram counts different ones: the panel must
    /// recompute for the selection instead of riding the #125 view-only fast path, and its
    /// Pixels statistic reports the selection's size.
    #[test]
    fn the_panel_histogram_follows_the_selection() {
        let pixels = |h: &std::sync::Arc<crate::tone::Histograms>| h.pixel_count;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 640, "height": 480})).unwrap();
        frame(&mut app, "hist-follow-1");
        let whole = app.doc_hist.clone().expect("landed");
        assert_eq!(pixels(&whole.3), 640 * 480);
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
        age_cache(&mut app);
        frame(&mut app, "hist-follow-2");
        let selected = app.doc_hist.clone().expect("landed");
        assert_eq!(selected.1, app.session.active().unwrap().revision, "fresh for the selection revision");
        assert_eq!(pixels(&selected.3), 200, "Pixels counts the selection");
        assert_ne!(selected.3.tag, whole.3.tag, "the selection changed the histogram's identity");
        // A later view-only edit (no selection change) keeps the selection histogram.
        app.run("layer.new.layer", json!({})).unwrap();
        age_cache(&mut app);
        frame(&mut app, "hist-follow-3");
        let after = app.doc_hist.clone().unwrap();
        assert_eq!(after.3.tag, selected.3.tag, "same selection, same histogram");
        assert_eq!(pixels(&after.3), 200);
    }

    /// A worker result computed before a selection change is dropped at the door: it must not
    /// be cached for the selection now on screen (which would reset the 250 ms throttle and
    /// delay the recomputation).
    #[test]
    fn a_result_computed_before_a_selection_change_is_not_cached() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        frame(&mut app, "hist-stale-1"); // fresh histogram for rev1, no selection
        let (doc_id, rev, at, h_old) = app.doc_hist.clone().expect("landed");
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap(); // rev2
        // A worker still holding rev1's no-selection result lands while the throttle blocks a
        // new job (doc_hist is fresher than 250 ms): the panel must reject it.
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send((doc_id, 1.0, h_old.clone()));
        app.hist_job = Some((doc_id, rev, rx));
        let ctx = egui::Context::default();
        ctx.begin_pass(Default::default());
        let mut ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("hist-stale-2"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 400.0))),
        );
        histogram_panel(&mut app, &mut ui);
        assert!(app.hist_job.is_none(), "the stale job was consumed, not kept");
        let (_, r, at_after, h) = app.doc_hist.clone().expect("the previous entry stands");
        assert_eq!((r, at_after as u64), (rev, at as u64), "the stale result was not cached over it");
        assert!(Arc::ptr_eq(&h, &h_old), "the previous histogram still stands");
    }

    /// The tone editors' histogram shows the adjustment's whole input (Photoshop's dialogs are
    /// not selection-masked), so a selection change — view-only, no pixels moved — keeps the
    /// cached histogram (#125). The panel's masked histogram is covered above.
    #[test]
    fn the_tone_histogram_cache_ignores_selection_changes() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let src = HistSource::BelowLayer(LayerId(u64::MAX));
        let first = histograms(&mut app, src, ToneSpace::Rgb);
        assert!(Arc::ptr_eq(&first, &histograms(&mut app, src, ToneSpace::Rgb)), "unchanged state reuses the cache");
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
        assert!(Arc::ptr_eq(&first, &histograms(&mut app, src, ToneSpace::Rgb)), "a selection change is view-only for the whole-input histogram");
        app.run("select.deselect", json!({})).unwrap();
        assert!(Arc::ptr_eq(&first, &histograms(&mut app, src, ToneSpace::Rgb)), "deselecting keeps it too");
    }
}
