//! Save identity belongs to the editable document, after a successful write only.

use crate::{ExportSettings, PhotocraftApp, Services, menus};
use photocraft_engine::Session;
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            import: Some(Box::new(|name, bytes| photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))),
            export: Some(Box::new(|doc, path, _| {
                photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(|_, _| Ok(()))),
            automation_write: Some(Box::new(|_, _| Ok(()))),
            ..Default::default()
        },
    );
    app.run("file.new", json!({"width": 4, "height": 4, "name": "Untitled-1"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

fn identity(app: &PhotocraftApp) -> (String, Option<String>, u64, u64) {
    let st = app.session.active().unwrap();
    (st.doc.name.clone(), st.path.clone(), st.revision, st.saved_revision)
}

#[test]
fn new_document_psd_and_subsequent_save_as_adopt_the_written_file_name() {
    for depth in [8, 16, 32] {
        let mut app = app();
        app.run("file.new", json!({"width": 4, "height": 4, "depth": depth, "name": "Untitled-1"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let bytes = Rc::new(RefCell::new(Vec::new()));
        let written = bytes.clone();
        app.services.write = Some(Box::new(move |_, data| {
            *written.borrow_mut() = data.to_vec();
            Ok(())
        }));
        let id = app.session.active().unwrap().doc.id;
        let revision = app.session.active().unwrap().revision;
        let ctx = egui::Context::default();
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "作品/My image.PSD"})).unwrap();
        assert_eq!(identity(&app), ("My image.PSD".into(), Some("作品/My image.PSD".into()), revision, revision));
        assert_eq!(app.session.active().unwrap().doc.id, id);
        let reopened = photocraft_io::import("My image.PSD", &bytes.borrow()).unwrap();
        assert_eq!(reopened.document.size, app.session.active().unwrap().doc.size);
        assert_eq!(reopened.document.depth.bits(), depth);
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "作品/次の画像.psd"})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.name, "次の画像.psd");
        menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("作品/次の画像.psd"));
        app.open_bytes("existing.psd", &bytes.borrow()).unwrap();
        let id = app.active_doc_id().unwrap();
        assert_eq!(app.session.active().unwrap().doc.name, "existing.psd");
        app.save_as(Some("renamed existing.psd".into())).unwrap();
        assert_eq!(app.active_doc_id().unwrap(), id);
        assert_eq!(app.session.active().unwrap().doc.name, "renamed existing.psd");
    }
}

#[test]
fn failed_encode_or_write_keeps_the_previous_name_path_and_dirty_state() {
    for saved_before in [false, true] {
        let mut app = app();
        if saved_before {
            app.save_as(Some("before.psd".into())).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
        }
        let before = identity(&app);
        app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
        assert!(app.save_as(Some("after.psd".into())).is_err());
        assert_eq!(identity(&app), before);
        assert!(app.save_automation(Some("after.psd".into())).is_ok());
        app.run("layer.new.layer", json!({})).unwrap();
        let before = identity(&app);
        app.services.automation_write = Some(Box::new(|_, _| Err("denied".into())));
        assert!(app.save_automation(Some("another.psd".into())).is_err());
        assert_eq!(identity(&app), before);
        app.services.export = Some(Box::new(|_, _, _| Err("cannot encode".into())));
        assert!(app.save_as(Some("after.psd".into())).is_err());
        assert_eq!(identity(&app), before);
    }
}

#[test]
fn save_copy_and_export_keep_the_editable_document_identity() {
    let mut app = app();
    app.save_as(Some("original.psd".into())).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let before = identity(&app);
    app.write_document("copy.psd".into(), &ExportSettings::default(), true).unwrap();
    assert_eq!(identity(&app), before);
    let (dialog, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo("export.png".into()))]);
    app.services.file_dialog = Some(dialog);
    let ctx = egui::Context::default();
    crate::export_dialog::confirm(&mut app, json!({"format": "png", "scale": 100}).as_object().unwrap()).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(identity(&app), before);
    assert!(app.ui.status.starts_with("Exported export.png"), "{}", app.ui.status);
}

#[test]
fn web_download_and_automation_saves_use_the_same_identity_completion() {
    let mut app = app();
    let (dialog, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::SaveTo("Untitled-1.psd".into()))]);
    app.services.file_dialog = Some(dialog);
    app.save_as(None).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert_eq!(app.session.active().unwrap().doc.name, "Untitled-1.psd");
    app.save_automation(Some("保存/agent save.psd".into())).unwrap();
    assert_eq!(app.session.active().unwrap().doc.name, "agent save.psd");
}
