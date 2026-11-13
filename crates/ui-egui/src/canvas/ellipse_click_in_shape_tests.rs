//! #13231: with the Ellipse tool, a click inside the active live ellipse does nothing; elsewhere it
//! opens Create Ellipse. Ellipse 1 is 400,300 to 600,400 in an 800 x 600 document.

use super::rectangle_click_in_shape_tests::{app, gesture, shape, state};
use crate::PhotocraftApp;
use crate::state::Tool;
use serde_json::json;

fn ellipse_app() -> PhotocraftApp {
    let mut app = app();
    app.ui.tool = Tool::EllipseShape;
    shape(&mut app, "ellipse", 0.0);
    app
}

fn assert_create_ellipse(app: &PhotocraftApp) {
    let [dialog] = app.ui.dialogs.as_slice() else { panic!("expected one Create Ellipse dialog") };
    assert_eq!(dialog.fields["__label"], "Create Ellipse");
}

#[test]
fn inside_the_active_live_ellipse_is_a_no_op() {
    let mut app = ellipse_app();
    let before = state(&app);
    // The centre, a click with two pixels of travel, and a point just inside the right end.
    for (at, end) in [([500.0, 350.0], [500.0, 350.0]), ([500.0, 350.0], [502.0, 350.0]), ([598.0, 350.0], [598.0, 350.0])] {
        gesture(&mut app, at, end);
        assert!(app.ui.dialogs.is_empty(), "{at:?}");
        assert_eq!(state(&app), before, "{at:?}");
    }
}

#[test]
fn outside_the_ellipse_opens_create_ellipse() {
    // The measured point, a bounding box corner outside the ellipse, and the pasteboard.
    for point in [[100.0, 500.0], [405.0, 305.0], [-20.0, 500.0]] {
        let mut app = ellipse_app();
        let before = state(&app);
        gesture(&mut app, point, point);
        assert_create_ellipse(&app);
        assert_eq!(state(&app), before, "{point:?}");
    }
}

#[test]
fn a_pixel_layer_or_another_shape_leaves_the_dialog_available() {
    let point = [500.0, 350.0];
    for other_shape in [false, true] {
        let mut app = ellipse_app();
        if other_shape {
            shape(&mut app, "rect", 0.0);
        } else {
            app.run("layer.new.layer", json!({"name": "Pixels"})).unwrap();
        }
        let before = state(&app);
        gesture(&mut app, point, point);
        assert_create_ellipse(&app);
        assert_eq!(state(&app), before);
    }
}

#[test]
fn other_modes_keep_their_click_behavior() {
    let mut app = ellipse_app();
    let before = state(&app);
    let center = [500.0, 350.0];
    app.ui.tool_options.shape_mode = "path".into();
    gesture(&mut app, center, center);
    assert!(app.ui.dialogs.is_empty());
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    app.ui.tool_options.shape_mode = "pixels".into();
    gesture(&mut app, center, center);
    assert_create_ellipse(&app);
    assert_eq!(state(&app), before);
}

#[test]
fn a_drag_inside_still_draws_an_ellipse() {
    let mut app = ellipse_app();
    let (layers, history) = state(&app);
    gesture(&mut app, [500.0, 350.0], [540.0, 380.0]);
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(state(&app), (layers + 1, history + 1));
}

#[test]
fn a_bounding_box_handle_drag_still_scales_the_ellipse() {
    let mut app = ellipse_app();
    let (layers, history) = state(&app);
    gesture(&mut app, [600.0, 400.0], [650.0, 450.0]);
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(state(&app), (layers, history + 1));
    assert_eq!(app.session.active().unwrap().history.entries().last().map(String::as_str), Some("Transform Path"));
}
