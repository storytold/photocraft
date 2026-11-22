//! #14294: Define Custom Shape applies the path's operations and stores plain outlines in one
//! component, as Merge Shape Components does, instead of the raw operations.

use photocraft_doc::{Path, PathOp, Subpath};
use photocraft_geom::Rect;
use photocraft_vector::shapes::{ellipse, rect};
use serde_json::json;

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 640, "height": 220, "background": "white"})).unwrap();
    s
}

/// A rectangle from (x0, y0) to (x1, y1) with operation `op`.
fn r(x0: f64, y0: f64, x1: f64, y1: f64, op: PathOp) -> Subpath {
    let mut sp = rect(x0, y0, x1 - x0, y1 - y0).subpaths.remove(0);
    sp.op = op;
    sp
}

/// Defines a custom shape from `path` as the work path and returns what was stored.
fn define(path: Path) -> Path {
    let mut s = session();
    s.edit("path", |doc, _| {
        doc.work_path = Some(path);
        Ok(())
    })
    .unwrap();
    s.execute("edit.defineCustomShape", json!({"name": "Probe"})).unwrap();
    s.edit_state.custom_shapes.iter().find(|c| c.name == "Probe").expect("the stored shape").path.clone()
}

fn anchors(p: &Path) -> Vec<usize> {
    p.subpaths.iter().map(|sp| sp.knots.len()).collect()
}

/// The fill coverage of `p` over the pixel at (x, y).
fn covered(p: &Path, x: i32, y: i32) -> bool {
    photocraft_vector::fill_rasterizer(p, 0.1).render(Rect::new(x, y, x + 1, y + 1))[0] > 0.5
}

/// One shape component of plain outlines: a combined first subpath, the others joined to it (their winding cuts the
/// holes), no subtract, intersect or exclude left.
fn plain(p: &Path) -> bool {
    p.components().len() == 1 && p.subpaths[0].op == PathOp::Combine && p.subpaths[1..].iter().all(|sp| sp.op == PathOp::Join)
}

#[test]
fn add_subtract_exclude_is_stored_as_three_plain_outlines() {
    let source = Path::new(vec![
        r(300.0, 20.0, 500.0, 180.0, PathOp::Combine),
        r(350.0, 60.0, 450.0, 140.0, PathOp::Subtract),
        r(520.0, 20.0, 580.0, 80.0, PathOp::Exclude),
    ]);
    let stored = define(source.clone());
    assert_eq!(stored.subpaths.len(), 3, "{:?}", anchors(&stored));
    assert!(plain(&stored), "one component of combined outlines");
    for (x, y) in [(320, 100), (400, 100), (550, 50), (400, 30), (600, 100), (510, 100)] {
        assert_eq!(covered(&stored, x, y), covered(&source, x, y), "coverage at ({x}, {y})");
    }
    assert!(!covered(&stored, 400, 100), "the hole stays empty");
}

#[test]
fn two_overlapping_adds_make_one_eight_anchor_outline() {
    let stored = define(Path::new(vec![r(20.0, 20.0, 120.0, 120.0, PathOp::Combine), r(70.0, 70.0, 170.0, 170.0, PathOp::Combine)]));
    assert_eq!(anchors(&stored), [8]);
    assert!(plain(&stored));
}

#[test]
fn add_and_intersect_make_the_overlap_square() {
    let stored = define(Path::new(vec![r(20.0, 20.0, 120.0, 120.0, PathOp::Combine), r(70.0, 70.0, 170.0, 170.0, PathOp::Intersect)]));
    assert_eq!(anchors(&stored), [4]);
    assert!(plain(&stored));
    assert!(covered(&stored, 95, 95) && !covered(&stored, 40, 40) && !covered(&stored, 150, 150));
}

#[test]
fn add_and_exclude_make_two_l_shapes() {
    let stored = define(Path::new(vec![r(20.0, 20.0, 120.0, 120.0, PathOp::Combine), r(70.0, 70.0, 170.0, 170.0, PathOp::Exclude)]));
    assert_eq!(anchors(&stored), [6, 6]);
    assert!(plain(&stored));
    assert!(!covered(&stored, 95, 95), "the overlap is empty");
    assert!(covered(&stored, 40, 40) && covered(&stored, 150, 150));
}

#[test]
fn an_ellipse_keeps_its_four_smooth_anchors() {
    let source = ellipse(40.0, 30.0, 120.0, 80.0);
    let stored = define(source.clone());
    assert_eq!(stored.subpaths.len(), 1);
    assert_eq!(stored.subpaths[0].knots, source.subpaths[0].knots);
}

#[test]
fn an_open_path_alone_stays_open() {
    let source = Path::new(vec![Subpath::polyline(&[(20.0, 20.0), (90.0, 60.0), (40.0, 120.0)])]);
    let stored = define(source.clone());
    assert_eq!(stored.subpaths, source.subpaths);
}

#[test]
fn two_adds_sharing_an_edge_make_one_rectangle() {
    let stored = define(Path::new(vec![r(20.0, 20.0, 80.0, 90.0, PathOp::Combine), r(80.0, 20.0, 150.0, 90.0, PathOp::Combine)]));
    assert_eq!(anchors(&stored), [4]);
    assert!(covered(&stored, 79, 50) && covered(&stored, 81, 50));
}

#[test]
fn degenerate_paths_are_stored_without_a_panic() {
    let flat = Subpath::polygon(&[(10.0, 10.0), (60.0, 10.0), (60.0, 10.0), (10.0, 10.0)]);
    let point = Subpath::polygon(&[(30.0, 30.0), (30.0, 30.0), (30.0, 30.0)]);
    let nan = Subpath::polygon(&[(f64::NAN, 5.0), (40.0, 5.0), (40.0, 40.0)]);
    for subpaths in
        [vec![flat.clone(), point.clone()], vec![r(5.0, 5.0, 50.0, 50.0, PathOp::Combine), nan], vec![flat, r(0.0, 0.0, 30.0, 30.0, PathOp::Subtract)]]
    {
        let mut s = session();
        s.edit("path", |doc, _| {
            doc.work_path = Some(Path::new(subpaths));
            Ok(())
        })
        .unwrap();
        let _ = s.execute("edit.defineCustomShape", json!({"name": "Odd"}));
    }
}

/// A path whose crossings would need more work than the budget is stored as it is, not after seconds of
/// classification (CodeRabbit on #16189).
#[test]
fn a_path_with_too_much_crossing_work_is_stored_as_it_is() {
    let mut subpaths = Vec::new();
    for i in 0..250 {
        let x = f64::from(i) * 8.0;
        subpaths.push(r(x, 0.0, x + 4.0, 2000.0, PathOp::Combine));
    }
    for j in 0..250 {
        let y = f64::from(j) * 8.0;
        subpaths.push(r(0.0, y, 2000.0, y + 4.0, PathOp::Combine));
    }
    let stored = define(Path::new(subpaths));
    assert_eq!(stored.subpaths.len(), 500, "the original path is kept");
}

/// A hole narrower than the sampling distance can't be classified from the samples: the original path is kept
/// instead of storing a filled square (CodeRabbit on #16189).
#[test]
fn a_hole_narrower_than_the_sampling_distance_keeps_the_original_path() {
    let stored = define(Path::new(vec![r(0.0, 0.0, 20.0, 20.0, PathOp::Combine), r(10.0, 10.0, 10.00005, 10.00005, PathOp::Subtract)]));
    assert_eq!(stored.subpaths.len(), 2, "both subpaths are kept");
    assert_eq!(stored.subpaths[1].op, PathOp::Subtract);
}

/// A hole smaller than both sampling distances is crossed whole by every sample, so the samples agree and still say
/// nothing: the original path is kept (CodeRabbit on #16189).
#[test]
fn a_hole_smaller_than_every_sampling_distance_keeps_the_original_path() {
    let stored = define(Path::new(vec![r(0.0, 0.0, 20.0, 20.0, PathOp::Combine), r(10.000_000_1, 10.000_000_1, 10.000_000_9, 10.000_000_9, PathOp::Subtract)]));
    assert_eq!(stored.subpaths.len(), 2, "both subpaths are kept");
    assert_eq!(stored.subpaths[1].op, PathOp::Subtract);
}
