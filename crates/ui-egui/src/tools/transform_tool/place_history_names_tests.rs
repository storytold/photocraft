//! #14106, #14117: committing the Free Transform box of a placed file makes the place and the
//! transform one History state that keeps the place's name, after what it made: "Place Linked
//! Smart Object", "Place Embedded Smart Object" or "Place Image". Escape takes the place back
//! and adds no state.

use super::*;

/// An app with the reference app's 400 × 300 RGB 8-bit document.
fn reference_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300, "depth": 8, "resolution": 72, "background": "white"})).unwrap();
    app.sync_views();
    app
}

/// The bytes of the reference app's 100 × 60 PNG, red, written to a temporary folder of its own
/// that is removed when the guard drops: (the guard, bytes, path).
fn reference_png() -> (tempfile::TempDir, Vec<u8>, String) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 100, "height": 60, "background": "#ff0000"})).unwrap();
    let bytes = photocraft_io::export(&s.active().unwrap().doc, "art.png", &Default::default()).unwrap().bytes;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("art.png");
    std::fs::write(&path, &bytes).unwrap();
    (dir, bytes, path.to_string_lossy().into_owned())
}

/// Place the reference PNG as File › Place Embedded / Place Linked does (`linked`, with Always
/// Create Smart Objects when Placing `smart`) and open its Free Transform box: (app, the placed
/// layer, history steps before the place, the guard of the file's folder, kept while the box is
/// open so a linked file stays readable).
fn placed_in_free_transform(linked: bool, smart: bool) -> (PhotocraftApp, LayerId, usize, tempfile::TempDir) {
    let mut app = reference_app();
    app.session.execute("prefs.set", json!({"path": "general.alwaysCreateSmartObjectsWhenPlacing", "value": smart})).unwrap();
    let (dir, bytes, path) = reference_png();
    let steps = app.session.active().unwrap().history.past_len();
    app.place_bytes("art.png", bytes, linked.then_some(path)).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    begin_placed(&mut app, &egui::Context::default()).unwrap();
    assert!(app.ui.transform.is_some(), "the placed file is in Free Transform");
    (app, layer, steps, dir)
}

/// The three kinds of place: (linked, Always Create Smart Objects, the History name).
const PLACES: [(bool, bool, &str); 4] = [
    (true, true, "Place Linked Smart Object"),
    (true, false, "Place Linked Smart Object"),
    (false, true, "Place Embedded Smart Object"),
    (false, false, "Place Image"),
];

#[test]
fn a_moved_and_committed_place_is_one_state_named_as_the_place() {
    for (linked, smart, name) in PLACES {
        let (mut app, layer, steps, _dir) = placed_in_free_transform(linked, smart);
        let t = app.ui.transform.as_mut().unwrap();
        t.quad = t.quad.map(|[x, y]| [x + 10.0, y]);
        commit(&mut app);
        assert!(app.ui.transform.is_none(), "{name}: the box ended");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), steps + 1, "{name}: the place and its move are one state");
        assert_eq!(st.history.undo_label(), Some(name), "linked {linked}, smart objects {smart}");
        let moved = photocraft_engine::transform_cmds::transform_frame(&st.doc, st.doc.layer(layer).unwrap());
        assert_eq!(moved, photocraft_geom::Rect::new(160, 120, 260, 180), "{name}: the move is in that state");
    }
}

#[test]
fn a_place_committed_unchanged_keeps_its_name() {
    for (linked, smart, name) in PLACES {
        let (mut app, _, steps, _dir) = placed_in_free_transform(linked, smart);
        commit(&mut app);
        let st = app.session.active().unwrap();
        assert_eq!((st.history.past_len(), st.history.undo_label()), (steps + 1, Some(name)), "linked {linked}, smart objects {smart}");
    }
}

#[test]
fn escape_takes_the_place_back_without_a_state() {
    for (linked, smart, name) in PLACES {
        let (mut app, layer, steps, _dir) = placed_in_free_transform(linked, smart);
        let entries = app.session.active().unwrap().history.entries();
        cancel(&mut app);
        let st = app.session.active().unwrap();
        assert!(st.doc.layer(layer).is_none(), "{name}: the placed layer is gone");
        assert_eq!(st.history.past_len(), steps, "{name}: no state added");
        assert_eq!(st.history.entries()[..], entries[..steps + 1], "{name}");
        assert!(!st.history.can_redo(), "{name}: nothing to redo");
    }
}
