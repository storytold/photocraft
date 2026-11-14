//! #13979: the batch Lens Correction dialog's Source Files is the files row (Use: Files or Folder, the list,
//! Browse…, Remove, Add Open Files). The list is the command's `input`: the full paths in list order; Folder mode
//! lists the folder's images the way the batch reads a folder and sends the folder.

use std::path::PathBuf;

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, ParamKind};
use serde_json::json;

use super::lens_batch_layout_tests::{opened, set, source_folder};
use crate::PhotocraftApp;

/// The params OK sends for `fields`.
fn params(fields: &DialogFields) -> serde_json::Value {
    crate::filter_dialog::command_params(super::COMMAND, fields)
}

#[test]
fn source_files_is_a_files_row_that_also_picks_a_folder() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let rows = super::rows(&app, ".".into());
    let row = rows.iter().find(|p| p.key == "input").expect("a Source Files row");
    assert_eq!(row.label.as_str(), "Source Files");
    let ParamKind::Files { folder_mode, .. } = &row.kind else { panic!("Source Files is a files row: {:?}", row.kind) };
    assert!(*folder_mode, "Use offers Folder");
}

/// The reference case: a folder with `gray.psd, grid.jpg, grid16.tif, notes.txt` (and here a sub-folder) lists
/// `gray.psd, grid.jpg, grid16.tif`.
#[test]
fn folder_mode_lists_the_folders_images_by_name() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["gray.psd", "grid.jpg", "grid16.tif", "notes.txt"] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub").join("deep.png"), b"x").unwrap();
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let names: Vec<String> = super::listing(&app, dir.path()).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["gray.psd", "grid.jpg", "grid16.tif"]);
}

#[test]
fn the_list_is_sent_in_list_order_and_folder_mode_sends_the_folder() {
    let list = |paths: &[&str]| FieldValue::List(paths.iter().map(|p| FieldValue::Text((*p).to_owned())).collect());
    let mut fields = DialogFields::new();
    fields.set("input", list(&["/b/grid16.tif", "/a/gray.psd", "/a/grid.jpg"]));
    assert_eq!(params(&fields)["input"], json!(["/b/grid16.tif", "/a/gray.psd", "/a/grid.jpg"]));
    fields.set("input.useFolder", true);
    fields.set("input.folder", "/photos");
    let p = params(&fields);
    assert_eq!(p["input"], json!("/photos"), "Folder mode sends the folder");
    assert!(p.get("input.useFolder").is_none() && p.get("input.folder").is_none(), "only the command's keys: {p}");
    // Nothing listed yet: an empty list, which the command refuses.
    assert_eq!(params(&DialogFields::new())["input"], json!([]));
}

/// Add Open Files offers the saved, unchanged documents only: a never-saved or modified one has nothing on disk.
#[test]
fn add_open_files_offers_saved_unchanged_documents_only() {
    let dir = source_folder();
    let saved = dir.path().join("a.png");
    let services = crate::Services {
        import: Some(Box::new(|name, bytes, opts| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.open_file(&saved.to_string_lossy(), &std::fs::read(&saved).unwrap()).unwrap();
    app.run("file.new", json!({"width": 10, "height": 10})).unwrap();
    let rows = super::rows(&app, ".".into());
    let row = rows.iter().find(|p| p.key == "input").unwrap();
    let ParamKind::Files { open_files, .. } = &row.kind else { panic!("a files row") };
    assert_eq!(open_files, &[PathBuf::from(&saved)]);
}

/// OK runs the command over the listed files: the full paths, in list order.
#[test]
fn ok_sends_the_listed_paths() {
    let (mut h, id) = opened();
    let dir = source_folder();
    let out = dir.path().join("out");
    set(&mut h, id, "input", json!({"files": [dir.path().join("a.png")], "useFolder": false}));
    set(&mut h, id, "output", json!(out.to_string_lossy()));
    crate::dialogs::testing::confirm(h.state_mut(), id).expect("OK is accepted");
    h.run_steps(4);
    assert!(out.join("a.psd").exists(), "the listed file was corrected");
}
