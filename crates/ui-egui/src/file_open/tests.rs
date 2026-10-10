use super::*;
use crate::file_dialog::{self, FileDialogAnswer, FileDialogRequest};
use crate::{Services, menus, notices};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

/// Writes recorded by the fake writer: (path, bytes).
type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app whose importer names the document after the file (like `photocraft-io`) and reports a
/// warning for names containing "warn", and fails for bytes "bad"; the exporter warns for ".png"
/// and the writer records what it wrote. Its file dialogs answer with `answers`, in order.
fn app_with(answers: Vec<Option<FileDialogAnswer>>) -> (PhotocraftApp, Written) {
    let written: Written = Rc::default();
    let w = written.clone();
    let services = Services {
        import: Some(Box::new(|name: &str, bytes: &[u8]| {
            if bytes == b"bad" {
                return Err("not an image".into());
            }
            let warnings = if name.contains("warn") { vec!["Adjustment layer \"Curves 1\" was flattened".to_string()] } else { Vec::new() };
            Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), warnings))
        })),
        export: Some(Box::new(|_doc: &Document, path: &str, _s: &crate::ExportSettings| {
            let warnings = if path.ends_with(".png") { vec!["Layers were flattened".to_string()] } else { Vec::new() };
            Ok((b"out".to_vec(), warnings))
        })),
        file_dialog: Some(file_dialog::fake(answers).0),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    (PhotocraftApp::new(photocraft_engine::Session::new(), services), written)
}

/// The end of a frame: the dialog asked for is shown, and its answer acted on.
fn answer(app: &mut PhotocraftApp) {
    app.poll_file_dialog(&egui::Context::default(), None);
}

/// Files named `names` (holding `bytes`) in a fresh directory for `tag`: (directory, paths).
fn files(tag: &str, names: &[&str], bytes: &[u8]) -> (std::path::PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-file-open-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let paths = names
        .iter()
        .map(|name| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path.to_string_lossy().into_owned()
        })
        .collect();
    (dir, paths)
}

#[test]
fn display_name_is_the_file_name() {
    assert_eq!(display_name("/a/b/photo.psd"), "photo.psd");
    assert_eq!(display_name("photo.psd"), "photo.psd");
    assert_eq!(display_name(""), "");
    assert_eq!(display_name("/"), "/");
}

#[test]
fn open_file_sets_name_path_and_recent() {
    let (mut app, _) = app_with(Vec::new());
    let w = app.open_file("/pics/cat.psd", b"x").unwrap();
    assert!(w.is_empty());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.name, "cat.psd");
    assert_eq!(st.path.as_deref(), Some("/pics/cat.psd"));
    assert_eq!(app.ui.recent_files, vec!["/pics/cat.psd".to_string()]);
    assert!(!app.ui.status_error);
    assert!(app.ui.notices.is_empty());
}

#[test]
fn affinity_preview_does_not_acquire_the_source_path_even_when_renamed() {
    for automation in [false, true] {
        for path in ["/pics/source.af", "/pics/renamed.psd"] {
            let (mut app, written) = app_with(vec![None]);
            // The fake importer supplies the preview; its source signature controls path policy.
            if automation {
                app.open_automation_bytes(&display_name(path), b"\x00\xffKA").unwrap();
                app.opened_from(path);
            } else {
                app.open_file(path, b"\x00\xffKA").unwrap();
            }
            assert!(app.session.active().unwrap().source_read_only);
            assert!(app.session.active().unwrap().path.is_none());
            assert_eq!(app.ui.recent_files.first().map(String::as_str), Some(path));
            // Save asks for a new file (cancelled here) instead of writing over the source.
            assert_eq!(menus::invoke(&mut app, &egui::Context::default(), "file.save", json!({})).unwrap(), json!({"fileDialog": "save"}));
            answer(&mut app);
            assert!(written.borrow().is_empty());
        }
    }
}

#[test]
fn file_open_dialog_sets_path_so_save_writes_in_place() {
    // The dialog returns a full path: the document is named after the file, not the path.
    let (dir, paths) = files("in-place", &["cat.psd"], b"x");
    let (mut app, written) = app_with(vec![Some(FileDialogAnswer::Paths(paths.clone()))]);
    let ctx = egui::Context::default();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap(), json!({"fileDialog": "open"}));
    answer(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.name, "cat.psd");
    assert_eq!(st.path.as_ref(), paths.first());
    assert_eq!(app.ui.recent_files.first(), paths.first());
    // File › Save goes straight back to the file, without a dialog.
    let r = menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(r["path"], paths[0]);
    assert!(!app.file_dialog_open());
    assert_eq!(written.borrow().last().map(|(p, _)| p), paths.first());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_open_dialog_opens_every_selected_path() {
    let (dir, paths) = files("issue-595", &["one.psd", "two.png"], b"x");
    let (mut app, _) = app_with(vec![Some(FileDialogAnswer::Paths(paths.clone()))]);

    menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({})).unwrap();
    answer(&mut app);

    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(
        app.session.documents().iter().map(|doc| doc.path.as_deref()).collect::<Vec<_>>(),
        paths.iter().map(|path| Some(path.as_str())).collect::<Vec<_>>()
    );
    assert_eq!(app.ui.recent_files, vec![paths[1].clone(), paths[0].clone()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancelling_file_open_opens_nothing() {
    let (mut app, _) = app_with(vec![None]);

    menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({})).unwrap();
    answer(&mut app);

    assert!(app.session.documents().is_empty());
    assert!(!app.ui.status_error, "Cancel is not an error");
    assert!(!app.file_dialog_open());
}

#[test]
fn the_web_hands_over_contents_rather_than_paths() {
    let (mut app, _) = app_with(vec![Some(FileDialogAnswer::Contents("cat.png".into(), b"x".to_vec()))]);
    menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({})).unwrap();
    answer(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.name.as_str(), st.path.as_deref()), ("cat.png", None));
}

#[test]
fn pcraft_documents_save_in_place_but_flat_files_ask() {
    let (mut app, written) = app_with(vec![None]);
    let ctx = egui::Context::default();
    app.open_file("/pics/work.pcraft", b"x").unwrap();
    let r = menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(r["path"], "/pics/work.pcraft");
    assert_eq!(written.borrow().len(), 1);
    // A PNG goes through Save As (cancelled here), not silently flattened over the original.
    app.open_file("/pics/flat.png", b"x").unwrap();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap(), json!({"fileDialog": "save"}));
    answer(&mut app);
    assert_eq!(written.borrow().len(), 1);
    assert!(!app.ui.status_error);
}

#[test]
fn templates_open_as_new_untitled_documents() {
    let (mut app, written) = app_with(Vec::new());
    let ctx = egui::Context::default();
    app.open_file("/pics/card.PSDT", b"x").unwrap();
    app.open_file("/pics/card.psdt", b"x").unwrap();
    let names: Vec<_> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
    assert_eq!(names, ["Untitled-1", "Untitled-2"]);
    assert!(app.session.documents().iter().all(|d| d.path.is_none()));
    assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("/pics/card.psdt"));
    // File › Save asks for a new name (cancelled here) rather than suggesting the template.
    let (show, asked) = file_dialog::fake(vec![None]);
    app.services.file_dialog = Some(show);
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    answer(&mut app);
    assert_eq!(*asked.borrow(), [FileDialogRequest::Save { suggested: "Untitled-2.psd".into() }]);
    assert!(written.borrow().is_empty());
}

#[test]
fn import_warnings_reach_status_notice_and_control_response() {
    let (dir, paths) = files("warn", &["warn.psd"], b"x");
    let p = paths[0].clone();
    let (mut app, _) = app_with(Vec::new());
    let ctx = egui::Context::default();
    let r = menus::invoke(&mut app, &ctx, "file.open", json!({ "path": p })).unwrap();
    assert_eq!(r["warnings"][0], "Adjustment layer \"Curves 1\" was flattened");
    assert!(app.ui.status.contains("was flattened"), "{}", app.ui.status);
    assert!(app.ui.status_error);
    assert_eq!(app.ui.notices.len(), 1);
    assert!(!app.ui.notices[0].error);
    assert_eq!(app.ui.notices[0].lines.len(), 1);
    // The document still opened, with its path.
    assert_eq!(app.session.active().and_then(|s| s.path.clone()), Some(p));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_warnings_reach_status_notice_and_control_response() {
    let (mut app, _) = app_with(Vec::new());
    let ctx = egui::Context::default();
    app.open_file("/pics/cat.psd", b"x").unwrap();
    let r = menus::invoke(&mut app, &ctx, "file.saveAs", json!({ "path": "/pics/cat.png" })).unwrap();
    assert_eq!(r["path"], "/pics/cat.png");
    assert_eq!(r["warnings"][0], "Layers were flattened");
    assert!(app.ui.status.contains("Saved cat.png"), "{}", app.ui.status);
    assert_eq!(app.ui.notices.len(), 1);
    // No warnings: no notice, and the response says so.
    let r = menus::invoke(&mut app, &ctx, "file.saveAs", json!({ "path": "/pics/cat.psd" })).unwrap();
    assert_eq!(r["warnings"], json!([]));
    assert_eq!(app.ui.notices.len(), 1);
}

#[test]
fn open_failures_are_errors_and_leave_no_document() {
    let (dir, paths) = files("broken", &["broken.psd"], b"bad");
    let (mut app, _) = app_with(vec![Some(FileDialogAnswer::Paths(paths))]);
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap();
    answer(&mut app);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.recent_files.is_empty());
    assert!(app.ui.status_error);
    assert!(app.ui.status.starts_with("Couldn't open broken.psd"), "{}", app.ui.status);
    assert!(app.ui.notices.last().is_some_and(|n| n.error));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_picked_file_that_cannot_be_read_is_reported_like_any_open_failure() {
    let missing = std::env::temp_dir().join("photocraft-missing-dir").join("big.psb").to_string_lossy().into_owned();
    let picked = || Some(FileDialogAnswer::Paths(vec![missing.clone()]));
    let (mut app, _) = app_with(vec![picked(), picked(), None]);
    app.open_dialog_file().unwrap();
    answer(&mut app);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.status_error);
    let notice = app.ui.notices.last().unwrap();
    assert!(notice.error && notice.title.starts_with("Couldn't open big.psb: "), "{}", notice.title);
    // Commands that read a picked file (scripts, notes, Place) fail with the file's name.
    app.ui.status_error = false;
    app.pick_file_bytes(|_, _, _| Err("read".into())).unwrap();
    answer(&mut app);
    assert!(app.ui.status_error && app.ui.status.starts_with("big.psb: "), "{}", app.ui.status);
    app.ui.status_error = false;
    app.pick_file_bytes(|_, _, _| Err("read".into())).unwrap();
    answer(&mut app);
    assert!(!app.ui.status_error, "cancelled");
}

#[test]
fn open_paths_reports_each_failure_without_panicking() {
    let (mut app, _) = app_with(Vec::new());
    let missing = std::env::temp_dir().join("photocraft-definitely-missing-file.psd").to_string_lossy().to_string();
    let dir = std::env::temp_dir().to_string_lossy().to_string();
    let n = app.open_paths(&[missing, String::new(), dir, "\u{0}".into()]);
    assert_eq!(n, 0);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.status_error);
    assert_eq!(app.ui.notices.len(), notices::MAX_NOTICES);
    // Bad control params error instead of panicking.
    let ctx = egui::Context::default();
    assert!(menus::invoke(&mut app, &ctx, "file.open", json!({ "path": "" })).is_err());
}

#[test]
fn os_open_events_open_files_with_paths() {
    let dir = std::env::temp_dir().join(format!("photocraft-os-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.psd");
    let b = dir.join("b.png");
    std::fs::write(&a, b"x").unwrap();
    std::fs::write(&b, b"x").unwrap();
    let paths: Vec<String> = [&a, &b].iter().map(|p| p.to_string_lossy().to_string()).collect();
    let (mut app, _) = app_with(Vec::new());
    let mut queue = vec![OsEvent::Open(paths.clone())];
    app.services.os_events = Some(Box::new(move || std::mem::take(&mut queue)));
    let ctx = egui::Context::default();
    app.drain_os_events(&ctx);
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.active().and_then(|s| s.path.clone()).as_deref(), Some(paths[1].as_str()));
    assert_eq!(app.ui.recent_files, vec![paths[1].clone(), paths[0].clone()]);
    // Drained: a second poll opens nothing more.
    app.drain_os_events(&ctx);
    assert_eq!(app.session.documents().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn os_quit_and_empty_events_do_not_panic() {
    let (mut app, _) = app_with(Vec::new());
    let mut queue = vec![OsEvent::Open(Vec::new()), OsEvent::Quit];
    app.services.os_events = Some(Box::new(move || std::mem::take(&mut queue)));
    app.drain_os_events(&egui::Context::default());
    assert!(app.session.documents().is_empty());
}

#[derive(Debug)]
struct FakeDrop(std::path::PathBuf, Result<Vec<u8>, String>);

impl egui::DroppedFile for FakeDrop {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        self.1.clone()
    }
}

/// A dropped file at `path` whose bytes (if read through egui) are `bytes`.
fn dropped(path: impl Into<std::path::PathBuf>, bytes: Result<Vec<u8>, String>) -> egui::DroppedFileHandle {
    std::sync::Arc::new(FakeDrop(path.into(), bytes))
}

fn doc_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

#[test]
fn dropped_files_open_with_path_and_recent() {
    let (mut app, _) = app_with(Vec::new());
    let dir = std::env::temp_dir().join(format!("photocraft-drop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let abs = dir.join("dropped.psd");
    std::fs::write(&abs, b"x").unwrap();
    let abs_s = abs.to_string_lossy().to_string();
    // Dropped where the pointer can't be read: each file opens as a document.
    app.open_dropped(
        &egui::Context::default(),
        vec![
            // A desktop drop (absolute path) is read from disk like File › Open (#375), never
            // through egui's whole-file `std::fs::read`.
            dropped(abs, Err("egui's reader must not be used".into())),
            // Unreadable and undecodable drops are errors; no path means no recent entry.
            dropped("gone.psd", Err("permission denied".into())),
            dropped("rel.psd", Ok(b"x".to_vec())),
            dropped("", Ok(b"bad".to_vec())),
        ],
        None,
    );
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.documents()[0].path.as_deref(), Some(abs_s.as_str()));
    assert_eq!(app.session.documents()[0].doc.name, "dropped.psd");
    assert_eq!(app.session.documents()[1].path, None);
    assert_eq!(app.ui.recent_files, vec![abs_s]);
    assert!(app.ui.status_error);
    assert_eq!(app.ui.notices.iter().filter(|n| n.error).count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// PNG bytes of a white `w`×`h` image.
fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "p.png", &Default::default()).unwrap().bytes
}

/// An app with one open document whose canvas showed at (100, 50)–(700, 550) last frame.
fn app_with_canvas() -> PhotocraftApp {
    let (mut app, _) = app_with(Vec::new());
    app.open_bytes("canvas.psd", b"x").unwrap();
    app.drop_canvas_rect = Some(egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(700.0, 550.0)));
    app
}

#[test]
fn pdf_drop_on_canvas_waits_for_page_selection_instead_of_placing_pages() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "pdf", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    app.open_dropped(&ctx, vec![dropped("pages.PDF", Ok(bytes))], Some(egui::pos2(400.0, 300.0)));
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.pending_pdf_pages(), Some(1));
    assert!(app.drop_places.is_empty());
    app.open_pdf_pages(&[0]).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    assert!(app.session.active().unwrap().source_read_only);
    assert!(app.session.active().unwrap().path.is_none());
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn history_steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// As in the reference app, files dropped on the canvas are placed one at a time, each in Free
/// Transform; the next is placed once the previous transform is committed or cancelled.
#[test]
fn files_dropped_onto_the_canvas_are_placed_in_free_transform_one_by_one() {
    let dir = std::env::temp_dir().join(format!("photocraft-drop-place-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let abs = dir.join("photo.png");
    std::fs::write(&abs, png_bytes(200, 100)).unwrap();
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    app.open_dropped(
        &ctx,
        vec![
            // Read from its path, placed like File › Place Embedded (fitted to the canvas).
            dropped(abs, Err("egui's reader must not be used".into())),
            // Undecodable: an error, skipped.
            dropped("bad.png", Ok(b"bad".to_vec())),
            // No path: placed from the dropped bytes.
            dropped("small.png", Ok(png_bytes(2, 2))),
        ],
        Some(egui::pos2(400.0, 300.0)),
    );
    app.place_next_dropped(&ctx);
    assert_eq!(app.session.documents().len(), 1, "drops onto the canvas don't open documents");
    assert_eq!(layer_count(&app), layers + 1, "one at a time");
    let t = app.ui.transform.as_ref().expect("the placed layer is in Free Transform");
    assert_eq!(t.made, Some(crate::state::MadeLayer::Place));
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "photo");
    // ↩ without changes: the place stays as it is.
    crate::transform_tool::commit(&mut app);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 1, steps + 1));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Place Embedded"));
    // Next frame: the undecodable file is reported, the next one placed and transformed.
    app.place_next_dropped(&ctx);
    assert_eq!(app.ui.notices.iter().filter(|n| n.error).count(), 1);
    assert_eq!(layer_count(&app), layers + 2);
    let st = app.session.active().unwrap();
    let top = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(top.name, "small");
    assert!(matches!(top.content, photocraft_doc::LayerContent::Smart(_)));
    // Esc takes the place back, leaving nothing to redo.
    crate::transform_tool::cancel(&mut app);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 1, steps + 1));
    assert!(!app.session.active().unwrap().history.can_redo());
    app.place_next_dropped(&ctx);
    assert!(app.ui.transform.is_none() && app.drop_places.is_empty());
    assert!(app.ui.recent_files.is_empty(), "placing isn't opening");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1099: switching tabs after an actual canvas drop cancels the place in its source document.
#[test]
fn switching_documents_after_a_canvas_drop_undoes_only_the_place() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let original = app.session.active().unwrap().doc.clone();
    let steps = history_steps(&app);
    app.open_dropped(&ctx, vec![dropped("dot.png", Ok(png_bytes(2, 2)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    assert_eq!(app.ui.transform.as_ref().unwrap().made, Some(crate::state::MadeLayer::Place));
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(app.session.undo());
    let st = app.session.active().unwrap();
    let (other, history, redo, revision) = (st.doc.clone(), st.history.entries(), st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), st.revision);
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
    let st = app.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&st.doc, &other));
    assert_eq!((st.history.entries(), st.revision), (history, revision));
    assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), redo);
    let origin = &app.session.documents()[0];
    assert!(std::sync::Arc::ptr_eq(&origin.doc, &original));
    assert_eq!(origin.history.past_len(), steps);
    assert!(!origin.history.can_redo());
}

/// Moving a placed file in its Free Transform and committing makes one Place Embedded step.
#[test]
fn a_transformed_place_is_one_history_step() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    app.open_dropped(&ctx, vec![dropped("dot.png", Ok(png_bytes(2, 2)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    let t = app.ui.transform.as_mut().expect("in Free Transform");
    t.quad = t.quad.map(|[x, y]| [x - 1.0, y - 1.0]);
    crate::transform_tool::commit(&mut app);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 1, steps + 1));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Place Embedded"));
    app.session.undo();
    assert_eq!(layer_count(&app), layers, "one undo removes the place and the move");
}

#[test]
fn files_dropped_on_the_tabs_open_at_that_slot() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.ui.views[0].zoom = 3.0;
    // One tab at (100, 20)–(200, 46): its left half is slot 0, its right half and beyond slot 1.
    let tab = egui::Rect::from_min_max(egui::pos2(100.0, 20.0), egui::pos2(200.0, 46.0));
    let strip = egui::Rect::from_min_max(egui::pos2(92.0, 14.0), egui::pos2(700.0, 50.0));
    app.tab_strip = Some(crate::canvas::TabStrip { rect: strip, tabs: vec![(0, tab)] });
    assert_eq!(app.drop_target(&ctx, Some(egui::pos2(120.0, 30.0))), crate::file_open::DropTarget::Tabs(0));
    assert_eq!(app.drop_target(&ctx, Some(egui::pos2(180.0, 30.0))), crate::file_open::DropTarget::Tabs(1));
    assert_eq!(app.drop_target(&ctx, Some(egui::pos2(600.0, 30.0))), crate::file_open::DropTarget::Tabs(1));
    let files = vec![dropped("first.png", Ok(b"x".to_vec())), dropped("bad.png", Ok(b"bad".to_vec())), dropped("second.png", Ok(b"x".to_vec()))];
    app.open_dropped(&ctx, files, Some(egui::pos2(120.0, 30.0)));
    assert_eq!(doc_names(&app), ["first.png", "second.png", "canvas.psd"], "in drop order from the slot");
    assert_eq!(app.session.active_index(), Some(1), "the last one opened is active");
    assert_eq!(app.ui.views.len(), 3);
    assert_eq!(app.ui.views[2].zoom, 3.0, "views move with their documents");
    // Past the last tab: opened at the end.
    let tabs = (0..3).map(|i| (i, tab.translate(egui::vec2(104.0 * i as f32, 0.0)))).collect();
    app.tab_strip = Some(crate::canvas::TabStrip { rect: strip, tabs });
    app.open_dropped(&ctx, vec![dropped("last.png", Ok(b"x".to_vec()))], Some(egui::pos2(600.0, 30.0)));
    assert_eq!(app.session.documents().last().map(|d| d.doc.name.as_str()), Some("last.png"));
}

#[test]
fn a_drop_slot_counts_documents_not_the_tabs_shown() {
    // #1276: when the tabs overflow into the » menu, the shown tabs aren't documents 0, 1, 2…
    // Documents 4, 5 and 6 shown: a drop before document 5's tab opens at position 5.
    let tab = |i: usize, x: f32| (i, egui::Rect::from_min_max(egui::pos2(x, 20.0), egui::pos2(x + 100.0, 46.0)));
    let strip = crate::canvas::TabStrip {
        rect: egui::Rect::from_min_max(egui::pos2(0.0, 14.0), egui::pos2(700.0, 50.0)),
        tabs: vec![tab(4, 0.0), tab(5, 104.0), tab(6, 208.0)],
    };
    assert_eq!(strip.slot(20.0), 4);
    assert_eq!(strip.slot(130.0), 5);
    assert_eq!(strip.slot(500.0), 7, "past the last shown tab: after it");
    assert_eq!(crate::canvas::TabStrip { rect: strip.rect, tabs: vec![] }.slot(10.0), 0);
}

/// The whole app (eframe harness, real layout): over the tab strip, the pointer read from the OS
/// service picks a slot; files dropped there open (in the background) at it; dropped on the canvas they're placed in Free Transform, and Esc takes the place back.
#[test]
fn drag_and_drop_in_the_running_app() {
    let dir = std::env::temp_dir().join(format!("photocraft-drop-app-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("photo.png");
    std::fs::write(&file, png_bytes(40, 30)).unwrap();
    let pointer = Rc::new(std::cell::Cell::new(egui::Pos2::ZERO));
    let reported = pointer.clone();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(8).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        for _ in 0..2 {
            s.execute("file.new", json!({"width": 320, "height": 240, "background": "white"})).unwrap();
        }
        let services = Services { cursor_pos: Some(Box::new(move |_: &egui::Context| Some(reported.get()))), ..Default::default() };
        let mut app = PhotocraftApp::new(s, services);
        app.background_jobs = true;
        app
    });
    h.run_steps(4);
    let strip = h.state().tab_strip.clone().expect("the tab strip is drawn");
    assert_eq!(strip.tabs.len(), 2);
    // Hovering over the first tab's left half: slot 0.
    let over_first = egui::pos2(strip.tabs[0].1.left() + 4.0, strip.tabs[0].1.center().y);
    pointer.set(over_first);
    h.input_mut().hovered_files.push(egui::HoveredFile { path: Some(file.clone()), ..Default::default() });
    h.step();
    assert_eq!(h.state().drop_target(&h.ctx, Some(over_first)), crate::file_open::DropTarget::Tabs(0));
    // Dropped there: opens in the background, then takes the first tab.
    h.input_mut().hovered_files.clear();
    h.input_mut().dropped_files.push(dropped(file.clone(), Err("read from the path".into())));
    h.step();
    let t = std::time::Instant::now();
    while !h.state().jobs.opens.is_empty() && t.elapsed() < std::time::Duration::from_secs(30) {
        h.step();
    }
    h.run_steps(2);
    let names = doc_names(h.state());
    assert_eq!(names.len(), 3);
    assert_eq!(names[0], "photo.png", "{names:?}");
    assert_eq!(h.state().session.active_index(), Some(0));
    // Dropped on the canvas: placed into the active document, in Free Transform.
    let layers = h.state().session.active().unwrap().doc.layers.len();
    pointer.set(h.state().drop_canvas_rect.expect("the canvas is shown").center());
    h.input_mut().dropped_files.push(dropped(file, Err("read from the path".into())));
    h.run_steps(2);
    assert_eq!(h.state().session.documents().len(), 3, "no new document");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1);
    assert_eq!(h.state().ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.transform.is_none());
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "Esc takes the place back");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn views_and_windows_stay_with_their_documents() {
    let (mut app, _) = app_with(Vec::new());
    for name in ["a.psd", "b.psd", "c.psd", "d.psd"] {
        app.open_bytes(name, b"x").unwrap();
    }
    for (i, v) in app.ui.views.iter_mut().enumerate() {
        v.zoom = i as f32 + 1.0;
    }
    app.ui.windows = (0..4).map(|document| crate::state::DocWindow { id: document as u64, document, view: Default::default(), open: true }).collect();
    let zooms = |app: &PhotocraftApp| app.ui.views.iter().map(|v| v.zoom).collect::<Vec<_>>();
    let windows = |app: &PhotocraftApp| app.ui.windows.iter().map(|w| (w.id, w.document)).collect::<Vec<_>>();
    assert_eq!(app.run("document.move", json!({"document": 0, "to": 3})).unwrap(), json!({"document": 3}));
    assert_eq!(doc_names(&app), ["b.psd", "c.psd", "d.psd", "a.psd"]);
    assert_eq!(zooms(&app), [2.0, 3.0, 4.0, 1.0]);
    assert_eq!(windows(&app), [(0, 3), (1, 0), (2, 1), (3, 2)]);
    // Out of range: an error, nothing moves.
    assert!(app.run("document.move", json!({"document": 7, "to": 0})).is_err());
    assert_eq!(zooms(&app), [2.0, 3.0, 4.0, 1.0]);
    // Closing a middle tab takes its view and windows with it; the others keep theirs.
    app.run("file.close", json!({"document": 1})).unwrap();
    assert_eq!(doc_names(&app), ["b.psd", "d.psd", "a.psd"]);
    assert_eq!(zooms(&app), [2.0, 4.0, 1.0]);
    assert_eq!(windows(&app), [(0, 2), (1, 0), (3, 1)]);
}

#[test]
fn files_dropped_off_the_canvas_open_as_documents() {
    let other = || vec![dropped("other.png", Ok(b"x".to_vec()))];
    let ctx = egui::Context::default();
    // On the tab bar or a panel (outside the canvas), or where the pointer can't be read.
    for at in [Some(egui::pos2(400.0, 30.0)), Some(egui::pos2(750.0, 300.0)), None] {
        let mut app = app_with_canvas();
        let before = layer_count(&app);
        app.open_dropped(&ctx, other(), at);
        assert_eq!(app.session.documents().len(), 2, "{at:?}");
        assert_eq!(app.session.documents()[0].doc.layers.len(), before, "{at:?}");
    }
    // Over a window floating above the canvas (shown from its second frame, once sized).
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::Window::new("panel").fixed_pos(egui::pos2(300.0, 200.0)).show(ui.ctx(), |ui| ui.label("over the canvas"));
        });
        out.textures_delta.clear();
    }
    let mut app = app_with_canvas();
    app.open_dropped(&ctx, other(), Some(egui::pos2(320.0, 220.0)));
    assert_eq!(app.session.documents().len(), 2);
    // The start screen or an opening file's card (no canvas shown) opens too.
    let mut app = app_with_canvas();
    app.drop_canvas_rect = None;
    app.open_dropped(&ctx, other(), Some(egui::pos2(400.0, 300.0)));
    assert_eq!(app.session.documents().len(), 2);
}

#[test]
fn notices_are_capped_and_dismissable_state_round_trips() {
    let (mut app, _) = app_with(Vec::new());
    for i in 0..10 {
        notices::post(&mut app, format!("n{i}"), vec![], false, None);
    }
    assert_eq!(app.ui.notices.len(), notices::MAX_NOTICES);
    assert_eq!(app.ui.notices.last().map(|n| n.title.as_str()), Some("n9"));
    // Notices are part of the serialized UI state (ui.inspect / ui.set).
    let v = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(v["notices"].as_array().map(Vec::len), Some(notices::MAX_NOTICES));
    let back: crate::UiState = serde_json::from_value(v).unwrap();
    assert_eq!(back.notices, app.ui.notices);
}

#[test]
fn notices_render_without_panicking() {
    let (mut app, _) = app_with(Vec::new());
    notices::post(&mut app, "Opened a.psd with 9 warnings", (0..9).map(|i| format!("warning {i}")).collect(), false, None);
    notices::error(&mut app, "Couldn't open b.psd: not an image".into());
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| notices::show(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert_eq!(app.ui.notices.len(), 2);
}
