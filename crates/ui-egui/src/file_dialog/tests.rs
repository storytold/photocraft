use super::*;
use crate::{Services, menus};
use std::cell::RefCell;
use std::rc::Rc;

/// The requests the fake dialog service was given, with their replies: dialogs the user is still in.
type Open = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;

/// An app with an untitled document, whose file dialogs stay open until the test answers them, and
/// whose writer records the paths it wrote.
fn app() -> (PhotocraftApp, Open, Rc<RefCell<Vec<String>>>) {
    let (open, written): (Open, Rc<RefCell<Vec<String>>>) = Default::default();
    let (held, w) = (open.clone(), written.clone());
    let services = Services {
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        file_dialog: Some(Box::new(move |request, _parent, reply| held.borrow_mut().push((request, reply)))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    (app, open, written)
}

#[test]
fn pdn_save_requests_a_native_copy_without_overwriting_the_source() {
    let (mut app, open, written) = app();
    let st = app.session.active_mut().unwrap();
    st.path = Some("/pics/layers.PDN".into());
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "/pics/layers.pcraft"));
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/layers.pcraft".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/layers.pcraft"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/layers.pcraft"));
}

/// A real flat JPEG import, optionally edited to contain another layer. A filename without a
/// path is the web/download case; desktop opens also retain their source location.
fn jpeg_app(path: Option<&str>, layered: bool) -> (PhotocraftApp, Open, Rc<RefCell<Vec<String>>>) {
    let (mut app, open, written) = app();
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "My image.JPEG", &Default::default()).unwrap().bytes;
    let doc = photocraft_io::import("My image.JPEG", &bytes).unwrap().document;
    app.session.add_document(doc, path.map(str::to_string));
    if layered {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    (app, open, written)
}

#[test]
fn layered_flat_source_save_defaults_to_psd_without_overwriting_source() {
    let ctx = egui::Context::default();
    for command in ["file.save", "file.saveAs"] {
        let (mut app, open, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
        menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(
            matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "/pics/My image.psd"),
            "{command}: {:?}",
            open.borrow().first().map(|(r, _)| r.clone())
        );
        assert!(written.borrow().is_empty(), "asking where to save must not overwrite the JPEG");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.JPEG"));
        answer(&open, Some(FileDialogAnswer::SaveTo("/pics/My image.psd".into())));
        app.poll_file_dialog(&ctx, None);
        assert_eq!(*written.borrow(), ["/pics/My image.psd"]);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.psd"));
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
    }
}

#[test]
fn layered_filename_only_save_defaults_to_psd() {
    let (mut app, open, written) = jpeg_app(None, true);
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "My image.psd"));
    answer(&open, Some(FileDialogAnswer::SaveTo("My image.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["My image.psd"]);
}

#[test]
fn save_dialog_preserves_flat_and_existing_layered_format_defaults() {
    let ctx = egui::Context::default();
    for (path, layered) in [
        ("/pics/My image.JPEG", false),
        ("/pics/My image.JP2", false),
        ("/pics/My image.j2k", false),
        ("/pics/My image.J2C", false),
        ("/pics/My image.PSD", true),
        ("/pics/My image.psb", true),
        ("/pics/My image.pcraft", true),
        ("/pics/My image.TIF", true),
        ("/pics/My image.tiff", true),
    ] {
        let (mut app, open, written) = jpeg_app(Some(path), layered);
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == path), "{path}");
        answer(&open, None);
        app.poll_file_dialog(&ctx, None);
        assert!(written.borrow().is_empty());
    }
}

#[test]
fn transparent_png_keeps_its_format_until_its_lone_layer_becomes_smart() {
    let (mut app, open, written) = app();
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).layers[0].surface_mut().unwrap().write_pixel(0, 0, &[1.0, 0.0, 0.0, 0.0]);
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "transparent.PNG", &Default::default()).unwrap().bytes;
    let doc = photocraft_io::import("transparent.PNG", &bytes).unwrap().document;
    assert_eq!(doc.layers.len(), 1);
    assert!(!doc.layers[0].locks.transparency, "transparent PNG imports as an unlocked raster layer");
    assert_eq!(doc.layers[0].surface().unwrap().pixel(0, 0)[3], 0.0);
    app.session.add_document(doc, Some("/pics/transparent.PNG".into()));
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "/pics/transparent.PNG"));
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
    assert!(matches!(app.session.active().unwrap().doc.layers[0].content, photocraft_doc::LayerContent::Smart(_)));
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "/pics/transparent.psd"));
    assert!(written.borrow().is_empty());
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
}

#[test]
fn layered_save_respects_explicit_jpeg_choices() {
    let ctx = egui::Context::default();
    let (mut app, open, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/exports/chosen.JPEG".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/exports/chosen.jpeg"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/exports/chosen.jpeg"));
    // Explicit command paths carry the same choice and bypass the picker.
    for command in ["file.save", "file.saveAs"] {
        let (mut app, open, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
        menus::invoke(&mut app, &ctx, command, json!({"path": "/exports/chosen.jpg"})).unwrap();
        assert!(!app.file_dialog_open() && open.borrow().is_empty());
        assert_eq!(*written.borrow(), ["/exports/chosen.jpg"]);
    }
}

/// The user answers the open dialog, from the dialog's own thread.
fn answer(open: &Open, answer: Option<FileDialogAnswer>) {
    let (_, reply) = open.borrow_mut().pop().expect("a dialog is open");
    std::thread::spawn(move || reply.send(answer)).join().unwrap();
}

#[test]
fn open_starts_in_the_last_used_folder() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    app.ui.recent_files = vec!["/pics/cat.psd".into(), "/old/dog.png".into()];
    menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(
        matches!(open.borrow().as_slice(), [(FileDialogRequest::Open { multiple: true, initial_dir: Some(dir), extensions: None }, _)] if dir == "/pics"),
        "{:?}",
        open.borrow().first().map(|(r, _)| r.clone())
    );
}

#[test]
fn filtered_picker_requests_only_the_given_extensions() {
    let (mut app, open, _) = app();
    app.pick_file_bytes_filtered(&["cube", "3dl", "look"], |_, _, _| Ok(Value::Null)).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert!(matches!(
        open.borrow().first().map(|(r, _)| r),
        Some(FileDialogRequest::Open { multiple: false, extensions: Some(exts), .. })
            if exts.iter().map(String::as_str).collect::<Vec<_>>() == ["cube", "3dl", "look"]
    ));
}

#[test]
fn last_used_dir_takes_the_most_recent_parent() {
    assert_eq!(last_used_dir(&[]), None);
    assert_eq!(last_used_dir(&["/pics/cat.psd".into()]), Some("/pics".into()));
    assert_eq!(last_used_dir(&["/pics/cat.psd".into(), "/old/dog.png".into()]), Some("/pics".into()));
    assert_eq!(last_used_dir(&["cat.psd".into()]), None, "no folder to start in");
    assert_eq!(last_used_dir(&["/".into()]), None, "the root has no parent");
}

#[test]
fn the_app_keeps_running_while_a_dialog_is_open() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap(), json!({"fileDialog": "save"}));
    assert!(open.borrow().is_empty(), "shown at the end of the frame, which has the window to parent it to");
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested.ends_with(".psd")));
    // Frames go on while the user is in the dialog: nothing is written, commands still run, and
    // a second dialog is refused.
    for _ in 0..3 {
        app.poll_file_dialog(&ctx, None);
    }
    assert!(written.borrow().is_empty());
    assert!(app.file_dialog_open());
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap_err(), "a file dialog is already open");
    app.poll_file_dialog(&ctx, None);
    assert_eq!(open.borrow().len(), 1);
    // The answer wakes the app, and the save goes on from where it asked.
    for _ in 0..4 {
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    }
    assert!(!ctx.has_requested_repaint(), "idle");
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/a.psd".into())));
    assert!(ctx.has_requested_repaint());
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/a.psd"]);
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/pics/a.psd"));
    assert_eq!(st.doc.name, "a.psd", "a successful Save As adopts the file name");
    assert!(!st.is_dirty());
    assert!(!app.file_dialog_open());
}

/// Preferences ▸ File Handling ▸ Lowercase Extension (default on): the chosen path's extension
/// is lowercased on the way to the writer; unchecked, the user's spelling is kept.
#[test]
fn save_paths_get_a_lowercase_extension_when_the_preference_is_on() {
    let ctx = egui::Context::default();
    {
        let (mut app, open, written) = app();
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        answer(&open, Some(FileDialogAnswer::SaveTo("/pics/CAT.PSD".into())));
        app.poll_file_dialog(&ctx, None);
        assert_eq!(*written.borrow(), ["/pics/CAT.psd"], "only the extension is lowercased");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/CAT.psd"));
    }
    let (mut app, open, written) = app();
    app.run("prefs.set", json!({"path": "fileHandling.lowercaseExtension", "value": false})).unwrap();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/CAT.PSD".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/CAT.PSD"], "off keeps the user's spelling");
}

#[test]
fn save_and_export_dialogs_start_beside_the_document() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    let dir = std::env::temp_dir().join("PhotoCraft projects").join("图像");
    let source = dir.join("original.psd");
    app.session.active_mut().unwrap().path = Some(source.to_string_lossy().into_owned());
    for command in ["file.saveAs", "file.export.exportAs", "file.export.saveForWebLegacy"] {
        let r = menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        if let Some(id) = r["dialog"].as_u64() {
            crate::dialogs::confirm(&mut app, id).unwrap();
        }
        app.poll_file_dialog(&ctx, None);
        {
            let held = open.borrow();
            let (FileDialogRequest::Save { suggested }, _) = &held[0] else { panic!("save dialog expected") };
            assert_eq!(std::path::Path::new(suggested).parent(), Some(dir.as_path()), "{command}");
            if command == "file.saveAs" {
                assert_eq!(std::path::Path::new(suggested), source);
            }
        }
        answer(&open, None);
        app.poll_file_dialog(&ctx, None);
    }
}

#[test]
fn save_suggestions_preserve_explicit_directories_and_untitled_names() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    for (document_path, suggested) in
        [(None, "Untitled.png"), (Some("project/source.psd"), "exports/result.png"), (Some("project/source.psd"), "/other/result.png")]
    {
        app.session.active_mut().unwrap().path = document_path.map(str::to_string);
        app.pick_save(suggested, |_, _| Ok(Value::Null)).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(matches!(&open.borrow()[0].0, FileDialogRequest::Save { suggested: actual } if actual == suggested));
        answer(&open, None);
        app.poll_file_dialog(&ctx, None);
    }
}

#[test]
fn the_answer_goes_to_the_document_it_was_asked_for() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.session.set_active(0);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.session.set_active(1);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/first.psd".into())));
    app.poll_file_dialog(&ctx, None);
    let paths: Vec<_> = app.session.documents().iter().map(|d| d.path.clone()).collect();
    assert_eq!(paths, [Some("/pics/first.psd".to_string()), None]);
    assert_eq!(app.session.active_index(), Some(1), "a completed save preserves the selected tab");
    assert_eq!(app.session.documents()[0].doc.name, "first.psd");
    assert_ne!(app.session.documents()[1].doc.name, "first.psd");
    // Once that document is closed, the answer has nothing to save.
    app.session.set_active(0);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("file.close", json!({"document": 0})).unwrap();
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/again.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/first.psd"]);
    assert!(app.ui.status_error && app.session.documents()[0].path.is_none());
}

#[test]
fn cancel_and_a_dialog_that_could_not_show_end_quietly() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    app.run("layer.new.layer", json!({})).unwrap();
    let before = app.session.active().unwrap().clone();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert!(!app.file_dialog_open() && !app.ui.status_error);
    // A reply dropped unanswered (the dialog failed to show) is a Cancel, not a dialog left open forever.
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    open.borrow_mut().clear();
    app.poll_file_dialog(&ctx, None);
    assert!(!app.file_dialog_open() && !app.ui.status_error);
    assert!(written.borrow().is_empty());
    let after = app.session.active().unwrap();
    assert_eq!(after.doc.name, before.doc.name);
    assert_eq!(after.path, before.path);
    assert_eq!((after.revision, after.saved_revision), (before.revision, before.saved_revision));
    // Without a dialog service, asking is a Cancel straight away.
    app.services.file_dialog = None;
    assert_eq!(app.open_dialog_file().unwrap_err(), CANCELLED);
    assert!(!app.file_dialog_open());
}

#[test]
fn opening_another_document_before_the_save_answer_keeps_its_identity() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    let first_id = app.active_doc_id().unwrap();
    app.save_as(None).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("file.new", json!({"width": 4, "height": 4, "name": "Untitled-2"})).unwrap();
    let second_id = app.active_doc_id().unwrap();
    app.run("document.move", json!({"document": 0, "to": 1})).unwrap();
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/保存 first.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/保存 first.psd"]);
    let first = app.session.documents().iter().find(|s| s.doc.id == first_id).unwrap();
    assert_eq!(first.doc.name, "保存 first.psd");
    assert_eq!(first.path.as_deref(), Some("/pics/保存 first.psd"));
    assert_eq!(app.active_doc_id().unwrap(), second_id);
    assert_eq!(app.session.active().unwrap().doc.name, "Untitled-2");
}

#[test]
fn failed_deferred_save_preserves_both_identities_and_the_selected_tab() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    app.run("layer.new.layer", json!({})).unwrap();
    app.save_as(None).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("file.new", json!({"width": 4, "height": 4, "name": "second"})).unwrap();
    let before: Vec<_> = app.session.documents().iter().map(|d| (d.doc.name.clone(), d.path.clone(), d.revision, d.saved_revision)).collect();
    app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
    answer(&open, Some(FileDialogAnswer::SaveTo("failed.psd".into())));
    app.poll_file_dialog(&ctx, None);
    let after: Vec<_> = app.session.documents().iter().map(|d| (d.doc.name.clone(), d.path.clone(), d.revision, d.saved_revision)).collect();
    assert_eq!(after, before);
    assert_eq!(app.session.active_index(), Some(1));
    assert!(app.ui.status_error && app.ui.status == "disk full");
    assert!(written.borrow().is_empty());
}

#[test]
fn an_answer_of_the_wrong_kind_is_an_error_not_a_crash() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Paths(vec!["/pics/a.psd".into()])));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error && app.ui.status == UNEXPECTED);
    assert!(written.borrow().is_empty());
    app.open_dialog_file().unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/a.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(app.session.documents().len(), 1);
}
