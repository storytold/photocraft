//! The Color Replacement tool (#1045): a canvas stroke dispatches `paint.colorReplacement` with
//! the options bar's Mode, Sampling, Limits, Tolerance and Anti-alias and the foreground colour,
//! drawn live, one History step. The bar's values reaching the command are tested in
//! `tools/retouch_ui/tests.rs`.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// An 80×40 document at `depth` bits, red on the left half and blue on the right, foreground
/// green and background blue, with the Color Replacement tool picked (12 px, hard, no smoothing).
fn app(depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 80, "height": 40, "background": "#ff0000", "depth": depth})).unwrap();
    fill(&mut app, [40, 0, 40, 40], "#0000ff");
    app.run("tools.setColors", json!({"foreground": "#00ff00", "background": "#0000ff"})).unwrap();
    app.ui.tool = Tool::ColorReplacement;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0, "pressureSize": false, "smoothing": {"amount": 0.0}}})).unwrap();
    let o = &mut app.ui.tool_options;
    (o.cr_mode, o.cr_limits, o.cr_tolerance) = ("color".into(), "discontiguous".into(), 30.0);
    app
}

fn fill(app: &mut PhotocraftApp, [x, y, width, height]: [i32; 4], color: &str) {
    app.run("select.rect", json!({"x": x, "y": y, "width": width, "height": height})).unwrap();
    app.run("edit.fill", json!({"color": color})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
}

fn rgba(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn drag(app: &mut PhotocraftApp, pts: &[(f64, f64)]) {
    let m = Modifiers::NONE;
    let (x, y) = pts[0];
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    for &(x, y) in &pts[1..] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
    }
    let &(x, y) = pts.last().unwrap();
    tool_event(app, ToolEvent::Up { x, y }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// Red under the brush took the foreground's green hue (Color mode keeps its luminosity).
fn greened(c: [f32; 4]) -> bool {
    c[1] > c[0] + 0.05 && c[1] > c[2] + 0.05
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
/// From red (left half) across into blue (right half).
const STROKE: [(f64, f64); 4] = [(10.0, 20.0), (30.0, 20.0), (50.0, 20.0), (70.0, 20.0)];

#[test]
fn sampling_once_recolours_the_start_colour_and_continuous_follows_the_brush() {
    for depth in [8, 16] {
        let mut app = app(depth);
        app.ui.tool_options.cr_sampling = "once".into();
        drag(&mut app, &STROKE);
        let (id, p) = app.session.journal.last().cloned().unwrap();
        assert_eq!(id, "paint.colorReplacement");
        assert_eq!(p["sampling"], "once");
        assert!(greened(rgba(&app, 20, 20)), "{depth}-bit: red under the brush is recoloured: {:?}", rgba(&app, 20, 20));
        assert_eq!(rgba(&app, 20, 5), RED, "{depth}-bit: outside the brush");
        assert_eq!(rgba(&app, 60, 20), BLUE, "{depth}-bit: blue doesn't match the colour sampled once");

        let mut app = self::app(depth);
        app.ui.tool_options.cr_sampling = "continuous".into();
        drag(&mut app, &STROKE);
        assert!(greened(rgba(&app, 20, 20)), "{depth}-bit");
        let blue = rgba(&app, 60, 20);
        assert!(blue[1] > 0.05 && blue != BLUE, "{depth}-bit: continuous sampling recolours the blue too: {blue:?}");
    }
}

#[test]
fn background_swatch_recolours_only_the_background_colour() {
    for depth in [8, 16] {
        let mut app = app(depth);
        app.ui.tool_options.cr_sampling = "backgroundSwatch".into();
        drag(&mut app, &STROKE);
        assert_eq!(rgba(&app, 20, 20), RED, "{depth}-bit: red isn't the background colour");
        let blue = rgba(&app, 60, 20);
        assert!(blue[1] > 0.05 && blue != BLUE, "{depth}-bit: blue (the background swatch) is recoloured: {blue:?}");
    }
}

#[test]
fn contiguous_limits_skip_a_matching_island_that_discontiguous_recolours() {
    for (limits, island_recoloured) in [("contiguous", false), ("discontiguous", true)] {
        let mut app = app(8);
        fill(&mut app, [0, 0, 80, 40], "#0000ff");
        // A red patch under the brush centre, and a red island inside the brush beyond blue.
        fill(&mut app, [16, 16, 8, 8], "#ff0000");
        fill(&mut app, [27, 18, 3, 4], "#ff0000");
        app.run("tools.setBrush", json!({"brush": {"size": 30}})).unwrap();
        app.ui.tool_options.cr_limits = limits.into();
        app.ui.tool_options.cr_sampling = "once".into();
        drag(&mut app, &[(20.0, 20.0)]);
        assert!(greened(rgba(&app, 20, 20)), "{limits}: the patch under the centre");
        assert_eq!(greened(rgba(&app, 28, 20)), island_recoloured, "{limits}: the island {:?}", rgba(&app, 28, 20));
        assert_eq!(rgba(&app, 25, 20), BLUE, "{limits}: blue between them");
    }
}

/// The preview at the last move is the committed stroke: exactly at 32 bits, within one level at
/// 8 (the live stroke keeps its working pixels in the layer's depth between dabs).
#[test]
fn a_stroke_is_drawn_live_and_is_one_history_step() {
    // Color Replacement refuses 32-bit RGB documents, as the reference application does (#11923).
    for depth in [8, 16] {
        live_stroke_at(depth);
    }
}

fn live_stroke_at(depth: u32) {
    let mut app = app(depth);
    let before = rgba(&app, 20, 20);
    let steps = app.session.active().unwrap().history.past_len();
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 20.0, pressure: 1.0 }, m);
    for &(x, y) in &STROKE[1..] {
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
    }
    assert!(app.live_stroke.is_some(), "the canvas shows the stroke while dragging");
    let (shown, _) = crate::canvas::display_doc(&mut app, 0);
    let id = app.session.active().unwrap().active_layer.unwrap();
    let shown = shown.layer(id).unwrap().surface().unwrap().clone();
    assert!(greened(shown.rgba(20, 20)), "the preview recolours as it paints");
    tool_event(&mut app, ToolEvent::Up { x: 70.0, y: 20.0 }, m);
    assert!(app.live_stroke.is_none());
    // The preview at the last move is the committed stroke.
    // 8 bits may round a level or two apart between the live working copy and the commit.
    let level = if depth == 8 { 2.0 / 255.0 + 1e-6 } else { 0.0 };
    for y in 0..40 {
        for x in 0..80 {
            let (got, want) = (shown.rgba(x, y), rgba(&app, x, y));
            assert!((0..4).all(|c| (got[c] - want[c]).abs() <= level), "{depth}-bit ({x}, {y}): {got:?} vs {want:?}");
        }
    }
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one History step");
    assert_eq!(st.history.entries().last().map(|e| e.to_string()).as_deref(), Some("Color Replacement Tool"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(rgba(&app, 20, 20), before, "one undo restores the pixels");
    app.run("edit.redo", json!({})).unwrap();
    assert!(greened(rgba(&app, 20, 20)), "redo reapplies the stroke");
}

/// A targeted layer mask isn't recoloured, and neither is the layer behind it.
#[test]
fn a_mask_target_is_refused_without_painting() {
    let mut app = app(8);
    app.run("layer.new.layer", json!({})).unwrap();
    fill(&mut app, [0, 0, 80, 40], "#ff0000");
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    let steps = app.session.active().unwrap().history.past_len();
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 20.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 20.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 20.0 }, m);
    assert!(app.ui.status_error && app.ui.status.contains("layer pixels only"), "{}", app.ui.status);
    assert_eq!(rgba(&app, 20, 20), RED);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}
