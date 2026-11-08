//! Shape stroke geometry controls shared by the tool defaults and Properties.
use egui::vec2;
use photocraft_doc::{LineCap, LineJoin, ShapeStroke, StrokeAlign};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, PoisonError};

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

impl StrokeOptions {
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

    fn label(&self) -> &'static str {
        if self.dashes.is_empty() {
            "Solid"
        } else if self.dashes == [4.0, 2.0] {
            "Dashed"
        } else if self.dashes == [0.0, 2.0] && self.cap == LineCap::Round {
            "Dotted"
        } else {
            "Custom"
        }
    }

    fn preset(&mut self, name: &str) {
        self.dash_offset = 0.0;
        self.dashes = match name {
            "Dashed" => vec![4.0, 2.0],
            "Dotted" => {
                self.cap = LineCap::Round;
                vec![0.0, 2.0]
            }
            _ => Vec::new(),
        };
    }

    pub(crate) fn valid(&self) -> bool {
        self.stroke(3.0, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK)).validate().is_ok()
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

mod document_preview;
mod editor;
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

/// Clip source coverage before choosing its bounded texture layout.
fn document_preview_bounds(compiled: &photocraft_vector::CompiledShape, visible: photocraft_geom::Rect) -> Option<photocraft_geom::Rect> {
    let bounds = compiled.bounds()?.intersect(&visible);
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
        painted_pass: painter.ctx().cumulative_pass_nr(),
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
    let id = egui::Id::unique("shape-document-drag-render");
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
    cache.painted_pass = painter.ctx().cumulative_pass_nr();
    painter.ctx().data_mut(|d| d.insert_temp(id, cache));
}

pub use editor::{StrokeEditor, button};

fn open_frame_id() -> egui::Id {
    egui::Id::unique("shape-stroke-open-frame")
}

pub(super) fn mark_rendered(ctx: &egui::Context, key: &str) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("shape-stroke-rendered", key)), pass));
}

/// A hidden options row or collapsed Appearance section no longer owns input.
/// Called after all options bars and Properties windows have had a chance to draw.
pub(crate) fn finish_frame(app: &mut crate::PhotocraftApp, ctx: &egui::Context) {
    // A released/cancelled drag must not retain a large, unfinished accumulator.
    let preview_id = egui::Id::unique("shape-document-drag-render");
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        if d.get_temp::<DocumentDragCache>(preview_id).is_some_and(|c| c.painted_pass != pass) {
            d.remove::<DocumentDragCache>(preview_id);
        }
    });
    let Some(e) = &app.ui.stroke_editor else { return };
    let shown = ctx.data(|d| d.get_temp::<u64>(egui::Id::unique(("shape-stroke-rendered", e.key.as_str()))));
    if shown != Some(ctx.cumulative_pass_nr()) {
        editor::close_children(ctx, &e.key);
        app.ui.stroke_editor = None;
    }
}

#[derive(Clone, Default)]
struct CanvasDismissal {
    held: bool,
    consumed_frame: Option<u64>,
}

/// Pressing the canvas dismisses this popup and consumes that entire gesture. In particular,
/// its release must not open a shape-size dialog or commit a shape dragged behind the popup.
pub(crate) fn dismiss_from_canvas(app: &mut crate::PhotocraftApp, response: &egui::Response) -> bool {
    let ctx = &response.ctx;
    let frame = ctx.cumulative_frame_nr();
    let was_open = app.ui.stroke_editor.is_some() || ctx.data(|d| d.get_temp::<u64>(open_frame_id())) == Some(frame);
    let (pressed, down) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.any_down()));
    let dismiss = was_open && pressed && response.contains_pointer();
    if dismiss && let Some(e) = app.ui.stroke_editor.take() {
        editor::close_children(ctx, &e.key);
        ctx.request_repaint();
    }
    ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_default::<CanvasDismissal>(egui::Id::unique("shape-stroke-canvas-dismissal"));
        state.held |= dismiss;
        let consumed = state.held || state.consumed_frame == Some(frame);
        if consumed {
            state.held = down;
            state.consumed_frame = Some(frame);
        }
        consumed
    })
}

/// Drop a popup whose source disappeared or changed outside this editor (including Undo).
pub(crate) fn sync_editor(app: &mut crate::PhotocraftApp, ctx: &egui::Context) {
    let Some(e) = &app.ui.stroke_editor else { return };
    let valid = if let Some((doc, layer)) = e.target {
        app.session.active().is_some_and(|st| {
            let locks = st.doc.effective_locks(layer);
            st.doc.id == doc
                && st.active_layer == Some(layer)
                && !locks.all
                && !locks.pixels
                && st.doc.layer(layer).is_some_and(|l| match &l.content {
                    photocraft_doc::LayerContent::Shape(shape) => shape.stroke.as_ref().map(StrokeOptions::from).unwrap_or_default() == e.original,
                    _ => false,
                })
        })
    } else {
        let tool = app.ui.tool;
        ((e.key == "tool-stroke" && crate::vector_ui::is_shape_tool(tool))
            || (e.key == "pen-stroke" && tool == crate::state::Tool::Pen && app.ui.tool_options.vector_mode == "shape"))
            && app.ui.tool_options.shape_stroke == e.original
    };
    if !valid {
        editor::close_children(ctx, &e.key);
        app.ui.stroke_editor = None;
    } else {
        // The options row can process a batched press+release before the canvas is drawn.
        // Keep the opening state for this frame so that click is still dismissal-only.
        let frame = ctx.cumulative_frame_nr();
        ctx.data_mut(|d| d.insert_temp(open_frame_id(), frame));
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod shortcut_tests;
