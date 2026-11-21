//! Shape stroke geometry shared by the tool defaults and Properties.
use craft::draw::stroke::{DashAlign, DashUnit, LineCap as FwCap, LineJoin as FwJoin, StrokeAlign as FwAlign, StrokeStyle};
use egui::vec2;
use photocraft_doc::{LineCap, LineJoin, ShapeStroke, StrokeAlign};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Colour and width stay in their existing controls. These options never replace a shape's paint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StrokeOptions {
    pub align: StrokeAlign,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub dashes: Vec<f32>,
    pub dash_offset: f32,
    pub opacity: f32,
}

impl Default for StrokeOptions {
    fn default() -> Self {
        Self::from(&ShapeStroke::default())
    }
}

impl From<&ShapeStroke> for StrokeOptions {
    fn from(s: &ShapeStroke) -> Self {
        Self { align: s.align, cap: s.cap, join: s.join, miter_limit: s.miter_limit, dashes: s.dashes.clone(), dash_offset: s.dash_offset, opacity: s.opacity }
    }
}

/// The framework's stroke style for the options: dash lengths count in stroke widths and sit exactly
/// along the path, as the document stores them. The width is the caller's.
impl From<&StrokeOptions> for StrokeStyle {
    fn from(o: &StrokeOptions) -> Self {
        StrokeStyle {
            align: match o.align {
                StrokeAlign::Inside => FwAlign::Inside,
                StrokeAlign::Center => FwAlign::Center,
                StrokeAlign::Outside => FwAlign::Outside,
            },
            cap: match o.cap {
                LineCap::Butt => FwCap::Butt,
                LineCap::Round => FwCap::Round,
                LineCap::Square => FwCap::Square,
            },
            join: match o.join {
                LineJoin::Miter => FwJoin::Miter,
                LineJoin::Round => FwJoin::Round,
                LineJoin::Bevel => FwJoin::Bevel,
            },
            miter_limit: o.miter_limit,
            dashes: o.dashes.clone(),
            dash_offset: o.dash_offset,
            dash_unit: DashUnit::Widths,
            dash_align: DashAlign::Exact,
            opacity: o.opacity,
            ..StrokeStyle::default()
        }
    }
}

impl StrokeOptions {
    /// The options of a framework stroke style; its width, dash unit and dash alignment have no
    /// place here.
    pub fn from_style(style: &StrokeStyle) -> Self {
        Self {
            align: match style.align {
                FwAlign::Inside => StrokeAlign::Inside,
                FwAlign::Outside => StrokeAlign::Outside,
                _ => StrokeAlign::Center,
            },
            cap: match style.cap {
                FwCap::Round => LineCap::Round,
                FwCap::Square => LineCap::Square,
                _ => LineCap::Butt,
            },
            join: match style.join {
                FwJoin::Round => LineJoin::Round,
                FwJoin::Bevel => LineJoin::Bevel,
                _ => LineJoin::Miter,
            },
            miter_limit: style.miter_limit,
            dashes: style.dashes.clone(),
            dash_offset: style.dash_offset,
            opacity: style.opacity,
        }
    }

    pub fn params(&self) -> Value {
        json!({"align": format!("{:?}", self.align).to_ascii_lowercase(),
            "cap": format!("{:?}", self.cap).to_ascii_lowercase(), "join": format!("{:?}", self.join).to_ascii_lowercase(),
            "miterLimit": self.miter_limit, "dashes": self.dashes, "dashOffset": self.dash_offset, "opacity": self.opacity * 100.0})
    }

    pub fn stroke(&self, width: f32, paint: photocraft_doc::Fill) -> ShapeStroke {
        ShapeStroke {
            width,
            paint,
            align: self.align,
            cap: self.cap,
            join: self.join,
            miter_limit: self.miter_limit,
            dashes: self.dashes.clone(),
            dash_offset: self.dash_offset,
            opacity: self.opacity,
        }
    }
}

#[derive(Clone)]
struct DragCache {
    shape: photocraft_doc::ShapeLayer,
    clip: egui::Rect,
    image_rect: egui::Rect,
    texture: egui::TextureHandle,
}

/// The drag uses the committed vector rasterizer. Cache by geometry/style/visible region, and
/// cap proxy resolution so oversized windows cannot allocate a document-sized preview.
pub fn paint_shape_preview(painter: &egui::Painter, shape: &photocraft_doc::ShapeLayer) {
    let clip = painter.clip_rect();
    if !clip.is_finite() || clip.is_negative() {
        return;
    }
    let id = egui::Id::unique("shape-drag-render");
    let cached = painter.ctx().data(|d| d.get_temp::<DragCache>(id));
    let cache = cached.filter(|c| c.shape == *shape && c.clip == clip).or_else(|| {
        let scale = (2048.0 / clip.width().max(clip.height()).max(1.0)).min(1.0);
        let mut proxy = shape.clone();
        let tr = photocraft_geom::Affine { m: [scale as f64, 0.0, 0.0, scale as f64, -(clip.left() * scale) as f64, -(clip.top() * scale) as f64] };
        proxy.path = proxy.path.transform(&tr);
        if let Some(stroke) = proxy.stroke.as_mut() {
            stroke.width *= scale;
        }
        let canvas = photocraft_geom::Rect::from_size(photocraft_geom::Size::new((clip.width() * scale).ceil() as u32, (clip.height() * scale).ceil() as u32));
        let compiled = photocraft_vector::CompiledShape::new(&proxy, photocraft_vector::DEFAULT_TOLERANCE, canvas);
        let bounds = compiled.bounds()?.intersect(&canvas);
        if bounds.is_empty() {
            return None;
        }
        let rgba = compiled.render_rgba(bounds);
        let bytes: Vec<u8> = rgba.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        let texture = painter.ctx().load_texture(
            "shape-drag",
            egui::ColorImage::from_rgba_unmultiplied([bounds.width() as usize, bounds.height() as usize], &bytes),
            egui::TextureOptions::LINEAR,
        );
        let screen = |x: i32, y: i32| clip.min + vec2(x as f32, y as f32) / scale;
        Some(DragCache {
            shape: shape.clone(),
            clip,
            image_rect: egui::Rect::from_min_max(screen(bounds.x0, bounds.y0), screen(bounds.x1, bounds.y1)),
            texture,
        })
    });
    if let Some(cache) = cache {
        painter.image(cache.texture.id(), cache.image_rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        painter.ctx().data_mut(|d| d.insert_temp(id, cache));
    }
}

use std::sync::{Arc, Mutex, PoisonError};
mod document_preview;
use document_preview::{PreviewLayout, PreviewReduction};

/// Cache document pixels independently of zoom: the view only transforms the textured quad.
#[derive(Clone)]
struct DocumentDragCache {
    shape: photocraft_doc::ShapeLayer,
    visible: photocraft_geom::Rect,
    canvas: photocraft_geom::Rect,
    bounds: photocraft_geom::Rect,
    texture: egui::TextureHandle,
    options: egui::TextureOptions,
    size: [usize; 2],
    factor: u32,
    reduction: Option<Arc<Mutex<PreviewReduction>>>,
    painted_pass: u64,
}

fn document_preview_id(viewport: egui::ViewportId, layer: egui::LayerId) -> egui::Id {
    egui::Id::unique("shape-document-drag-render").with((viewport, layer))
}

fn document_preview_registry() -> egui::Id {
    egui::Id::unique("shape-document-drag-views")
}

/// Clip source coverage before choosing its bounded texture layout.
fn document_preview_bounds(compiled: &photocraft_vector::CompiledShape, visible: photocraft_geom::Rect) -> Option<photocraft_geom::Rect> {
    let source = compiled.bounds()?;
    let clipped = source.intersect(&visible);
    if clipped.is_empty() {
        return None;
    }
    // Linear filtering needs transparent neighbors outside the tight shape bounds.
    // Reserve one reduced texel too, without exceeding the existing texture limit.
    let padding = clipped.width().max(clipped.height()).div_ceil(document_preview::TEXTURE_EDGE - 2).max(1) as i32;
    let bounds = photocraft_geom::Rect::new(
        source.x0.saturating_sub(padding),
        source.y0.saturating_sub(padding),
        source.x1.saturating_add(padding),
        source.y1.saturating_add(padding),
    )
    .intersect(&visible);
    (!bounds.is_empty()).then_some(bounds)
}

/// Preserve native coverage when reducing a large preview, with one bounded pass per frame.
fn new_document_preview(
    painter: &egui::Painter,
    shape: &photocraft_doc::ShapeLayer,
    visible: photocraft_geom::Rect,
    canvas: photocraft_geom::Rect,
    options: egui::TextureOptions,
) -> Option<DocumentDragCache> {
    let compiled = photocraft_vector::CompiledShape::with_frames(shape, photocraft_vector::DEFAULT_TOLERANCE, visible, canvas);
    let bounds = document_preview_bounds(&compiled, visible)?;
    let layout = PreviewLayout::new(bounds)?;
    let (image, reduction, options) = if layout.factor == 1 {
        let rgba = compiled.render_rgba(bounds);
        let bytes: Vec<u8> = rgba.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        (egui::ColorImage::from_rgba_unmultiplied(layout.size, &bytes), None, options)
    } else {
        let mut work = PreviewReduction::new(compiled, layout);
        work.advance();
        if !work.complete() {
            // Keep the whole changing shape visible while exact document coverage is streamed.
            // Cap this extra raster at 512 pixels per axis, independently of document dimensions.
            let preview_factor = bounds.width().max(bounds.height()).div_ceil(512).max(layout.factor);
            let size = [bounds.width().div_ceil(preview_factor) as usize, bounds.height().div_ceil(preview_factor) as usize];
            let scale = 1.0 / f64::from(preview_factor);
            let mut proxy = shape.clone();
            proxy.path =
                proxy.path.transform(&photocraft_geom::Affine { m: [scale, 0.0, 0.0, scale, -f64::from(bounds.x0) * scale, -f64::from(bounds.y0) * scale] });
            proxy.live = None;
            proxy.use_aligned_rendering = false;
            proxy.feather *= scale as f32;
            if let Some(stroke) = &mut proxy.stroke {
                stroke.width *= scale as f32;
            }
            let output = photocraft_geom::Rect::new(0, 0, size[0] as i32, size[1] as i32);
            let frame = photocraft_geom::Rect::new(
                ((f64::from(canvas.x0) - f64::from(bounds.x0)) * scale).floor() as i32,
                ((f64::from(canvas.y0) - f64::from(bounds.y0)) * scale).floor() as i32,
                ((f64::from(canvas.x1) - f64::from(bounds.x0)) * scale).ceil() as i32,
                ((f64::from(canvas.y1) - f64::from(bounds.y0)) * scale).ceil() as i32,
            );
            let fallback = photocraft_vector::CompiledShape::with_frames(&proxy, photocraft_vector::DEFAULT_TOLERANCE, output, frame).render_rgba(output);
            let bytes: Vec<u8> = fallback
                .iter()
                .flat_map(|p| {
                    let mut rgba = p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                    // A 1px document stroke can be below half an alpha byte in this
                    // coarse proxy. Keep positive coverage until native pixels replace it.
                    if p[3] > 0.0 && rgba[3] == 0 {
                        rgba[3] = 1;
                    }
                    rgba
                })
                .collect();
            work.set_fallback(egui::ColorImage::from_rgba_unmultiplied(size, &bytes));
        }
        let image = work.image();
        let reduction = (!work.complete()).then(|| Arc::new(Mutex::new(work)));
        (image, reduction, egui::TextureOptions::LINEAR)
    };
    let texture = painter.ctx().load_texture("shape-document-drag", image, options);
    Some(DocumentDragCache {
        shape: shape.clone(),
        visible,
        canvas,
        bounds,
        texture,
        options,
        size: layout.size,
        factor: layout.factor,
        reduction,
        painted_pass: painter.ctx().cumulative_pass_nr_for(egui::ViewportId::ROOT),
    })
}

/// Rasterize aligned strokes in document pixels, then project that coverage into the view.
/// Curves, diagonals and ineligible widths therefore retain committed antialiasing at any zoom.
pub(crate) fn paint_document_shape_preview(
    painter: &egui::Painter,
    shape: &photocraft_doc::ShapeLayer,
    transform: &photocraft_geom::Affine,
    canvas: photocraft_geom::Rect,
) {
    use photocraft_geom::{Point, Rect};
    let clip = painter.clip_rect();
    let Some(inverse) = transform.inverse() else { return };
    if !clip.is_finite() || clip.is_negative() {
        return;
    }
    let points = [clip.left_top(), clip.right_top(), clip.right_bottom(), clip.left_bottom()].map(|p| inverse.apply(Point::new(p.x as f64, p.y as f64)));
    let visible = Rect::new(
        (points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min).floor() as i32).saturating_sub(1),
        (points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min).floor() as i32).saturating_sub(1),
        (points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max).ceil() as i32).saturating_add(1),
        (points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max).ceil() as i32).saturating_add(1),
    );
    let id = document_preview_id(painter.ctx().viewport_id(), painter.layer_id());
    painter.ctx().data_mut(|d| {
        let views = d.get_temp_mut_or_default::<Vec<egui::Id>>(document_preview_registry());
        if !views.contains(&id) {
            views.push(id);
        }
    });
    // Exact pixel-grid zooms stay crisp. Other transforms filter document coverage just as
    // displaying a rotated or fractionally positioned rasterized document requires.
    let integer = |v: f64| (v - v.round()).abs() < 1e-6;
    let options = if transform.m[1].abs() < 1e-6
        && transform.m[2].abs() < 1e-6
        && transform.m[0].abs() >= 1.0
        && transform.m[3].abs() >= 1.0
        && [transform.m[0], transform.m[3], transform.m[4], transform.m[5]].into_iter().all(integer)
    {
        egui::TextureOptions::NEAREST
    } else {
        egui::TextureOptions::LINEAR
    };
    let cached = painter.ctx().data(|d| d.get_temp::<DocumentDragCache>(id));
    let cached = cached.filter(|c| c.shape == *shape && c.visible == visible && c.canvas == canvas && (c.factor > 1 || c.options == options));
    let was_cached = cached.is_some();
    if !was_cached {
        // Release an obsolete progressive accumulator before allocating its replacement.
        painter.ctx().data_mut(|d| d.remove::<DocumentDragCache>(id));
    }
    let Some(mut cache) = cached.or_else(|| new_document_preview(painter, shape, visible, canvas, options)) else { return };
    if was_cached && let Some(reduction) = &cache.reduction {
        let (image, complete) = {
            let mut work = reduction.lock().unwrap_or_else(PoisonError::into_inner);
            work.advance();
            (work.image(), work.complete())
        };
        cache.texture.set(image, cache.options);
        if complete {
            cache.reduction = None;
        }
    }
    if cache.reduction.is_some() {
        painter.ctx().request_repaint();
    }
    let b = cache.bounds;
    let mut mesh = egui::Mesh::with_texture(cache.texture.id());
    for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        let p = transform.apply(Point::new(
            f64::from(b.x0) + u * cache.size[0] as f64 * f64::from(cache.factor),
            f64::from(b.y0) + v * cache.size[1] as f64 * f64::from(cache.factor),
        ));
        mesh.vertices.push(egui::epaint::Vertex { pos: egui::pos2(p.x as f32, p.y as f32), uv: egui::pos2(u as f32, v as f32), color: egui::Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
    cache.painted_pass = painter.ctx().cumulative_pass_nr_for(egui::ViewportId::ROOT);
    painter.ctx().data_mut(|d| d.insert_temp(id, cache));
}

pub(crate) fn finish_frame(_app: &mut crate::PhotocraftApp, ctx: &egui::Context) {
    // A released/cancelled drag must not retain a large, unfinished accumulator.
    // Floating document viewports have independent pass counters.
    let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|d| {
        let mut views = d.get_temp::<Vec<egui::Id>>(document_preview_registry()).unwrap_or_default();
        views.retain(|id| {
            let current = d.get_temp::<DocumentDragCache>(*id).is_some_and(|c| c.painted_pass == pass);
            if !current {
                d.remove::<DocumentDragCache>(*id);
            }
            current
        });
        d.insert_temp(document_preview_registry(), views);
    });
}

#[cfg(test)]
mod tests;
