//! Layer styles in the Free Transform live preview (#3653).
//!
//! The box previews the moving pixels as a texture on a mesh, which leaves the layer's style out:
//! Stroke, Drop Shadow and the rest would vanish while the box was open. Baking the style into
//! that texture would go wrong as soon as the box scales or turns, because a style isn't
//! transformed with its layer: it is drawn again from the layer's new shape (a stroke keeps its
//! width, a shadow its angle and distance). So a styled layer is composited instead. The
//! framework's `StyledPreview` plans the work (resolution, budget, reach), resamples the moving
//! texels through the box and keeps the result; [`LayerStyleSource`] is the compositor it asks:
//! the style is laid over the resampled texels by the CPU compositor's `composite_with_effects`
//! (the code that draws the layer once the transform is applied).
//!
//! The composite is drawn over the preview document's backdrop, so a layer's blend mode and
//! effects show the same result while the box is open as they do after the commit.

use craft::imaging::ui::transform_preview::{StylePlan, StyledContent, StyledSource};
use craft::ui::canvas::PixelRect;
use egui::{Color32, ColorImage};
use photocraft_color::{BlendMode, PixelFormat};
use photocraft_compose::Buffer;
use photocraft_compose::effects::{self, TextureCtx};
use photocraft_doc::{Document, Effect, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::PhotocraftApp;

/// Rough cost of compositing one work pixel with `layer`'s style, in nanoseconds: measured per
/// effect at default settings on an Apple M3 Pro, release build. A Stroke costs the most by far
/// (two chamfer distance fields over the whole composite, 46 ns whatever its width); shadows and
/// glows build one blurred map (5 to 7 ns), Satin and Bevel & Emboss a little more (8 to 14 ns)
/// and overlays next to nothing. Resampling, laying the style on and the upload add about 12 ns.
pub fn pixel_cost_ns(layer: &Layer) -> f64 {
    let enabled = || layer.effects.items.iter().filter(|e| e.enabled());
    let maps: f64 = enabled()
        .map(|e| match e {
            Effect::Stroke(_) | Effect::ColorOverlay { .. } | Effect::GradientOverlay { .. } | Effect::PatternOverlay { .. } => 1.0,
            Effect::Satin(_) | Effect::BevelEmboss(_) => 12.0,
            Effect::DropShadow(_) | Effect::InnerShadow(_) | Effect::OuterGlow(_) | Effect::InnerGlow(_) => 6.0,
        })
        .sum();
    // Every Stroke reads the same two distance fields.
    let fields = if enabled().any(|e| matches!(e, Effect::Stroke(_))) { 46.0 } else { 0.0 };
    12.0 + maps + fields
}

/// The moving layer as the compositor styles it at `scale`: its style at that scale with its
/// opacity, fill and Advanced Blending, over plain pixels (the texels are rendered already, with
/// the masks applied).
pub fn work_layer(layer: &Layer, scale: f64) -> Layer {
    let mut l = layer.clone();
    l.content = LayerContent::Raster(Surface::new(PixelFormat::RGBA8));
    l.mask = None;
    l.vector_mask = None;
    l.fill_cache = None;
    photocraft_engine::image_cmds::scale_effects_preview(&mut l.effects, scale as f32);
    l.effects.reference = l.effects.reference.map(|(x, y)| (x * scale, y * scale));
    l
}

/// The Free Transform session's moving layer as the framework's styled preview composites it.
pub struct LayerStyleSource<'a> {
    /// The document: its global light and patterns.
    doc: &'a Document,
    /// The preview document, with the moving layer hidden and the covering layers removed.
    preview: &'a Document,
    backdrop_cache: Option<Arc<Mutex<BackdropCache>>>,
    preview_revision: u64,
    /// The moving layer as it is now: its style, opacity, fill and Advanced Blending, so edits
    /// made while the box is open show.
    layer: &'a Layer,
    /// Changes whenever the document or the layer's opacity or fill does.
    revision: u64,
}

impl<'a> LayerStyleSource<'a> {
    /// The source of the open session's moving layer. `None` for a layer without a style (also
    /// once it is switched off while the box is open) or a hidden one: the bare pixels are drawn
    /// instead.
    pub fn new(app: &'a PhotocraftApp) -> Option<Self> {
        let (t, pv) = (app.ui.transform.as_ref()?, app.transform_preview.as_ref()?);
        let st = app.session.active().filter(|st| st.doc.id == pv.doc.id)?;
        let layer = st
            .doc
            .layer(LayerId(t.layer))
            .filter(|l| l.visible && (effects::has_effects(l) || !matches!(l.blend, BlendMode::Normal | BlendMode::PassThrough)))?;
        Some(Self {
            doc: &st.doc,
            preview: &pv.doc,
            backdrop_cache: Some(Arc::clone(&pv.backdrop_cache)),
            preview_revision: st.revision,
            layer,
            revision: revision_of(st.revision, layer),
        })
    }
}

/// The document's `revision` mixed with what the preview shows beyond the document's pixels.
fn revision_of(revision: u64, layer: &Layer) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (revision, layer.id.0, layer.opacity.to_bits(), layer.fill_opacity.to_bits(), layer.blend).hash(&mut hasher);
    hasher.finish()
}

impl StyledSource for LayerStyleSource<'_> {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn pixel_cost_ns(&self) -> f64 {
        pixel_cost_ns(self.layer)
    }

    fn reach(&self, scale: f64) -> i32 {
        effects::margin(&work_layer(self.layer, scale))
    }

    fn compose(&self, plan: &StylePlan, content: &StyledContent) -> Option<ColorImage> {
        let work = work_layer(self.layer, plan.scale);
        compose(self.preview, self.doc, &work, plan, content, self.backdrop_cache.as_ref(), self.preview_revision).map(|buf| image(&buf))
    }
}

/// `r` as the compositor's rectangle.
fn rect(r: PixelRect) -> Rect {
    Rect::new(r.x0, r.y0, r.x1, r.y1)
}

/// Composite the resampled `content` with `work`'s style and blend mode over `plan.shown`.
/// `preview` supplies the layers below the moving layer; `doc` gives global light and patterns.
/// `None` when `content` does not fill its rectangle.
fn compose(
    preview: &Document,
    doc: &Document,
    work: &Layer,
    plan: &StylePlan,
    content: &StyledContent,
    backdrop_cache: Option<&Arc<Mutex<BackdropCache>>>,
    preview_revision: u64,
) -> Option<Buffer> {
    let content_rect = rect(content.rect);
    if content.texels.len() != content_rect.width() as usize * content_rect.height() as usize {
        return None;
    }
    let content = Buffer { rect: content_rect, px: content.texels.clone() };
    let bounds = rect(plan.bounds);
    let shape: Vec<f32> = content.px.iter().map(|p| p[3]).collect();
    let anchor = work.effects.reference.unwrap_or((f64::from(bounds.x0), f64::from(bounds.y0)));
    let maps = effects::build_maps(work, shape, content_rect, &doc.global_light, &TextureCtx { rect: content_rect, patterns: &doc.patterns, anchor });
    // The maps hold what every pixel needs of its surroundings, so the style is laid over the
    // content band by band, in parallel, as the compositor does tile by tile.
    let shown = rect(plan.shown);
    let width = shown.width() as usize;
    let mut out = Buffer::transparent(shown);
    bands(&mut out.px, width * BAND_ROWS, |i, px| {
        let y0 = shown.y0.saturating_add((i * BAND_ROWS) as i32);
        let band = Rect::new(shown.x0, y0, shown.x1, y0.saturating_add((px.len() / width.max(1)) as i32));
        let blended = !matches!(work.blend, BlendMode::Normal | BlendMode::PassThrough);
        let mut over =
            if !blended { Buffer::transparent(band) } else { scaled_backdrop_with_cache(preview, band, plan.scale, backdrop_cache, preview_revision) };
        let backdrop = blended.then(|| over.clone());
        effects::composite_with_effects(work, &crop(&content, band), &mut over, &maps, bounds, &doc.patterns);
        if let Some(backdrop) = backdrop.as_ref() {
            for (result, backdrop) in over.px.iter_mut().zip(&backdrop.px) {
                *result = overlay_without_backdrop(*result, *backdrop);
            }
        }
        // Copied, not moved out, so `over` hands its band back to the scratch pool on drop.
        for (o, p) in px.iter_mut().zip(over.px.iter().copied()) {
            *o = p;
        }
    });
    Some(out)
}

/// Render the stationary document in document coordinates, then map it into the work-pixel
/// coordinates used by the styled preview. Rendering `shown` directly would sample the wrong
/// document pixels whenever the style budget lowers `plan.scale` below one.
/// A small LRU-like tile cache shared by one open transform. It is deliberately bounded: moving
/// the box can visit many tiles, but retaining a handful is enough to avoid re-rendering the
/// stationary backdrop on every geometry update without growing with document size.
#[derive(Default)]
pub(crate) struct BackdropCache {
    revision: u64,
    scale: u64,
    tiles: HashMap<Rect, Buffer>,
}

fn scaled_backdrop_with_cache(doc: &Document, shown: Rect, scale: f64, cache: Option<&Arc<Mutex<BackdropCache>>>, revision: u64) -> Buffer {
    const TILE_WIDTH: i32 = 128;
    const TILE_HEIGHT: i32 = 64;
    let mut out = Buffer::transparent(shown);
    let mut y = shown.y0;
    while y < shown.y1 {
        let y1 = y.saturating_add(TILE_HEIGHT).min(shown.y1);
        let mut x = shown.x0;
        while x < shown.x1 {
            let tile = Rect::new(x, y, x.saturating_add(TILE_WIDTH).min(shown.x1), y1);
            let part = if let Some(cache) = cache {
                let mut cache = cache.lock().expect("backdrop cache lock");
                let scale_key = scale.to_bits();
                if cache.revision != revision || cache.scale != scale_key {
                    cache.revision = revision;
                    cache.scale = scale_key;
                    cache.tiles.clear();
                }
                if let Some(part) = cache.tiles.get(&tile) {
                    part.clone()
                } else {
                    let part = scaled_backdrop_tile(doc, tile, scale);
                    if cache.tiles.len() >= 32 {
                        if let Some(key) = cache.tiles.keys().next().copied() {
                            cache.tiles.remove(&key);
                        }
                    }
                    cache.tiles.insert(tile, part.clone());
                    part
                }
            } else {
                scaled_backdrop_tile(doc, tile, scale)
            };
            copy_into(&mut out, &part);
            x = tile.x1;
        }
        y = y1;
    }
    out
}

fn scaled_backdrop_tile(doc: &Document, shown: Rect, scale: f64) -> Buffer {
    let scale = scale.max(f64::EPSILON);
    let source_rect = Rect::new(
        (f64::from(shown.x0) / scale).floor() as i32,
        (f64::from(shown.y0) / scale).floor() as i32,
        (f64::from(shown.x1) / scale).ceil() as i32,
        (f64::from(shown.y1) / scale).ceil() as i32,
    );
    let source = photocraft_compose::render(doc, source_rect);
    let mut out = Buffer::transparent(shown);
    if source.rect.is_empty() {
        return out;
    }
    let sw = source.rect.width().max(1) as usize;
    for y in shown.y0..shown.y1 {
        let sy = (f64::from(y) + 0.5) / scale - f64::from(source.rect.y0) - 0.5;
        let y0 = sy.floor();
        let fy = sy - y0;
        for x in shown.x0..shown.x1 {
            let sx = (f64::from(x) + 0.5) / scale - f64::from(source.rect.x0) - 0.5;
            let x0 = sx.floor();
            let fx = sx - x0;
            let sample = |ix: f64, iy: f64| {
                let ix = ix.clamp(0.0, f64::from(source.rect.width() - 1)) as usize;
                let iy = iy.clamp(0.0, f64::from(source.rect.height() - 1)) as usize;
                source.px[iy * sw + ix]
            };
            let a = sample(x0, y0);
            let b = sample(x0 + 1.0, y0);
            let c = sample(x0, y0 + 1.0);
            let d = sample(x0 + 1.0, y0 + 1.0);
            let mix = |i| {
                let fx = fx as f32;
                let fy = fy as f32;
                let alpha = a[3] * (1.0 - fx) * (1.0 - fy) + b[3] * fx * (1.0 - fy) + c[3] * (1.0 - fx) * fy + d[3] * fx * fy;
                if alpha <= 0.0 {
                    return 0.0;
                }
                let premul = a[i] * a[3] * (1.0 - fx) * (1.0 - fy) + b[i] * b[3] * fx * (1.0 - fy) + c[i] * c[3] * (1.0 - fx) * fy + d[i] * d[3] * fx * fy;
                premul / alpha
            };
            let index = (y - shown.y0) as usize * shown.width() as usize + (x - shown.x0) as usize;
            let alpha = a[3] * (1.0 - fx as f32) * (1.0 - fy as f32)
                + b[3] * fx as f32 * (1.0 - fy as f32)
                + c[3] * (1.0 - fx as f32) * fy as f32
                + d[3] * fx as f32 * fy as f32;
            out.px[index] = [mix(0), mix(1), mix(2), alpha];
        }
    }
    out
}

/// Keep only the moving layer's contribution: the document backdrop is already painted behind
/// the styled texture by the canvas. Pixels where the blend result equals the backdrop become
/// transparent, avoiding a second source-over of translucent backdrop pixels.
fn overlay_without_backdrop(result: [f32; 4], backdrop: [f32; 4]) -> [f32; 4] {
    const EPS: f32 = 1.0e-5;
    let result_premul = [result[0] * result[3], result[1] * result[3], result[2] * result[3]];
    let backdrop_premul = [backdrop[0] * backdrop[3], backdrop[1] * backdrop[3], backdrop[2] * backdrop[3]];
    let alpha = if backdrop[3] < 1.0 - EPS {
        ((result[3] - backdrop[3]) / (1.0 - backdrop[3])).clamp(0.0, 1.0)
    } else if result[3] <= EPS || result_premul.iter().zip(backdrop_premul).all(|(a, b)| (a - b).abs() <= EPS) {
        0.0
    } else {
        1.0
    };
    if alpha <= EPS {
        return [0.0; 4];
    }
    let rgb = [0, 1, 2].map(|i| (result_premul[i] - backdrop_premul[i] * (1.0 - alpha)) / alpha);
    [rgb[0].clamp(0.0, 1.0), rgb[1].clamp(0.0, 1.0), rgb[2].clamp(0.0, 1.0), alpha]
}

fn copy_into(dst: &mut Buffer, src: &Buffer) {
    let part = dst.rect.intersect(&src.rect);
    if part.is_empty() {
        return;
    }
    let (sw, dw, n) = (src.rect.width() as usize, dst.rect.width() as usize, part.width() as usize);
    for y in part.y0..part.y1 {
        let s0 = (y - src.rect.y0) as usize * sw + (part.x0 - src.rect.x0) as usize;
        let d0 = (y - dst.rect.y0) as usize * dw + (part.x0 - dst.rect.x0) as usize;
        if let (Some(source), Some(target)) = (src.px.get(s0..s0 + n), dst.px.get_mut(d0..d0 + n)) {
            target.copy_from_slice(source);
        }
    }
}

/// Rows composited per parallel band.
const BAND_ROWS: usize = 64;

/// `rect` of `buf` (transparent where `buf` doesn't reach).
fn crop(buf: &Buffer, rect: Rect) -> Buffer {
    let mut out = Buffer::transparent(rect);
    let part = rect.intersect(&buf.rect);
    if part.is_empty() {
        return out;
    }
    let (sw, dw, n) = (buf.rect.width() as usize, rect.width() as usize, part.width() as usize);
    for y in part.y0..part.y1 {
        let s0 = (y - buf.rect.y0) as usize * sw + (part.x0 - buf.rect.x0) as usize;
        let d0 = (y - rect.y0) as usize * dw + (part.x0 - rect.x0) as usize;
        if let (Some(src), Some(dst)) = (buf.px.get(s0..s0 + n), out.px.get_mut(d0..d0 + n)) {
            dst.copy_from_slice(src);
        }
    }
    out
}

/// `out` in chunks of `len`, each with its index, in parallel on native targets.
fn bands<T: Send>(out: &mut [T], len: usize, band: impl Fn(usize, &mut [T]) + Sync) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.par_chunks_mut(len.max(1)).enumerate().for_each(|(i, o)| band(i, o));
    }
    #[cfg(target_arch = "wasm32")]
    for (i, o) in out.chunks_mut(len.max(1)).enumerate() {
        band(i, o);
    }
}

/// `buf` as premultiplied texels.
fn image(buf: &Buffer) -> ColorImage {
    let (w, h) = (buf.rect.width() as usize, buf.rect.height() as usize);
    let mut px = vec![Color32::TRANSPARENT; w * h];
    bands(&mut px, w, |y, row| {
        let src = buf.px.get(y * w..).unwrap_or(&[]);
        for (o, p) in row.iter_mut().zip(src) {
            let v = p.map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            *o = Color32::from_rgba_unmultiplied(v[0], v[1], v[2], v[3]);
        }
    });
    ColorImage::new([w, h], px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::behavior::geometry::Homography;
    use craft::behavior::kurbo::Point;
    use craft::imaging::ui::transform_preview::{StyledBudget, resample};
    use photocraft_color::Color;
    use photocraft_doc::{Contour, FxCommon, FxPaint, Shadow, StrokeFx, StrokePosition};

    const BLUE: Color32 = Color32::from_rgb(0, 0, 255);

    fn stroke(size: f32) -> Effect {
        Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            size,
            position: StrokePosition::Outside,
            paint: FxPaint::Color(Color::rgb(1.0, 0.0, 0.0)),
            overprint: false,
        })
    }

    fn shadow(distance: f32) -> Effect {
        Effect::DropShadow(Shadow {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            color: Color::rgb(0.0, 1.0, 0.0),
            // Lit from the top-left: the shadow falls to the bottom-right.
            angle: 135.0,
            use_global_light: false,
            distance,
            spread: 1.0,
            size: 0.0,
            contour: Contour::Linear,
            anti_alias: false,
            noise: 0.0,
            knocks_out: true,
        })
    }

    fn styled(effects: Vec<Effect>) -> Layer {
        let mut l = Layer::raster("styled", PixelFormat::RGBA8);
        l.effects.items = effects;
        l
    }

    fn pattern_stroke_scale(l: &Layer) -> Option<f32> {
        l.effects.items.iter().find_map(|e| match e {
            Effect::Stroke(StrokeFx { paint: FxPaint::Pattern { scale, .. }, .. }) => Some(*scale),
            _ => None,
        })
    }

    /// The preview draws a pattern at the work scale exactly: the dialog's 1..1000 % range is for
    /// stored styles, so a 2 % pattern at a quarter of the size is 0.5 %, and an imported 2000 %
    /// one stays 2000 % at full size, as the committed style draws them.
    #[test]
    fn work_layer_scales_patterns_past_the_dialog_range() {
        let with_pattern = |scale: f32| {
            let Effect::Stroke(mut st) = stroke(3.0) else { panic!("a stroke") };
            st.paint = FxPaint::Pattern { name: "Bubbles".into(), id: String::new(), scale };
            styled(vec![Effect::Stroke(st)])
        };
        assert_eq!(pattern_stroke_scale(&work_layer(&with_pattern(0.02), 0.25)), Some(0.005));
        assert_eq!(pattern_stroke_scale(&work_layer(&with_pattern(20.0), 1.0)), Some(20.0));
    }

    fn doc() -> Document {
        Document::new("t", photocraft_geom::Size::new(200, 200), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8)
    }

    fn rect_quad(r: [f64; 4]) -> [Point; 4] {
        [Point::new(r[0], r[1]), Point::new(r[2], r[1]), Point::new(r[2], r[3]), Point::new(r[0], r[3])]
    }

    /// The plan the framework makes for `quad` and the content it resamples from `texels`.
    fn planned(source: &LayerStyleSource, texels: &ColorImage, quad: [Point; 4], zoom: f32) -> (StylePlan, StyledContent) {
        let pixels = StyledBudget::default().pixels(source.pixel_cost_ns(), false);
        let plan = StylePlan::new(quad, [200, 200], zoom, pixels, |scale| source.reach(scale)).expect("on the canvas");
        let scaled = quad.map(|p| Point::new(p.x * plan.scale, p.y * plan.scale));
        let to_unit = Homography::unit_to_quad(scaled).and_then(|h| h.inverse()).expect("invertible");
        let inside = Point::new(scaled.iter().map(|p| p.x / 4.0).sum(), scaled.iter().map(|p| p.y / 4.0).sum());
        let content = resample(texels, [1.0, 1.0], &to_unit, inside, plan.content, false);
        (plan, content)
    }

    /// The composite of a blue `w`×`h` block of texels moved onto `quad`, and its plan.
    fn composite(layer: &Layer, w: usize, h: usize, quad: [Point; 4], zoom: f32) -> (ColorImage, StylePlan) {
        let texels = ColorImage::new([w, h], vec![BLUE; w * h]);
        let doc = doc();
        let source = LayerStyleSource { doc: &doc, preview: &doc, backdrop_cache: None, preview_revision: 0, layer, revision: 0 };
        let (plan, content) = planned(&source, &texels, quad, zoom);
        (source.compose(&plan, &content).expect("composited"), plan)
    }

    /// The composite at a document point.
    fn at(out: &ColorImage, plan: &StylePlan, x: f64, y: f64) -> Color32 {
        let (x, y) = ((x * plan.scale).floor() as i32, (y * plan.scale).floor() as i32);
        let shown = plan.shown;
        if x < shown.x0 || x >= shown.x1 || y < shown.y0 || y >= shown.y1 {
            return Color32::TRANSPARENT;
        }
        out.pixels.get((y - shown.y0) as usize * shown.width() as usize + (x - shown.x0) as usize).copied().unwrap_or(Color32::TRANSPARENT)
    }

    fn is(p: Color32, rgb: [u8; 3]) -> bool {
        p.a() == 255 && p.to_array().iter().zip(rgb).all(|(a, b)| a.abs_diff(b) <= 5)
    }

    #[test]
    fn a_non_normal_layer_blends_with_the_preview_backdrop() {
        let doc = Document::with_background(
            "t",
            photocraft_geom::Size::new(200, 200),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
            Color::WHITE,
        );
        let mut layer = Layer::raster("screen", PixelFormat::RGBA8);
        layer.blend = BlendMode::Screen;
        let source = LayerStyleSource { doc: &doc, preview: &doc, backdrop_cache: None, preview_revision: 0, layer: &layer, revision: 0 };
        let texels = ColorImage::new([20, 20], vec![Color32::BLACK; 400]);
        let (plan, content) = planned(&source, &texels, rect_quad([60.0, 60.0, 140.0, 140.0]), 1.0);
        let out = source.compose(&plan, &content).expect("composited");
        // Screen black over an opaque white backdrop contributes no overlay pixels: the canvas
        // already paints that backdrop underneath the styled texture.
        assert_eq!(at(&out, &plan, 100.0, 100.0).a(), 0);
    }

    #[test]
    fn a_scaled_backdrop_samples_document_coordinates() {
        let mut doc = doc();
        let mut backdrop = Layer::raster("backdrop", PixelFormat::RGBA8);
        backdrop.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 100, 200), &photocraft_raster::from_rgba(&PixelFormat::RGBA8, [1.0, 0.0, 0.0, 1.0]));
        backdrop.surface_mut().unwrap().fill_rect(Rect::new(100, 0, 200, 200), &photocraft_raster::from_rgba(&PixelFormat::RGBA8, [0.0, 0.0, 1.0, 1.0]));
        doc.layers.push(backdrop);
        let out = scaled_backdrop_with_cache(&doc, Rect::new(0, 0, 100, 100), 0.5, None, 0);
        assert!(is(
            Color32::from_rgba_premultiplied(
                (out.px[25 * 100 + 25][0] * 255.0) as u8,
                (out.px[25 * 100 + 25][1] * 255.0) as u8,
                (out.px[25 * 100 + 25][2] * 255.0) as u8,
                255
            ),
            [255, 0, 0]
        ));
        assert!(is(
            Color32::from_rgba_premultiplied(
                (out.px[25 * 100 + 75][0] * 255.0) as u8,
                (out.px[25 * 100 + 75][1] * 255.0) as u8,
                (out.px[25 * 100 + 75][2] * 255.0) as u8,
                255
            ),
            [0, 0, 255]
        ));
    }

    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 255, 0];
    const FILL: [u8; 3] = [0, 0, 255];

    #[test]
    fn a_stroke_keeps_its_width_when_the_box_is_enlarged() {
        // The 20×20 block enlarged four times: the stroke stays 4 px wide, not 16.
        let l = styled(vec![stroke(4.0)]);
        let (out, plan) = composite(&l, 20, 20, rect_quad([60.0, 60.0, 140.0, 140.0]), 1.0);
        assert!(is(at(&out, &plan, 100.0, 100.0), FILL));
        assert!(is(at(&out, &plan, 57.0, 100.0), RED) && is(at(&out, &plan, 142.0, 100.0), RED));
        assert_eq!(at(&out, &plan, 53.0, 100.0).a(), 0, "{:?}", at(&out, &plan, 53.0, 100.0));
        assert_eq!(at(&out, &plan, 146.0, 100.0).a(), 0);
    }

    #[test]
    fn a_shadow_keeps_its_direction_when_the_box_is_turned() {
        // Turned a half turn (the quad's corners swapped round): the shadow still falls to the
        // bottom-right of the block, not to the top-left with the pixels.
        let l = styled(vec![shadow(10.0)]);
        let upright = rect_quad([80.0, 80.0, 120.0, 120.0]);
        let turned = [upright[2], upright[3], upright[0], upright[1]];
        for quad in [upright, turned] {
            let (out, plan) = composite(&l, 40, 40, quad, 1.0);
            assert!(is(at(&out, &plan, 100.0, 100.0), FILL));
            assert!(is(at(&out, &plan, 124.0, 124.0), GREEN), "{:?}", at(&out, &plan, 124.0, 124.0));
            assert_eq!(at(&out, &plan, 76.0, 76.0).a(), 0, "no shadow on the lit side");
        }
    }

    /// The style is laid on in parallel bands: the result is the one a single pass gives.
    #[test]
    fn bands_join_without_a_seam() {
        let l = styled(vec![stroke(5.0), shadow(12.0)]);
        let quad = [Point::new(40.0, 20.0), Point::new(150.0, 40.0), Point::new(130.0, 180.0), Point::new(20.0, 150.0)];
        let texels = ColorImage::new([30, 40], (0..1200).map(|i| if i % 7 == 0 { Color32::TRANSPARENT } else { BLUE }).collect());
        let doc = doc();
        let source = LayerStyleSource { doc: &doc, preview: &doc, backdrop_cache: None, preview_revision: 0, layer: &l, revision: 0 };
        let (plan, content) = planned(&source, &texels, quad, 1.0);
        assert!(plan.shown.height() as usize > 2 * BAND_ROWS, "{:?}", plan.shown);
        let banded = source.compose(&plan, &content).expect("composited");
        // The same in one pass.
        let work = work_layer(&l, plan.scale);
        let content_rect = rect(content.rect);
        let content = Buffer { rect: content_rect, px: content.texels };
        let shape = content.px.iter().map(|p| p[3]).collect();
        let (bounds, shown) = (rect(plan.bounds), rect(plan.shown));
        let anchor = (f64::from(bounds.x0), f64::from(bounds.y0));
        let maps = effects::build_maps(&work, shape, content_rect, &doc.global_light, &TextureCtx { rect: content_rect, patterns: &doc.patterns, anchor });
        let mut whole = Buffer::transparent(shown);
        effects::composite_with_effects(&work, &crop(&content, shown), &mut whole, &maps, bounds, &doc.patterns);
        assert!(banded.pixels == image(&whole).pixels, "banded and whole composites differ");
        assert!(banded.pixels.iter().any(|p| is(*p, RED)) && banded.pixels.iter().any(|p| is(*p, GREEN)) && banded.pixels.iter().any(|p| is(*p, FILL)));
    }
}
