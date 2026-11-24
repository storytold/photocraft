//! #12748, #13174: the Convert Point tool hits an anchor within 5 screen pixels, as measured: at
//! 100 % a press 5 px from an anchor converts it and one 6 px away does nothing; at 200 % that is
//! 2.5 document px. The reach stays 5 screen px on every display scale. Direct Selection, the
//! Pen's ⌥ and the tool's shown handles keep the overlay's reach (`size.hit.target`, in points).

use egui::Modifiers;
use photocraft_doc::Knot;
use photocraft_doc::vector::Path;
use serde_json::json;

use super::convert_point_history_tests::drag;
use crate::PhotocraftApp;
use crate::state::Tool;

/// The measured document: 800 × 600 px with a closed corner square as the Work Path, shown at
/// `zoom` (device pixels per document pixel) on a display of `ppp` device pixels per point.
fn app(tool: Tool, zoom: f32, ppp: f32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ctx().set_pixels_per_point(ppp);
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = zoom;
    // The new view's fit-on-screen request would replace the zoom on the first pass.
    app.ui.views[0].clear_pending();
    app.ui.tool = tool;
    let path = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app
}

/// The Work Path.
fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

/// The active document's History length.
fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// Does a drag that starts `dx` document px left of the corner (100, 300) pull out its handles,
/// in one History step?
fn converts_from(app: &mut PhotocraftApp, dx: f64) -> bool {
    let n = steps(app);
    drag(app, [100.0 - dx, 300.0], [100.0 - dx, 340.0], Modifiers::NONE);
    let corner = work(app).subpaths[0].knots[0];
    if corner == Knot::corner(100.0, 300.0) {
        assert_eq!(steps(app), n, "a press that converts nothing adds no step");
        return false;
    }
    assert_eq!(steps(app), n + 1);
    true
}

#[test]
fn at_100_percent_a_press_5_px_from_an_anchor_converts_it_and_6_px_does_nothing() {
    assert!(converts_from(&mut app(Tool::ConvertPoint, 1.0, 1.0), 5.0));
    assert!(!converts_from(&mut app(Tool::ConvertPoint, 1.0, 1.0), 6.0));
}

/// #13174: the reach is 5 screen px on every display scale, not points (the overlay's 8 points are 10 px at display
/// scale 1.25 and 16 px at 2), also when the zoom isn't 100 %.
#[test]
fn the_reach_is_5_screen_pixels_on_any_display_scale() {
    for ppp in [0.75, 1.25, 1.5, 2.0] {
        for zoom in [1.0, 2.0] {
            let (inside, outside) = (5.0 / f64::from(zoom), 6.0 / f64::from(zoom));
            let at = format!("display scale {ppp}, zoom {zoom}");
            assert!(converts_from(&mut app(Tool::ConvertPoint, zoom, ppp), inside), "5 px at {at}");
            assert!(!converts_from(&mut app(Tool::ConvertPoint, zoom, ppp), outside), "6 px at {at}");
        }
    }
}

#[test]
fn at_200_percent_the_reach_is_2_5_document_pixels() {
    assert!(converts_from(&mut app(Tool::ConvertPoint, 2.0, 1.0), 2.5));
    assert!(!converts_from(&mut app(Tool::ConvertPoint, 2.0, 1.0), 3.0));
}

/// Not measured, so as the overlay has it: the converted anchor's shown handle keeps the overlay's reach in points
/// (12 px, 6 points, at display scale 2 still takes it).
#[test]
fn a_shown_handle_keeps_the_overlays_reach() {
    let mut app = app(Tool::ConvertPoint, 1.0, 2.0);
    drag(&mut app, [100.0, 300.0], [100.0, 340.0], Modifiers::NONE);
    let n = steps(&app);
    drag(&mut app, [112.0, 340.0], [140.0, 340.0], Modifiers::NONE);
    let k = work(&app).subpaths[0].knots[0];
    assert_eq!((k.out_ctrl.x, k.out_ctrl.y), (128.0, 340.0), "the handle moved with the drag");
    assert_eq!(steps(&app), n + 1);
}

/// Direct Selection keeps the overlay's reach: an anchor 12 px (6 points) away at display scale 2 is picked.
#[test]
fn direct_selection_keeps_the_overlays_reach() {
    let mut app = app(Tool::DirectSelection, 1.0, 2.0);
    super::convert_point_history_tests::click(&mut app, [88.0, 300.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 0]], "12 px at display scale 2");
}

/// The Pen's ⌥ convert keeps the overlay's reach: an anchor 12 px (6 points) away at display scale 2 converts.
#[test]
fn the_pens_alt_keeps_the_overlays_reach() {
    let mut app = app(Tool::Pen, 1.0, 2.0);
    drag(&mut app, [88.0, 300.0], [88.0, 340.0], Modifiers::ALT);
    assert!(app.ui.pen.is_none() && work(&app).subpaths[0].knots[0] != Knot::corner(100.0, 300.0), "12 px at display scale 2");
}
