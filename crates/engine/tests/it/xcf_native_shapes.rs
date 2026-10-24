//! Synthetic XCF shapes follow ordinary native preparation, editing and history.
//! No local research/corpus file is required by these tests.

use photocraft_color::Color;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::vector::{FillRule, Knot, LineCap, LineJoin, Path, PathOp, ShapeLayer, ShapeStroke, Subpath};
use photocraft_doc::{Document, Fill, Layer, LayerContent};
use photocraft_engine::{Session, file_cmds::open_bytes_as, jobs::JobCtx};
use photocraft_geom::{Point, Size};
use serde_json::json;

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

fn counterpart() -> Document {
    let mut doc = Document::new("Original native shape", Size::new(16, 12), ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = 72.0;
    doc.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    let shape = expected();
    doc.paths.push(photocraft_doc::vector::NamedPath { name: "曲線".into(), path: shape.path.clone(), psd_raw: None });
    doc.layers.push(Layer::new("Native curve", LayerContent::Shape(shape)));
    doc
}

fn compare(imported: &Session, native: &Session) {
    let a = &imported.active().unwrap().doc;
    let b = &native.active().unwrap().doc;
    assert_eq!((a.size, a.mode, a.depth), (b.size, b.mode, b.depth));
    assert_eq!(a.paths, b.paths);
    assert_eq!(a.layers.len(), b.layers.len());
    for (a, b) in a.layers.iter().zip(&b.layers) {
        let mut actual = a.clone();
        actual.id = b.id;
        let (LayerContent::Shape(actual_shape), LayerContent::Shape(expected_shape)) = (&mut actual.content, &b.content) else {
            panic!("ordinary editable native shapes")
        };
        // ICC primary conversion can leave sub-ULP matrix roundoff. Establish
        // the same paint independently before normalizing only that roundoff.
        let (Some(Fill::Solid(actual_fill)), Some(Fill::Solid(expected_fill))) = (&actual_shape.fill, &expected_shape.fill) else {
            panic!("native solid fill")
        };
        assert_eq!(actual_fill.mode, expected_fill.mode);
        assert!(actual_fill.c.iter().zip(expected_fill.c).all(|(a, b)| (*a - b).abs() < 1e-6));
        assert!((actual_fill.alpha - expected_fill.alpha).abs() < 1e-6);
        actual_shape.fill = expected_shape.fill.clone();
        assert!(actual_shape.cache.is_some(), "first open and edits prepare a native shape cache");
        assert_eq!(&actual, b, "native geometry, paint/stroke, cache, placement and layer state");
    }
    let actual = photocraft_compose::flatten(a).px;
    let expected = photocraft_compose::flatten(b).px;
    assert_eq!(actual, expected, "ordinary native rendering, not the unrelated source pixel cache");
    assert!(actual.iter().any(|pixel| pixel[3] > 0.0));
}

#[test]
fn synthetic_xcf_shape_first_open_edit_duplicate_transform_history_and_native_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("original.xcf");
    let saved = directory.path().join("editable.pcraft");
    let bytes = source(false, 73, 2);
    std::fs::write(&source_path, &bytes).unwrap();
    let mut imported = Session::new();
    imported.color.settings.ask_on_missing = false;
    open_bytes_as(&mut imported, source_path.to_str().unwrap(), &bytes, None, Some(source_path.to_string_lossy().into_owned())).unwrap();
    let report = imported.active().unwrap().doc.metadata.import_conversion.clone();
    assert!(report.is_some());
    assert!(imported.active().unwrap().source_read_only);
    let mut native = Session::new();
    native.color.settings.ask_on_missing = false;
    native.open_imported_document(counterpart(), None, &JobCtx::new()).unwrap();
    compare(&imported, &native);
    for (command, params) in [
        ("shape.edit", json!({"fill":"#00ff00","stroke":{"width":1.25,"color":"#0000ff"}})),
        ("shape.edit", json!({"transform":[1,0.25,-0.125,1,2,-1]})),
        ("layer.duplicate", json!({})),
        ("layer.translate", json!({"dx":-2,"dy":3})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
    ] {
        imported.execute(command, params.clone()).unwrap();
        native.execute(command, params).unwrap();
        compare(&imported, &native);
        assert_eq!(imported.active().unwrap().doc.metadata.import_conversion, report);
    }
    imported.execute("file.saveACopy", json!({"path":saved})).unwrap();
    std::fs::remove_file(&source_path).unwrap();
    let mut reopened = Session::new();
    reopened.color.settings.ask_on_missing = false;
    open_bytes_as(&mut reopened, saved.to_str().unwrap(), &std::fs::read(&saved).unwrap(), None, None).unwrap();
    assert!(!reopened.active().unwrap().source_read_only);
    compare(&reopened, &native);
    assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, report);
    reopened.execute("shape.edit", json!({"move":[1.5,-0.5]})).unwrap();
    reopened.execute("edit.undo", json!({})).unwrap();
    compare(&reopened, &native);
}
