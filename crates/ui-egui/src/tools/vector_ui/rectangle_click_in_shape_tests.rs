use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use crate::{PhotocraftApp, Services};
use craft::draw::ui::VectorMode;
use craft::draw::ui::shape_drag::CreateShapeDialog;
use serde_json::json;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.ui.tool = Tool::Rectangle;
    app.ui.extras.snap = false;
    app.sync_views();
    app
}

fn shape(app: &mut PhotocraftApp, kind: &str, radius: f64) {
    let radii = [radius; 4];
    app.run("shape.create", json!({"kind": kind, "rect": [400, 300, 200, 100], "radii": radii, "fill": "#ff0000"})).unwrap();
}

fn gesture(app: &mut PhotocraftApp, [x, y]: [f64; 2], [end_x, end_y]: [f64; 2]) {
    let mods = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: end_x, y: end_y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: end_x, y: end_y }, mods);
}

fn state(app: &PhotocraftApp) -> (usize, usize) {
    let active = app.session.active().unwrap();
    (active.doc.layer_count(), active.history.past_len())
}

fn assert_create_rectangle(app: &PhotocraftApp) {
    let dialog = app.framework.dialogs.get::<CreateShapeDialog>().expect("one Create Rectangle dialog");
    assert_eq!(dialog.options().size, (100.0, 100.0));
    assert!(!dialog.options().from_center);
}

#[test]
fn inside_the_active_live_rectangle_is_a_no_op() {
    let mut app = app();
    shape(&mut app, "rect", 0.0);
    let before = state(&app);
    let center = [500.0, 350.0];
    for end in [center, [501.0, 350.0]] {
        gesture(&mut app, center, end);
        assert!(app.framework.dialogs.is_empty());
        assert_eq!(state(&app), before);
    }
}

#[test]
fn outside_the_rectangle_and_on_the_pasteboard_open_create_rectangle() {
    for point in [[100.0, 500.0], [-20.0, 500.0]] {
        let mut app = app();
        shape(&mut app, "rect", 0.0);
        let before = state(&app);
        gesture(&mut app, point, point);
        assert_create_rectangle(&app);
        assert_eq!(state(&app), before);
    }
}

#[test]
fn a_pixel_layer_or_non_rectangle_shape_leaves_the_dialog_available() {
    let point = [500.0, 350.0];
    for other_shape in [false, true] {
        let mut app = app();
        shape(&mut app, "rect", 0.0);
        if other_shape {
            shape(&mut app, "ellipse", 0.0);
        } else {
            app.run("layer.new.layer", json!({"name": "Pixels"})).unwrap();
        }
        let before = state(&app);
        gesture(&mut app, point, point);
        assert_create_rectangle(&app);
        assert_eq!(state(&app), before);
    }
}

#[test]
fn rounded_corners_use_the_outline() {
    let mut app = app();
    // Radius 50 on the 200 x 100 box: a pill. The point below is inside the box but outside the outline, and well clear of
    // the live shape's corner handle, which would take a press at the box corner itself.
    shape(&mut app, "roundedRect", 50.0);
    let before = state(&app);
    let center = [500.0, 350.0];
    gesture(&mut app, center, center);
    assert!(app.framework.dialogs.is_empty());
    assert_eq!(state(&app), before);
    let corner = [410.0, 310.0];
    gesture(&mut app, corner, corner);
    assert_create_rectangle(&app);
}

#[test]
fn other_modes_keep_their_click_behavior() {
    let mut app = app();
    shape(&mut app, "rect", 0.0);
    let before = state(&app);
    let center = [500.0, 350.0];
    app.ui.tool_options.shape_mode = VectorMode::Path;
    gesture(&mut app, center, center);
    assert!(app.framework.dialogs.is_empty());
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    app.ui.tool_options.shape_mode = VectorMode::Pixels;
    gesture(&mut app, center, center);
    assert_create_rectangle(&app);
    assert_eq!(state(&app), before);
}

#[test]
fn more_than_one_pixel_of_travel_still_draws_a_shape() {
    let mut app = app();
    shape(&mut app, "rect", 0.0);
    let (layers, history) = state(&app);
    gesture(&mut app, [500.0, 350.0], [502.0, 351.0]);
    assert!(app.framework.dialogs.is_empty());
    assert_eq!(state(&app), (layers + 1, history + 1));
}
