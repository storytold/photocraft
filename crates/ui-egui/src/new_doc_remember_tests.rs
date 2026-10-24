//! #1810: File › New starts from the last document made with the dialog (Photoshop does), with the
//! clipboard image's size still taking priority.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// Open New Document, set `fields`, press Create.
fn create(app: &mut PhotocraftApp, fields: Value) {
    let f = app.new_document_fields();
    let id = app.ui.open_dialog(DialogKind::NewDocument, f);
    let d = app.ui.dialog_mut(id).unwrap();
    for (k, v) in fields.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
    crate::dialogs::confirm(app, id).unwrap();
}

#[test]
fn new_document_starts_from_the_last_settings() {
    let mut app = app();
    // The first time: the defaults (1920 × 1080, 72 ppi).
    assert_eq!(app.new_document_fields(), crate::state::UiState::new_document_fields());
    create(
        &mut app,
        json!({"name": "Poster", "width": 640, "height": 480, "resolution": 300.0, "mode": "cmyk", "depth": 16, "background": "transparent", "__unit": "in", "__resUnit": "ppi"}),
    );
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height, d.resolution_dpi), (640, 480, 300.0));
    // The next time: those settings, but not the name.
    let f = app.new_document_fields();
    for (k, v) in [
        ("width", json!(640)),
        ("height", json!(480)),
        ("resolution", json!(300.0)),
        ("mode", json!("cmyk")),
        ("depth", json!(16)),
        ("background", json!("transparent")),
        ("__unit", json!("in")),
        ("__resUnit", json!("ppi")),
    ] {
        assert_eq!(f.get(k), Some(&v), "{k}");
    }
    assert_eq!(f["name"], "Untitled-1");
    // They live in the preferences, so they survive a restart.
    assert_eq!(app.session.prefs().dialogs["file.new"]["resolution"], json!(300.0));

    // An image on the clipboard still sets the size (the Clipboard preset); the rest is remembered.
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 123, "height": 45})).unwrap();
    app.run("edit.fill", json!({"color": "#336699"})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    let f = app.new_document_fields();
    assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(123), Some(45)));
    assert_eq!((f["mode"].as_str(), f["depth"].as_u64()), (Some("cmyk"), Some(16)));
}

#[test]
fn a_corrupt_remembered_value_is_ignored() {
    let mut app = app();
    app.session.prefs.edit(|p| p.dialogs.insert("file.new".into(), json!({"width": -5, "height": 600, "resolution": "x", "depth": 7, "mode": 3})));
    let f = app.new_document_fields();
    let defaults = crate::state::UiState::new_document_fields();
    assert_eq!(f["height"], json!(600), "a valid value is used");
    for k in ["width", "depth", "mode"] {
        assert_eq!(f.get(k), defaults.get(k), "{k}");
    }
    assert!(f.get("resolution").is_none());
    // A failed Create remembers nothing.
    let id = app.ui.open_dialog(DialogKind::NewDocument, f);
    app.ui.dialog_mut(id).unwrap().fields.insert("width".into(), json!(0));
    let _ = crate::dialogs::confirm(&mut app, id);
    assert_eq!(app.session.prefs().dialogs["file.new"]["height"], json!(600), "unchanged");
}

/// #3131: File › New › Recent lists the documents created with the dialog, newest first, with
/// every setting restored by a click, and survives a restart.
#[test]
fn created_documents_fill_the_recent_category() {
    let mut app = app();
    create(&mut app, json!({"width": 640, "height": 480, "resolution": 300.0, "mode": "cmyk", "depth": 16, "background": "transparent", "__unit": "in"}));
    create(&mut app, json!({"width": 1920, "height": 1080, "resolution": 72.0, "mode": "rgb", "depth": 8, "background": "white", "__unit": "px"}));
    let recent = app.session.presets.recent_documents.clone();
    assert_eq!(recent.iter().map(|r| (r.width, r.unit.as_str())).collect::<Vec<_>>(), [(1920, "px"), (640, "in")]);
    // The settings are kept with the preferences.
    let mut restarted = photocraft_engine::Session::new();
    restarted.load_prefs_json(&app.session.prefs_to_json()).unwrap();
    assert_eq!(restarted.presets.recent_documents, recent);
    // A recent card restores all of its settings but keeps the new document's name.
    let mut f = app.new_document_fields();
    f.insert("name".into(), json!("Next"));
    crate::new_doc_ui::apply_settings(&mut f, &recent[1]);
    let p = crate::new_doc_ui::command_params(&f);
    assert_eq!(
        (&p["width"], &p["height"], &p["mode"], &p["depth"], &p["background"], &p["name"], &f["__unit"]),
        (&json!(640), &json!(480), &json!("cmyk"), &json!(16), &json!("transparent"), &json!("Next"), &json!("in"))
    );
}

/// #5410: File › New suggests "Untitled-N" choosing the first unused index among open documents:
/// - Creating 1 then 2, closing 2 -> first unused is 2.
/// - Creating 1 then 2, closing 1 (keeping 2 open) -> first unused is 1.
#[test]
fn new_document_increments_default_name_while_untitled_documents_exist() {
    let mut app = app();
    assert_eq!(app.new_document_fields()["name"], "Untitled-1");

    // Creating Untitled-1 increments the next suggested name to Untitled-2.
    create(&mut app, json!({}));
    assert_eq!(app.session.documents()[0].doc.name, "Untitled-1");
    assert_eq!(app.new_document_fields()["name"], "Untitled-2");

    // Creating Untitled-2 increments the next suggested name to Untitled-3.
    create(&mut app, json!({}));
    assert_eq!(app.session.documents()[1].doc.name, "Untitled-2");
    assert_eq!(app.new_document_fields()["name"], "Untitled-3");

    // Scenario 1: 1 and 2 open. Close 2 -> open is [1] -> first unused is 2.
    app.session.close(1).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.documents()[0].doc.name, "Untitled-1");
    assert_eq!(app.new_document_fields()["name"], "Untitled-2");

    // Re-create 2.
    create(&mut app, json!({}));
    assert_eq!(app.session.documents()[1].doc.name, "Untitled-2");

    // Scenario 2: 1 and 2 open. Close 1 (2 remains open) -> first unused is 1.
    app.session.close(0).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.documents()[0].doc.name, "Untitled-2");
    assert_eq!(app.new_document_fields()["name"], "Untitled-1");

    // If an untitled document has been saved (e.g. Untitled-1.psd), its stem is counted.
    app.session.set_active(0);
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).name = "Untitled-1.psd".into();
    // Now Untitled-1.psd is open -> first unused is 2.
    assert_eq!(app.new_document_fields()["name"], "Untitled-2");

    // An open document with a large number (e.g. Untitled-4294967295) must not overflow or panic.
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).name = "Untitled-4294967295".into();
    assert_eq!(app.new_document_fields()["name"], "Untitled-1");

    // Closing all documents resets back to Untitled-1.
    app.session.close(0).unwrap();
    assert!(app.session.documents().is_empty());
    assert_eq!(app.new_document_fields()["name"], "Untitled-1");
}
