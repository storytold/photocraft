//! #16747 (EPIC-COLOR-PICKER #16738): with a Color Picker as the top dialog, `ui.pointer`
//! `down` and `move` sample the composite at their document points into the picker's new
//! colour, as a click on the canvas does, and the tool underneath is not driven
//! (`docs/control-protocol.md`).

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::control::{ControlRequest, Reply, handle};
use crate::dialogs::canvas_input::{self, DialogPick};
use crate::state::Tool;

/// The document points the pixel checks watch: the gesture's points, the line between them,
/// the cyan patch's outside and the canvas corners.
const WATCHED: &[(f64, f64)] = &[(40.0, 40.0), (32.0, 32.0), (28.0, 28.0), (24.0, 24.0), (16.0, 16.0), (55.0, 9.0), (9.0, 55.0), (3.0, 3.0)];

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Reply::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

fn nothing_queued(ctx: &egui::Context) -> bool {
    craft::ui::runtime::windows::WindowHost::of(ctx).injected_idle()
}

/// `ui.pointer` as the canvas takes it: the request is handled on the app's own context, then
/// canvas passes run until the window host reports its queue idle.
fn pointer(app: &mut PhotocraftApp, params: Value) -> Value {
    let ctx = app.ctx().clone();
    crate::canvas::testing::frame(app);
    let reply = call(app, &ctx, "ui.pointer", params);
    for _ in 0..8 {
        if nothing_queued(&ctx) {
            break;
        }
        crate::canvas::testing::frame(app);
    }
    assert!(nothing_queued(&ctx), "the canvas took all the queued input: {reply}");
    reply
}

/// The `ui.inspect` `dialogs` list: the hosted dialogs in stack order, the top last.
fn dialogs(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<Value> {
    call(app, ctx, "ui.inspect", json!({}))["result"]["dialogs"].as_array().cloned().unwrap_or_default()
}

/// The colour the top dialog lists for the picker (`ui.inspect`'s `fields.color`).
fn shown_color(app: &mut PhotocraftApp, ctx: &egui::Context) -> Value {
    dialogs(app, ctx).last().cloned().unwrap_or(Value::Null)["fields"]["color"].clone()
}

/// The composite colour at every watched point: the pixel content around the gesture.
fn composites(app: &mut PhotocraftApp) -> Vec<Option<[f32; 3]>> {
    WATCHED.iter().map(|(x, y)| crate::canvas::composite_color(app, *x, *y)).collect()
}

fn history_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// A 64x64 document, every pixel orange `#ff8000` except the square x/y 12..20 cyan `#00ffff`,
/// and the Pencil selected: a gesture that drives a tool shows as a stroke.
fn pencil_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.session.execute("tools.setColors", json!({"foreground": "#ff8000"})).unwrap();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 64, "height": 64})).unwrap();
    app.session.execute("edit.fillForeground", json!({})).unwrap();
    app.session.execute("tools.setColors", json!({"foreground": "#00ffff"})).unwrap();
    app.session.execute("select.rect", json!({"x": 12, "y": 12, "width": 8, "height": 8})).unwrap();
    app.session.execute("edit.fillForeground", json!({})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    // A foreground that is neither sample colour: what the picker shows when it opens can
    // never be mistaken for a sampled colour.
    app.session.execute("tools.setColors", json!({"foreground": "#123456"})).unwrap();
    app.ui.tool = Tool::Pencil;
    app.sync_views();
    app
}

/// The app with the foreground Color Picker open as the top dialog, its canvas eyedropper armed.
fn with_picker_open() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = pencil_app();
    let ctx = app.ctx().clone();
    let opened = call(&mut app, &ctx, "ui.dialog.open", json!({"kind": "colorPicker", "target": "foreground"}));
    assert_eq!(opened["ok"], true, "{opened}");
    crate::canvas::testing::frame(&mut app);
    crate::canvas::testing::frame(&mut app);
    let top = dialogs(&mut app, &ctx).last().cloned().unwrap_or(Value::Null);
    assert_eq!(top["key"], "foreground", "the Color Picker is the top dialog: {top}");
    assert_eq!(shown_color(&mut app, &ctx), "#123456", "the picker opens on the foreground colour");
    assert_eq!(canvas_input::armed(&app), Some(DialogPick::Color), "the top picker arms its canvas eyedropper");
    (app, ctx)
}

/// The Done-when rule: `down` and `move` over the image land the colour of their own document
/// point in the picker (the last one shown), no stroke appears, the tool stays put and no
/// history state is added.
#[test]
fn pointer_down_and_move_sample_their_points_into_the_open_color_picker() {
    let (mut app, ctx) = with_picker_open();
    let pixels_before = composites(&mut app);
    let tool_before = app.ui.tool;
    let history_before = history_len(&app);
    let reply = pointer(
        &mut app,
        json!({"events": [
            {"kind": "down", "x": 40, "y": 40},
            {"kind": "move", "x": 16, "y": 16},
            {"kind": "up", "x": 16, "y": 16},
        ]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    crate::canvas::testing::frame(&mut app);
    assert_eq!(shown_color(&mut app, &ctx), "#00ffff", "the picker shows the colour of the move's point");
    assert_eq!(composites(&mut app), pixels_before, "the tool underneath is not driven: the watched pixels are identical");
    assert_eq!(app.ui.tool, tool_before, "the active tool is unchanged");
    assert_eq!(history_len(&app), history_before, "no history state is added");
}

/// A press alone samples its own point; the release at another point samples nothing (the
/// gesture ends where the picker shows the pressed point).
#[test]
fn pointer_down_samples_the_pressed_point_and_the_release_samples_nothing() {
    let (mut app, ctx) = with_picker_open();
    let reply = pointer(
        &mut app,
        json!({"events": [
            {"kind": "down", "x": 40, "y": 40},
            {"kind": "up", "x": 16, "y": 16},
        ]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    crate::canvas::testing::frame(&mut app);
    assert_eq!(shown_color(&mut app, &ctx), "#ff8000", "the picker shows the colour of the pressed point");
}

/// Off the image nothing changes: not the picker's colour, not the canvas, not the history.
#[test]
fn pointer_events_off_the_image_change_nothing_while_the_picker_is_open() {
    let (mut app, ctx) = with_picker_open();
    let pixels_before = composites(&mut app);
    let history_before = history_len(&app);
    let reply = pointer(
        &mut app,
        json!({"events": [
            {"kind": "down", "x": -4, "y": -4},
            {"kind": "move", "x": -4, "y": -4},
            {"kind": "up", "x": -4, "y": -4},
        ]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    crate::canvas::testing::frame(&mut app);
    assert_eq!(shown_color(&mut app, &ctx), "#123456", "off the image the picker keeps its colour");
    assert_eq!(composites(&mut app), pixels_before, "and the canvas changes nothing");
    assert_eq!(history_len(&app), history_before, "no history state is added");
}

/// Not measured, kept as today: the picker's pass-under-the-modal-layer registration is the
/// primary press, so a right-button event follows the unchanged queueing route and samples nothing.
#[test]
fn a_secondary_press_while_the_picker_is_open_still_samples_nothing() {
    let (mut app, ctx) = with_picker_open();
    let pixels_before = composites(&mut app);
    let reply = pointer(
        &mut app,
        json!({"button": "secondary", "events": [
            {"kind": "down", "x": 40, "y": 40},
            {"kind": "move", "x": 16, "y": 16},
            {"kind": "up", "x": 16, "y": 16},
        ]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    crate::canvas::testing::frame(&mut app);
    assert_eq!(shown_color(&mut app, &ctx), "#123456", "the right button is not the picker's press");
    assert_eq!(composites(&mut app), pixels_before, "and the canvas changes nothing");
}

/// Not measured, kept as today: a `move` before any `down` is a hover; the picker's pipette
/// samples on a press and on the drag of a held press, not on hovering.
#[test]
fn a_leading_move_while_the_picker_is_open_still_samples_nothing() {
    let (mut app, ctx) = with_picker_open();
    let pixels_before = composites(&mut app);
    let reply = pointer(
        &mut app,
        json!({"events": [
            {"kind": "move", "x": 40, "y": 40},
            {"kind": "move", "x": 16, "y": 16},
        ]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    crate::canvas::testing::frame(&mut app);
    assert_eq!(shown_color(&mut app, &ctx), "#123456", "a hover changes nothing in the picker");
    assert_eq!(composites(&mut app), pixels_before, "and nothing on the canvas");
}

/// The guard: with no picker open the same sequence keeps today's behaviour exactly - the
/// Pencil paints the line and the stroke is one history state.
#[test]
fn pointer_without_the_picker_open_still_drives_the_pencil_over_the_same_points() {
    let mut app = pencil_app();
    app.session.execute("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    let pixels_before = composites(&mut app);
    let history_before = history_len(&app);
    let reply = pointer(
        &mut app,
        json!({"events": [
            {"kind": "down", "x": 40, "y": 40},
            {"kind": "move", "x": 16, "y": 16},
            {"kind": "up", "x": 16, "y": 16},
        ]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    crate::canvas::testing::frame(&mut app);
    assert_eq!(crate::canvas::composite_color(&mut app, 28.0, 28.0), Some([1.0, 0.0, 0.0]), "the Pencil drew the same gesture");
    assert_eq!(crate::canvas::composite_color(&mut app, 3.0, 3.0), pixels_before[WATCHED.len() - 1], "pixels off the stroke keep their colour");
    assert_eq!(history_len(&app), history_before + 1, "the stroke is one history state");
}
