//! #14293: with a saved path or the Work Path targeted while a shape layer (or a layer with a
//! vector mask) is active, Define Custom Shape stores both paths together in one shape, at their
//! document positions; an open path is closed in the result. With a pixel layer active only the
//! targeted path is stored, and a missing or empty path is still an error.

use photocraft_doc::vector::{Path, PathOp, Subpath, VectorMask};
use photocraft_doc::{Color, ColorMode, Document, Fill, Layer, LayerContent, LayerId, NamedPath, SampleType, ShapeLayer, Size};
use photocraft_vector::shapes;
use serde_json::{Value, json};

use crate::Session;

const CMD: &str = "edit.defineCustomShape";

/// The issue's 600 x 400 test document, with each layer's id.
struct Fixture {
    s: Session,
    rect: LayerId,
    ell: LayerId,
    paint: LayerId,
}

fn rect_path() -> Path {
    shapes::rect(200.0, 100.0, 150.0, 100.0)
}

fn ell_path() -> Path {
    shapes::ellipse(400.0, 250.0, 150.0, 100.0)
}

fn tri_path() -> Path {
    Path::new(vec![Subpath::polygon(&[(100.0, 300.0), (180.0, 380.0), (20.0, 380.0)])])
}

fn open_path() -> Path {
    Path::new(vec![Subpath::polyline(&[(220.0, 250.0), (300.0, 300.0), (220.0, 350.0)])])
}

fn shape_layer(name: &str, path: Path) -> Layer {
    Layer::new(name, LayerContent::Shape(ShapeLayer { path, fill: Some(Fill::Solid(Color::rgb(0.2, 0.4, 0.8))), ..Default::default() }))
}

/// Shape layers "Rect" and "Ell", a pixel layer "Paint", and the saved paths "Tri" and "Open".
fn fixture() -> Fixture {
    let mut doc = Document::with_background("Shapes", Size::new(600, 400), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let rect = doc.insert_above(None, shape_layer("Rect", rect_path()));
    let ell = doc.insert_above(None, shape_layer("Ell", ell_path()));
    let paint = doc.insert_above(None, Layer::new("Paint", LayerContent::Raster(photocraft_raster::Surface::new(doc.pixel_format()))));
    for (name, path) in [("Tri", tri_path()), ("Open", open_path())] {
        doc.paths.push(NamedPath { name: name.into(), path, psd_raw: None });
    }
    let mut s = crate::session();
    s.add_document(doc, None);
    Fixture { s, rect, ell, paint }
}

impl Fixture {
    fn select(&mut self, layer: LayerId) {
        self.s.execute("layer.select", json!({"layer": layer.0})).unwrap();
    }

    fn edit(&mut self, f: impl FnOnce(&mut Document)) {
        self.s
            .edit("test setup", |doc, _| {
                f(doc);
                Ok(())
            })
            .unwrap();
    }

    /// Defines a shape named `name` from `params` and returns its stored path.
    fn define(&mut self, name: &str, mut params: Value) -> Path {
        params["name"] = json!(name);
        self.s.execute(CMD, params).unwrap();
        self.s.edit_state.custom_shapes.iter().find(|c| c.name == name).unwrap().path.clone()
    }
}

fn anchors(p: &Path) -> Vec<Vec<(f64, f64)>> {
    p.subpaths.iter().map(|s| s.knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect()).collect()
}

/// `stored` holds every subpath of each of `parts`, and nothing else, whatever the order.
fn holds_exactly(stored: &Path, parts: &[Path]) -> bool {
    let mut want: Vec<_> = parts.iter().flat_map(anchors).collect();
    let mut got = anchors(stored);
    want.sort_by(|a, b| a.partial_cmp(b).unwrap());
    got.sort_by(|a, b| a.partial_cmp(b).unwrap());
    want == got
}

#[test]
fn a_targeted_saved_path_is_stored_together_with_the_active_shape_layer() {
    let mut f = fixture();
    f.select(f.rect);
    let stored = f.define("Rect + Tri", json!({"path": "Tri"}));
    assert!(holds_exactly(&stored, &[rect_path(), tri_path()]), "both paths: {:?}", anchors(&stored));
    assert_eq!(stored.control_bounds(), Some((20.0, 100.0, 350.0, 380.0)), "at their document offsets");
}

#[test]
fn an_open_targeted_path_is_closed_when_stored_with_a_shape_layer() {
    let mut f = fixture();
    f.select(f.ell);
    let stored = f.define("Ell + Open", json!({"path": "Open"}));
    assert!(holds_exactly(&stored, &[ell_path(), open_path()]), "both paths: {:?}", anchors(&stored));
    assert!(stored.subpaths.iter().all(|s| s.closed), "the open path is closed in the result");

    f.select(f.paint);
    let alone = f.define("Open alone", json!({"path": "Open"}));
    assert_eq!(alone, open_path(), "stored alone, the open path stays open");
}

#[test]
fn with_a_pixel_layer_active_only_the_targeted_path_is_stored() {
    let mut f = fixture();
    f.select(f.paint);
    assert_eq!(f.define("Tri", json!({"path": "Tri"})), tri_path());
}

#[test]
fn the_work_path_and_overlapping_paths_are_stored_together_too() {
    let mut f = fixture();
    // Overlaps Rect, and starts with a Subtract that only applies inside its own path: stored with
    // the layer it must still add its own area instead of cutting into the rectangle.
    let over = Path::new(vec![Subpath::polygon(&[(300.0, 150.0), (400.0, 150.0), (400.0, 250.0), (300.0, 250.0)]).with_op(PathOp::Subtract)]);
    let work = Path::new(vec![Subpath::polygon(&[(250.0, 50.0), (320.0, 50.0), (320.0, 120.0)])]);
    let (o, w) = (over.clone(), work.clone());
    f.edit(move |doc| {
        doc.paths.push(NamedPath { name: "Over".into(), path: o, psd_raw: None });
        doc.work_path = Some(w);
    });
    f.select(f.rect);

    let stored = f.define("Rect + Over", json!({"path": "Over"}));
    assert!(holds_exactly(&stored, &[rect_path(), over.clone()]), "both paths: {:?}", anchors(&stored));
    let i = anchors(&stored).iter().position(|a| *a == anchors(&over)[0]).unwrap();
    assert_eq!(stored.effective_op(i), PathOp::Combine, "the targeted path adds its own shape");

    let stored = f.define("Rect + Work", json!({"path": "work"}));
    assert!(holds_exactly(&stored, &[rect_path(), work]), "both paths: {:?}", anchors(&stored));
}

#[test]
fn a_targeted_path_is_stored_together_with_the_active_layers_vector_mask() {
    let mut f = fixture();
    let paint = f.paint;
    f.edit(move |doc| doc.layer_mut(paint).unwrap().vector_mask = Some(VectorMask::new(rect_path())));
    f.select(paint);
    let stored = f.define("Mask + Tri", json!({"path": "Tri"}));
    assert!(holds_exactly(&stored, &[rect_path(), tri_path()]), "both paths: {:?}", anchors(&stored));
}

#[test]
fn a_missing_or_empty_targeted_path_is_still_an_error_with_a_shape_layer_active() {
    let mut f = fixture();
    f.edit(|doc| doc.paths.push(NamedPath { name: "Empty".into(), path: Path::default(), psd_raw: None }));
    f.select(f.rect);
    let before = f.s.edit_state.custom_shapes.len();
    for path in ["Nope", "Empty", "work"] {
        assert!(f.s.execute(CMD, json!({"path": path, "name": path})).is_err(), "{path}: refused, not the layer's path alone");
    }
    assert_eq!(f.s.edit_state.custom_shapes.len(), before, "nothing was stored");
}

#[test]
fn without_a_targeted_path_the_active_shape_layer_alone_is_stored_as_before() {
    let mut f = fixture();
    f.select(f.rect);
    assert_eq!(f.define("Rect", json!({})), rect_path());
}

/// Fill coverage of `p` over the test document.
fn area(p: &Path) -> Vec<f32> {
    photocraft_vector::path_coverage(p, photocraft_geom::Rect::new(0, 0, 600, 400))
}

/// `stored` fills exactly what `a` or `b` fills on its own.
fn fills_the_union(stored: &Path, a: &Path, b: &Path) -> bool {
    let (s, a, b) = (area(stored), area(a), area(b));
    s.iter().zip(a.iter().zip(&b)).all(|(s, (a, b))| (s - a.max(*b)).abs() < 1e-3)
}

/// A square from `x, y`, `w` pixels wide, with `op`.
fn square(x: f64, y: f64, w: f64, op: PathOp) -> Subpath {
    Subpath::polygon(&[(x, y), (x + w, y), (x + w, y + w), (x, y + w)]).with_op(op)
}

#[test]
fn a_cut_in_either_path_stays_inside_that_paths_own_area() {
    // Overlapping Rect (200,100 to 350,200), with a hole whose corner lies inside Rect: stored with
    // the layer, the hole must not cut into the rectangle.
    let holed = Path::new(vec![square(300.0, 150.0, 150.0, PathOp::Combine), square(320.0, 170.0, 60.0, PathOp::Subtract)]);
    let plain = Path::new(vec![square(320.0, 170.0, 60.0, PathOp::Combine)]);
    for (layer, targeted) in [(rect_path(), holed.clone()), (holed, plain)] {
        let mut f = fixture();
        let (rect, l, t) = (f.rect, layer.clone(), targeted.clone());
        f.edit(move |doc| {
            if let LayerContent::Shape(sh) = &mut doc.layer_mut(rect).unwrap().content {
                sh.path = l;
            }
            doc.paths.push(NamedPath { name: "Target".into(), path: t, psd_raw: None });
        });
        f.select(f.rect);
        let stored = f.define("Both", json!({"path": "Target"}));
        assert!(holds_exactly(&stored, &[layer.clone(), targeted.clone()]), "both paths: {:?}", anchors(&stored));
        assert!(fills_the_union(&stored, &layer, &targeted), "the layer and the targeted path each keep their own area");
    }
}

#[test]
fn paths_that_cannot_keep_both_areas_are_refused() {
    let holed = || Path::new(vec![square(300.0, 150.0, 150.0, PathOp::Combine), square(320.0, 170.0, 60.0, PathOp::Subtract)]);
    let inverted = Path { inverted: true, ..tri_path() };
    let even_odd = Path { fill_rule: photocraft_doc::vector::FillRule::EvenOdd, ..tri_path() };
    for (case, layer, targeted) in [("both cut", holed(), holed()), ("inverted", rect_path(), inverted), ("fill rules differ", rect_path(), even_odd)] {
        let mut f = fixture();
        let rect = f.rect;
        f.edit(move |doc| {
            if let LayerContent::Shape(sh) = &mut doc.layer_mut(rect).unwrap().content {
                sh.path = layer;
            }
            doc.paths.push(NamedPath { name: "Target".into(), path: targeted, psd_raw: None });
        });
        f.select(f.rect);
        let before = f.s.edit_state.custom_shapes.len();
        assert!(f.s.execute(CMD, json!({"path": "Target", "name": case})).is_err(), "{case}: refused");
        assert_eq!(f.s.edit_state.custom_shapes.len(), before, "{case}: nothing was stored");
    }
}
