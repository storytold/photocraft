//! The shape split's limits are process-wide: this binary holds the one test that changes them,
//! so no other test composites under them.

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_compose::{render, shape_split};
use photocraft_doc::vector::{Path, ShapeLayer, ShapeStroke, StrokeAlign, Subpath, VectorMask};
use photocraft_doc::{Document, Fill, Layer, LayerContent};
use photocraft_geom::{Rect, Size};

fn white(canvas: Rect) -> Layer {
    let mut l = Layer::raster("Background", PixelFormat::RGBA8);
    if let Some(s) = l.surface_mut() {
        s.fill_rect(canvas, &[1.0; 4]);
    }
    l
}

/// A green rectangle with a blue stroke, its own pixels as Photoshop stores them, under a red
/// clipped layer: the split draws the stroke above the red.
fn stroked_shape_under_a_clipped_layer(canvas: Rect) -> Document {
    let mut sh = ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(40.0, 30.0), (400.0, 30.0), (400.0, 200.0), (40.0, 200.0)])]),
        fill: Some(Fill::Solid(Color::rgb(0.0, 1.0, 0.0))),
        stroke: Some(ShapeStroke { width: 8.0, paint: Fill::Solid(Color::rgb(0.0, 0.0, 1.0)), ..ShapeStroke::default() }),
        ..ShapeLayer::default()
    };
    sh.cache = Some(photocraft_vector::render_shape(&sh, PixelFormat::RGBA8, canvas));
    let mut clipped = Layer::raster("Red", PixelFormat::RGBA8);
    if let Some(s) = clipped.surface_mut() {
        s.fill_rect(canvas, &[1.0, 0.0, 0.0, 1.0]);
    }
    clipped.clipped = true;
    let mut doc = Document::new("split", Size::new(canvas.width(), canvas.height()), ColorMode::Rgb, SampleType::U8);
    doc.layers.extend([white(canvas), Layer::new("Shape", LayerContent::Shape(sh)), clipped]);
    doc
}

/// An empty Photoshop Frame with a blue outside stroke: it stores no stroke pixels of its own.
fn stroked_frame(canvas: Rect) -> Document {
    let mut frame = Layer::group("Frame 1", vec![]);
    frame.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(60.0, 40.0), (300.0, 40.0), (300.0, 180.0), (60.0, 180.0)])])));
    if let LayerContent::Group(g) = &mut frame.content {
        g.frame_stroke = Some(ShapeStroke { width: 6.0, align: StrokeAlign::Outside, paint: Fill::Solid(Color::rgb(0.0, 0.0, 1.0)), ..Default::default() });
    }
    let mut doc = Document::new("frame", Size::new(canvas.width(), canvas.height()), ColorMode::Rgb, SampleType::U8);
    doc.layers.extend([white(canvas), frame]);
    doc
}

/// A split past the cache's budget but within the draw limit is drawn as before, for each
/// request (stroke above the clipped layer, a Frame's border), and not kept. Past the draw
/// limit the shape isn't split: its own pixels composite as they are, the stroke under the
/// clipped layer, and a Frame, which stores no stroke, has no border.
#[test]
fn splits_past_the_cache_budget_draw_as_before_and_past_the_draw_limit_fall_back() {
    let canvas = Rect::new(0, 0, 512, 256);
    let doc = stroked_shape_under_a_clipped_layer(canvas);
    let mut unsplit = stroked_shape_under_a_clipped_layer(canvas);
    if let LayerContent::Shape(sh) = &mut unsplit.layers[1].content {
        sh.stroke = None;
    }
    let frame = stroked_frame(canvas);

    shape_split::purge_cache();
    let stacked = render(&doc, canvas);
    let bordered = render(&frame, canvas);
    let flat = render(&unsplit, canvas);
    assert!(stacked != flat, "the split draws the stroke above the clipped layer");
    assert!(bordered != render(&white_only(canvas), canvas), "the Frame has a border");

    // Past the cache's budget (the parts and their source are over 64 KiB), within the limit.
    let defaults = shape_split::set_split_limits(64 << 10, shape_split::MAX_SPLIT_DRAW_BYTES);
    shape_split::purge_cache();
    assert!(render(&doc, canvas) == stacked, "the stroke still stacks above the clipped layer");
    assert!(render(&frame, canvas) == bordered, "the Frame keeps its border");
    assert_eq!(shape_split::purge_cache(), 0, "drawn for each request, not kept");

    // Past the draw limit: no split.
    shape_split::set_split_limits(64 << 10, 64 << 10);
    assert!(render(&doc, canvas) == flat, "the shape's own pixels, the stroke under the clipped layer");
    assert!(render(&frame, canvas) == render(&white_only(canvas), canvas), "no border");

    shape_split::set_split_limits(defaults.0, defaults.1);
    shape_split::purge_cache();
    assert!(render(&doc, canvas) == stacked, "the defaults split again");

    // Lowering the cache's budget below what it holds trims it at once.
    let held = shape_split::set_split_limits(defaults.0, defaults.1);
    assert_eq!(held, defaults);
    shape_split::set_split_limits(64 << 10, defaults.1);
    assert_eq!(shape_split::purge_cache(), 0, "nothing past the lowered budget is held");
    shape_split::set_split_limits(defaults.0, defaults.1);
}

fn white_only(canvas: Rect) -> Document {
    let mut doc = Document::new("white", Size::new(canvas.width(), canvas.height()), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(white(canvas));
    doc
}
