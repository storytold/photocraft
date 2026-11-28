//! Shape stroke geometry shared by the tool defaults and Properties.
use craft::draw::stroke::{DashAlign, DashUnit, LineCap as FwCap, LineJoin as FwJoin, StrokeAlign as FwAlign, StrokeStyle};
use egui::vec2;
use photocraft_doc::{LineCap, LineJoin, ShapeStroke, StrokeAlign};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Colour and width stay in their existing controls. These options never replace a shape's paint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft::ui::contract::PlainData)]
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

#[cfg(test)]
mod tests;
