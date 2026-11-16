//! #13456: `path.alignComponents` and `path.distributeComponents`, the Path alignment popup's
//! Align, Distribute and Distribute Spacing entries, on the components of one path.

use photocraft_doc::{LayerContent, LayerId, Path};
use serde_json::{Value, json};

use crate::Session;

/// A document `w` × `h` px.
fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    s
}

/// A closed polygon subpath in the compact path form.
fn poly(pts: &[[f64; 2]]) -> Value {
    json!({"closed": true, "knots": pts})
}

/// The axis-aligned rectangle `x0..x1` × `y0..y1` as a closed subpath.
fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    poly(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
}

/// Every subpath's anchors, in order.
fn anchors(path: &Path) -> Vec<Vec<[f64; 2]>> {
    path.subpaths.iter().map(|s| s.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()).collect()
}

/// The saved path `name`.
fn saved(s: &Session, name: &str) -> Path {
    s.active().unwrap().doc.paths.iter().find(|p| p.name == name).unwrap().path.clone()
}

/// The path of shape layer `id`.
fn shape_path(s: &Session, id: u64) -> Path {
    match &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
        LayerContent::Shape(sh) => sh.path.clone(),
        _ => panic!("not a shape layer"),
    }
}

fn history_len(s: &Session) -> usize {
    s.active().unwrap().history.entries().len()
}

fn undo_label(s: &Session) -> Option<String> {
    s.active().unwrap().history.undo_label().map(str::to_owned)
}

/// A shape layer whose components are `subpaths`, made with `shape.create`.
fn shape_layer(s: &mut Session, subpaths: Vec<Value>) -> u64 {
    let r = s.execute("shape.create", json!({"kind": "path", "path": {"subpaths": subpaths}, "fill": "#ff0000"})).unwrap();
    r["layer"].as_u64().unwrap()
}

/// The measured example: saved path with square A at 131..331 × 330..530 and triangle B at
/// 400,300 / 600,300 / 500,500, both selected, Align To Selection, Align Left Edges: B moves to
/// x 131 and A stays, in one History step named Align Path Components.
#[test]
fn align_left_edges_matches_the_measured_example() {
    let mut s = session(800, 800);
    let path = json!({"subpaths": [rect(131.0, 330.0, 331.0, 530.0), poly(&[[400.0, 300.0], [600.0, 300.0], [500.0, 500.0]])]});
    s.execute("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    let before = saved(&s, "Path 1");
    let steps = history_len(&s);
    s.execute("path.alignComponents", json!({"name": "Path 1", "subpaths": [0, 1], "edge": "leftEdges", "to": "selection"})).unwrap();
    assert_eq!(history_len(&s), steps + 1);
    assert_eq!(undo_label(&s).as_deref(), Some("Align Path Components"));
    let after = anchors(&saved(&s, "Path 1"));
    assert_eq!(after[0], anchors(&before)[0], "A stays");
    assert_eq!(after[1], [[131.0, 300.0], [331.0, 300.0], [231.0, 500.0]], "B's left edge at 131");
    assert!(s.undo());
    assert_eq!(saved(&s, "Path 1"), before);
    assert!(s.redo());
    assert_eq!(anchors(&saved(&s, "Path 1"))[1], [[131.0, 300.0], [331.0, 300.0], [231.0, 500.0]]);
}

/// Every Align entry on a shape layer's three components (default: all of them), against their
/// combined bounds 10..150 × 20..140: each lands exactly on that edge or centre line.
#[test]
fn every_align_edge_lines_the_components_up_on_a_shape_layer() {
    // A 10..30 × 20..60, B 60..100 × 40..50, C 120..150 × 100..140.
    let parts = || vec![rect(10.0, 20.0, 30.0, 60.0), rect(60.0, 40.0, 100.0, 50.0), rect(120.0, 100.0, 150.0, 140.0)];
    let x0 = |p: &Path, i: usize| p.subpaths[i].knots[0].anchor.x;
    let y0 = |p: &Path, i: usize| p.subpaths[i].knots[0].anchor.y;
    for (edge, want) in [
        // The first corner of each component afterwards.
        ("leftEdges", [[10.0, 20.0], [10.0, 40.0], [10.0, 100.0]]),
        ("horizontalCenters", [[70.0, 20.0], [60.0, 40.0], [65.0, 100.0]]),
        ("rightEdges", [[130.0, 20.0], [110.0, 40.0], [120.0, 100.0]]),
        ("topEdges", [[10.0, 20.0], [60.0, 20.0], [120.0, 20.0]]),
        ("verticalCenters", [[10.0, 60.0], [60.0, 75.0], [120.0, 60.0]]),
        ("bottomEdges", [[10.0, 100.0], [60.0, 130.0], [120.0, 100.0]]),
    ] {
        let mut s = session(200, 200);
        let id = shape_layer(&mut s, parts());
        let before = shape_path(&s, id);
        s.execute("path.alignComponents", json!({"name": "layer", "layer": id, "edge": edge})).unwrap();
        assert_eq!(undo_label(&s).as_deref(), Some("Align Path Components"), "{edge}");
        let p = shape_path(&s, id);
        let got: Vec<[f64; 2]> = (0..3).map(|i| [x0(&p, i), y0(&p, i)]).collect();
        assert_eq!(got, want, "{edge}");
        // Each component moves as a whole: its size is unchanged.
        for i in 0..3 {
            let (b, a) = (&before.subpaths[i].knots, &p.subpaths[i].knots);
            assert_eq!(a[2].anchor.x - a[0].anchor.x, b[2].anchor.x - b[0].anchor.x, "{edge} width {i}");
            assert_eq!(a[2].anchor.y - a[0].anchor.y, b[2].anchor.y - b[0].anchor.y, "{edge} height {i}");
        }
        assert!(s.undo());
        assert_eq!(shape_path(&s, id), before, "{edge}: undo");
    }
}

/// Align To Canvas lines the components up on the canvas, even a single one.
#[test]
fn align_to_canvas_uses_the_canvas_bounds() {
    let mut s = session(300, 200);
    s.execute("path.set", json!({"path": {"subpaths": [rect(10.0, 20.0, 30.0, 60.0), rect(60.0, 40.0, 100.0, 50.0)]}})).unwrap();
    s.execute("path.alignComponents", json!({"edge": "rightEdges", "to": "canvas"})).unwrap();
    let work = s.active().unwrap().doc.work_path.clone().unwrap();
    assert_eq!(anchors(&work), [[[280.0, 20.0], [300.0, 20.0], [300.0, 60.0], [280.0, 60.0]], [[260.0, 40.0], [300.0, 40.0], [300.0, 50.0], [260.0, 50.0]]]);
    s.execute("path.alignComponents", json!({"subpaths": [1], "edge": "verticalCenters", "to": "canvas"})).unwrap();
    let work = s.active().unwrap().doc.work_path.clone().unwrap();
    assert_eq!(anchors(&work)[1], [[260.0, 95.0], [300.0, 95.0], [300.0, 105.0], [260.0, 105.0]]);
    assert_eq!(anchors(&work)[0][0], [280.0, 20.0], "the component not named stays");
}

/// A component's bounds are its outline's, not its handles': a curve bulging to y 70 (handles at
/// y 60) is what the other component's top edge lines up with.
#[test]
fn bounds_follow_the_curve_not_its_handles() {
    let mut s = session(300, 200);
    let bump = json!({"closed": true, "knots": [
        {"anchor": [100, 100], "in": [100, 100], "out": [100, 60]},
        {"anchor": [140, 100], "in": [140, 60], "out": [140, 100]},
    ]});
    s.execute("path.set", json!({"path": {"subpaths": [bump, rect(200.0, 80.0, 220.0, 100.0)]}})).unwrap();
    s.execute("path.alignComponents", json!({"edge": "topEdges"})).unwrap();
    let work = s.active().unwrap().doc.work_path.clone().unwrap();
    assert_eq!(anchors(&work)[1], [[200.0, 70.0], [220.0, 70.0], [220.0, 90.0], [200.0, 90.0]]);
    assert_eq!(anchors(&work)[0], [[100.0, 100.0], [140.0, 100.0]], "the bump is already at the top");
}

/// A component moves with its handles and with the subpaths joined to it (a hole).
#[test]
fn a_component_moves_with_its_handles_and_joined_subpaths() {
    let mut s = session(300, 200);
    // A curve spanning x 100..140 (its handles stay inside that range).
    let curve = json!({"closed": true, "knots": [
        {"anchor": [100, 100], "in": [100, 110], "out": [110, 90]},
        {"anchor": [140, 100], "in": [130, 90], "out": [140, 110]},
        [120, 140],
    ]});
    let mut hole = rect(30.0, 30.0, 40.0, 40.0);
    hole["op"] = json!("join");
    s.execute("path.set", json!({"path": {"subpaths": [rect(20.0, 20.0, 50.0, 50.0), hole, curve]}})).unwrap();
    let before = s.active().unwrap().doc.work_path.clone().unwrap();
    // Naming the hole names its component, the square.
    s.execute("path.alignComponents", json!({"subpaths": [1, 2], "edge": "leftEdges"})).unwrap();
    let after = s.active().unwrap().doc.work_path.clone().unwrap();
    assert_eq!(anchors(&after)[..2], anchors(&before)[..2], "the square and its hole stay");
    let k = &after.subpaths[2].knots[0];
    assert_eq!([k.anchor.x, k.in_ctrl.x, k.out_ctrl.x, k.anchor.y, k.in_ctrl.y, k.out_ctrl.y], [20.0, 20.0, 30.0, 100.0, 110.0, 90.0], "moved by -80");
    // The curve now spans 20..60: aligning right edges moves the square and its hole by +10.
    s.execute("path.alignComponents", json!({"subpaths": [0, 2], "edge": "rightEdges"})).unwrap();
    let p = anchors(&s.active().unwrap().doc.work_path.clone().unwrap());
    assert_eq!(p[0], [[30.0, 20.0], [60.0, 20.0], [60.0, 50.0], [30.0, 50.0]]);
    assert_eq!(p[1], [[40.0, 30.0], [50.0, 30.0], [50.0, 40.0], [40.0, 40.0]]);
    assert_eq!(p[2][0], [20.0, 100.0], "the curve stays");
}

/// Distribute: the outermost two stay, the middle ones are spaced evenly by the chosen edge or
/// centre, whatever their order in the path.
#[test]
fn distribute_spaces_edges_and_centres_evenly() {
    // In the path: C 160..200 (front), A 0..20, B 30..40 (back), 10 px tall each at different y.
    let parts = || vec![rect(160.0, 0.0, 200.0, 10.0), rect(0.0, 20.0, 20.0, 30.0), rect(30.0, 40.0, 40.0, 50.0)];
    for (edge, want_b_x0) in [("leftEdges", 80.0), ("horizontalCenters", 90.0), ("rightEdges", 100.0)] {
        let mut s = session(300, 200);
        s.execute("path.set", json!({"path": {"subpaths": parts()}})).unwrap();
        s.execute("path.distributeComponents", json!({"edge": edge})).unwrap();
        assert_eq!(undo_label(&s).as_deref(), Some("Distribute Path Components"), "{edge}");
        let p = anchors(&s.active().unwrap().doc.work_path.clone().unwrap());
        assert_eq!(p[0][0], [160.0, 0.0], "{edge}: the last stays");
        assert_eq!(p[1][0], [0.0, 20.0], "{edge}: the first stays");
        assert_eq!(p[2][0], [want_b_x0, 40.0], "{edge}: the middle one");
    }
    // Vertically: tops at 0, 20, 100 → the middle one's top moves to 50.
    let mut s = session(300, 200);
    s.execute("path.set", json!({"path": {"subpaths": [rect(0.0, 100.0, 10.0, 120.0), rect(0.0, 0.0, 10.0, 10.0), rect(50.0, 20.0, 60.0, 30.0)]}})).unwrap();
    s.execute("path.distributeComponents", json!({"edge": "topEdges"})).unwrap();
    let p = anchors(&s.active().unwrap().doc.work_path.clone().unwrap());
    assert_eq!([p[0][0], p[1][0], p[2][0]], [[0.0, 100.0], [0.0, 0.0], [50.0, 50.0]]);
    s.execute("path.distributeComponents", json!({"edge": "bottomEdges"})).unwrap();
    let p = anchors(&s.active().unwrap().doc.work_path.clone().unwrap());
    assert_eq!(p[2][2], [60.0, 65.0], "bottoms at 10, 65, 120");
}

/// Distribute Spacing: equal gaps between neighbours of different sizes; the outermost stay.
#[test]
fn distribute_spacing_makes_the_gaps_equal() {
    // Widths 20, 40, 10 spanning 0..200: 130 px of gap, 65 each → the 40-wide one at 85..125.
    let mut s = session(300, 300);
    s.execute("path.set", json!({"path": {"subpaths": [rect(190.0, 0.0, 200.0, 10.0), rect(0.0, 0.0, 20.0, 10.0), rect(30.0, 50.0, 70.0, 60.0)]}})).unwrap();
    s.execute("path.distributeComponents", json!({"edge": "horizontally"})).unwrap();
    let p = anchors(&s.active().unwrap().doc.work_path.clone().unwrap());
    assert_eq!([p[0][0], p[1][0], p[2][0], p[2][2]], [[190.0, 0.0], [0.0, 0.0], [85.0, 50.0], [125.0, 60.0]]);
    // Heights 10, 30, 20 spanning 0..140: 80 px of gap, 40 each → the 30-tall one at 50..80.
    s.execute("path.set", json!({"path": {"subpaths": [rect(0.0, 0.0, 5.0, 10.0), rect(0.0, 120.0, 5.0, 140.0), rect(0.0, 10.0, 5.0, 40.0)]}})).unwrap();
    s.execute("path.distributeComponents", json!({"edge": "vertically"})).unwrap();
    let p = anchors(&s.active().unwrap().doc.work_path.clone().unwrap());
    assert_eq!([p[0][0], p[1][0], p[2][0], p[2][2]], [[0.0, 0.0], [0.0, 120.0], [0.0, 50.0], [5.0, 80.0]]);
    assert!(s.undo());
    let p = anchors(&s.active().unwrap().doc.work_path.clone().unwrap());
    assert_eq!(p[2][0], [0.0, 10.0], "undo");
}

/// A shape layer is re-rendered: its pixels follow the moved component.
#[test]
fn a_shape_layer_is_rerendered() {
    let mut s = session(200, 200);
    let id = shape_layer(&mut s, vec![rect(10.0, 10.0, 30.0, 30.0), rect(100.0, 100.0, 120.0, 120.0)]);
    s.execute("path.alignComponents", json!({"name": "layer", "layer": id, "edge": "leftEdges"})).unwrap();
    let st = s.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(LayerId(id)).unwrap().content else { panic!("shape") };
    let cache = sh.cache.as_ref().expect("re-rendered");
    let bounds = cache.content_bounds();
    assert_eq!((bounds.x0, bounds.x1, bounds.y0, bounds.y1), (10, 30, 10, 120), "both squares in the x 10..30 column");
}

/// Bad params, too few components and missing targets are errors that change nothing.
#[test]
fn bad_params_and_too_few_components_change_nothing() {
    let mut s = session(200, 200);
    s.execute("path.set", json!({"path": {"subpaths": [rect(10.0, 10.0, 20.0, 20.0), rect(50.0, 50.0, 60.0, 70.0)]}})).unwrap();
    let before = s.active().unwrap().doc.work_path.clone();
    let steps = history_len(&s);
    for (cmd, p, msg) in [
        ("path.alignComponents", json!({}), "`edge`"),
        ("path.alignComponents", json!({"edge": "middle"}), "`edge`"),
        ("path.alignComponents", json!({"edge": 3}), "`edge`"),
        ("path.alignComponents", json!({"edge": "horizontally"}), "`edge`"),
        ("path.alignComponents", json!({"edge": "leftEdges", "to": "layers"}), "`to`"),
        ("path.alignComponents", json!({"edge": "leftEdges", "to": null}), "`to`"),
        ("path.alignComponents", json!({"edge": "leftEdges", "subpaths": []}), "`subpaths`"),
        ("path.alignComponents", json!({"edge": "leftEdges", "subpaths": 1}), "`subpaths`"),
        ("path.alignComponents", json!({"edge": "leftEdges", "subpaths": [-1]}), "index"),
        ("path.alignComponents", json!({"edge": "leftEdges", "subpaths": [0, 9]}), "no subpath 9"),
        ("path.alignComponents", json!({"edge": "leftEdges", "subpaths": [1]}), "two or more"),
        ("path.alignComponents", json!({"edge": "leftEdges", "name": "Nope"}), ""),
        ("path.alignComponents", json!({"edge": "leftEdges", "name": "layer", "layer": "x"}), "`layer`"),
        ("path.alignComponents", json!({"edge": "leftEdges", "name": 5}), "`name`"),
        ("path.distributeComponents", json!({}), "`edge`"),
        ("path.distributeComponents", json!({"edge": "sideways"}), "`edge`"),
        ("path.distributeComponents", json!({"edge": "leftEdges"}), "three or more"),
        ("path.distributeComponents", json!({"edge": "horizontally", "subpaths": [0, 1]}), "three or more"),
        ("path.distributeComponents", json!({"edge": "vertically", "subpaths": [7]}), "no subpath 7"),
    ] {
        let e = s.execute(cmd, p.clone()).unwrap_err().to_string();
        assert!(e.contains(msg), "{cmd} {p}: {e}");
    }
    assert_eq!(history_len(&s), steps);
    assert_eq!(s.active().unwrap().doc.work_path, before);

    // No Work Path, and no document at all.
    let mut s = session(100, 100);
    assert!(s.execute("path.alignComponents", json!({"edge": "leftEdges"})).is_err());
    assert!(s.execute("path.alignComponents", json!({"edge": "leftEdges", "to": "canvas"})).is_err());
    assert!(s.execute("path.distributeComponents", json!({"edge": "leftEdges"})).is_err());
    let mut s = Session::new();
    assert!(s.execute("path.alignComponents", json!({"edge": "leftEdges"})).is_err());
    assert!(s.execute("path.distributeComponents", json!({"edge": "horizontally"})).is_err());
}
