//! #14047: File › Open As takes one file: its dialog is single-select, and an answer with several
//! files (two names typed into the dialog's field) opens only the first. File › Open still opens
//! several.

use std::sync::Arc;

use craft::host::ScriptedFiles;
use serde_json::json;

use super::*;
use crate::{Services, menus};

fn app() -> (Box<PhotocraftApp>, Arc<ScriptedFiles>) {
    let files = Arc::new(ScriptedFiles::held());
    // Any bytes open as a small document named after the file: the test counts documents.
    let import = |name: &str, _: &[u8], _: &photocraft_io::ImportOptions| {
        use photocraft_doc::{ColorMode, Document, SampleType, Size};
        Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), Vec::new()))
    };
    let services = Services { files: Some(files.clone()), import: Some(Box::new(import)), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    (app, files)
}

/// PNG bytes of a small red document.
fn png() -> Vec<u8> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 6, "height": 5, "background": "white"})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "shot.png", &Default::default()).unwrap().bytes
}

fn documents(app: &PhotocraftApp) -> usize {
    app.session.documents().len()
}

/// Invokes `command` from the menu, answers its dialog with `picked` and lets the opens finish (they
/// run as jobs): ticks until `want` documents are open, then a while longer for any more.
fn pick(app: &mut PhotocraftApp, files: &ScriptedFiles, command: &str, picked: Picked, want: usize) -> PickKind {
    let ctx = egui::Context::default();
    menus::invoke(app, &ctx, command, json!({})).unwrap();
    let kind = files.requests().pop().expect("a request").kind;
    files.answer(picked).expect("a dialog is open");
    app.poll_file_dialog();
    for _ in 0..500 {
        crate::jobs_ui::tick(app, &ctx);
        if documents(app) >= want {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    for _ in 0..30 {
        crate::jobs_ui::tick(app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    kind
}

#[test]
fn open_as_asks_for_one_file_and_opens_only_the_first_of_several() {
    let (mut app, files) = app();
    let before = documents(&app);
    let two = Picked::Contents(vec![("noext".into(), png()), ("other.png".into(), png())]);
    assert_eq!(pick(&mut app, &files, "file.openAs", two, before + 1), PickKind::Open, "a single-select dialog");
    assert_eq!(documents(&app), before + 1, "only the first file opens");
    assert_eq!(app.session.active().unwrap().doc.name, "noext");
}

#[test]
fn open_as_with_several_paths_opens_only_the_first() {
    let (mut app, files) = app();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("first.png"), dir.path().join("second.png"));
    std::fs::write(&a, png()).unwrap();
    std::fs::write(&b, png()).unwrap();
    let before = documents(&app);
    pick(&mut app, &files, "file.openAs", Picked::Paths(vec![a, b]), before + 1);
    assert_eq!(documents(&app), before + 1, "one document");
}

#[test]
fn file_open_still_opens_several() {
    let (mut app, files) = app();
    let before = documents(&app);
    let two = Picked::Contents(vec![("one.png".into(), png()), ("two.png".into(), png())]);
    assert_eq!(pick(&mut app, &files, "file.open", two, before + 2), PickKind::OpenMany);
    assert_eq!(documents(&app), before + 2);
}
