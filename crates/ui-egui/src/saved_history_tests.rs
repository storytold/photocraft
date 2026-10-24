//! File › Save As to `.pcraft` with Preferences › Performance › Save History in Document (#6053),
//! then File › Open: the steps come back and can be undone.

use crate::{PhotocraftApp, Services, menus};
use photocraft_engine::Session;
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

/// An app with three steps on its document whose writes land in the returned buffer.
fn app(save_history: bool) -> (PhotocraftApp, Rc<RefCell<Vec<u8>>>) {
    let bytes = Rc::new(RefCell::new(Vec::new()));
    let written = bytes.clone();
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            import: Some(Box::new(|name, bytes, opts| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            export: Some(Box::new(|doc, path, _| {
                photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |_, data| {
                *written.borrow_mut() = data.to_vec();
                Ok(())
            })),
            ..Default::default()
        },
    );
    app.run("prefs.set", json!({"path": "performance.saveHistoryInDocument", "value": save_history})).unwrap();
    app.run("file.new", json!({"width": 16, "height": 16, "name": "Untitled-1"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    (app, bytes)
}

#[test]
fn save_as_pcraft_then_open_brings_the_steps_back() {
    let (mut app, bytes) = app(true);
    let entries = app.session.active().unwrap().history.entries();
    menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({"path": "steps.pcraft"})).unwrap();
    let saved = bytes.borrow().clone();
    app.run("file.closeAll", json!({})).unwrap();
    app.open_bytes("steps.pcraft", &saved).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.history.entries(), entries);
    assert!(st.doc.saved_history.is_none());
    assert!(app.session.undo(), "the fill is undoable after reopening");
}

#[test]
fn with_the_preference_off_a_reopened_document_has_no_steps() {
    let (mut app, bytes) = app(false);
    menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({"path": "steps.pcraft"})).unwrap();
    let saved = bytes.borrow().clone();
    app.open_bytes("steps.pcraft", &saved).unwrap();
    assert!(!app.session.active().unwrap().history.can_undo());
}

#[test]
fn saving_as_psd_ignores_the_history() {
    let (mut app, bytes) = app(true);
    menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({"path": "steps.psd"})).unwrap();
    let saved = bytes.borrow().clone();
    app.open_bytes("steps.psd", &saved).unwrap();
    assert!(!app.session.active().unwrap().history.can_undo());
}

#[test]
fn a_background_save_gets_the_history_too() {
    // The desktop saves on a worker (#2017): its snapshot must carry the history as well.
    let (mut app, _) = app(true);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let saw = seen.clone();
    app.background_jobs = true;
    app.services.save_file = Some(std::sync::Arc::new(move |doc, _, _, _| {
        *saw.lock().unwrap() = Some(doc.saved_history.as_ref().map(|h| h.states.len()));
        Ok(Vec::new())
    }));
    menus::invoke(&mut app, &egui::Context::default(), "file.saveAs", json!({"path": "steps.pcraft"})).unwrap();
    if let Some(job) = app.jobs.last_started {
        let _ = app.session.wait_job(job);
    }
    assert_eq!(*seen.lock().unwrap(), Some(Some(2)), "New Layer and Fill go to the worker");
}

#[test]
fn save_a_copy_as_pcraft_follows_the_preference() {
    for on in [true, false] {
        let (mut app, bytes) = app(on);
        let (dialog, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo("copy.pcraft".into()))]);
        app.services.file_dialog = Some(dialog);
        menus::invoke(&mut app, &egui::Context::default(), "file.saveACopy", json!({})).unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        let saved = bytes.borrow().clone();
        assert!(!saved.is_empty(), "the copy was written");
        assert!(app.session.active().unwrap().doc.saved_history.is_none(), "the document itself carries no history");
        app.run("file.closeAll", json!({})).unwrap();
        app.open_bytes("copy.pcraft", &saved).unwrap();
        assert_eq!(app.session.active().unwrap().history.can_undo(), on, "Save History in Document {on}");
    }
}

#[test]
fn a_replayed_save_a_copy_follows_the_preference() {
    // A Save a Copy step played from an action (#4373) writes the same copy as the dialog.
    for on in [true, false] {
        let (mut app, bytes) = app(on);
        app.replay_save("file.saveACopy", &json!({"path": "copy.pcraft"})).unwrap();
        let saved = bytes.borrow().clone();
        app.run("file.closeAll", json!({})).unwrap();
        app.open_bytes("copy.pcraft", &saved).unwrap();
        assert_eq!(app.session.active().unwrap().history.can_undo(), on, "Save History in Document {on}");
    }
}
