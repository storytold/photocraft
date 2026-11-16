//! #13799: Export Preferences › Export As Location: "Export assets to the location of the current
//! document" starts Export As's save dialog in the saved document's folder, whatever was exported
//! last; "Export assets to the last location specified" (the default) keeps today's start.

use super::*;
use egui::accesskit::{Role, Toggled};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

type Asked = Rc<RefCell<Vec<FileDialogRequest>>>;

/// A scratch folder that is removed, on success and on failure, when the guard drops.
fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// A document saved at `doc` (or unsaved), with `last` as the last export folder.
fn app_with(doc: Option<&Path>, last: &Path, location: &str) -> (PhotocraftApp, Asked) {
    let asked = Asked::default();
    let log = asked.clone();
    let services = crate::Services { file_dialog: Some(Box::new(move |request, _parent, _reply| log.borrow_mut().push(request))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 6, "name": "photo.psd"})).unwrap();
    app.session.active_mut().unwrap().path = doc.map(|p| p.to_string_lossy().into_owned());
    let last = last.to_string_lossy().into_owned();
    app.session.prefs.edit(|p| p.export.last_folder = last);
    app.run("file.export.exportPreferences", json!({"exportAsLocation": location})).unwrap();
    (app, asked)
}

/// Export As's OK, and the folder its save dialog starts in.
fn export_as_start(app: &mut PhotocraftApp, asked: &Asked) -> std::path::PathBuf {
    crate::export_dialog::confirm(app, json!({"format": "png", "scale": 100, "__export": true}).as_object().unwrap()).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    match asked.borrow().last().expect("a save dialog") {
        FileDialogRequest::Save { suggested } => Path::new(suggested).parent().unwrap().to_path_buf(),
        other => panic!("not a save dialog: {other:?}"),
    }
}

#[test]
fn the_setting_defaults_to_the_last_location_and_round_trips() {
    assert_eq!(photocraft_engine::prefs::Export::default().export_as_location, photocraft_engine::prefs::ExportAsLocation::Last);
    let older: photocraft_engine::prefs::Export = serde_json::from_value(json!({"quickExportFormat": "png"})).unwrap();
    assert_eq!(older.export_as_location, photocraft_engine::prefs::ExportAsLocation::Last, "an older preferences file");
    let mut s = photocraft_engine::Session::new();
    let r = s.execute("file.export.exportPreferences", json!({"exportAsLocation": "document"})).unwrap();
    assert_eq!(r["values"]["exportAsLocation"], "document");
    assert!(s.execute("file.export.exportPreferences", json!({"exportAsLocation": "desktop"})).is_err());
}

#[test]
fn the_document_location_starts_in_the_documents_folder_whatever_was_exported_last() {
    let (docs, last) = (scratch(), scratch());
    let (mut app, asked) = app_with(Some(&docs.path().join("photo.psd")), last.path(), "document");
    assert_eq!(export_as_start(&mut app, &asked), docs.path());
    let (mut app, asked) = app_with(Some(&docs.path().join("photo.psd")), last.path(), "last");
    assert_eq!(export_as_start(&mut app, &asked), last.path(), "the last location: as today");
}

#[test]
fn an_unsaved_document_and_other_dialogs_keep_todays_start() {
    let (docs, last) = (scratch(), scratch());
    let (mut app, asked) = app_with(None, last.path(), "document");
    assert_eq!(export_as_start(&mut app, &asked), last.path(), "unsaved: today's folder");
    // Quick Export's save dialog is not Export As: it starts where it always did.
    let (mut app, asked) = app_with(Some(&docs.path().join("photo.psd")), last.path(), "document");
    crate::export_dialog::quick_export_png(&mut app).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    match asked.borrow().last().expect("a save dialog") {
        FileDialogRequest::Save { suggested } => assert_eq!(Path::new(suggested).parent().unwrap(), last.path()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_export_page_shows_the_two_radio_buttons() {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 1000.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.export.exportPreferences", json!({})).unwrap();
    h.run_steps(8);
    h.get_by_label("Export As Location");
    let on = |h: &Harness<'_, PhotocraftApp>, l: &str| h.get_by_role_and_label(Role::RadioButton, l).accesskit_node().toggled() == Some(Toggled::True);
    let (document, last) = ("Export assets to the location of the current document", "Export assets to the last location specified");
    assert!(on(&h, last) && !on(&h, document), "last location by default");
    h.get_by_role_and_label(Role::RadioButton, document).click();
    h.run_steps(4);
    assert!(on(&h, document) && !on(&h, last));
}

/// CodeRabbit on #15086: a document folder that was deleted or moved is not where the save dialog
/// starts: Export As falls back to the last export folder. (With no last folder the shared start every
/// export flow has, beside the document, is unchanged: #10192 pins it.)
#[test]
fn a_document_folder_that_is_gone_is_not_the_start() {
    let last = scratch();
    let gone = {
        let dir = scratch();
        dir.path().to_path_buf()
    };
    assert!(!gone.exists(), "the folder is gone");
    let (mut app, asked) = app_with(Some(&gone.join("photo.psd")), last.path(), "document");
    assert_eq!(export_as_start(&mut app, &asked), last.path(), "the last export folder instead");
}
