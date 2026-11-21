//! While Edit › Fill is open, the Eyedropper is current and a click on the image picks the Foreground colour (#14654).

use craft::color_ui::ui::SampleSize;
use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::testing;
use crate::dialogs::testing::{cancel, confirm};
use crate::state::Tool;

/// 40 × 20 transparent, one pixel layer: red on the left half, blue on the right; Foreground green.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
    for (x, color) in [(0, "#ff0000"), (20, "#0000ff")] {
        app.run("select.rect", json!({"x": x, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run(COMMAND, json!({"contents": "color", "color": color})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
    crate::tool_memory::select(&mut app, Tool::Brush);
    app
}

fn foreground(app: &PhotocraftApp) -> [f32; 3] {
    let [r, g, b, _] = app.session.tools.foreground;
    [r, g, b]
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.5 / 255.0)
}

/// A click on the canvas at document point `at`. The dialog opens centred over the canvas and takes the presses that
/// land on it, so the view first shows a point the click can reach.
fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    testing::clear_of_dialogs(app, at);
    testing::click(app, at, mods);
}

fn is_open(app: &PhotocraftApp, id: DialogId) -> bool {
    app.framework.dialogs.stack().top() == Some(id)
}

fn shown(app: &PhotocraftApp) -> Tool {
    crate::dialogs::canvas_input::shown_tool(app)
}

/// The Eyedropper shows as current while Fill is open; a click picks the Foreground colour and leaves the dialog open,
/// the Brush paints nothing, and OK fills with the picked colour and shows the Brush again.
#[test]
fn a_click_picks_the_foreground_and_ok_fills_with_it() {
    let mut app = app();
    let id = open(&mut app);
    assert_eq!(shown(&app), Tool::Eyedropper, "the Eyedropper shows as current while Fill is open");
    let revision = app.session.active().unwrap().revision;

    click(&mut app, [5.5, 10.5], Modifiers::NONE);
    assert!(near(foreground(&app), [1.0, 0.0, 0.0]), "the click picks the red: {:?}", foreground(&app));
    assert!(is_open(&app, id), "the dialog stays open");
    assert_eq!(app.session.active().unwrap().revision, revision, "the click edits nothing");

    // Alt-click was not measured, so it keeps today's behaviour: nothing is picked.
    click(&mut app, [30.5, 10.5], Modifiers::ALT);
    assert!(near(foreground(&app), [1.0, 0.0, 0.0]), "Alt-click changed the colours: {:?}", foreground(&app));

    app.run("select.rect", json!({"x": 25, "y": 5, "width": 5, "height": 5})).unwrap();
    confirm(&mut app, id).unwrap();
    let st = app.session.active().unwrap();
    let filled = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(27, 7);
    assert!(near([filled[0], filled[1], filled[2]], [1.0, 0.0, 0.0]), "OK fills with the picked colour: {filled:?}");
    assert_eq!(shown(&app), Tool::Brush, "after OK the Brush shows again");
}

/// Agents: `ui.pointer` on the image while Fill is open picks the Foreground colour, as a click does, and the selected
/// Brush paints nothing.
#[test]
fn ui_pointer_picks_the_foreground_while_fill_is_open() {
    let mut app = app();
    let id = open(&mut app);
    let revision = app.session.active().unwrap().revision;
    testing::clear_of_dialogs(&mut app, [30.5, 10.5]);
    let ctx = app.ctx().clone();
    let events = json!([{"kind": "down", "x": 30.5, "y": 10.5}, {"kind": "up", "x": 30.5, "y": 10.5}]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"events": events}));
    let _ = crate::control::handle(&mut app, &ctx, &req);
    for _ in 0..4 {
        testing::frame(&mut app);
    }
    assert!(near(foreground(&app), [0.0, 0.0, 1.0]), "ui.pointer picks the blue: {:?}", foreground(&app));
    assert_eq!(app.session.active().unwrap().revision, revision, "the Brush must not paint");
    cancel(&mut app, id).unwrap();
}

/// The pick uses the Eyedropper's Sample Size: an average across the red and blue edge mixes them, the point does not.
#[test]
fn the_pick_follows_the_eyedroppers_sample_size() {
    let mut app = app();
    let id = open(&mut app);
    click(&mut app, [19.5, 10.5], Modifiers::NONE);
    let point = foreground(&app);
    assert!(point[0] > 0.99 && point[2] < 0.01, "Point Sample takes the one red pixel: {point:?}");

    app.ui.tool_options.eyedropper.size = SampleSize::Avg5;
    click(&mut app, [19.5, 10.5], Modifiers::NONE);
    let average = foreground(&app);
    assert!(average[0] < point[0] && average[2] > point[2], "5 by 5 Average mixes in the blue: {average:?}");
    cancel(&mut app, id).unwrap();
}

/// After Cancel the tool that was current shows again, and the picked colour stays.
#[test]
fn cancel_shows_the_previous_tool_again() {
    let mut app = app();
    crate::tool_memory::select(&mut app, Tool::Lasso);
    let id = open(&mut app);
    click(&mut app, [30.5, 10.5], Modifiers::NONE);
    cancel(&mut app, id).unwrap();
    assert_eq!(shown(&app), Tool::Lasso);
    assert!(near(foreground(&app), [0.0, 0.0, 1.0]), "the picked blue stays: {:?}", foreground(&app));
}

/// Opening and cancelling Fill leaves a path being drawn with the Pen as it was: the Eyedropper only shows as current
/// while the dialog is open, and the selected tool is never switched.
#[test]
fn a_pen_path_being_drawn_survives_opening_and_cancelling_fill() {
    let mut app = app();
    crate::tool_memory::select(&mut app, Tool::Pen);
    for (x, y) in [(4.0, 4.0), (16.0, 4.0), (16.0, 16.0)] {
        testing::click(&mut app, [x, y], Modifiers::NONE);
    }
    let knots = app.ui.pen.as_ref().expect("the Pen is drawing a path").triples();
    let id = open(&mut app);
    testing::frame(&mut app);
    cancel(&mut app, id).unwrap();
    testing::frame(&mut app);
    assert_eq!(app.ui.tool, Tool::Pen);
    assert_eq!(app.ui.pen.as_ref().map(|pen| pen.triples()), Some(knots), "the open path is still on the Pen");
}
