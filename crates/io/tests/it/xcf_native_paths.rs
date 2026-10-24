//! Independent public-wire geometry and ordinary native vector counterparts.
//! No local research/corpus file is required by these tests.

use photocraft_color::Color;
use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};
use photocraft_doc::vector::{FillRule, Knot, LineCap, LineJoin, Path, PathOp, ShapeLayer, ShapeStroke, Subpath};
use photocraft_doc::{Fill, LayerContent};
use photocraft_geom::{Point, Rect};

struct Wire(Vec<u8>);
impl Wire {
    fn word(&mut self, value: u32) {
        self.0.extend(value.to_be_bytes());
    }
    fn float(&mut self, value: f32) {
        self.word(value.to_bits());
    }
    fn ptr(&mut self, value: usize) {
        self.0.extend((value as u64).to_be_bytes());
    }
    fn slot(&mut self) -> usize {
        let at = self.0.len();
        self.ptr(0);
        at
    }
    fn patch(&mut self, at: usize) {
        let offset = self.0.len() as u64;
        self.0[at..at + 8].copy_from_slice(&offset.to_be_bytes());
    }
    fn text(&mut self, value: &str) {
        self.word(value.len() as u32 + 1);
        self.0.extend(value.as_bytes());
        self.0.push(0);
    }
    fn property(&mut self, kind: u32, value: &[u8]) {
        self.word(kind);
        self.word(value.len() as u32);
        self.0.extend(value);
    }
    fn end(&mut self) {
        self.word(0);
        self.word(0);
    }
    fn paint(&mut self, rgba: [u8; 4]) {
        self.word(0); // Solid paint.
        self.word(1); // Antialiased.
        self.text("R'G'B'A u8");
        self.word(4);
        self.0.extend(rgba);
        self.word(0); // Portable sRGB encoding; no source profile.
        self.text(""); // No resource identity or permission.
    }
}

fn knots() -> Vec<Knot> {
    vec![
        Knot { anchor: Point::new(-2.0, 3.0), in_ctrl: Point::new(-3.0, 1.0), out_ctrl: Point::new(2.0, 8.0), smooth: false },
        Knot { anchor: Point::new(12.0, 7.0), in_ctrl: Point::new(8.0, -2.0), out_ctrl: Point::new(15.0, 10.0), smooth: false },
    ]
}
fn expected() -> ShapeLayer {
    ShapeLayer {
        path: Path { subpaths: vec![Subpath { knots: knots(), closed: true, op: PathOp::Combine }], fill_rule: FillRule::EvenOdd, inverted: false },
        fill: Some(Fill::Solid(Color::rgba(1.0, 0.0, 0.0, 1.0))),
        stroke: Some(ShapeStroke {
            width: 2.5,
            paint: Fill::Solid(Color::BLACK),
            cap: LineCap::Round,
            join: LineJoin::Bevel,
            miter_limit: 4.0,
            dashes: vec![2.0, 3.0],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn source(modified: bool, referenced_tattoo: u32, axes: u32) -> Vec<u8> {
    let mut w = Wire(b"gimp xcf v026\0".to_vec());
    for value in [16, 12, 0, 150] {
        w.word(value);
    }
    w.property(17, &[0]);
    w.end();
    let layer = w.slot();
    w.ptr(0);
    w.ptr(0);
    let path = w.slot();
    w.ptr(0);
    w.patch(layer);
    for value in [1, 1, 1] {
        w.word(value);
    }
    w.text("Native curve");
    w.property(15, &[(-2i32).to_be_bytes(), 3i32.to_be_bytes()].concat());
    let mut style = Wire(Vec::new());
    for value in [u32::from(modified), referenced_tattoo, 1, 1] {
        style.word(value);
    }
    style.paint([255, 0, 0, 255]);
    style.paint([0, 0, 0, 255]);
    style.float(2.5);
    for value in [1, 2] {
        style.word(value);
    }
    style.float(4.0);
    style.word(2);
    style.float(2.0);
    style.float(3.0);
    w.property(47, &style.0);
    w.end();
    let hierarchy = w.slot();
    w.ptr(0);
    w.ptr(0);
    w.patch(hierarchy);
    for value in [1, 1, 4] {
        w.word(value);
    }
    let level = w.slot();
    w.ptr(0);
    w.patch(level);
    w.word(1);
    w.word(1);
    let tile = w.slot();
    w.ptr(0);
    w.patch(tile);
    w.0.extend([17, 31, 57, 255]);
    w.patch(path);
    w.text("曲線");
    let mut payload = Wire(Vec::new());
    payload.property(20, &73u32.to_be_bytes());
    payload.property(43, &[]);
    payload.end();
    for value in [1, 1, 1, 1, axes, 6] {
        payload.word(value);
    } // Version, stroke count/type/closure/axes/points.
    for knot in knots() {
        for (kind, point) in [(1, knot.in_ctrl), (0, knot.anchor), (1, knot.out_ctrl)] {
            payload.word(kind);
            payload.float(point.x as f32);
            payload.float(point.y as f32);
            for _ in 2..axes {
                payload.float(0.5);
            }
        }
    }
    w.word(payload.0.len() as u32);
    w.0.extend(payload.0);
    w.0
}

#[test]
fn cubic_shape_fill_stroke_and_auxiliary_axes_use_native_geometry_and_rendering() {
    for axes in [2, 6] {
        let result = photocraft_io::import("original.xcf", &source(false, 73, axes)).unwrap();
        let doc = result.document;
        let LayerContent::Shape(shape) = &doc.layers[0].content else { panic!("native editable shape") };
        let expected = expected();
        let Some(Fill::Solid(color)) = &shape.fill else { panic!("native solid fill") };
        assert!(color.c.iter().zip([1.0, 0.0, 0.0, 0.0]).all(|(actual, wanted)| (actual - wanted).abs() < 1e-6));
        assert_eq!(color.alpha, 1.0);
        // Ordinary ICC conversion can leave sub-ULP matrix roundoff at a
        // nominal zero primary. All geometry/style fields remain exact.
        let mut structural = shape.clone();
        structural.fill = expected.fill.clone();
        assert_eq!(structural, expected);
        assert!(shape.cache.is_none(), "source raster is never a native shape cache");
        assert_eq!(doc.paths[0].name, "曲線");
        assert_eq!(doc.paths[0].path, expected.path);
        assert_eq!(result.initial_view.unwrap().selected_path, Some(0));
        let bounds = Rect::new(-4, -4, 20, 16);
        let actual = photocraft_vector::render_shape(shape, doc.pixel_format(), bounds);
        let wanted = photocraft_vector::render_shape(&expected, doc.pixel_format(), bounds);
        assert_eq!(actual, wanted);
        assert!(actual.read_region(bounds).as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0.0));
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
        assert_eq!(reopened.paths, doc.paths);
        assert_eq!(reopened.layers[0].content, doc.layers[0].content);
        assert_eq!(reopened.metadata.import_conversion, doc.metadata.import_conversion);
    }
}

#[test]
fn modified_or_missing_vector_reference_preserves_separate_stored_pixels_and_saved_path() {
    for (modified, tattoo, reason) in [(true, 73, ConversionReason::NativePolicy), (false, 99, ConversionReason::InvalidOptionalData)] {
        let doc = photocraft_io::import("fallback.xcf", &source(modified, tattoo, 2)).unwrap().document;
        assert_eq!(doc.paths[0].path, expected().path);
        let LayerContent::Raster(surface) = &doc.layers[0].content else { panic!("native raster fallback") };
        assert_eq!(surface.read_region(Rect::new(-2, 3, -1, 4)), vec![17.0 / 255.0, 31.0 / 255.0, 57.0 / 255.0, 1.0]);
        let Some(ImportConversion::Report(report)) = &doc.metadata.import_conversion else { panic!("report") };
        assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.vector_layer" && entry.reason == reason));
        assert!(report.attachments.iter().any(|entry| entry.kind == "xcf.property.47"));
    }
}
