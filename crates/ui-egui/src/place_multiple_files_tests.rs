//! #14107: File › Place Embedded with several files chosen at once places them one after another,
//! each with its own Free Transform box, its own layer above the previous one and its own history
//! state, as the reference application does.

use super::*;
use crate::menus;
use craft::host::{PickKind, Picked, ScriptedFiles};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

/// An app with one 40×30 document whose file dialogs answer `answers` in order (then Cancel), and
/// its file service.
fn app_answering(answers: impl IntoIterator<Item = Picked>) -> (Box<PhotocraftApp>, Arc<ScriptedFiles>) {
    let files = Arc::new(ScriptedFiles::answering(answers));
    let services = crate::Services { files: Some(files.clone()), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    (app, files)
}

/// An app with one 40×30 document whose File › Place Embedded dialog will answer `picked`.
fn app_picking(picked: Picked) -> (Box<PhotocraftApp>, Arc<ScriptedFiles>) {
    app_answering([picked])
}

/// File › Place Embedded from the menu, its dialog shown and answered at the end of the frame.
fn place_embedded(app: &mut PhotocraftApp, ctx: &egui::Context) {
    menus::invoke(app, ctx, "file.placeEmbedded", json!({})).unwrap();
    app.poll_file_dialog();
}

/// PNG bytes of a white `w`×`h` image.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "p.png", &Default::default()).unwrap().bytes
}

/// The active document's layer names, bottom to top.
fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect()
}

/// The active layer's name.
fn active_name(app: &PhotocraftApp) -> String {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().name.clone()
}

/// How many history states the active document has behind it.
fn history_steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The open Free Transform box of a placed layer, as (left, top, right, bottom) in document pixels.
fn place_box(app: &PhotocraftApp) -> [f64; 4] {
    let t = app.ui.transform.as_ref().expect("a placed file is in Free Transform");
    assert_eq!(t.made, Some(crate::state::MadeLayer::Place));
    let quad = t.session.quad();
    let xs = quad.map(|c| c.x);
    let ys = quad.map(|c| c.y);
    let fold = |v: [f64; 4], f: fn(f64, f64) -> f64| v.into_iter().reduce(f).unwrap();
    [fold(xs, f64::min), fold(ys, f64::min), fold(xs, f64::max), fold(ys, f64::max)]
}

/// Three files (20×10, 8×6 and 12×4), as the web picker hands them over: each with its contents.
fn three_contents() -> Picked {
    Picked::Contents(vec![("first.png".into(), png(20, 10)), ("second.png".into(), png(8, 6)), ("third.png".into(), png(12, 4))])
}

#[test]
fn place_embedded_asks_for_several_files_and_place_linked_for_one() {
    let ctx = egui::Context::default();
    let (mut app, files) = app_answering([Picked::Contents(vec![("one.png".into(), png(4, 4))]), Picked::Cancelled]);
    place_embedded(&mut app, &ctx);
    assert_eq!(files.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [PickKind::OpenMany]);
    // Place Linked keeps its single-file dialog (several files there were not measured).
    crate::transform_tool::commit(&mut app);
    menus::invoke(&mut app, &ctx, "file.placeLinked", json!({})).unwrap();
    app.poll_file_dialog();
    assert_eq!(files.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [PickKind::OpenMany, PickKind::Open]);
}

/// The measured sequence: the first file appears with its box, ↩ commits it as its own history
/// state, the second then appears with its own box as its own layer above the first, and Esc on it
/// removes only that file. The third still follows.
#[test]
fn several_files_are_placed_one_after_another_each_in_its_own_transform() {
    let ctx = egui::Context::default();
    let (mut app, _) = app_picking(three_contents());
    let steps = history_steps(&app);
    place_embedded(&mut app, &ctx);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    // Only the first file is placed: its box is its own size, centred on the 40×30 canvas.
    assert_eq!(names(&app), ["Background", "first"]);
    assert_eq!(place_box(&app), [10.0, 10.0, 30.0, 20.0]);
    assert_eq!(app.drop_places.len(), 2, "the other two wait their turn");
    // Frames go by while the box is open: nothing else is placed.
    app.place_next_dropped(&ctx);
    assert_eq!(names(&app), ["Background", "first"]);
    assert_eq!(app.drop_places.len(), 2);

    // ↩ commits the first file: one history state.
    crate::transform_tool::commit(&mut app);
    assert_eq!(history_steps(&app), steps + 1);
    // The next frame places the second one above it, with its own box.
    app.place_next_dropped(&ctx);
    assert_eq!(names(&app), ["Background", "first", "second"]);
    assert_eq!(active_name(&app), "second");
    assert_eq!(place_box(&app), [16.0, 12.0, 24.0, 18.0]);

    // Esc removes only the second file; the first stays placed and committed.
    crate::transform_tool::cancel(&mut app);
    assert_eq!(names(&app), ["Background", "first"]);
    assert_eq!(history_steps(&app), steps + 1);
    assert!(!app.session.active().unwrap().history.can_redo());
    // And the third is placed next, above the first.
    app.place_next_dropped(&ctx);
    assert_eq!(names(&app), ["Background", "first", "third"]);
    assert_eq!(place_box(&app), [14.0, 13.0, 26.0, 17.0]);
    crate::transform_tool::commit(&mut app);
    assert_eq!(history_steps(&app), steps + 2);
    app.place_next_dropped(&ctx);
    assert!(app.ui.transform.is_none() && app.drop_places.is_empty());

    // Each file is its own history state: one Undo takes back only the third.
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(names(&app), ["Background", "first"]);
}

/// On the desktop the dialog answers paths: each file is read when its turn comes and placed as
/// the dialog's single file is (named after the file, an embedded smart object), in the order the
/// dialog returned them. A file that can't be read is reported and the next one placed.
#[test]
fn chosen_paths_are_read_in_turn_and_a_missing_one_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (b, a) = (dir.join("b.png"), dir.join("a.png"));
    std::fs::write(&b, png(20, 10)).unwrap();
    std::fs::write(&a, png(8, 6)).unwrap();
    let missing = dir.join("gone.png");
    let paths: Vec<PathBuf> = [&b, &missing, &a].map(|p| p.to_path_buf()).to_vec();
    let ctx = egui::Context::default();
    let (mut app, _) = app_picking(Picked::Paths(paths));
    place_embedded(&mut app, &ctx);
    // The dialog's order, not name order: b first.
    assert_eq!(names(&app), ["Background", "b"]);
    assert_eq!(place_box(&app), [10.0, 10.0, 30.0, 20.0]);
    // Waiting files hold their paths, not their contents, and won't fill a selected frame.
    assert!(app.drop_places.iter().all(|(_, p)| matches!(p, DropPlace::At { source, frame: None } if matches!(**source, DropPlace::Path(_)))));
    crate::transform_tool::commit(&mut app);
    app.place_next_dropped(&ctx);
    assert_eq!(app.ui.notices.iter().filter(|n| n.tone == crate::notices::Tone::Bad && n.title.contains("gone.png")).count(), 1);
    assert_eq!(names(&app), ["Background", "b", "a"]);
    assert_eq!(place_box(&app), [16.0, 12.0, 24.0, 18.0]);
    let st = app.session.active().unwrap();
    assert!(matches!(
        &st.doc.layer(st.active_layer.unwrap()).unwrap().content,
        photocraft_doc::LayerContent::Smart(sm) if matches!(&sm.source, photocraft_doc::SmartSource::Embedded { .. })
    ));
    crate::transform_tool::commit(&mut app);
    assert!(app.drop_places.is_empty());
}

/// With a Frame layer selected, the chosen files don't fill it one over another: each is placed
/// as its own layer with its own box, and the frame stays empty.
#[test]
fn several_files_with_a_frame_selected_are_each_placed_as_their_own_layer() {
    let ctx = egui::Context::default();
    let (mut app, _) = app_picking(three_contents());
    let frame = app.run("layer.new.frame", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap()["layer"].as_u64().unwrap();
    let frame = photocraft_doc::LayerId(frame);
    assert_eq!(app.session.active().unwrap().active_layer, Some(frame), "the frame is selected");
    let steps = history_steps(&app);
    place_embedded(&mut app, &ctx);
    for (i, name) in ["first", "second", "third"].into_iter().enumerate() {
        assert_eq!(active_name(&app), name);
        place_box(&app);
        crate::transform_tool::commit(&mut app);
        assert_eq!(history_steps(&app), steps + i + 1);
        app.place_next_dropped(&ctx);
    }
    assert!(app.ui.transform.is_none() && app.drop_places.is_empty());
    assert!(names(&app).ends_with(&["first".to_string(), "second".to_string(), "third".to_string()]), "{:?}", names(&app));
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.layer(frame).unwrap().children().is_none_or(<[_]>::is_empty), "nothing filled the frame");
}

/// With Skip Transform when Placing on, all the chosen files are placed and committed at once, one
/// history state each, without Free Transform.
#[test]
fn skip_transform_when_placing_commits_every_file_at_once() {
    let ctx = egui::Context::default();
    let (mut app, _) = app_picking(three_contents());
    app.session.execute("prefs.set", json!({"path": "general.skipTransformWhenPlacing", "value": true})).unwrap();
    let steps = history_steps(&app);
    place_embedded(&mut app, &ctx);
    assert!(app.ui.transform.is_none() && app.drop_places.is_empty());
    assert_eq!(names(&app), ["Background", "first", "second", "third"]);
    assert_eq!(history_steps(&app), steps + 3);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(names(&app), ["Background", "first", "second"]);
}

/// With background jobs on, each file is decoded on a worker: the next one starts only once the
/// previous one has landed and its box has been committed.
#[test]
fn background_places_follow_one_another() {
    let ctx = egui::Context::default();
    let (mut app, _) = app_picking(Picked::Contents(vec![("first.png".into(), png(20, 10)), ("second.png".into(), png(8, 6))]));
    app.background_jobs = true;
    place_embedded(&mut app, &ctx);
    let wait = |app: &mut PhotocraftApp, done: &dyn Fn(&PhotocraftApp) -> bool| {
        let t = std::time::Instant::now();
        while !done(app) {
            assert!(t.elapsed() < std::time::Duration::from_secs(30), "not placed after 30 s");
            crate::jobs_ui::tick(app, &ctx);
            app.place_next_dropped(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    wait(&mut app, &|app| app.ui.transform.is_some());
    assert_eq!(names(&app), ["Background", "first"]);
    assert_eq!(app.drop_places.len(), 1);
    crate::transform_tool::commit(&mut app);
    wait(&mut app, &|app| app.ui.transform.is_some());
    assert_eq!(names(&app), ["Background", "first", "second"]);
    assert_eq!(place_box(&app), [16.0, 12.0, 24.0, 18.0]);
}

/// One file chosen is placed as before: at once, in Free Transform, with
/// nothing queued, and a file that can't be read is the command's error.
#[test]
fn a_single_chosen_file_is_placed_as_before() {
    let ctx = egui::Context::default();
    let (mut app, _) = app_picking(Picked::Contents(vec![("one.png".into(), png(20, 10))]));
    place_embedded(&mut app, &ctx);
    assert_eq!(names(&app), ["Background", "one"]);
    assert_eq!(place_box(&app), [10.0, 10.0, 30.0, 20.0]);
    assert!(app.drop_places.is_empty());
    let missing = std::env::temp_dir().join(format!("photocraft-place-missing-{}.png", std::process::id()));
    let (mut app, _) = app_picking(Picked::Paths(vec![missing]));
    place_embedded(&mut app, &ctx);
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("photocraft-place-missing-"), "{}", app.ui.status);
    assert_eq!(names(&app), ["Background"]);
    assert!(app.ui.transform.is_none() && app.drop_places.is_empty());
}

/// Every chosen file is placed, even past the limits that bound drops waiting for their document
/// (32 browser drops): the web's picks are already read when the dialog answers, so none is lost.
#[test]
fn every_chosen_file_is_placed_past_the_drop_queue_limit() {
    let ctx = egui::Context::default();
    let files: Vec<(String, Vec<u8>)> = (0..MAX_QUEUED_DROPS + 2).map(|i| (format!("f{i}.png"), png(2, 2))).collect();
    let (mut app, _) = app_picking(Picked::Contents(files));
    app.session.execute("prefs.set", json!({"path": "general.skipTransformWhenPlacing", "value": true})).unwrap();
    let steps = history_steps(&app);
    place_embedded(&mut app, &ctx);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.drop_places.is_empty());
    let expected: Vec<String> = std::iter::once("Background".to_string()).chain((0..MAX_QUEUED_DROPS + 2).map(|i| format!("f{i}"))).collect();
    assert_eq!(names(&app), expected);
    assert_eq!(history_steps(&app), steps + MAX_QUEUED_DROPS + 2);
}
