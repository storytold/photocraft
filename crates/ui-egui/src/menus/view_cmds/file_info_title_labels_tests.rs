//! File Info (#13824): the dialog is titled with the active document's name without its
//! extension, and the Basic page's two remaining labels are title case. Other generated form
//! dialogs keep their sentence-case labels.

use super::*;

fn app_with_document(name: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"name": name, "width": 64, "height": 64})).unwrap();
    app.sync_views();
    app
}

fn open_file_info(app: &mut PhotocraftApp) -> crate::state::Dialog {
    crate::menus::invoke(app, &egui::Context::default(), "file.fileInfo", json!({})).unwrap();
    app.ui.dialogs.last().cloned().unwrap()
}

#[test]
fn the_title_is_the_document_name_without_its_extension() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        for (document, title) in [("oracle.psd", "oracle"), ("oracle", "oracle"), ("my.photo.png", "my.photo"), (".hidden", ".hidden")] {
            let mut app = app_with_document(document);
            let d = open_file_info(&mut app);
            assert_eq!(crate::dialogs::display_title(&d, &app), title, "{document}");
        }
    });
}

#[test]
fn the_author_title_and_copyright_status_labels_are_title_case() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut app = app_with_document("oracle.psd");
        let d = open_file_info(&mut app);
        let labels = d.fields["__labels"].as_object().unwrap();
        assert_eq!(labels["authorTitle"], "Author Title");
        assert_eq!(labels["copyrightStatus"], "Copyright Status");
        // The labels that were already fixed stay as they are.
        assert_eq!(labels["title"], "Document Title");
        assert_eq!(labels["copyright"], "Copyright Notice");
        assert_eq!(labels["copyrightUrl"], "Copyright Info URL");
    });
}

#[test]
fn other_form_dialogs_keep_their_generated_labels() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        assert_eq!(label_of("authorTitle"), "Author title");
        assert_eq!(label_of("copyrightStatus"), "Copyright status");
    });
}
