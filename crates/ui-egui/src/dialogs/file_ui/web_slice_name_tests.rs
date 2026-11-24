//! #16662: with slices, the name typed in Save for Web's save dialog names the generated slice files and the HTML
//! page. The suggested name keeps the document's, and the export runs with the folder alone, so a recorded action
//! names each document's slices after that document.

use craft::ui::patterns::form_dialogs::{HostedSaveForWeb, SAVE_PATH_KEY, WebFormat};

use super::tests::{app, pass, set_option};
use super::*;

/// Saves the document "web", with an unnamed slice over each half, as `name` in a folder of its own. Returns the
/// slice images written and the params the export ran with.
fn save_slices_as(name: &str) -> (Vec<String>, Value) {
    let dir = std::env::temp_dir().join(format!("pc-webui-slice-names-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = app();
    app.run("slice.new", json!({"rect": [0, 0, 120, 40]})).unwrap();
    app.run("slice.new", json!({"rect": [0, 40, 120, 40]})).unwrap();
    let id = open_web(&mut app).expect("an open document");
    set_option(&mut app, id, "format", json!(WebFormat::Png24));
    let target = dir.join(name);
    app.framework.dialogs.get_mut_by_id::<HostedSaveForWeb>(id).expect("Save for Web is open").answer_path(SAVE_PATH_KEY, target.clone());
    pass(&mut app);
    let ctx = app.ctx().clone();
    if app.framework.dialogs.get_by_id::<HostedSaveForWeb>(id).is_some() {
        crate::dialogs::confirm(&mut app, &ctx, id).expect("OK needs the path answered");
    }
    take(&mut app, &ctx);
    let (command, params) = app.session.journal.last().cloned().expect("the export ran");
    assert_eq!(command, WEB_COMMAND);
    // The page's spacer image is not a slice.
    let mut files: Vec<String> = std::fs::read_dir(dir.join("images"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".png"))
        .collect();
    files.sort();
    let _ = std::fs::remove_dir_all(&dir);
    (files, params)
}

#[test]
fn a_typed_name_names_the_slice_files() {
    let (files, params) = save_slices_as("audi_en.png");
    assert_eq!(files, ["audi_en_01.png", "audi_en_02.png"], "{params}");
    assert!(params["path"].as_str().is_some_and(|p| p.ends_with("audi_en.png")), "{params}");
}

#[test]
fn the_suggested_name_keeps_the_document_name_and_records_the_folder_alone() {
    let (files, params) = save_slices_as("web.png");
    assert_eq!(files, ["web_01.png", "web_02.png"], "{params}");
    assert!(params.get("path").is_none() && params["dir"].is_string(), "{params}");
}
