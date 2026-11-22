//! #13687: Contact Sheet II's Include Subfolders and Group Images by Folder checkboxes.
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::json;

use super::tests::app;
use crate::dialogs::file_forms::open_contact_sheet;

#[test]
fn the_checkboxes_open_with_the_reference_defaults() {
    let mut app = app();
    let id = open_contact_sheet(&mut app);
    let fields = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields();
    assert_eq!((fields.bool("includeSubfolders"), fields.bool("groupByFolder")), (Some(true), Some(false)));
}

#[test]
fn ok_runs_the_sheet_with_both_settings() {
    /// Removes the test folder when it goes out of scope, also when the test panics.
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = std::env::temp_dir().join(format!("pc-contact-subfolders-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _cleanup = Cleanup(dir.clone());
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    for (i, rel) in ["top.png", "sub/inner.png"].into_iter().enumerate() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 8 + i, "height": 6})).unwrap();
        s.execute("file.saveACopy", json!({"path": dir.join(rel).to_string_lossy()})).unwrap();
    }
    let sheets = |group: bool| {
        let mut app = app();
        let ctx = app.ctx().clone();
        let id = open_contact_sheet(&mut app);
        let set = |app: &mut crate::PhotocraftApp, field: &str, value| app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap();
        set(&mut app, "input", json!(dir.to_string_lossy()));
        // No captions: they wait on the app's fonts, which load in the background.
        set(&mut app, "caption", json!(false));
        if group {
            set(&mut app, "groupByFolder", json!(true));
        }
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        // Each sheet's image layers: all but its background.
        let docs = app.session.documents().iter().skip(1).map(|d| (d.doc.name.clone(), d.doc.layers.len() - 1)).collect::<Vec<_>>();
        (docs, app.ui.status.clone())
    };
    // Include Subfolders (on) takes `sub/inner.png` too; Group puts it on its own sheet.
    let ((all, all_status), (grouped, grouped_status)) = (sheets(false), sheets(true));
    assert_eq!(all, [("ContactSheet-001".to_string(), 2)], "status: {all_status}");
    assert_eq!(grouped, [("ContactSheet-001".to_string(), 1), ("ContactSheet-002".to_string(), 1)], "status: {grouped_status}");
}
