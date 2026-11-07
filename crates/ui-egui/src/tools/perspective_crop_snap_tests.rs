//! #11803: with View › Snap on, a Perspective Crop drag that starts on the pasteboard beyond a
//! canvas corner starts at that corner, and one starting within the snap distance of an edge starts
//! on it. Snap off, clicked corners, corner drags and moves are as before.

use super::*;

fn app(snap: bool) -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", serde_json::json!({"width": 800, "height": 500})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.tool = Tool::PerspectiveCrop;
    app.ui.extras.snap = snap;
    app.ui.view.snap_to.document_bounds = true;
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    pointer(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 });
    pointer(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 });
    pointer(app, ToolEvent::Up { x: to[0], y: to[1] });
}

fn click(app: &mut PhotocraftApp, p: [f64; 2]) {
    drag(app, p, p);
}

#[test]
fn a_drag_from_the_pasteboard_starts_at_the_canvas_corner() {
    let mut app = app(true);
    drag(&mut app, [-78.0, -100.0], [700.0, 450.0]);
    assert_eq!(app.ui.perspective_crop, Some([[0.0, 0.0], [700.0, 0.0], [700.0, 450.0], [0.0, 450.0]]));
}

#[test]
fn a_drag_near_an_edge_starts_on_it() {
    let mut app = app(true);
    drag(&mut app, [3.0, 4.0], [700.0, 450.0]);
    assert_eq!(app.ui.perspective_crop, Some([[0.0, 0.0], [700.0, 0.0], [700.0, 450.0], [0.0, 450.0]]));
    // Well inside, nothing moves.
    let mut app = self::app(true);
    drag(&mut app, [100.0, 120.0], [700.0, 450.0]);
    assert_eq!(app.ui.perspective_crop.map(|q| q[0]), Some([100.0, 120.0]));
}

#[test]
fn with_snap_off_the_drag_is_as_before() {
    let mut app = app(false);
    drag(&mut app, [-78.0, -100.0], [700.0, 450.0]);
    assert_eq!(app.ui.perspective_crop, Some([[-78.0, -100.0], [700.0, -100.0], [700.0, 450.0], [-78.0, 450.0]]));
}

#[test]
fn clicked_corners_stay_where_they_were_clicked() {
    let mut app = app(true);
    for p in [[-50.0, -40.0], [850.0, -30.0], [820.0, 540.0], [-20.0, 520.0]] {
        click(&mut app, p);
    }
    assert_eq!(app.ui.perspective_crop, Some([[-50.0, -40.0], [850.0, -30.0], [820.0, 540.0], [-20.0, 520.0]]));
}

#[test]
fn corner_drags_and_moves_are_as_before() {
    let mut app = app(true);
    app.ui.perspective_crop = Some([[100.0, 100.0], [600.0, 100.0], [600.0, 400.0], [100.0, 400.0]]);
    drag(&mut app, [100.0, 100.0], [-60.0, -60.0]);
    assert_eq!(app.ui.perspective_crop.map(|q| q[0]), Some([-60.0, -60.0]), "a corner goes where it is dragged");
    drag(&mut app, [300.0, 250.0], [310.0, 250.0]);
    assert_eq!(app.ui.perspective_crop.map(|q| q[1]), Some([610.0, 100.0]), "a move keeps its offset");
}

#[test]
fn far_presses_and_no_document_never_panic() {
    let mut app = app(true);
    drag(&mut app, [-1.0e12, -1.0e12], [1.0e12, 1.0e12]);
    assert!(app.ui.perspective_crop.is_some_and(|q| q.iter().flatten().all(|v| v.is_finite())));
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty.ui.tool = Tool::PerspectiveCrop;
    drag(&mut empty, [-100.0, -100.0], [50.0, 50.0]);
    assert_eq!(empty.ui.perspective_crop, None);
}
