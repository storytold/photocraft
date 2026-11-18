use craft::behavior::PixelSize;
use craft::behavior::geometry::Homography;
use craft::behavior::kurbo::Point;
use craft::imaging::ui::transform_preview::{PreviewMask, PreviewPixels};
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::primitives::Revision;

use super::{Color32, Document, LayerContent, LayerId};

/// How a source rectangle is read into texels: the integer factor `k` that brings its long side
/// within the GPU's texture limit, and the texel counts that follow. The pixels and a mask of the
/// same rectangle use one `Reduction`, so the mask has one value per texel.
struct Reduction {
    /// Source pixels wide and high.
    size: [usize; 2],
    /// Source pixels per texel along each axis.
    k: usize,
    /// Texels wide and high.
    texels: [usize; 2],
}

impl Reduction {
    /// `None` for an empty rectangle.
    fn of(b: photocraft_geom::Rect, max_side: usize) -> Option<Self> {
        let (w, h) = (b.width() as usize, b.height() as usize);
        if w == 0 || h == 0 {
            return None;
        }
        let k = w.max(h).div_ceil(max_side.max(1)).max(1);
        Some(Self { size: [w, h], k, texels: [w.div_ceil(k), h.div_ceil(k)] })
    }

    /// The uv extent of the texels that covers the source.
    fn uv(&self) -> [f32; 2] {
        [self.size[0] as f32 / (self.texels[0] * self.k) as f32, self.size[1] as f32 / (self.texels[1] * self.k) as f32]
    }
}

/// `out` in rows of `len` texels, each with its row index, in parallel on native targets.
fn par_rows<T: Send>(out: &mut [T], len: usize, row: impl Fn(usize, &mut [T]) + Sync) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(len.max(1)).enumerate().for_each(|(i, o)| row(i, o));
    }
    #[cfg(target_arch = "wasm32")]
    for (i, o) in out.chunks_mut(len.max(1)).enumerate() {
        row(i, o);
    }
}

/// Reads `b` of `surf` as premultiplied texels, at full resolution when it fits in `max_side`
/// (else box-filtered down by the smallest integer factor that fits), with the uv extent of them
/// that covers `b`. The layer's masks are [`read_mask`]'s, applied with `PreviewPixels::masked`.
pub(crate) fn read_surface(surf: &photocraft_raster::Surface, b: photocraft_geom::Rect, max_side: usize) -> PreviewPixels {
    let Some(r) = Reduction::of(b, max_side) else {
        return PreviewPixels::new(egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]), [1.0, 1.0]);
    };
    let (w, k, [tw, th]) = (r.size[0], r.k, r.texels);
    let mut px = vec![Color32::TRANSPARENT; tw * th];
    // One band of `k` source rows per output row.
    par_rows(&mut px, tw, |ty, out| {
        let y0 = b.y0 + (ty * k) as i32;
        let y1 = (y0 + k as i32).min(b.y1);
        let rows = (y1 - y0).max(0) as usize;
        let mut buf = vec![[0u8; 4]; w * rows];
        surf.read_rgba8_into(photocraft_geom::Rect::new(b.x0, y0, b.x1, y1), &mut buf);
        if k == 1 {
            for (o, p) in out.iter_mut().zip(&buf) {
                *o = if p[3] == 255 { Color32::from_rgb(p[0], p[1], p[2]) } else { Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]) };
            }
            return;
        }
        for (tx, o) in out.iter_mut().enumerate() {
            let (x0, x1) = (tx * k, ((tx + 1) * k).min(w));
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for row in 0..rows {
                for p in buf.get(row * w + x0..row * w + x1).unwrap_or(&[]) {
                    let c = Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]);
                    for (a, v) in acc.iter_mut().zip(c.to_array()) {
                        *a += u32::from(v);
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            let v = acc.map(|a| ((a + n / 2) / n) as u8);
            *o = Color32::from_rgba_premultiplied(v[0], v[1], v[2], v[3]);
        }
    });
    PreviewPixels::new(egui::ColorImage::new([tw, th], px), r.uv())
}

/// The mask `surface` over `b` at the factor [`read_surface`] reads `b` with: one value per texel
/// (the mean of the source pixels it covers), 0 hiding and 255 showing, at `density`.
pub(crate) fn read_mask(surface: &photocraft_raster::Surface, b: photocraft_geom::Rect, max_side: usize, density: f32) -> PreviewMask {
    let Some(r) = Reduction::of(b, max_side) else { return PreviewMask::new(vec![255], density) };
    let (w, k, [tw, th]) = (r.size[0], r.k, r.texels);
    let channels = surface.channels().max(1);
    let mut values = vec![255u8; tw * th];
    par_rows(&mut values, tw, |ty, out| {
        let y0 = b.y0 + (ty * k) as i32;
        let y1 = (y0 + k as i32).min(b.y1);
        let rows = (y1 - y0).max(0) as usize;
        let read = surface.read_region(photocraft_geom::Rect::new(b.x0, y0, b.x1, y1));
        let at = |x: usize, row: usize| read.get((row * w + x) * channels).copied().unwrap_or(1.0);
        for (tx, o) in out.iter_mut().enumerate() {
            let (x0, x1) = (tx * k, ((tx + 1) * k).min(w));
            let (mut sum, mut n) = (0.0f32, 0u32);
            for row in 0..rows {
                for x in x0..x1 {
                    sum += at(x, row);
                    n += 1;
                }
            }
            *o = ((sum / n.max(1) as f32).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
    PreviewMask::new(values, density)
}

/// Texels of the moving pixels over `b`: full resolution unless the long side exceeds the GPU's
/// `max_side` (#91: this used to be capped at 2048 px, so big layers previewed blurry), plus the
/// uv extent covering `b`. `bare` leaves the layer's own style, opacity and fill out of a
/// flattened render: the styled preview applies them to the moved pixels itself.
pub(super) fn preview_image(
    doc: &Document,
    id: LayerId,
    lifted: Option<&photocraft_raster::Surface>,
    b: photocraft_geom::Rect,
    max_side: usize,
    bare: bool,
) -> (egui::ColorImage, [f32; 2]) {
    let layer = doc.layer(id);
    // Groups and type/fill layers preview from a flattened render of just that layer.
    let rendered;
    let surf: Option<&photocraft_raster::Surface> = match (lifted, layer.and_then(|l| l.surface())) {
        (Some(s), _) => Some(s),
        (None, Some(s)) if layer.is_some_and(|l| matches!(l.content, LayerContent::Raster(_))) => Some(s),
        _ => {
            let mut solo = doc.clone();
            if layer.is_some() {
                solo_layer(&mut solo.layers, id);
            } else {
                // Keep the previous fallback for an invalid id: render the document, with only
                // top-level clipping disabled. Production callers get ids from this document.
                for l in &mut solo.layers {
                    l.clipped = false;
                }
            }
            if let Some(l) = solo.layer_mut(id).filter(|_| bare) {
                l.effects.enabled = false;
                l.opacity = 1.0;
                l.fill_opacity = 1.0;
            }
            let buf = photocraft_compose::render(&solo, b);
            let mut s = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
            let flat: Vec<f32> = buf.px.iter().flat_map(|p| *p).collect();
            s.write_region(b, &flat);
            rendered = s;
            Some(&rendered)
        }
    };
    let Some(surf) = surf else { return (egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]), [1.0, 1.0]) };
    let pixels = read_surface(surf, b, max_side);
    // A flattened render already has the masks applied; linked masks move with the pixels of a
    // raster layer, so its moving texels carry them (#205).
    let raw = lifted.is_some() || layer.is_some_and(|l| matches!(l.content, LayerContent::Raster(_)));
    let pixels = match layer.filter(|_| raw).and_then(|l| preview_mask(l, b, max_side)) {
        Some(mask) => pixels.masked(&mask),
        None => pixels,
    };
    (pixels.image, pixels.base_uv)
}

/// The layer's masks as the moving texels carry them: the enabled masks when all of them are linked
/// (the vector mask and feathering folded in as the compositor applies them), else the linked
/// pixel mask alone. `None` when no enabled mask moves with the pixels.
fn preview_mask(l: &photocraft_doc::Layer, b: photocraft_geom::Rect, max_side: usize) -> Option<PreviewMask> {
    let pixel = l.mask.as_ref().filter(|m| m.enabled);
    let vector = l.vector_mask.as_ref().filter(|v| v.enabled);
    let linked_pixel = pixel.filter(|m| m.linked);
    if pixel.is_none_or(|m| m.linked)
        && vector.is_none_or(|v| v.linked)
        && let Some(surface) = photocraft_compose::masks::combined_mask(l, b)
    {
        return Some(read_mask(&surface, b, max_side, 1.0));
    }
    linked_pixel.map(|m| read_mask(&m.surface, b, max_side, m.density))
}

/// The layers covering the moving one ([`covering`]), composited on their own and drawn over its
/// preview, so the layer keeps its place in the stack while the box is open (#2439): they cover
/// it, as in Photoshop, instead of the moving pixels covering the whole document. The canvas
/// under the box leaves them out ([`Above::hide_in`]). Built when the session starts and again
/// when an edit changes the document; the box itself moves without re-compositing.
pub(super) struct Above {
    pub(super) max_side: usize,
    /// The covering layers (whole layers or groups, at any depth).
    pub(super) ids: Vec<LayerId>,
    /// Their composite (premultiplied texels) and the canvas area it covers; `None` when nothing
    /// visible covers the layer.
    pub(super) shown: Option<(egui::ColorImage, photocraft_geom::Rect)>,
    /// Counts the composites made, for the raster display's texture cache.
    revision: u64,
}

/// What covers a layer ([`covering`]) and the document to composite it from: the cheap half of
/// [`Above`], so the composite ([`AbovePlan::image`]) can run off the UI thread.
pub(super) struct AbovePlan {
    pub(super) ids: Vec<LayerId>,
    up: Document,
    rect: photocraft_geom::Rect,
}

impl AbovePlan {
    pub(super) fn new(doc: &Document, id: LayerId) -> Self {
        let (mut ids, mut applied) = (Vec::new(), Vec::new());
        covering(&doc.layers, id, &mut ids, &mut applied);
        let mut up = doc.clone();
        let mut covers = doc.clone();
        show_only(&mut covers.layers, &ids);
        // Adjustment layers over the covering ones (`applied`) stay in the document under the box
        // as well, and are composited with the covering layers so those keep the adjustment.
        let keep: Vec<LayerId> = ids.iter().chain(&applied).copied().collect();
        show_only(&mut up.layers, &keep);
        let canvas = up.bounds();
        // An adjustment touches only pixels the covering layers have, so they alone bound the composite.
        let rect = covers
            .layers
            .iter()
            .filter(|l| l.visible)
            .fold(photocraft_geom::Rect::EMPTY, |r, l| r.union(&photocraft_compose::composite_bounds(l, canvas).unwrap_or(canvas)))
            .intersect(&canvas);
        Self { ids, up, rect }
    }

    /// How many canvas pixels the covering composite spans.
    pub(super) fn area(&self) -> u64 {
        self.rect.size().area()
    }

    /// The covering layers' composite (the heavy part; any thread). `None` when nothing covers.
    pub(super) fn image(&self, max_side: usize) -> Option<(egui::ColorImage, [f32; 2])> {
        (!self.rect.is_empty()).then(|| composite_image(&self.up, self.rect, max_side))
    }

    /// Leave the covering layers out of `layers` (the document under the box).
    pub(super) fn hide_in(&self, layers: &mut [photocraft_doc::Layer]) {
        hide_ids(&self.ids, layers);
    }
}

fn hide_ids(ids: &[LayerId], layers: &mut [photocraft_doc::Layer]) {
    for l in layers.iter_mut() {
        if ids.contains(&l.id) {
            l.visible = false;
        } else if let LayerContent::Group(g) = &mut l.content {
            hide_ids(ids, &mut g.children);
        }
    }
}

impl Above {
    // PLAN: `new` and `from_plan` lost their `ctx` and `name` arguments (the composite is a
    // `RasterSource`, not an uploaded texture); the callers in begin.rs (tools-w27) pass neither.
    pub(super) fn new(doc: &Document, id: LayerId, max_side: usize) -> Self {
        let plan = AbovePlan::new(doc, id);
        let image = plan.image(max_side);
        Self::from_plan(max_side, plan, image)
    }

    /// An `Above` from its plan and the composite made from it.
    pub(super) fn from_plan(max_side: usize, plan: AbovePlan, image: Option<(egui::ColorImage, [f32; 2])>) -> Self {
        Self { max_side, ids: plan.ids, shown: image.map(|(image, _)| (image, plan.rect)), revision: 0 }
    }

    /// Composite what covers layer `id` of `doc` (the live document) again.
    pub(super) fn rebuild(&mut self, doc: &Document, id: LayerId) {
        let plan = AbovePlan::new(doc, id);
        let image = plan.image(self.max_side);
        let revision = self.revision + 1;
        *self = Self::from_plan(self.max_side, plan, image);
        self.revision = revision;
    }

    /// Leave the covering layers out of `layers` (the document under the box).
    pub(super) fn hide_in(&self, layers: &mut [photocraft_doc::Layer]) {
        hide_ids(&self.ids, layers);
    }

    /// The map that places the composite's unit square on the canvas area it covers; `None` when
    /// nothing covers the layer.
    pub(super) fn frame(&self) -> Option<Homography> {
        let (_, r) = self.shown.as_ref()?;
        let at = |x: i32, y: i32| Point::new(f64::from(x), f64::from(y));
        Homography::unit_to_quad([at(r.x0, r.y0), at(r.x1, r.y0), at(r.x1, r.y1), at(r.x0, r.y1)])
    }
}

/// The composite as a raster source: the texels as they are, at the composite's own size (it can
/// be smaller than the area it covers when the area was reduced to fit the texture limit; the
/// display's [frame](Above::frame) scales it).
impl RasterSource for Above {
    fn size(&self) -> PixelSize {
        self.shown.as_ref().map_or(PixelSize::new(0, 0), |(image, _)| PixelSize::new(image.size[0] as u32, image.size[1] as u32))
    }

    fn revision(&self) -> Revision {
        Revision::new(self.revision)
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        // A new composite replaces every texel, so only an unchanged revision has no damage.
        (since == self.revision()).then(PixelRect::default)
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut egui::ColorImage) {
        let Some((image, _)) = &self.shown else { return };
        let factor = factor.max(1) as i64;
        let [width, height] = image.size;
        let [out_width, out_height] = out.size;
        for y in 0..out_height {
            for x in 0..out_width {
                let sx = i64::from(region.x0) + x as i64 * factor;
                let sy = i64::from(region.y0) + y as i64 * factor;
                let inside = sx >= 0 && sy >= 0 && (sx as usize) < width && (sy as usize) < height;
                let texel = if inside { image.pixels.get(sy as usize * width + sx as usize).copied() } else { None };
                if let Some(o) = out.pixels.get_mut(y * out_width + x) {
                    *o = texel.unwrap_or(Color32::TRANSPARENT);
                }
            }
        }
    }
}

/// `rect` of `doc`'s composite as premultiplied texels: full resolution when it fits in
/// `max_side`, composited in bands (no full-size float copy), else area-averaged down to fit.
pub(super) fn composite_image(doc: &Document, rect: photocraft_geom::Rect, max_side: usize) -> (egui::ColorImage, [f32; 2]) {
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let to_color = |p: &[u8; 4]| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]);
    if w.max(h) <= max_side.max(1) {
        let mut px = Vec::with_capacity(w * h);
        let _ = photocraft_compose::render_bands(doc, rect, 0, |band| {
            px.extend(band.to_rgba8().pixels.as_chunks::<4>().0.iter().map(to_color));
            Ok::<(), ()>(())
        });
        px.resize(w * h, Color32::TRANSPARENT);
        return (egui::ColorImage::new([w, h], px), [1.0, 1.0]);
    }
    let k = w.max(h).div_ceil(max_side.max(1));
    let (tw, th) = (w.div_ceil(k).max(1), h.div_ceil(k).max(1));
    let buf = photocraft_compose::render_reduced_rect(doc, rect, tw as u32, th as u32);
    let px: Vec<Color32> = buf.to_rgba8().pixels.as_chunks::<4>().0.iter().map(to_color).collect();
    let [bw, bh] = [buf.rect.width() as usize, buf.rect.height() as usize];
    if px.len() != bw * bh || bw == 0 || bh == 0 {
        return (egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]), [1.0, 1.0]);
    }
    (egui::ColorImage::new([bw, bh], px), [1.0, 1.0])
}

/// A layer that composites over anything the same as over transparency, so it can be drawn over
/// the moving pixels on its own: a Normal layer that isn't an adjustment, or a Normal or Pass
/// Through group of such layers (hidden layers count, they draw nothing). Adjustment layers and
/// the other blend modes depend on what is under them.
fn plain(l: &photocraft_doc::Layer) -> bool {
    use photocraft_color::BlendMode;
    !l.visible
        || match &l.content {
            LayerContent::Adjustment(_) => false,
            LayerContent::Group(g) => matches!(l.blend, BlendMode::Normal | BlendMode::PassThrough) && g.children.iter().all(plain),
            _ => l.blend == BlendMode::Normal,
        }
}

/// The layers over `id` in `layers` (bottom to top, groups holding `id` searched too) that cover
/// it in the preview, into `out`: every [`plain`] one, from the top down. A layer that isn't plain
/// (an adjustment, a blend mode) is passed over and stays in the document under the box: it can't
/// be drawn on its own, so the plain layers under it no longer wait for it to cover the moving
/// layer (#9387). An adjustment layer over covering layers goes to `applied`: it stays under the
/// box and is composited with them as well, so they keep its adjustment. A clipping group goes as
/// a whole, and the layers clipped to `id` (or to the group holding it) stay with it.
pub(super) fn covering(layers: &[photocraft_doc::Layer], id: LayerId, out: &mut Vec<LayerId>, applied: &mut Vec<LayerId>) {
    // Adjustments met since the last covering layer: they count once one lies under them.
    let mut pending: Vec<LayerId> = Vec::new();
    let mut end = layers.len();
    while end > 0 {
        // The clipping group ending at `end`: its base and the layers clipped to it.
        let mut start = end - 1;
        while start > 0 && layers.get(start).is_some_and(|l| l.clipped) {
            start -= 1;
        }
        let unit = layers.get(start..end).unwrap_or_default();
        if unit.iter().any(|l| contains(l, id)) {
            // A group holding `id` with nothing clipped to it: go on inside it.
            if let [g] = unit
                && g.id != id
                && let LayerContent::Group(group) = &g.content
                && matches!(g.blend, photocraft_color::BlendMode::Normal | photocraft_color::BlendMode::PassThrough)
            {
                let before = out.len();
                covering(&group.children, id, out, applied);
                if out.len() > before {
                    applied.append(&mut pending);
                }
            }
            return;
        }
        if unit.iter().all(plain) {
            out.extend(unit.iter().map(|l| l.id));
            if unit.iter().any(|l| l.visible) {
                applied.append(&mut pending);
            }
        } else if let [l] = unit
            && l.visible
            && matches!(l.content, LayerContent::Adjustment(_))
        {
            pending.push(l.id);
        }
        end = start;
    }
}

/// Hide everything in `layers` but the `keep` layers and the groups holding them.
pub(super) fn show_only(layers: &mut [photocraft_doc::Layer], keep: &[LayerId]) {
    for l in layers.iter_mut() {
        if keep.contains(&l.id) {
            continue;
        }
        match &mut l.content {
            LayerContent::Group(g) if g.children.iter().any(|c| keep.iter().any(|k| contains(c, *k))) => {
                // This ancestor only clips the covering children. Its background is already
                // on the base canvas; drawing it again would cover the moving layer in white.
                if let Some(board) = &mut g.artboard {
                    board.background = photocraft_doc::ArtboardBackground::Transparent;
                }
                show_only(&mut g.children, keep);
            }
            _ => l.visible = false,
        }
    }
}

/// Leave only `id`, its descendants and the groups holding it visible. Every layer on that path
/// is detached from its clipping base because the base is hidden in this isolated render (#5239).
/// Hide every layer but `id` and the groups holding it, and unclip those, so a render shows that
/// layer alone (the Free Transform preview, Liquify's one-layer backdrop).
pub(crate) fn solo_layer(layers: &mut [photocraft_doc::Layer], id: LayerId) {
    for l in layers {
        if l.id == id {
            l.clipped = false;
            continue;
        }
        match &mut l.content {
            LayerContent::Group(g) if g.children.iter().any(|c| contains(c, id)) => {
                l.clipped = false;
                // Isolate the child, not the artboard's paper. Keep the board for clipping.
                if let Some(board) = &mut g.artboard {
                    board.background = photocraft_doc::ArtboardBackground::Transparent;
                }
                solo_layer(&mut g.children, id);
            }
            _ => l.visible = false,
        }
    }
}

fn contains(l: &photocraft_doc::Layer, id: LayerId) -> bool {
    l.id == id || matches!(&l.content, LayerContent::Group(g) if g.children.iter().any(|c| contains(c, id)))
}
