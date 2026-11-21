//! #14074: File › Package asks for the destination with the platform folder picker, titled "Choose package
//! destination" and starting in the document's folder every time. The chosen folder is packaged into; Cancel writes
//! nothing and shows no message. No form opens.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft::host::{PickKind, Picked, ScriptedFiles};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing::frame;

/// An app over `files` with a document saved (as far as the session knows) at `<home>/report.pcraft`.
fn app(files: Arc<ScriptedFiles>, home: &Path) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { files: Some(files), ..Default::default() });
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.session.active_mut().unwrap().path = Some(home.join("report.pcraft").to_string_lossy().into_owned());
    app
}

fn package(app: &mut PhotocraftApp) -> serde_json::Value {
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(app, &ctx, "file.package", json!({})).unwrap();
    // The frames that deliver the pick, then the app's poll that hands the answer back.
    frame(app);
    app.poll_file_dialog();
    frame(app);
    reply
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|it| it.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default()
}

#[test]
fn package_asks_for_a_folder_and_packages_into_it() {
    let (home, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![dest.path().to_path_buf()])]));
    let mut app = app(files.clone(), home.path());
    let reply = package(&mut app);
    assert!(reply.get("dialog").is_none() && app.framework.dialogs.open_dialogs().is_empty(), "no form opens: {reply}");
    let asked = files.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].kind, PickKind::Folder);
    assert_eq!(asked[0].title.as_deref(), Some("Choose package destination"));
    assert_eq!(asked[0].directory.as_deref(), Some(home.path()), "starts in the document's folder");
    assert!(!entries(dest.path()).is_empty(), "the package was written into the chosen folder");
    assert!(
        app.session.journal.iter().any(|(id, p)| id == "file.package" && p["dir"] == json!(dest.path().to_string_lossy())),
        "file.package ran with that dir"
    );
}

#[test]
fn the_picker_starts_in_the_documents_folder_every_time() {
    let (home, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![dest.path().to_path_buf()]), Picked::Cancelled]));
    let mut app = app(files.clone(), home.path());
    package(&mut app);
    package(&mut app);
    let asked = files.requests();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1].directory.as_deref(), Some(home.path()), "not the folder picked last time");
}

#[test]
fn cancel_writes_nothing_and_says_nothing() {
    let (home, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let files = Arc::new(ScriptedFiles::answering([Picked::Cancelled]));
    let mut app = app(files, home.path());
    package(&mut app);
    assert!(entries(dest.path()).is_empty() && entries(home.path()).is_empty(), "nothing written");
    assert!(!app.session.journal.iter().any(|(id, _)| id == "file.package"), "file.package did not run");
    assert!(!app.ui.status_error, "no message: {:?}", app.ui.status);
}
