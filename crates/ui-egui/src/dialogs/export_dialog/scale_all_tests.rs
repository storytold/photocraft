//! #13734: Export As's Scale All rows. One row exports through the save dialog at its size; several
//! ask for a folder and write `<name><suffix>.<ext>` per row at its size, refusing names two rows
//! share and files already there. The rows themselves (+, trash, the suffix limit, remembering) are
//! the framework's Export As.

use super::*;
use craft::host::{PickKind, Picked, ScriptedFiles};
use craft::ui::patterns::form_dialogs::ScaleSize;
use photocraft_doc::{Color, SampleType, Size};
use std::{cell::RefCell, rc::Rc};

type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn rows(list: &[(ScaleSize, &str)]) -> Vec<ScaleRow> {
    list.iter().map(|(size, suffix)| ScaleRow::new(*size, *suffix)).collect()
}

/// A 200 x 150 document named `harbor.psd` with Export As open, its exporter encoding for real and its writer keeping
/// each file. Its file dialogs stay open until the test answers them.
fn opened() -> (PhotocraftApp, DialogId, Writes, Arc<ScriptedFiles>) {
    let writes = Writes::default();
    let captured = writes.clone();
    let files = Arc::new(ScriptedFiles::held());
    let services = crate::Services {
        files: Some(files.clone()),
        export: Some(Box::new(|doc, path, settings| {
            photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        write: Some(Box::new(move |path, bytes| {
            captured.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.add_document(Document::with_background("harbor.psd", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
    let id = open(&mut app).unwrap();
    (app, id, writes, files)
}

/// Sets the dialog's Scale All rows, then presses Export.
fn export_rows(app: &mut PhotocraftApp, id: DialogId, list: &[ScaleRow]) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "scale_all", serde_json::to_value(list).unwrap()).unwrap();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

/// The user answers the open file dialog with `path`, the app takes the answer, and the dialog's next pass exports.
fn answer(app: &mut PhotocraftApp, id: DialogId, files: &ScriptedFiles, path: &std::path::Path) {
    files.answer(Picked::Paths(vec![path.to_path_buf()])).expect("a file dialog is open");
    app.poll_file_dialog();
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

/// The decoded file's pixel size and resolution.
fn size_of(bytes: &[u8]) -> (u32, u32, f32) {
    let doc = photocraft_io::import("out.png", bytes).unwrap().document;
    (doc.size.width, doc.size.height, doc.resolution_dpi)
}

#[test]
fn several_rows_ask_for_a_folder_and_write_one_file_per_size() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, id, writes, files) = opened();
    export_rows(&mut app, id, &rows(&[(ScaleSize::X1, ""), (ScaleSize::X0_5, "@0.5x")]));
    assert_eq!(files.requests().last().map(|r| r.kind), Some(PickKind::Folder), "a folder choice");
    answer(&mut app, id, &files, dir.path());
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let writes = writes.borrow();
    let paths: Vec<&str> = writes.iter().map(|(p, _)| p.as_str()).collect();
    let want = [dir.path().join("harbor.png"), dir.path().join("harbor@0.5x.png")];
    assert_eq!(paths, want.iter().map(|p| p.to_str().unwrap()).collect::<Vec<_>>());
    let (full, half) = (size_of(&writes[0].1), size_of(&writes[1].1));
    assert_eq!((full.0, full.1), (200, 150));
    assert_eq!((half.0, half.1), (100, 75), "half the size");
    assert_eq!(half.2, full.2, "the same ppi");
}

/// Two rows that would write one file are refused before anything is written (#15028 review).
#[test]
fn rows_that_share_an_output_file_are_refused() {
    let same = rows(&[(ScaleSize::X1, ""), (ScaleSize::X0_5, "")]);
    let err = batch_file_names("harbor", "png", &same).unwrap_err();
    assert!(err.contains("harbor.png"), "{err}");
    // Suffixes that sanitize alike, or differ only in case.
    assert!(batch_file_names("harbor", "png", &rows(&[(ScaleSize::X1, "a/b"), (ScaleSize::X2, "a:b")])).is_err());
    assert!(batch_file_names("harbor", "png", &rows(&[(ScaleSize::X1, "@Big"), (ScaleSize::X2, "@big")])).is_err());
    // Distinct suffixes pass, in row order.
    let ok = batch_file_names("harbor", "png", &rows(&[(ScaleSize::X1, ""), (ScaleSize::X0_5, "@0.5x")])).unwrap();
    assert_eq!(ok, ["harbor.png", "harbor@0.5x.png"]);
    assert_eq!(file_suffix("@2x/../a:b"), "@2x_.._a_b");
}

/// Only a folder was chosen, so a file already there is never replaced: any existing target stops the batch before
/// the first write (#15030 review).
#[test]
fn a_file_already_in_the_folder_stops_the_batch_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("harbor@0.5x.png"), b"keep me").unwrap();
    let (mut app, id, writes, files) = opened();
    export_rows(&mut app, id, &rows(&[(ScaleSize::X1, ""), (ScaleSize::X0_5, "@0.5x")]));
    answer(&mut app, id, &files, dir.path());
    assert!(writes.borrow().is_empty(), "not even the first row (harbor.png) was written");
    assert!(app.ui.status.contains("already exists"), "{}", app.ui.status);
    assert_eq!(std::fs::read(dir.path().join("harbor@0.5x.png")).unwrap(), b"keep me");
}

/// A final sigma and a sigma are one file on Windows (#15039 review).
#[test]
fn names_that_fold_together_by_uppercase_are_one_file() {
    assert!(batch_file_names("harbor", "png", &rows(&[(ScaleSize::X1, "σ"), (ScaleSize::X2, "ς")])).is_err());
    assert!(batch_file_names("harbor", "png", &rows(&[(ScaleSize::X1, "σ"), (ScaleSize::X2, "s")])).is_ok());
    // Lower-casing joins `K` and the Kelvin sign (APFS treats them as one name); upper-casing does not.
    assert!(batch_file_names("harbor", "png", &rows(&[(ScaleSize::X1, "K"), (ScaleSize::X2, "\u{212A}")])).is_err());
}
