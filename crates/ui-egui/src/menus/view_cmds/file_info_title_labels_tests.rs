//! File Info (#13824): the dialog is titled with the document it opened on (the name without
//! its extension), edits that document even if another tab becomes active, and the Basic page's
//! two remaining labels are title case. Other generated form dialogs keep their labels.

use craft::ui::dialog::DialogId;

use super::*;

fn app_with_document(name: &str) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"name": name, "width": 64, "height": 64})).unwrap();
    app.sync_views();
    app
}

fn open_file_info(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, "file.fileInfo", json!({})).unwrap();
    app.framework.dialogs.stack().top().expect("File Info opens")
}

#[test]
fn the_title_is_the_document_name_without_its_extension() {
    for (document, title) in [("oracle.psd", "oracle"), ("oracle", "oracle"), ("my.photo.png", "my.photo"), (".hidden", ".hidden")] {
        let app = app_with_document(document);
        assert_eq!(crate::dialogs::file_info_title(&app), title, "{document}");
    }
}

#[test]
fn the_author_title_and_copyright_status_labels_are_title_case() {
    let mut app = app_with_document("oracle.psd");
    let (rows, _) = super::dialogs::file_info_form(&mut app).expect("File Info form");
    let label = |key: &str| rows.iter().find(|p| p.key == key).map(|p| p.label.as_str()).unwrap_or_default();
    assert_eq!(label("authorTitle"), "Author Title");
    assert_eq!(label("copyrightStatus"), "Copyright Status");
    // The labels that were already fixed stay as they are.
    assert_eq!(label("title"), "Document Title");
    assert_eq!(label("copyright"), "Copyright Notice");
    assert_eq!(label("copyrightUrl"), "Copyright Info URL");
}

#[test]
fn other_form_dialogs_keep_their_generated_labels() {
    assert_eq!(super::dialogs::label_of("authorTitle"), "Author title");
    assert_eq!(super::dialogs::label_of("copyrightStatus"), "Copyright status");
}

/// The dialog belongs to the document it opened on. Activating another tab keeps its title, and
/// OK writes the metadata there, never into the newly active document.
#[test]
fn switching_documents_keeps_the_edit_on_the_document_it_opened_on() {
    let mut app = app_with_document("oracle.psd");
    let id = open_file_info(&mut app);
    assert_eq!(crate::dialogs::file_info_title(&app), "oracle");

    app.run("file.new", json!({"name": "other.psd", "width": 32, "height": 32})).unwrap();
    assert_eq!(app.session.active().map(|st| st.doc.name.as_str()), Some("other.psd"));

    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "title", json!("Oracle title")).unwrap();
    crate::dialogs::testing::confirm(&mut app, id).unwrap();

    let has_title = |name: &str| {
        app.session
            .documents()
            .iter()
            .find(|st| st.doc.name == name)
            .is_some_and(|st| st.doc.metadata.xmp.as_deref().is_some_and(|xmp| xmp.contains("Oracle title")))
    };
    assert!(has_title("oracle.psd"), "the edited document gets the metadata");
    assert!(!has_title("other.psd"), "the newly active document is untouched");
    assert_eq!(app.session.active().map(|st| st.doc.name.as_str()), Some("other.psd"), "the active tab is restored");
}
