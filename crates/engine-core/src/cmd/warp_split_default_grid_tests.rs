//! Which rule-of-thirds guides a split on the Default warp grid turns into real sections (#12495).

use super::*;

/// The 400 x 300 layer box of the issue, at (200, 150).
const BOX: [f64; 4] = [200.0, 150.0, 600.0, 450.0];

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// Runs a split command on the warp being edited, as the Free Transform warp session does.
fn split_cmd(s: &mut Session, cmd: &str, warp: Value, at: [f64; 2]) -> BezierMesh {
    let r = s.execute(cmd, json!({"warp": warp, "at": at})).unwrap();
    let out: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
    out.mesh.unwrap()
}

fn default_grid() -> Value {
    serde_json::to_value(Warp::custom(BezierMesh::identity(BOX, 1, 1), BOX)).unwrap()
}

/// The anchor columns (x along the top row) and anchor rows (y down the left column).
fn anchors(m: &BezierMesh) -> (Vec<f64>, Vec<f64>) {
    let xs = (0..m.nx()).step_by(3).map(|i| m.point(i, 0)[0]).collect();
    let ys = (0..m.ny()).step_by(3).map(|j| m.point(0, j)[1]).collect();
    (xs, ys)
}

fn assert_near(got: &[f64], want: &[f64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?}, want {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 0.01, "{what}: {got:?}, want {want:?}");
    }
}

const THIRDS_X: [f64; 4] = [200.0, 200.0 + 400.0 / 3.0, 200.0 + 800.0 / 3.0, 600.0];

// Measured in the reference application (Edit › Transform › Warp, Split buttons on the Default
// grid of a 302 x 202 smart object; the saved mesh, and dragging the ⅓ points on a split line
// moves that point alone, as a handle): a split adds only its own line. The ⅓ guides are the
// patches' handles, drawn; they never become sections.

#[test]
fn horizontal_split_of_the_default_grid_is_one_column_by_two_rows() {
    let mut s = session();
    let m = split_cmd(&mut s, "edit.transform.splitWarpHorizontally", default_grid(), [400.0, 320.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &[200.0, 600.0], "no anchor columns at the thirds");
    assert_near(&ys, &[150.0, 320.0, 450.0], "one row line at the click");
    assert!(m.is_valid());
}

#[test]
fn vertical_split_of_the_default_grid_is_two_columns_by_one_row() {
    let mut s = session();
    let m = split_cmd(&mut s, "edit.transform.splitWarpVertically", default_grid(), [300.0, 200.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &[200.0, 300.0, 600.0], "one column line at the click");
    assert_near(&ys, &[150.0, 450.0], "no anchor rows at the thirds");
    assert!(m.is_valid());
}

#[test]
fn crosswise_split_of_the_default_grid_is_two_by_two() {
    let mut s = session();
    let m = split_cmd(&mut s, "edit.transform.splitWarpCrosswise", default_grid(), [300.0, 320.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &[200.0, 300.0, 600.0], "columns");
    assert_near(&ys, &[150.0, 320.0, 450.0], "rows");
    // The handles stay where the Default grid drew its guides: a third of each new patch.
    assert!((m.point(1, 0)[0] - (200.0 + 100.0 / 3.0)).abs() < 1e-6, "{:?}", m.point(1, 0));
}

#[test]
fn a_split_of_an_already_split_grid_adds_only_its_line() {
    let mut s = session();
    let rows = split_cmd(&mut s, "edit.transform.splitWarpHorizontally", default_grid(), [400.0, 320.0]);
    let warp = serde_json::to_value(Warp::custom(rows, BOX)).unwrap();
    let m = split_cmd(&mut s, "edit.transform.splitWarpVertically", warp, [300.0, 200.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &[200.0, 300.0, 600.0], "one more column");
    assert_near(&ys, &[150.0, 320.0, 450.0], "rows are not rebuilt");
    // A 3 x 3 grid: a horizontal split keeps every section.
    let grid = serde_json::to_value(Warp::custom(BezierMesh::identity(BOX, 3, 3), BOX)).unwrap();
    let m = split_cmd(&mut s, "edit.transform.splitWarpHorizontally", grid, [400.0, 320.0]);
    let (xs, ys) = anchors(&m);
    assert_near(&xs, &THIRDS_X, "columns");
    assert_near(&ys, &[150.0, 250.0, 320.0, 350.0, 450.0], "rows");
}
