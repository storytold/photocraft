//! The Timeline panel's layer tracks: rows, what a bar drag does to a range, and the panel's New
//! Layer for This Frame button.

use super::*;
use craft::timeline::ClipEdge;
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};

const FPS: f64 = 24.0;

/// A 10-frame timeline at 24 fps with the playhead at `at`.
fn app_at(at: u64) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("timeline.create", json!({"duration": 10, "fps": FPS})).unwrap();
    app.run("timeline.setFrame", json!({"frame": at})).unwrap();
    app
}

fn row(start: usize, end: usize) -> LayerRow {
    LayerRow { id: LayerId(7), name: "Frame 3".into(), depth: 0, start, end, ranged: true, visible: true }
}

fn frames(n: i64) -> Tick {
    Tick::from_seconds_f64(n as f64 / FPS)
}

fn moved(delta: i64) -> ClipEdit {
    ClipEdit::Move { clips: vec![ClipId(7)], delta: frames(delta), track_delta: 0, new_track: false, copy: false }
}

fn trimmed(edge: ClipEdge, delta: i64) -> ClipEdit {
    ClipEdit::Trim { clip: ClipId(7), edge, kind: craft::timeline::TrimKind::Trim, delta: frames(delta), linked: false }
}

#[test]
fn rows_run_top_to_bottom_with_each_layers_frames() {
    let mut app = app_at(3);
    app.run("layer.frameRange.newLayerForFrame", json!({})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let rows = rows(&doc.layers, 10);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].ranged && (rows[0].start, rows[0].end) == (3, 4), "the new layer is on top, over frame 3 only: {rows:?}");
    assert!(!rows[1].ranged && (rows[1].start, rows[1].end) == (0, 10), "the Background spans every frame: {rows:?}");
}

#[test]
fn a_moved_bar_keeps_its_length_inside_the_timeline() {
    let r = row(2, 5);
    assert_eq!(edited(&r, &moved(3), FPS, 10), Some((5, 8)));
    assert_eq!(edited(&r, &moved(20), FPS, 10), Some((7, 10)), "stops at the end");
    assert_eq!(edited(&r, &moved(-9), FPS, 10), Some((0, 3)), "stops at frame 0");
    assert_eq!(edited(&r, &moved(0), FPS, 10), None, "no change, no command");
}

#[test]
fn a_trimmed_bar_keeps_at_least_one_frame() {
    let r = row(2, 5);
    assert_eq!(edited(&r, &trimmed(ClipEdge::In, -1), FPS, 10), Some((1, 5)));
    assert_eq!(edited(&r, &trimmed(ClipEdge::In, 8), FPS, 10), Some((4, 5)));
    assert_eq!(edited(&r, &trimmed(ClipEdge::Out, 30), FPS, 10), Some((2, 10)));
    assert_eq!(edited(&r, &trimmed(ClipEdge::Out, -8), FPS, 10), Some((2, 3)));
}

#[test]
fn another_layers_drag_leaves_the_row_alone() {
    let other = ClipEdit::Move { clips: vec![ClipId(99)], delta: frames(2), track_delta: 0, new_track: false, copy: false };
    assert!(commands_for(&[row(2, 5)], &other, FPS, 10).is_empty());
}

#[test]
fn a_committed_drag_sets_the_layers_range() {
    let mut app = app_at(3);
    app.run("layer.frameRange.newLayerForFrame", json!({})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let rows = rows(&doc.layers, 10);
    let id = rows[0].id;
    let drag = ClipEdit::Trim { clip: ClipId(id.0), edge: ClipEdge::Out, kind: craft::timeline::TrimKind::Trim, delta: frames(3), linked: false };
    for (cmd, p) in commands_for(&rows, &drag, FPS, 10) {
        app.run(cmd, p).unwrap();
    }
    let span = app.session.active().unwrap().doc.layer(id).unwrap().span.unwrap();
    assert_eq!((span.start, span.end), (3, 7));
}

#[test]
fn the_panels_button_adds_a_layer_for_the_playheads_frame() {
    let app = app_at(6);
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(1200.0, 600.0))
        .with_step_dt(0.001)
        .build_ui_state(|ui, app| crate::panels::timeline_ui::body(app, ui), app);
    h.run_steps(2);
    h.get_by_label("New Layer for This Frame").click();
    h.run_steps(2);
    let st = h.state().session.active().unwrap();
    let id = st.active_layer.unwrap();
    let span = st.doc.layer(id).unwrap().span.expect("the new layer has a range");
    assert_eq!((span.start, span.end), (6, 7));
}

/// The tracks open fitted to the shot: its whole length fits the panel, and takes most of it, so
/// a one-frame bar is wide enough to grab.
#[test]
fn the_tracks_open_with_the_whole_shot_in_view() {
    let width = 1200.0;
    let app = app_at(0);
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(width, 600.0))
        .with_step_dt(0.001)
        .build_ui_state(|ui, app| crate::panels::timeline_ui::body(app, ui), app);
    h.run_steps(2);
    let shot = h.state().ui.timeline.layers.view.points_per_second * (10.0 / FPS);
    assert!(shot <= f64::from(width), "the shot fits: {shot} points");
    assert!(shot > f64::from(width) / 2.0, "and fills most of the panel: {shot} points");
}
