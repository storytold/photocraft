//! Red Eye drag box (#12569): dragging the Red Eye tool draws the marquee's dotted box, Shift squares
//! it, Alt grows it from the press point, Space moves it, and the release runs `paint.redEye` over the
//! box as one History step; a move under 3 px stays the tool's one-click correction. The engine's
//! `rect` changes nothing outside the rectangle and answers bad values with errors, never a panic.

use craft::behavior::kurbo::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::testing::control;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const W: i32 = 120;
const H: i32 = 100;

/// A 120 x 100 document with a red disc (radius 20 px, `#f21818`) centred at (60, 50) on white
/// ground: white is no red eye to the tool and the disc is, so "what changed" follows the box.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": W, "height": H})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.run("paint.pencil", json!({"points": [[60, 50]], "size": 40, "hardness": 100, "color": "#f21818"})).unwrap();
    app.ui.tool = Tool::RedEye;
    *app
}

fn rgba(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn snapshot(app: &PhotocraftApp) -> Vec<[f32; 4]> {
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (0..H).flat_map(|y| (0..W).map(move |x| surf.rgba(x, y))).collect()
}

fn rgb_changed(a: [f32; 4], b: [f32; 4]) -> bool {
    (0..3).any(|c| (a[c] - b[c]).abs() > 1e-5)
}

/// Every pixel of the canvas that changed since `before`.
fn changed_pixels(app: &PhotocraftApp, before: &[[f32; 4]]) -> Vec<(i32, i32)> {
    let after = snapshot(app);
    after.iter().enumerate().filter(|(i, p)| rgb_changed(before[*i], **p)).map(|(i, _)| ((i as i32) % W, (i as i32) / W)).collect()
}

fn inside_box(box_: (i32, i32, i32, i32)) -> impl Fn((i32, i32)) -> bool {
    let (x, y, w, h) = box_;
    move |(px, py)| px >= x && px < x + w && py >= y && py < y + h
}

fn last_command(app: &PhotocraftApp) -> (String, Value) {
    app.session.journal.last().cloned().unwrap()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

const NONE: egui::Modifiers = egui::Modifiers::NONE;

/// A 6 px drag: the box rides under the pointer while dragging, and its release runs `paint.redEye`
/// over the box, once, in one History state, changing no pixel outside it.
#[test]
fn the_drag_shows_the_box_and_its_release_applies_only_inside_it() {
    let mut app = app();
    let past = steps(&app);
    let before = snapshot(&app);
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 60.0, pressure: 1.0 }, NONE);
    assert_eq!(app.canvas_tools.retouch.red_eye_box(), Some(Rect::new(50.0, 40.0, 70.0, 60.0)), "the box follows the pointer during the drag");
    tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 60.0 }, NONE);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.canvas_tools.retouch.red_eye_box(), None, "the box is gone after the release");
    let (cmd, params) = last_command(&app);
    assert_eq!(cmd, "paint.redEye");
    assert_eq!(params["rect"], json!([50, 40, 20, 20]), "the release runs the box once: {params}");
    assert!(params.get("x").is_none() && params.get("points").is_none(), "the box form replaces the click form: {params}");
    assert_eq!(steps(&app), past + 1, "one History state per release");
    assert_eq!(app.session.active().unwrap().history.entries().last().map(|e| e.to_string()).as_deref(), Some("Red Eye Removal Tool"));
    let changed = changed_pixels(&app, &before);
    assert!(!changed.is_empty(), "the box corrects the red inside it");
    let in_box = inside_box((50, 40, 20, 20));
    assert!(changed.iter().all(|p| in_box(*p)), "pixels outside the box changed: {changed:?}");
    let core = rgba(&app, 60, 50);
    assert!(core[0] < 0.9, "the box centre was corrected: {core:?}");
    let outside = rgba(&app, 42, 50);
    assert!(outside[0] > 0.8, "the disc outside the box is untouched: {outside:?}");
}

/// Shift during the drag makes the box a square (1 : 1), as the marquee's Shift does.
#[test]
fn shift_makes_the_box_a_square() {
    let mut app = app();
    let m = egui::Modifiers::SHIFT;
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 80.0, y: 40.0, pressure: 1.0 }, m);
    let box_ = app.canvas_tools.retouch.red_eye_box().expect("a box while dragging");
    assert_eq!(box_.width(), box_.height(), "Shift gives 1 : 1: {box_}");
    tool_event(&mut app, ToolEvent::Up { x: 80.0, y: 40.0 }, m);
    let (cmd, params) = last_command(&app);
    assert_eq!(cmd, "paint.redEye");
    let rect = params["rect"].as_array().unwrap();
    assert_eq!(rect[2], rect[3], "the square reaches the command: {params}");
}

/// Alt grows the box from the press point as its centre; the press point never moves.
#[test]
fn alt_grows_the_box_from_the_press_point_as_its_centre() {
    let mut app = app();
    let m = egui::Modifiers::ALT;
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 50.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 80.0, y: 60.0, pressure: 1.0 }, m);
    let box_ = app.canvas_tools.retouch.red_eye_box().expect("a box while dragging");
    assert_eq!(box_, Rect::new(40.0, 40.0, 80.0, 60.0), "Alt draws from the centre");
    assert_eq!((box_.x0 + box_.x1) / 2.0, 60.0, "the press point stays the centre");
    assert_eq!((box_.y0 + box_.y1) / 2.0, 50.0, "the press point stays the centre");
    tool_event(&mut app, ToolEvent::Up { x: 80.0, y: 60.0 }, m);
    assert_eq!(last_command(&app).1["rect"], json!([40, 40, 40, 20]));
}

/// Space held mid-drag moves the whole box by the pointer's step; the size and the drag stay alive.
#[test]
fn space_mid_drag_moves_the_box_and_keeps_its_size() {
    let mut app = app();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 40.0, pressure: 1.0 }, NONE);
    let first = app.canvas_tools.retouch.red_eye_box().expect("a box while dragging");
    assert_eq!(first, Rect::new(20.0, 20.0, 60.0, 40.0));
    control(&mut app, "ui.pointer", json!({"events": [{"kind": "move", "x": 70.0, "y": 50.0}], "modifiers": {"space": true}}));
    let moved = app.canvas_tools.retouch.red_eye_box().expect("the box survives the Space step");
    assert_eq!(moved, Rect::new(30.0, 30.0, 70.0, 50.0), "the box follows the pointer's step");
    assert_eq!(moved.width(), first.width());
    assert_eq!(moved.height(), first.height());
    tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 50.0 }, NONE);
    assert_eq!(last_command(&app).1["rect"], json!([30, 30, 40, 20]), "the release applies the moved box");
}

/// A release 2 px from the press is the tool's one-click correction at the press point: no `rect`,
/// same options, still one state.
#[test]
fn a_two_pixel_drag_is_still_the_one_click_correction() {
    let mut app = app();
    let before = rgba(&app, 60, 50);
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 50.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 62.0, y: 51.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Up { x: 62.0, y: 51.0 }, NONE);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let (cmd, params) = last_command(&app);
    assert_eq!(cmd, "paint.redEye");
    assert_eq!(params["x"].as_f64(), Some(60.0), "the click keeps the press point: {params}");
    assert_eq!(params["y"].as_f64(), Some(50.0), "the click keeps the press point: {params}");
    assert!(params.get("rect").is_none(), "a short drag runs no box: {params}");
    let after = rgba(&app, 60, 50);
    assert!(after[0] < before[0] - 0.1, "the click corrects: {before:?} -> {after:?}");
}

/// `paint.redEye` with `rect`: the command runs over the rectangle and changes nothing outside it,
/// as one History state.
#[test]
fn the_engine_rect_changes_only_inside_itself() {
    let mut app = app();
    let past = steps(&app);
    let before = snapshot(&app);
    app.run("paint.redEye", json!({"rect": [50, 40, 20, 20], "pupilSize": 50, "darken": 50})).unwrap();
    let changed = changed_pixels(&app, &before);
    assert!(!changed.is_empty(), "the box corrects the red inside it");
    let in_box = inside_box((50, 40, 20, 20));
    assert!(changed.iter().all(|p| in_box(*p)), "pixels outside the rect changed: {changed:?}");
    assert_eq!(steps(&app), past + 1, "one History state");
    let r = app.session.active().unwrap().history.undo_label().map(str::to_string);
    assert_eq!(r.as_deref(), Some("Red Eye Removal Tool"));
}

/// A rect that runs off the canvas edge is clipped to the document, not refused.
#[test]
fn an_edge_rect_is_clipped_to_the_document() {
    let mut app = app();
    let before = snapshot(&app);
    app.run("paint.redEye", json!({"rect": [50, 40, W - 40, H - 30]})).unwrap();
    let changed = changed_pixels(&app, &before);
    assert!(!changed.is_empty(), "the clipped box still corrects inside itself");
    let in_box = inside_box((50, 40, W - 40, H - 30));
    assert!(changed.iter().all(|p| in_box(*p)), "pixels outside the clipped rect changed: {changed:?}");
}

/// Bad `rect` values (empty, negative size, off the canvas, wrong type, huge) answer an error and
/// never a panic, and add no History state.
#[test]
fn bad_rect_values_are_errors_and_never_a_panic() {
    let mut app = app();
    let past = steps(&app);
    let bad: Vec<Value> = vec![
        json!([]),
        json!([50, 40]),
        json!([50, 40, 20]),
        json!("box"),
        json!([null, 40, 20, 20]),
        json!(["x", 40, 20, 20]),
        json!([50, 40, 0, 20]),
        json!([50, 40, -4, 20]),
        json!([50, 40, 20, -1]),
        json!([50, 40, 0.4, 0.4]),
        json!([1e12, 40, 20, 20]),
        json!([50, 40, 1e12, 20]),
        json!([-1000, -1000, 10, 10]),
        json!([500, 400, 20, 20]),
    ];
    for value in bad {
        let err = app.run("paint.redEye", json!({"rect": value.clone()})).unwrap_err();
        assert!(err.to_lowercase().contains("rect"), "{value}: {err}");
    }
    assert_eq!(steps(&app), past, "a refused rect leaves no History state");
    // `rect` is one form of the click parameters: mixing it with `x`/`y` or `points` is a bad call.
    assert!(app.run("paint.redEye", json!({"rect": [50, 40, 20, 20], "x": 60, "y": 50})).is_err());
    assert!(app.run("paint.redEye", json!({"rect": [50, 40, 20, 20], "points": [[60, 50]]})).is_err());
    assert_eq!(steps(&app), past, "a refused combination leaves no History state");
}
