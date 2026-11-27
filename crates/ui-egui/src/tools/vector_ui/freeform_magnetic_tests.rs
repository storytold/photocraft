//! The Freeform Pen's Magnetic mode (#12693), on our own synthetic documents: with the checkbox on,
//! the trace is driven by the edge machinery the Magnetic Lasso already uses (`PcEdges` over the
//! engine's `EdgeSource`), so it lies nearer a hard synthetic edge than the same gesture without
//! Magnetic; Width widens the band that still finds the edge, Frequency lays more fastening points
//! and holds a corner, Contrast lets a soft edge go. With the checkbox off the freehand trace stays
//! where the pointer put it, and Reset Tool leaves the checkbox while restoring the four settings.
//!
//! Every comparison here is relative to our own engine on our own fixtures: the reference
//! application's pixel tables are not asserted and its default values are not copied.

use craft::draw::ui::VectorMode;
use craft::imaging::ui::select::MagneticSettings;
use egui::Modifiers;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

/// The fixtures' top edge, in document y: the painted band runs from y = 100 to y = 250.
const TOP: f64 = 100.0;

/// Paints `app`'s one layer: a white document with the grey level `tone` of every column from `x0`
/// to `x1` between y = 100 and y = 250.
fn paint_columns(app: &mut PhotocraftApp, x0: i32, x1: i32, tone: impl Fn(i32) -> f32) {
    app.session
        .edit("paint", move |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
            bg.fill_rect(Rect::new(0, 0, 800, 600), &[1.0, 1.0, 1.0, 1.0]);
            for x in x0..x1 {
                let g = tone(x);
                bg.fill_rect(Rect::new(x, 100, x + 1, 250), &[g, g, g, 1.0]);
            }
            Ok(())
        })
        .unwrap();
}

/// The hard-edge fixture: an opaque block whose left edge is at x = 100 and whose top edge is at
/// y = 100, with the Freeform Pen in Path mode at 100 % zoom.
fn app() -> Box<PhotocraftApp> {
    let mut app = blank_app();
    // The block runs from x = 100 to x = 400, so its far edge stays out of every band used here.
    paint_columns(&mut app, 100, 400, |_| 0.1);
    app
}

/// The faint-edge fixture: the same block, but only half as far from the white around it. The engine
/// reads a step's height as its edge strength, so this edge is a weak one.
fn faint_app() -> Box<PhotocraftApp> {
    let mut app = blank_app();
    paint_columns(&mut app, 100, 400, |_| 0.5);
    app
}

/// The same document with the Freeform Pen selected in Path mode, no snapping and 100 % zoom.
fn blank_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::FreeformPen;
    app.ui.tool_options.vector_mode = VectorMode::Path;
    app.ui.extras.snap = false;
    for view in &mut app.ui.views {
        view.zoom = 1.0;
        view.clear_pending();
    }
    app
}

/// How far apart two document points are.
fn gap(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The pointer moves along the document polyline `pts` in 2 px steps, the way a hand moves a mouse,
/// with the stylus reading `pressure` (1 for a mouse) and `mods` held.
fn drag(app: &mut PhotocraftApp, pts: &[[f64; 2]], pressure: f32, mods: Modifiers) {
    for w in pts.windows(2) {
        let steps = (gap(w[0], w[1]) / 2.0).ceil().max(1.0) as usize;
        for k in 1..=steps {
            let t = k as f64 / steps as f64;
            let at = [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t];
            tool_event(app, ToolEvent::Move { x: at[0], y: at[1], pressure }, mods);
        }
    }
}

/// The gesture the reference application measures (#12693, as the review of this PR measured it):
/// a click places the first point where it was pressed, the pointer then walks `pts` with the button
/// UP, and the trace is left running.
fn traced_along(app: &mut PhotocraftApp, pts: &[[f64; 2]], pressure: f32) {
    let Some(first) = pts.first() else { return };
    tool_event(app, ToolEvent::Down { x: first[0], y: first[1], pressure }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: first[0], y: first[1] }, Modifiers::NONE);
    drag(app, pts, pressure, Modifiers::NONE);
}

/// Enter, the key that ends a magnetic trace.
fn finish(app: &mut PhotocraftApp) {
    crate::canvas::testing::key(app, egui::Key::Enter, Modifiers::NONE);
}

/// The measured gesture along `pts` with the stylus reading `pressure`, then Enter.
fn stroke_with(app: &mut PhotocraftApp, pts: &[[f64; 2]], pressure: f32) {
    traced_along(app, pts, pressure);
    finish(app);
}

/// The measured gesture along `pts` with a mouse, then Enter.
fn stroke(app: &mut PhotocraftApp, pts: &[[f64; 2]]) {
    stroke_with(app, pts, 1.0);
}

/// The anchors of the Work Path the stroke committed, in document units.
fn anchors(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    let path = app.session.active().and_then(|st| st.doc.work_path.clone()).expect("the stroke makes a Work Path");
    path.subpaths.iter().flat_map(|s| s.knots.iter()).map(|k| [k.anchor.x, k.anchor.y]).collect()
}

/// How far the anchors after the press sit from the horizontal line `y`. The press's own anchor
/// stays where it was clicked: the starting point of a magnetic trace is not snapped.
fn edge_deviation(anchors: &[[f64; 2]], y: f64) -> f64 {
    anchors.iter().skip(1).map(|a| (a[1] - y).abs()).fold(0.0, f64::max)
}

/// A Magnetic drag along `pts` stopped before the release: the fastening points the trace has laid
/// down by then, in document units. These are the points the pen places by themselves, and the ones
/// the committed path is fitted from.
fn fastened(with: fn() -> Box<PhotocraftApp>, pts: &[[f64; 2]], width: f32, contrast: f32, frequency: f32) -> Vec<[f64; 2]> {
    let mut app = with();
    let o = &mut app.ui.tool_options;
    o.freeform_magnetic_on = true;
    o.freeform_magnetic.width = width;
    o.freeform_magnetic.contrast = contrast;
    o.freeform_magnetic.frequency = frequency;
    traced_along(&mut app, pts, 1.0);
    app.canvas_tools.vector.freeform.draft.points().iter().map(|p| [p.x, p.y]).collect()
}

/// A Magnetic stroke along the line 8 px above the top edge: Width `width`, the Pen Pressure cell
/// `pressure_width`, and the stylus reading `pressure` (1 for a mouse). How far the anchors after the
/// press sit from that edge.
fn traced_edge(width: f32, pressure_width: bool, pressure: f32) -> f64 {
    let mut app = app();
    let o = &mut app.ui.tool_options;
    o.freeform_magnetic_on = true;
    o.freeform_magnetic.width = width;
    o.freeform_magnetic.pressure_width = pressure_width;
    stroke_with(&mut app, &[[140.0, 92.0], [280.0, 92.0]], pressure);
    edge_deviation(&anchors(&app), TOP)
}

/// A stroke with Magnetic on and those detection settings, on `with`'s document: its anchors.
fn magnetic(with: fn() -> Box<PhotocraftApp>, pts: &[[f64; 2]], width: f32, contrast: f32, frequency: f32) -> Vec<[f64; 2]> {
    let mut app = with();
    let o = &mut app.ui.tool_options;
    o.freeform_magnetic_on = true;
    o.freeform_magnetic.width = width;
    o.freeform_magnetic.contrast = contrast;
    o.freeform_magnetic.frequency = frequency;
    stroke(&mut app, pts);
    anchors(&app)
}

/// Today's freehand gesture, held press-drag-release: what the tool does with Magnetic off.
fn held_stroke(app: &mut PhotocraftApp, pts: &[[f64; 2]]) {
    let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) else { return };
    tool_event(app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, Modifiers::NONE);
    drag(app, pts, 1.0, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: last[0], y: last[1] }, Modifiers::NONE);
}

/// A stroke with Magnetic off: the freehand anchors of today's gesture.
fn freehand(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut app = app();
    held_stroke(&mut app, pts);
    anchors(&app)
}

#[test]
fn a_magnetic_trace_hugs_the_edge_the_pointer_runs_six_pixels_above() {
    // 6 px above the rectangle's top edge, Width 10: the band reaches the edge.
    let traced = magnetic(app, &[[140.0, 94.0], [280.0, 94.0]], 10.0, 10.0, 57.0);
    let plain = freehand(&[[140.0, 94.0], [280.0, 94.0]]);
    let (snapped, straight_from_pointer) = (edge_deviation(&traced, TOP), edge_deviation(&plain, TOP));
    assert!(traced.len() >= 2, "the trace commits a path: {traced:?}");
    assert!(snapped < 2.0, "the magnetic trace sits on the edge, deviation {snapped}: {traced:?}");
    assert!(straight_from_pointer > 4.0, "the same gesture without Magnetic stays where the pointer ran, deviation {straight_from_pointer}: {plain:?}");
    assert!(snapped + 3.0 < straight_from_pointer, "magnetic deviation {snapped} against freehand {straight_from_pointer}");
}

#[test]
fn a_wider_width_widens_the_band_that_still_finds_the_edge() {
    // 15 px above the edge: outside a 10 px band, inside a 20 px one.
    let narrow = magnetic(app, &[[140.0, 85.0], [280.0, 85.0]], 10.0, 10.0, 57.0);
    let wide = magnetic(app, &[[140.0, 85.0], [280.0, 85.0]], 20.0, 10.0, 57.0);
    let (narrow, wide) = (edge_deviation(&narrow, TOP), edge_deviation(&wide, TOP));
    assert!(wide < 2.0, "Width 20 reaches an edge 15 px away, deviation {wide}");
    assert!(narrow > 8.0, "Width 10 does not, deviation {narrow}");
    assert!(wide + 5.0 < narrow, "the wider band follows the edge, the narrower one does not");
}

#[test]
fn a_higher_frequency_fastens_more_points_on_the_edge() {
    // Around the block's top-left corner, running 6 px outside the two edges. Frequency is how often
    // the trace fastens to the edge by itself: at 100 the framework's rule fastens every 8 px at this
    // zoom, at 0 only every 98 px.
    let path = [[180.0, 94.0], [94.0, 94.0], [94.0, 180.0]];
    let slow = fastened(app, &path, 20.0, 10.0, 0.0);
    let fast = fastened(app, &path, 20.0, 10.0, 100.0);
    assert!(fast.len() > 3 * slow.len(), "Frequency 100 fastens {} points, Frequency 0 only {slow:?}", fast.len());
    // Both cadences ride the block's edges rather than the pointer's line: after the press, every
    // point sits on one of the two edges.
    let off_edge = |points: &[[f64; 2]]| points.iter().skip(1).map(|p| (p[0] - 100.0).abs().min((p[1] - 100.0).abs())).fold(0.0, f64::max);
    assert!(off_edge(&fast) < 2.0, "the dense trace sits on the edges: {fast:?}");
    assert!(off_edge(&slow) < 2.0, "the sparse one does too: {slow:?}");
}

#[test]
fn a_higher_contrast_ignores_a_fainter_edge() {
    // 6 px above a half-height step, inside a 10 px band: a Contrast under the edge's strength
    // follows it, a Contrast over that strength leaves the trace on the pointer's line.
    let gentle = magnetic(faint_app, &[[140.0, 94.0], [280.0, 94.0]], 10.0, 30.0, 57.0);
    let strict = magnetic(faint_app, &[[140.0, 94.0], [280.0, 94.0]], 10.0, 70.0, 57.0);
    let (followed, ignored) = (edge_deviation(&gentle, TOP), edge_deviation(&strict, TOP));
    assert!(followed < 2.0, "Contrast 30 follows the faint edge, deviation {followed}: {gentle:?}");
    assert!(ignored > 4.0, "Contrast 70 ignores it, deviation {ignored}: {strict:?}");
    assert!(followed + 2.0 < ignored, "the direction is the one the tooltip words");
}

#[test]
fn a_stylus_narrows_the_band_only_with_pen_pressure_on() {
    // 8 px above the edge with Width 10: a mouse keeps the whole band and finds the edge. A stylus at
    // half pressure narrows it to 1 + (10 - 1) · 0.5 px, which no longer reaches — but only while the
    // bar's Pen Pressure cell is on.
    let mouse = traced_edge(10.0, false, 1.0);
    let off = traced_edge(10.0, false, 0.5);
    let on = traced_edge(10.0, true, 0.5);
    assert!(mouse < 2.0, "a mouse at Width 10 reaches an edge 8 px away, deviation {mouse}");
    assert!(off < 2.0, "with Pen Pressure off a stylus keeps the Width the bar sets, deviation {off}");
    assert!(on > 4.0, "with it on, half pressure leaves the edge out of the band, deviation {on}");
}

#[test]
fn a_magnetic_trace_ends_open_with_its_last_anchor_on_the_edge_under_the_pointer() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    stroke(&mut app, &[[140.0, 94.0], [280.0, 94.0]]);
    let path = app.session.active().and_then(|st| st.doc.work_path.clone()).expect("the stroke makes a Work Path");
    let sub = path.subpaths.first().expect("one subpath");
    assert!(!sub.closed, "a trace released away from its start ends open: {sub:?}");
    let last = sub.knots.last().expect("an end anchor");
    assert!((last.anchor.y - TOP).abs() < 2.0, "its last anchor sits on the edge, not on the pointer's line: {last:?}");
    assert!((last.anchor.x - 280.0).abs() < 2.0, "and it is the end of the trace the pointer made: {last:?}");
}

#[test]
fn escape_cancels_a_magnetic_trace_with_no_path_and_no_history_step() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    let n = app.session.active().unwrap().history.past_len();
    let pts = [[140.0, 94.0], [280.0, 94.0]];
    let Some(first) = pts.first() else { return };
    tool_event(&mut app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, Modifiers::NONE);
    drag(&mut app, &pts, 1.0, Modifiers::NONE);
    assert!(app.canvas_tools.vector.freeform.draft.is_open(), "the trace is in progress");
    crate::canvas::testing::key(&mut app, egui::Key::Escape, Modifiers::NONE);
    let st = app.session.active().unwrap();
    assert!(!app.canvas_tools.vector.freeform.draft.is_open(), "Escape drops the trace");
    assert!(st.doc.work_path.is_none(), "and leaves no path");
    assert_eq!(st.history.past_len(), n, "and makes no History step");
}

/// A right-click while a magnetic trace is in flight: no canvas menu over the trace, and the trace
/// goes on to finish as it would have.
#[test]
fn a_right_click_during_a_magnetic_trace_opens_no_menu_and_the_trace_goes_on() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]], 1.0);
    assert!(app.canvas_tools.vector.freeform.draft.is_open(), "the trace is in flight");
    crate::canvas::testing::secondary_click(&mut app, [260.0, 92.0], Modifiers::NONE);
    let opened = app.ui.canvas_tool_menu.as_ref().map(|m| m.tool);
    assert!(opened.is_none(), "the right-click opens no menu over a trace in flight: {opened:?}");
    assert!(app.canvas_tools.vector.freeform.draft.is_open(), "and the trace is still running");
    finish(&mut app);
    let points = anchors(&app);
    assert!(edge_deviation(&points, TOP) < 2.0, "the finished path still rides the edge: {points:?}");
}

/// The measured gesture: one click begins the trace, and the fastening points come from the pointer
/// moving with the button up — the release leaves the trace running.
#[test]
fn a_magnetic_trace_begins_at_a_click_and_fastens_while_the_button_is_up() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    let pts = [[140.0, 94.0], [200.0, 94.0], [260.0, 94.0]];
    let Some(first) = pts.first() else { return };
    tool_event(&mut app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, Modifiers::NONE);
    let points = app.canvas_tools.vector.freeform.draft.points().to_vec();
    assert_eq!(points.len(), 1, "the press of the click places one point");
    assert!((points[0].y - first[1]).abs() < 0.5, "the starting point is where it was clicked, not on the edge: {:?}", points[0]);
    tool_event(&mut app, ToolEvent::Up { x: first[0], y: first[1] }, Modifiers::NONE);
    assert!(app.canvas_tools.vector.freeform.draft.is_open(), "the release does not end the trace");
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "and commits nothing");
    drag(&mut app, &pts, 1.0, Modifiers::NONE);
    let lived = app.canvas_tools.vector.freeform.draft.points().to_vec();
    assert!(lived.len() > points.len(), "the pointer's moves with the button up lay fastening points: {} → {}", points.len(), lived.len());
    let on_edge = lived.iter().filter(|p| (p.y - TOP).abs() < 2.0).count();
    assert!(on_edge >= 2, "and they sit on the edge, not on the pointer's line: {lived:?}");
}

/// Enter ends the trace as one History step, leaves the path open, and puts the end anchor where the
/// pointer last was on the edge.
#[test]
fn enter_ends_a_magnetic_trace_open_as_one_history_step() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    let n = app.session.active().unwrap().history.past_len();
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]], 1.0);
    finish(&mut app);
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 1, "one History step for the trace");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Magnetic Pen"), "the step the reference names");
    let path = app.session.active().and_then(|st| st.doc.work_path.clone()).expect("a Work Path");
    let sub = path.subpaths.first().expect("one subpath");
    assert!(!sub.closed, "Enter ends the path open: {sub:?}");
    let last = sub.knots.last().expect("an end anchor");
    assert!((last.anchor.x - 260.0).abs() < 2.0, "its end anchor is where the pointer was: {last:?}");
    assert!((last.anchor.y - TOP).abs() < 2.0, "on the edge under it: {last:?}");
}

/// A double-click ends the trace and closes the path it made.
#[test]
fn a_double_click_ends_a_magnetic_trace_and_closes_the_path() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    let n = app.session.active().unwrap().history.past_len();
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]], 1.0);
    crate::canvas::testing::double_click(&mut app, [260.0, 92.0], Modifiers::NONE);
    let path = app.session.active().and_then(|st| st.doc.work_path.clone()).expect("the double-click commits a Work Path");
    let sub = path.subpaths.first().expect("one subpath");
    assert!(sub.closed, "and closes it: {sub:?}");
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 1, "as one History step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Magnetic Pen"), "under the name the reference gives");
    assert!(!app.canvas_tools.vector.freeform.draft.is_open(), "the trace is over");
}

/// A single click on the point the trace began with closes the path.
#[test]
fn a_magnetic_click_on_the_start_point_closes_the_path() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    let pts = [[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]];
    traced_along(&mut app, &pts, 1.0);
    let Some(first) = pts.first() else { return };
    crate::canvas::testing::click(&mut app, [first[0], first[1]], Modifiers::NONE);
    let path = app.session.active().and_then(|st| st.doc.work_path.clone()).expect("the click on the start commits a Work Path");
    let sub = path.subpaths.first().expect("one subpath");
    assert!(sub.closed, "a click on the start point closes the path: {sub:?}");
}

/// Backspace (and Delete) takes the last fastening point back, again and again, and the trace stays
/// live between the presses.
#[test]
fn backspace_takes_magnetic_fastening_points_back_one_at_a_time_and_keeps_the_trace_live() {
    let mut app = app();
    let o = &mut app.ui.tool_options;
    o.freeform_magnetic_on = true;
    // A close cadence, so the walk along the edge lays a whole row of points to take back.
    o.freeform_magnetic.frequency = 100.0;
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]], 1.0);
    let laid = app.canvas_tools.vector.freeform.draft.points().len();
    assert!(laid >= 8, "the trace fastened points by itself: {laid}");
    for want in [laid - 1, laid - 2] {
        crate::canvas::testing::key(&mut app, egui::Key::Backspace, Modifiers::NONE);
        let have = app.canvas_tools.vector.freeform.draft.points().len();
        assert_eq!(have, want, "Backspace takes one fastening point back ({laid} → {want})");
        assert!(app.canvas_tools.vector.freeform.draft.is_open(), "and the trace stays live");
        assert!(app.session.active().unwrap().doc.work_path.is_none(), "and commits nothing");
    }
    finish(&mut app);
    let points = anchors(&app);
    assert!(edge_deviation(&points, TOP) < 2.0, "the shortened trace still rides the edge: {points:?}");
}

/// A hand-edited or damaged preferences file cannot start the tool with a value its own fields
/// could never set: the remembered detection values land inside the bands the bar allows, as the
/// Magnetic Lasso's do.
#[test]
fn remembered_detection_values_past_their_ranges_load_clamped_as_the_lasso_s_do() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.prefs.edit(|p| {
        p.tool_options = json!({
            "freeform_magnetic": {"width": 5000.0, "contrast": 0.0, "frequency": 99999.0},
            "freeform_magnetic_on": true,
            "magnetic": {"width": 5000.0, "contrast": 0.0, "frequency": 99999.0},
        });
    });
    crate::tool_memory::restore(&mut app);
    let o = &app.ui.tool_options;
    let (width_min, width_max) = MagneticSettings::WIDTH_RANGE;
    assert_eq!(o.magnetic.width, width_max, "the Lasso's Width is clamped");
    assert_eq!(o.magnetic.contrast, 1.0, "its Contrast as well");
    assert_eq!(o.magnetic.frequency, 100.0, "and its Frequency");
    assert_eq!(o.freeform_magnetic.width, width_max, "the pen's Width lands in the same band: {:?}", o.freeform_magnetic);
    assert_eq!(o.freeform_magnetic.contrast, 1.0, "its Contrast as well: {:?}", o.freeform_magnetic);
    assert_eq!(o.freeform_magnetic.frequency, 100.0, "and its Frequency: {:?}", o.freeform_magnetic);
    assert!(o.freeform_magnetic_on, "the checkbox comes back as it was left");
    // The other end of each band clamps the same way.
    let mut low = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    low.session.prefs.edit(|p| {
        p.tool_options = json!({ "freeform_magnetic": {"width": 0.0, "contrast": -20.0, "frequency": -1.0} });
    });
    crate::tool_memory::restore(&mut low);
    let low = low.ui.tool_options.freeform_magnetic;
    assert_eq!((low.width, low.contrast, low.frequency), (width_min, 1.0, 0.0), "the low end clamps too: {low:?}");
}

/// ⌥-click inside a magnetic trace goes where the pointer clicked and joins the two ends with a
/// straight segment; a plain click fastens to the edge.
#[test]
fn an_alt_click_in_a_magnetic_trace_adds_a_straight_segment_where_it_was_clicked() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0]], 1.0);
    crate::canvas::testing::press(&mut app, [260.0, 80.0], Modifiers::ALT);
    crate::canvas::testing::release(&mut app, [260.0, 80.0], Modifiers::ALT);
    let points = app.canvas_tools.vector.freeform.draft.points().to_vec();
    let last = points.last().copied().unwrap_or_default();
    assert!((last.y - 80.0).abs() < 1.0, "the ⌥-clicked point is where it was clicked, not on the edge: {last:?}");
    let straight = app.canvas_tools.vector.freeform.draft.straight().to_vec();
    assert!(straight.contains(&(points.len() - 1)), "and its incoming segment is straight: {points:?} / {straight:?}");
}

/// ⌥-drag inside a magnetic trace draws freehand from the pointer, and letting go of both goes back
/// to tracing along the edge.
#[test]
fn an_alt_drag_in_a_magnetic_trace_draws_freehand_and_goes_back_to_magnetic() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0]], 1.0);
    let free = [[220.0, 70.0], [240.0, 70.0], [260.0, 70.0]];
    let Some(first) = free.first() else { return };
    crate::canvas::testing::press(&mut app, [first[0], first[1]], Modifiers::ALT);
    drag(&mut app, &free, 1.0, Modifiers::ALT);
    crate::canvas::testing::release(&mut app, [260.0, 70.0], Modifiers::ALT);
    let points = app.canvas_tools.vector.freeform.draft.points().to_vec();
    let off_edge = points.iter().filter(|p| (p.y - 70.0).abs() < 1.0).count();
    assert!(off_edge >= 3, "the ⌥-drag drew freehand where the pointer went: {points:?}");
    drag(&mut app, &[[260.0, 70.0], [300.0, 94.0]], 1.0, Modifiers::NONE);
    let lived = app.canvas_tools.vector.freeform.draft.points().to_vec();
    let last = lived.last().copied().unwrap_or_default();
    assert!((last.y - TOP).abs() < 2.0, "and with the button up and no ⌥ it fastens to the edge again: {last:?}");
}

/// Pen Pressure starts on for this tool, and Reset Tool turns it back on.
#[test]
fn pen_pressure_starts_on_for_the_magnetic_pen_and_reset_tool_turns_it_back_on() {
    let fresh = ToolOptions::default();
    assert!(fresh.freeform_magnetic.pressure_width, "the Freeform Pen's Magnetic starts with Pen Pressure on");
    assert!(!fresh.magnetic.pressure_width, "and the Magnetic Lasso keeps its own default");
    let mut o = ToolOptions::default();
    o.freeform_magnetic.pressure_width = false;
    crate::tools::tool_reset::reset_options(&mut o, Tool::FreeformPen);
    assert!(o.freeform_magnetic.pressure_width, "Reset Tool turns it back on");
}

/// A remembered row that names some of its fields keeps the ones it names and takes the framework's
/// own default for the ones it does not, exactly as the Lasso's row does.
#[test]
fn a_remembered_row_without_pen_pressure_keeps_the_framework_default_for_that_field() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.prefs.edit(|p| p.tool_options = json!({ "freeform_magnetic": { "width": 20.0 } }));
    crate::tool_memory::restore(&mut app);
    let o = app.ui.tool_options.freeform_magnetic;
    assert_eq!(o.width, 20.0, "the width the file remembers");
    assert_eq!(o.pressure_width, MagneticSettings::default().pressure_width, "and the framework's default for the field the file does not name: {o:?}");
    let mut bare = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    bare.session.prefs.edit(|p| p.tool_options = json!({ "tolerance": 40.0 }));
    crate::tool_memory::restore(&mut bare);
    assert!(bare.ui.tool_options.freeform_magnetic.pressure_width, "a file that never mentions the row keeps this tool's default");
}

#[test]
fn magnetic_off_leaves_the_trace_where_the_pointer_ran() {
    let mut app = app();
    assert!(!app.ui.tool_options.freeform_magnetic_on, "Magnetic is off to start");
    // With the mode off the tool keeps today's gesture exactly: press, drag, release.
    let pts = [[140.0, 94.0], [280.0, 94.0]];
    let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) else { return };
    tool_event(&mut app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, Modifiers::NONE);
    drag(&mut app, &pts, 1.0, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: last[0], y: last[1] }, Modifiers::NONE);
    let points = anchors(&app);
    assert!(edge_deviation(&points, 94.0) < 1.0, "the freehand trace keeps the pointer's line: {points:?}");
    assert!(edge_deviation(&points, TOP) > 4.0, "and does not go to the edge: {points:?}");
}

#[test]
fn the_control_channel_turns_magnetic_on_and_reports_it() {
    let mut app = app();
    let reply = crate::canvas::testing::control(&mut app, "ui.set", json!({"freeformMagnetic": true}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(app.ui.tool_options.freeform_magnetic_on, "the channel's field is the bar's checkbox");
    let seen = crate::canvas::testing::control(&mut app, "ui.inspect", json!({}));
    let o = &seen["result"]["toolOptions"];
    assert_eq!(o["freeform_magnetic_on"], json!(true), "{o}");
    assert_eq!(o["freeform_magnetic"]["width"], json!(10.0), "the pen's own detection settings: {o}");
    assert_eq!(o["magnetic"]["width"], json!(10.0), "and not the lasso's: {o}");
    // The stroke it drives follows the edge, which is what the channel is there for.
    stroke(&mut app, &[[140.0, 94.0], [280.0, 94.0]]);
    assert!(edge_deviation(&anchors(&app), TOP) < 2.0, "the traced path rides the edge: {:?}", anchors(&app));
}

#[test]
fn reset_tool_keeps_the_magnetic_checkbox_and_restores_the_four_settings() {
    let mut o = crate::state::ToolOptions::default();
    let d = crate::state::ToolOptions::default();
    assert!(!o.freeform_magnetic_on, "off to start");
    o.freeform_magnetic.width = 40.0;
    o.freeform_magnetic.contrast = 80.0;
    o.freeform_magnetic.frequency = 3.0;
    o.freeform_magnetic.pressure_width = true;
    o.freeform_magnetic_on = true;
    crate::tools::tool_reset::reset_options(&mut o, Tool::FreeformPen);
    assert!(o.freeform_magnetic_on, "Reset Tool leaves the mode the user picked");
    let reset = (o.freeform_magnetic.width, o.freeform_magnetic.contrast, o.freeform_magnetic.frequency, o.freeform_magnetic.pressure_width);
    assert_eq!(
        reset,
        (d.freeform_magnetic.width, d.freeform_magnetic.contrast, d.freeform_magnetic.frequency, d.freeform_magnetic.pressure_width),
        "Reset Tool restores the four settings"
    );
    let keys = crate::tools::tool_reset::option_keys(Tool::FreeformPen);
    assert!(keys.contains(&"freeform_magnetic") && keys.contains(&"freeform_magnetic_on"), "the bar's cells are the tool's own: {keys:?}");
}

// ---------------------------------------------------------------------------------------------
// The options bar's row

/// The Freeform Pen's options bar.
fn bar() -> Harness<'static, Box<PhotocraftApp>> {
    let app = app();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// What the bar's `caption` number cell reports: greyed, and the value it shows.
fn cell(h: &Harness<'static, Box<PhotocraftApp>>, caption: &str) -> (bool, Option<f64>) {
    let node = h
        .query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(caption))
        .unwrap_or_else(|| panic!("a {caption} cell beside Curve Fit"));
    let a = node.accesskit_node();
    (a.is_disabled(), a.numeric_value())
}

/// The messages of the open alerts.
fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// Types `text` into the bar's `caption` field and Tabs out of it.
fn type_into(h: &mut Harness<'static, Box<PhotocraftApp>>, caption: &str, text: &str) {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref() == Some(caption))
        .unwrap_or_else(|| panic!("a {caption} cell"))
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// The pen's own detection value for `caption`.
fn value(h: &Harness<'static, Box<PhotocraftApp>>, caption: &str) -> f64 {
    let o = &h.state().ui.tool_options.freeform_magnetic;
    match caption {
        "Width" => f64::from(o.width),
        "Contrast" => f64::from(o.contrast),
        _ => f64::from(o.frequency),
    }
}

#[test]
fn the_bar_greys_and_empties_the_detection_cells_until_magnetic_is_on() {
    let mut h = bar();
    for caption in ["Width", "Contrast", "Frequency"] {
        let (greyed, shown) = cell(&h, caption);
        assert!(greyed, "{caption} is greyed with Magnetic off");
        assert_eq!(shown, None, "{caption} is empty with Magnetic off");
    }
    assert!(h.get_by_label("Pen Pressure").accesskit_node().is_disabled(), "Pen Pressure is greyed too");
    assert!(!h.get_by_label("Magnetic").accesskit_node().is_disabled(), "the checkbox itself is live");

    h.get_by_label("Magnetic").click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.freeform_magnetic_on, "the bar's checkbox turns Magnetic on");
    for caption in ["Width", "Contrast", "Frequency"] {
        let (greyed, shown) = cell(&h, caption);
        assert!(!greyed, "{caption} is live with Magnetic on");
        assert!(shown.is_some_and(|v| v > 0.0), "{caption} shows its value with Magnetic on");
    }
    assert!(!h.get_by_label("Pen Pressure").accesskit_node().is_disabled(), "Pen Pressure is live with Magnetic on");
}

#[test]
fn a_detection_value_past_its_range_takes_the_closest_one_with_the_range_alert() {
    for (caption, typed, kept) in [("Width", "300", 256.0), ("Width", "0", 1.0), ("Contrast", "150", 100.0), ("Frequency", "101", 100.0)] {
        let mut h = bar();
        h.get_by_label("Magnetic").click();
        h.run_steps(2);
        type_into(&mut h, caption, typed);
        assert_eq!(value(&h, caption), kept, "{caption}: {typed} clamps to {kept}");
        assert_eq!(alerts(&h).len(), 1, "{caption}: one range alert for {typed}: {:?}", alerts(&h));
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert!(alerts(&h).is_empty(), "{caption}: OK closes the alert");
    }
}

/// The bar's hint words the gesture the tool is in: today's freehand drag with Magnetic off, the
/// click-and-move trace with it on.
#[test]
fn the_bar_hint_follows_the_mode() {
    let mut h = bar();
    assert!(hint_of(&h).contains("draw freehand"), "with Magnetic off the hint is the freehand one: {:?}", hint_of(&h));
    h.get_by_label("Magnetic").click();
    h.run_steps(2);
    let hint = hint_of(&h);
    assert!(hint.contains("fastens to the edge") && hint.contains("Enter"), "with Magnetic on it words the click-and-move gesture: {hint:?}");
    assert!(!hint.contains("Release on the start point"), "and not the release that ends a held drag: {hint:?}");
}

/// The bar's hint line, picked out by the separators its clauses are joined with.
fn hint_of(h: &Harness<'static, Box<PhotocraftApp>>) -> String {
    h.query_all_by_role(Role::Label).filter_map(|n| n.accesskit_node().label()).find(|l| l.contains('·')).unwrap_or_default()
}

#[test]
fn the_magnetic_cells_are_the_freeform_pen_s_own_and_leave_the_lasso_alone() {
    let mut h = bar();
    h.get_by_label("Magnetic").click();
    h.run_steps(2);
    type_into(&mut h, "Width", "30");
    let o = &h.state().ui.tool_options;
    assert_eq!((f64::from(o.freeform_magnetic.width), f64::from(o.magnetic.width)), (30.0, 10.0), "the pen's Width is not the lasso's");
}

/// The gate as the right-click actually arrives over a trace: the primary button held, one fastening
/// already laid by itself, and the right button pressed over the canvas. A fastening leaves the draft
/// between its clicks, so a gate on the held press is off by then and the Pen's path menu opens over
/// the trace.
#[test]
fn a_right_click_in_a_held_magnetic_drag_opens_no_menu_once_a_point_has_fastened() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    let first = [140.0, 92.0];
    tool_event(&mut app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, Modifiers::NONE);
    drag(&mut app, &[[170.0, 92.0], [200.0, 92.0], [230.0, 92.0]], 1.0, Modifiers::NONE);
    let laid = app.canvas_tools.vector.freeform.draft.points().len();
    assert!(laid > 1, "the held drag lays fastening points of its own: {laid}");
    crate::canvas::testing::secondary_click(&mut app, [230.0, 92.0], Modifiers::NONE);
    let opened = app.ui.canvas_tool_menu.as_ref().map(|m| m.tool);
    assert!(opened.is_none(), "the right-click opens no menu over a trace that has fastened: {opened:?}");
    assert!(app.canvas_tools.vector.freeform.draft.is_open(), "and the trace goes on");
    tool_event(&mut app, ToolEvent::Up { x: 230.0, y: 92.0 }, Modifiers::NONE);
    finish(&mut app);
    let points = anchors(&app);
    assert!(edge_deviation(&points, TOP) < 2.0, "and it ends as a trace along the edge: {points:?}");
}

/// Turning the checkbox on under a trace that began freehand must not start the edge walk inside it.
/// The draft follows the rules it began with, and a fastening put into a draft that does not take one
/// closes the outline there and then, which commits a Work Path in the middle of a held gesture.
#[test]
fn turning_magnetic_on_under_an_open_freehand_trace_lays_no_fastening_and_commits_nothing() {
    let mut app = app();
    let first = [140.0, 92.0];
    tool_event(&mut app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, Modifiers::NONE);
    drag(&mut app, &[[170.0, 92.0], [200.0, 92.0]], 1.0, Modifiers::NONE);
    crate::canvas::testing::control(&mut app, "ui.set", json!({"freeformMagnetic": true}));
    assert!(app.ui.tool_options.freeform_magnetic_on, "the checkbox is on from here on");
    let before = app.canvas_tools.vector.freeform.draft.points().len();
    drag(&mut app, &[[230.0, 92.0], [260.0, 92.0]], 1.0, Modifiers::NONE);
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "the toggle mid-gesture commits no path");
    let after = app.canvas_tools.vector.freeform.draft.points().len();
    assert!(after > before, "the freehand trace goes on taking the pointer's own points: {before} → {after}");
    let Some(last) = app.canvas_tools.vector.freeform.draft.points().last().copied() else { panic!("the trace has points") };
    assert!((last.y - 92.0).abs() < 2.0, "and its last point is the pointer's, not the edge's: {last:?}");
    tool_event(&mut app, ToolEvent::Up { x: 260.0, y: 92.0 }, Modifiers::NONE);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Freeform Pen"), "the step is named for the mode the trace began in");
}

/// Inside a running trace a plain click fastens to the edge near the pointer rather than to where the
/// pointer clicked — the point the branch in `press` exists for.
#[test]
fn a_plain_click_in_a_magnetic_trace_fastens_to_the_edge_instead_of_the_pointer() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0]], 1.0);
    let before = app.canvas_tools.vector.freeform.draft.points().to_vec();
    crate::canvas::testing::press(&mut app, [260.0, 92.0], Modifiers::NONE);
    crate::canvas::testing::release(&mut app, [260.0, 92.0], Modifiers::NONE);
    let after = app.canvas_tools.vector.freeform.draft.points().to_vec();
    assert_eq!(after.len(), before.len() + 1, "the click adds one point: {before:?} → {after:?}");
    let Some(last) = after.last().copied() else { panic!("the trace has a last point") };
    assert!((last.y - TOP).abs() < 2.0, "and it lies on the edge, not under the pointer: {last:?}");
}

/// The guard a retracted fastening needs, pinned: a pointer that stands still lays nothing back.
#[test]
fn a_standing_still_pointer_lays_nothing_back_over_a_retracted_fastening() {
    let mut app = app();
    let o = &mut app.ui.tool_options;
    o.freeform_magnetic_on = true;
    o.freeform_magnetic.frequency = 100.0;
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]], 1.0);
    let laid = app.canvas_tools.vector.freeform.draft.points().len();
    assert!(laid >= 8, "a dense trace fastens often: {laid} points");
    crate::canvas::testing::key(&mut app, egui::Key::Backspace, Modifiers::NONE);
    let taken = app.canvas_tools.vector.freeform.draft.points().len();
    assert_eq!(taken, laid - 1, "Backspace takes the last fastening back");
    for _ in 0..2 {
        crate::canvas::testing::move_to(&mut app, [260.0, 92.0], Modifiers::NONE);
    }
    assert_eq!(app.canvas_tools.vector.freeform.draft.points().len(), taken, "and a pointer that has not moved lays nothing in its place");
}

/// The mode can change under a trace without changing the name of its step: the label follows the
/// options the trace captured when it began, not the bar's state when it ends.
#[test]
fn a_trace_keeps_the_history_name_of_the_mode_it_began_in() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]], 1.0);
    crate::canvas::testing::control(&mut app, "ui.set", json!({"freeformMagnetic": false}));
    let n = app.session.active().unwrap().history.past_len();
    finish(&mut app);
    assert_eq!(app.session.active().unwrap().history.past_len(), n + 1, "the trace ends as one step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Magnetic Pen"), "under the name of the mode it ran in");
}

/// The draft follows the freehand rules again once the Magnetic trace is over: the rules are chosen
/// between traces, so they have to come back the other way as well.
#[test]
fn the_freehand_gesture_returns_after_a_magnetic_trace_ends() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    stroke(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]]);
    assert!(app.session.active().unwrap().doc.work_path.is_some(), "the magnetic trace committed a path");
    crate::canvas::testing::control(&mut app, "ui.set", json!({"freeformMagnetic": false}));
    held_stroke(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]]);
    let points = anchors(&app);
    assert!(edge_deviation(&points, TOP) > 4.0, "the next trace is the pointer's own line again: {points:?}");
}

/// What a right-click between clicks does, pinned rather than left to chance: the framework's
/// click-a-vertex rule retracts the last point, as the Polygonal Lasso's right-click does. The
/// reference measured only that no menu opens and the trace stays live; the retract is the rule the
/// draft this mode uses already carries.
#[test]
fn a_right_click_between_clicks_takes_the_last_fastening_point_back() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]], 1.0);
    let laid = app.canvas_tools.vector.freeform.draft.points().len();
    crate::canvas::testing::secondary_click(&mut app, [260.0, 92.0], Modifiers::NONE);
    assert_eq!(app.canvas_tools.vector.freeform.draft.points().len(), laid - 1, "the right-click takes the last fastening back");
    assert!(app.ui.canvas_tool_menu.as_ref().map(|m| m.tool).is_none(), "and opens no menu");
    assert!(app.canvas_tools.vector.freeform.draft.is_open(), "with the trace still live");
}

/// A Magnetic trace keeps drawing while the pointer moves with the button up, so it must not wait out
/// a tool switch and go on laying points when the pen comes back.
#[test]
fn switching_tools_drops_an_open_magnetic_trace() {
    let mut app = app();
    app.ui.tool_options.freeform_magnetic_on = true;
    traced_along(&mut app, &[[140.0, 92.0], [200.0, 92.0], [260.0, 92.0]], 1.0);
    assert!(app.canvas_tools.vector.freeform.draft.is_open(), "the trace is open between its clicks");
    app.ui.tool = Tool::Move;
    crate::canvas::testing::move_to(&mut app, [300.0, 92.0], Modifiers::NONE);
    app.ui.tool = Tool::FreeformPen;
    crate::canvas::testing::move_to(&mut app, [340.0, 92.0], Modifiers::NONE);
    assert!(!app.canvas_tools.vector.freeform.draft.is_open(), "the open trace did not come back under the pen");
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "and it left no path behind");
}
