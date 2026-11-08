//! Experimental editable PDF import: a `craft-pdf` device turns painted paths into shape layers.
//!
//! The first step is deliberately narrow: solid fills and strokes become [`ShapeLayer`]s in
//! document pixels. Text, images, clipping paths, transparency groups, patterns, soft masks and
//! annotations mark the page unsupported, and the caller falls back to the raster importer for
//! that page with a warning. Nothing is ever half imported.
//!
//! This is the app-owned `Device` route: Hayro's interpreter walks the page and reports each
//! paint with its real colour and geometry, so shape layers need no page raster underneath. That
//! is what makes moving or deleting a layer behave, and what avoids the black-path and
//! doubled-text failures of the old PDFium import.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use craft_pdf::interpret::color::Color as HayroColor;
use craft_pdf::interpret::font::Glyph;
use craft_pdf::interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt,
    interpret_page,
};
use craft_pdf::syntax::page::Page;
use kurbo::{Affine as KurboAffine, BezPath, Cap, Join, PathEl};
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::vector::{FillRule, Knot, LineCap, LineJoin, Path, PathOp, ShapeLayer, ShapeStroke, StrokeAlign, Subpath};
use photocraft_doc::{Fill, Layer, LayerContent};
use photocraft_geom::{Affine, Point, Rect};
use photocraft_raster::Interrupt;

/// Most layers one editable page may create; past it the page falls back to raster.
const MAX_LAYERS: usize = 20_000;
/// Most path knots one editable page may hold; past it the page falls back to raster.
const MAX_KNOTS: usize = 1_000_000;
/// Pixel bytes of shape caches one editable page may retain before the rest render on demand.
const MAX_CACHE_BYTES: u64 = 256 << 20;
/// How often the watcher copies the interrupt into craft-pdf's cancellation flag.
const POLL: Duration = Duration::from_millis(5);

/// Why an editable page could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditableError {
    /// The caller cancelled the import.
    Cancelled,
    /// The page uses something this first step cannot represent; rasterise it instead.
    Unsupported(&'static str),
}

/// Builds one page's shape layers in document coordinates inside `rect`.
///
/// `rect` is the page's artboard in document pixels; the page's displayed size comes from
/// `Page::render_dimensions()` (CropBox, rotation and UserUnit already applied), so a point maps
/// to `rect` by the per-axis scale `rect.size / render_dimensions`. Nothing here re-applies the
/// CropBox offset or rotation.
pub fn page(page: &Page<'_>, rect: Rect, ctl: &Interrupt) -> Result<Vec<Layer>, EditableError> {
    ctl.check().map_err(|_| EditableError::Cancelled)?;
    let (width_pt, height_pt) = page.render_dimensions();
    if !width_pt.is_finite() || !height_pt.is_finite() || width_pt <= 0.0 || height_pt <= 0.0 {
        return Err(EditableError::Unsupported("the page has no area"));
    }
    let sx = f64::from(rect.width()) / f64::from(width_pt);
    let sy = f64::from(rect.height()) / f64::from(height_pt);
    let doc = Affine::translate(f64::from(rect.x0), f64::from(rect.y0)).mul(&Affine { m: [sx, 0.0, 0.0, sy, 0.0, 0.0] });
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let cancelled = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let settings = InterpreterSettings { cancelled: Some(Arc::clone(&cancelled)), ..InterpreterSettings::default() };
    let cache = InterpreterCache::new();
    let mut device = PathDevice { doc, clip: rect, fmt, ctl: *ctl, layers: Vec::new(), knots: 0, cached: 0, unsupported: None, cancelled: false };
    std::thread::scope(|scope| {
        let flag = Arc::clone(&cancelled);
        let done = Arc::clone(&finished);
        let ctl = *ctl;
        scope.spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if ctl.cancelled() {
                    flag.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(POLL);
            }
        });
        // A hostile page must not take the process down; the raster path already guards Hayro.
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let bbox = kurbo::Rect::new(0.0, 0.0, f64::from(width_pt), f64::from(height_pt));
            let mut context = Context::new(page.initial_transform(true).to_kurbo(), bbox, &cache, page.xref(), settings);
            interpret_page(page, &mut context, &mut device);
        }));
        if result.is_err() {
            device.unsupported = Some("the page could not be interpreted");
        }
        finished.store(true, Ordering::Relaxed);
    });
    if device.cancelled || ctl.cancelled() {
        return Err(EditableError::Cancelled);
    }
    if let Some(reason) = device.unsupported {
        return Err(EditableError::Unsupported(reason));
    }
    Ok(device.layers)
}

/// Collects filled and stroked paths; anything else marks the page unsupported.
struct PathDevice<'a> {
    /// Page points to document pixels: translation to the artboard plus the per-axis scale.
    doc: Affine,
    /// The page's artboard in document pixels; shape caches and strokes are clipped to it.
    clip: Rect,
    /// Document pixel format the shape caches are rendered in.
    fmt: PixelFormat,
    ctl: Interrupt<'a>,
    layers: Vec<Layer>,
    knots: usize,
    /// Bytes of shape caches retained so far.
    cached: u64,
    unsupported: Option<&'static str>,
    cancelled: bool,
}

impl PathDevice<'_> {
    /// Stops collecting at the first thing this step cannot represent.
    fn unsupported(&mut self, reason: &'static str) {
        if self.unsupported.is_none() {
            self.unsupported = Some(reason);
        }
    }

    /// Adds one shape layer unless a page cap is reached. Renders its pixel cache within the
    /// page's cache budget so the layer is visible before the engine refreshes it.
    fn push_shape(&mut self, path: Path, fill: Option<Fill>, stroke: Option<ShapeStroke>) {
        if self.ctl.check().is_err() {
            self.cancelled = true;
            return;
        }
        if path.is_empty() {
            return;
        }
        if self.layers.len() >= MAX_LAYERS {
            self.unsupported("too many objects");
            return;
        }
        let reach = stroke.as_ref().map_or(0.0, |stroke| f64::from(stroke.width));
        let b = path_bounds(&path);
        let area = Rect::new((b.0 - reach).floor() as i32, (b.1 - reach).floor() as i32, (b.2 + reach).ceil() as i32, (b.3 + reach).ceil() as i32)
            .intersect(&self.clip);
        let bytes = u64::from(area.width()).saturating_mul(u64::from(area.height())).saturating_mul(self.fmt.bytes_per_pixel() as u64);
        let mut shape = ShapeLayer { path, fill, stroke, ..ShapeLayer::default() };
        if area.width() > 0 && area.height() > 0 && self.cached.saturating_add(bytes) <= MAX_CACHE_BYTES {
            shape.cache = Some(photocraft_vector::render_shape(&shape, self.fmt, self.clip));
            self.cached = self.cached.saturating_add(bytes);
        }
        self.layers.push(Layer::new(format!("Path {}", self.layers.len() + 1), LayerContent::Shape(shape)));
    }
}

impl<'a> Device<'a> for PathDevice<'_> {
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        if mask.is_some() {
            self.unsupported("soft masks");
        }
    }

    fn set_blend_mode(&mut self, blend_mode: BlendMode) {
        if blend_mode != BlendMode::Normal {
            self.unsupported("blend modes");
        }
    }

    fn draw_path(&mut self, path: &BezPath, transform: KurboAffine, paint: &Paint<'a>, draw_mode: &PathDrawMode) {
        if self.unsupported.is_some() {
            return;
        }
        if self.ctl.check().is_err() {
            self.cancelled = true;
            return;
        }
        let color = match paint {
            Paint::Color(color) => to_color(color),
            Paint::Pattern(_) => {
                self.unsupported("patterns");
                return;
            }
        };
        let full = self.doc.mul(&knots_affine(&transform));
        let Some(path) = to_path(path, &full, &mut self.knots) else {
            self.unsupported("too many path points");
            return;
        };
        match draw_mode {
            PathDrawMode::Fill(rule) => {
                let mut path = path;
                path.fill_rule = match rule {
                    craft_pdf::interpret::FillRule::NonZero => FillRule::NonZero,
                    craft_pdf::interpret::FillRule::EvenOdd => FillRule::EvenOdd,
                };
                self.push_shape(path, Some(Fill::Solid(color)), None);
            }
            PathDrawMode::Stroke(stroke) => {
                let Some(stroke) = to_stroke(stroke, color, full.determinant().abs().sqrt()) else {
                    self.unsupported("stroke properties");
                    return;
                };
                self.push_shape(path, None, Some(stroke));
            }
        }
    }

    fn push_clip_path(&mut self, _clip_path: &ClipPath) {
        self.unsupported("clipping paths");
    }

    fn push_transparency_group(&mut self, _opacity: f32, mask: Option<SoftMask<'a>>, blend_mode: BlendMode) {
        // A group needs its own blend/opacity/mask handling; until that exists the page is
        // rasterised rather than flattened into the wrong stacking order.
        let _ = (mask, blend_mode);
        self.unsupported("transparency groups");
    }

    fn draw_glyph(&mut self, _glyph: &Glyph<'a>, _transform: KurboAffine, _glyph_transform: KurboAffine, _paint: &Paint<'a>, _draw_mode: &GlyphDrawMode) {
        self.unsupported("text is not editable yet");
    }

    fn draw_image(&mut self, _image: Image<'a, '_>, _transform: KurboAffine) {
        self.unsupported("images");
    }

    fn pop_clip_path(&mut self) {}

    fn pop_transparency_group(&mut self) {}

    fn begin_annotations(&mut self) {
        self.unsupported("annotations");
    }
}

/// A PDF colour as a straight-alpha PhotoCraft colour.
fn to_color(color: &HayroColor) -> Color {
    let rgba = color.to_rgba();
    let [r, g, b, a] = rgba.components();
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return Color::rgba(0.0, 0.0, 0.0, 0.0);
    }
    // `AlphaColor` components are premultiplied; PhotoCraft colours are straight.
    Color::rgba((r / a).clamp(0.0, 1.0), (g / a).clamp(0.0, 1.0), (b / a).clamp(0.0, 1.0), a)
}

/// A kurbo affine as PhotoCraft's affine (same `[a, b, c, d, e, f]` layout).
fn knots_affine(transform: &KurboAffine) -> Affine {
    let [a, b, c, d, e, f] = transform.as_coeffs();
    Affine { m: [a, b, c, d, e, f] }
}

/// The anchor bounds of a path, used only to estimate a shape cache's size.
fn path_bounds(path: &Path) -> (f64, f64, f64, f64) {
    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for knot in path.subpaths.iter().flat_map(|subpath| &subpath.knots) {
        b.0 = b.0.min(knot.anchor.x);
        b.1 = b.1.min(knot.anchor.y);
        b.2 = b.2.max(knot.anchor.x);
        b.3 = b.3.max(knot.anchor.y);
    }
    b
}

/// A Hayro stroke as PhotoCraft's stroke: width in pixels, dashes in multiples of the width.
fn to_stroke(stroke: &craft_pdf::interpret::StrokeProps, color: Color, scale: f64) -> Option<ShapeStroke> {
    if !stroke.line_width.is_finite() || stroke.line_width <= 0.0 || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let width = stroke.line_width * scale as f32;
    if !width.is_finite() || width <= 0.0 {
        return None;
    }
    let dashes = stroke.dash_array.iter().map(|dash| dash / stroke.line_width).collect();
    Some(ShapeStroke {
        width,
        paint: Fill::Solid(color),
        opacity: 1.0,
        align: StrokeAlign::Center,
        cap: match stroke.line_cap {
            Cap::Butt => LineCap::Butt,
            Cap::Round => LineCap::Round,
            Cap::Square => LineCap::Square,
        },
        join: match stroke.line_join {
            Join::Miter => LineJoin::Miter,
            Join::Round => LineJoin::Round,
            Join::Bevel => LineJoin::Bevel,
        },
        miter_limit: stroke.miter_limit,
        dashes,
        dash_offset: stroke.dash_offset / stroke.line_width,
    })
}

/// Converts a kurbo path to PhotoCraft's cubic knot model. `None` once `knots` passes the cap.
fn to_path(bez: &BezPath, transform: &Affine, knots: &mut usize) -> Option<Path> {
    let mut subpaths = Vec::new();
    let mut current: Option<Subpath> = None;
    let mut last = Point::new(0.0, 0.0);
    for element in bez.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                if let Some(subpath) = current.take() {
                    subpaths.push(subpath);
                }
                current = Some(Subpath { closed: false, knots: vec![Knot::corner(p.x, p.y)], op: PathOp::Combine });
                last = Point::new(p.x, p.y);
            }
            PathEl::LineTo(p) => {
                if let Some(subpath) = current.as_mut() {
                    if let Some(knot) = subpath.knots.last_mut() {
                        knot.out_ctrl = knot.anchor;
                    }
                    subpath.knots.push(Knot::corner(p.x, p.y));
                }
                last = Point::new(p.x, p.y);
            }
            PathEl::QuadTo(control, p) => {
                if let Some(subpath) = current.as_mut() {
                    let start = subpath.knots.last().map_or(last, |knot| knot.anchor);
                    if let Some(knot) = subpath.knots.last_mut() {
                        knot.out_ctrl = Point::new(start.x + 2.0 / 3.0 * (control.x - start.x), start.y + 2.0 / 3.0 * (control.y - start.y));
                    }
                    subpath.knots.push(Knot {
                        anchor: Point::new(p.x, p.y),
                        in_ctrl: Point::new(p.x + 2.0 / 3.0 * (control.x - p.x), p.y + 2.0 / 3.0 * (control.y - p.y)),
                        out_ctrl: Point::new(p.x, p.y),
                        smooth: false,
                    });
                }
                last = Point::new(p.x, p.y);
            }
            PathEl::CurveTo(c1, c2, p) => {
                if let Some(subpath) = current.as_mut() {
                    if let Some(knot) = subpath.knots.last_mut() {
                        knot.out_ctrl = Point::new(c1.x, c1.y);
                    }
                    subpath.knots.push(Knot { anchor: Point::new(p.x, p.y), in_ctrl: Point::new(c2.x, c2.y), out_ctrl: Point::new(p.x, p.y), smooth: false });
                }
                last = Point::new(p.x, p.y);
            }
            PathEl::ClosePath => {
                if let Some(mut subpath) = current.take() {
                    subpath.closed = true;
                    if subpath.knots.len() > 1 {
                        let first = subpath.knots[0].anchor;
                        let last_anchor = subpath.knots.last().map_or(first, |knot| knot.anchor);
                        if (first.x - last_anchor.x).abs() < 1e-9 && (first.y - last_anchor.y).abs() < 1e-9 {
                            if let Some(knot) = subpath.knots.pop() {
                                subpath.knots[0].in_ctrl = knot.in_ctrl;
                            }
                        } else {
                            subpath.knots[0].in_ctrl = first;
                        }
                    }
                    subpaths.push(subpath);
                }
            }
        }
    }
    if let Some(subpath) = current {
        subpaths.push(subpath);
    }
    *knots = knots.saturating_add(subpaths.iter().map(|subpath| subpath.knots.len()).sum::<usize>());
    if *knots > MAX_KNOTS {
        return None;
    }
    for subpath in &mut subpaths {
        for knot in &mut subpath.knots {
            *knot = knot.transform(transform);
        }
    }
    Some(Path::new(subpaths))
}

#[cfg(test)]
mod tests {
    use lopdf::{Document as PdfDocument, Object, Stream, dictionary};

    use super::*;
    use crate::{ImportOptions, IoError, import_with_options_and_interrupt};

    /// A one-page 200 x 100 pt PDF whose content stream is `content`.
    fn pdf(content: &str) -> Vec<u8> {
        let mut pdf = PdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let page_id = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => Object::Array(vec![Object::Real(0.0), Object::Real(0.0), Object::Real(200.0), Object::Real(100.0)]),
            "Contents" => content_id,
            "Resources" => dictionary! {},
        });
        pdf.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => Object::Array(vec![Object::Reference(page_id)]),
                "Count" => 1,
            }),
        );
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        pdf.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes).expect("writing the fixture");
        bytes
    }

    /// The artboard group's children.
    fn children(document: &photocraft_doc::Document) -> &[Layer] {
        match &document.layers.first().expect("a page group").content {
            LayerContent::Group(group) => &group.children,
            _ => panic!("the page is not a group"),
        }
    }

    /// A solid fill or stroke colour.
    fn solid(fill: &Fill) -> Color {
        match fill {
            Fill::Solid(color) => *color,
            _ => panic!("not a solid paint"),
        }
    }

    /// Path bounds in document pixels.
    fn bounds(path: &Path) -> (f64, f64, f64, f64) {
        let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for knot in path.subpaths.iter().flat_map(|subpath| &subpath.knots) {
            b.0 = b.0.min(knot.anchor.x);
            b.1 = b.1.min(knot.anchor.y);
            b.2 = b.2.max(knot.anchor.x);
            b.3 = b.3.max(knot.anchor.y);
        }
        b
    }

    #[test]
    fn painted_paths_become_shape_layers_with_their_colours_and_dashes() {
        let bytes = pdf("1 0 0 rg 20 20 60 40 re f 0 0 1 RG 3 w [9 4] 0 d 100 30 m 180 30 l S");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("paths.pdf", &bytes, &options, &Interrupt::NONE).expect("an editable import");
        let children = children(&imported.document);
        assert_eq!(children.len(), 2, "one fill and one stroke layer");
        let LayerContent::Shape(fill) = &children[0].content else { panic!("the first child is not a shape") };
        assert_eq!(solid(fill.fill.as_ref().expect("a fill")), Color::rgb(1.0, 0.0, 0.0));
        assert!(fill.stroke.is_none());
        let (x0, y0, x1, y1) = bounds(&fill.path);
        assert!((x0 - 20.0).abs() < 0.01 && (x1 - 80.0).abs() < 0.01, "x bounds {x0}..{x1}");
        assert!((y0 - 40.0).abs() < 0.01 && (y1 - 80.0).abs() < 0.01, "PDF y up becomes document y down: {y0}..{y1}");

        let LayerContent::Shape(stroke) = &children[1].content else { panic!("the second child is not a shape") };
        assert!(stroke.fill.is_none());
        let stroke = stroke.stroke.as_ref().expect("a stroke");
        assert_eq!(solid(&stroke.paint), Color::rgb(0.0, 0.0, 1.0));
        assert!((stroke.width - 3.0).abs() < 0.01, "stroke width {}", stroke.width);
        assert_eq!(stroke.dashes.len(), 2);
        assert!((stroke.dashes[0] - 3.0).abs() < 0.01 && (stroke.dashes[1] - 4.0 / 3.0).abs() < 0.01, "dashes {:?}", stroke.dashes);
    }

    #[test]
    fn the_editable_composite_matches_the_raster_page() {
        let bytes = pdf("1 0 0 rg 20 20 60 40 re f 0 0 1 RG 3 w [9 4] 0 d 100 30 m 180 30 l S");
        let raster = import_with_options_and_interrupt("paths.pdf", &bytes, &ImportOptions { pdf_dpi: 72.0, ..ImportOptions::default() }, &Interrupt::NONE)
            .expect("a raster import");
        let editable = import_with_options_and_interrupt(
            "paths.pdf",
            &bytes,
            &ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() },
            &Interrupt::NONE,
        )
        .expect("an editable import");
        let fmt = editable.document.pixel_format();
        let a = photocraft_compose::flatten_to_surface(&editable.document, fmt, None);
        let b = photocraft_compose::flatten_to_surface(&raster.document, fmt, None);
        let rect = editable.document.bounds();
        let av = a.read_region(rect);
        let bv = b.read_region(rect);
        assert_eq!(av.len(), bv.len());
        let mut sum = 0.0f64;
        let mut max = 0.0f32;
        for (x, y) in av.iter().zip(&bv) {
            let d = (x - y).abs();
            sum += f64::from(d);
            max = max.max(d);
        }
        let mean = sum / av.len() as f64;
        assert!(mean < 0.01, "mean pixel difference {mean:.4} (max {max:.3})");
    }

    #[test]
    fn text_falls_back_to_the_raster_page_with_a_warning() {
        let bytes = pdf("BT /F1 24 Tf 10 50 Td (Hi) Tj ET");
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        let imported = import_with_options_and_interrupt("text.pdf", &bytes, &options, &Interrupt::NONE).expect("a fallback import");
        assert!(
            imported.warnings.iter().any(|warning| warning.contains("text is not editable yet") && warning.contains("rasterised")),
            "{:?}",
            imported.warnings
        );
        assert!(matches!(children(&imported.document).first().map(|layer| &layer.content), Some(LayerContent::Raster(_))));
    }

    #[test]
    fn a_cancelled_editable_import_returns_cancelled() {
        let bytes = pdf("1 0 0 rg 20 20 60 40 re f");
        let cancel = || true;
        let ctl = Interrupt::cancel_only(&cancel);
        let options = ImportOptions { pdf_dpi: 72.0, pdf_editable: true, ..ImportOptions::default() };
        assert!(matches!(import_with_options_and_interrupt("paths.pdf", &bytes, &options, &ctl), Err(IoError::Cancelled)));
    }
}
