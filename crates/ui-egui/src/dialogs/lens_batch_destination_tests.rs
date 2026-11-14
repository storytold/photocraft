//! #13982: the batch Lens Correction destination is empty while the source list is, becomes
//! `results` next to the first source added (Browse…, Folder, Add Open Files) and keeps a folder
//! picked with Choose….

use std::path::Path;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::lens_batch_layout_tests::{H, fields, fixture, harness, harness_with, pick, set};
use super::lens_batch_source_tests::{click_answered, harness_answering, input};
use crate::PhotocraftApp;
use crate::file_dialog::FileDialogAnswer;

fn output(h: &H) -> Value {
    crate::filter_dialog::params_of(fields(h))["output"].clone()
}

fn shown(h: &H) -> String {
    h.get_by_role_and_label(Role::TextInput, "Output").accesskit_node().value().unwrap_or_default()
}

fn results(dir: impl AsRef<Path>) -> Value {
    json!(dir.as_ref().join("results").to_string_lossy())
}

fn paths(names: &[&str]) -> FileDialogAnswer {
    FileDialogAnswer::Paths(names.iter().map(|n| n.to_string()).collect())
}

/// [`harness_with`] over the saved, unchanged documents `docs` (in tab order), whose file dialogs
/// give `answers` in turn.
fn harness_documents(docs: &[&str], answers: Vec<FileDialogAnswer>) -> H {
    let mut session = photocraft_engine::Session::new();
    for path in docs {
        session.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        session.active_mut().unwrap().path = Some(path.to_string());
    }
    let (show, _) = crate::file_dialog::fake(answers.into_iter().map(Some).collect());
    harness_with(PhotocraftApp::new(session, crate::Services { file_dialog: Some(show), ..Default::default() }))
}

#[test]
fn the_destination_is_empty_until_the_first_file_is_browsed() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        assert_eq!(output(&harness()), json!(""), "empty with no document open");
        // Not the active document's folder either.
        let h = harness_documents(&["/tmp/pc-lens-doc/open.png"], vec![]);
        assert_eq!((output(&h), shown(&h)), (json!(""), String::new()));

        let (mut h, _) = harness_answering(vec![paths(&["/tmp/pc-lens-dest/a/grid.jpg"]), paths(&["/tmp/pc-lens-dest/b/gray.psd"])]);
        click_answered(&mut h, "Browse…");
        let first = results("/tmp/pc-lens-dest/a");
        assert_eq!(output(&h), first, "the first file's folder, in results");
        assert_eq!(json!(shown(&h)), first, "the field shows it");
        assert!(!shown(&h).ends_with(['/', '\\']), "no trailing separator");
        click_answered(&mut h, "Browse…");
        assert_eq!(input(&h).len(), 2);
        assert_eq!(output(&h), first, "more files do not change it");
    });
}

#[test]
fn a_folder_or_open_files_set_it_from_their_first_file() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (dir, other) = (fixture("dest-folder"), fixture("dest-folder-2"));
        let folders = [&dir, &other].map(|d| FileDialogAnswer::FolderTo(d.to_string_lossy().into_owned()));
        let (mut h, _) = harness_answering(folders.into());
        pick(&mut h, "Use", "Folder");
        click_answered(&mut h, "Browse…");
        let first = (input(&h), output(&h));
        click_answered(&mut h, "Browse…");
        let second = (input(&h), output(&h));
        for d in [&dir, &other] {
            std::fs::remove_dir_all(d).ok();
        }
        assert_eq!(first, (vec![dir.join("a.png").to_string_lossy().into_owned()], results(&dir)), "Folder: results in that folder");
        assert_eq!(second, (vec![other.join("a.png").to_string_lossy().into_owned()], results(&dir)), "another folder does not change it");

        let mut h = harness_documents(&["/tmp/pc-lens-dest/x/grid.jpg", "/tmp/pc-lens-dest/y/b.png"], vec![]);
        h.get_by_label("Add Open Files").click();
        h.run_steps(3);
        assert_eq!(input(&h).len(), 2);
        assert_eq!(output(&h), results("/tmp/pc-lens-dest/x"), "Add Open Files: next to the first document");

        // Add Open Files after a browsed file keeps the browsed file's folder.
        let mut h = harness_documents(&["/tmp/pc-lens-dest/x/grid.jpg"], vec![paths(&["/tmp/pc-lens-dest/a/gray.psd"])]);
        click_answered(&mut h, "Browse…");
        h.get_by_label("Add Open Files").click();
        h.run_steps(3);
        assert_eq!(input(&h).len(), 2);
        assert_eq!(output(&h), results("/tmp/pc-lens-dest/a"));
    });
}

#[test]
fn a_folder_picked_with_choose_is_kept() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        // Choose… before any file, then a file and the open documents.
        let answers = vec![FileDialogAnswer::FolderTo("/tmp/pc-lens-picked".into()), paths(&["/tmp/pc-lens-dest/a/grid.jpg"])];
        let mut h = harness_documents(&["/tmp/pc-lens-dest/x/b.png"], answers);
        click_answered(&mut h, "Choose…");
        click_answered(&mut h, "Browse…");
        assert_eq!(output(&h), json!("/tmp/pc-lens-picked"));
        h.get_by_label("Add Open Files").click();
        h.run_steps(3);
        assert_eq!(input(&h).len(), 2);
        assert_eq!(output(&h), json!("/tmp/pc-lens-picked"), "Add Open Files keeps it too");

        // A file, Choose…, then more files.
        let answers =
            vec![paths(&["/tmp/pc-lens-dest/a/grid.jpg"]), FileDialogAnswer::FolderTo("/tmp/pc-lens-picked".into()), paths(&["/tmp/pc-lens-dest/b/gray.psd"])];
        let (mut h, _) = harness_answering(answers);
        click_answered(&mut h, "Browse…");
        click_answered(&mut h, "Choose…");
        click_answered(&mut h, "Browse…");
        assert_eq!(input(&h).len(), 2);
        assert_eq!(output(&h), json!("/tmp/pc-lens-picked"), "Choose… replaces it and is kept");
    });
}

/// A destination set through `ui.dialog.set`, as scripts do, stays when the list gets its first
/// files; a folder set as `input` with no destination gives `results` in that folder.
#[test]
fn a_destination_set_by_a_script_is_kept() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("dest-script");
        let (mut h, _) = harness_answering(vec![paths(&["/tmp/pc-lens-dest/a/grid.jpg"])]);
        set(&mut h, "output", json!("/tmp/pc-lens-script"));
        set(&mut h, "input", json!(dir.to_string_lossy()));
        assert_eq!((input(&h).len(), output(&h)), (1, json!("/tmp/pc-lens-script")), "the folder is listed, the destination kept");

        let mut fresh = harness();
        set(&mut fresh, "input", json!(dir.to_string_lossy()));
        let from_folder = output(&fresh);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(from_folder, results(&dir), "no destination: results in the folder");

        let (mut h, _) = harness_answering(vec![paths(&["/tmp/pc-lens-dest/a/grid.jpg"])]);
        set(&mut h, "output", json!("/tmp/pc-lens-script"));
        click_answered(&mut h, "Browse…");
        assert_eq!(output(&h), json!("/tmp/pc-lens-script"), "the script's destination, not results");
    });
}

/// OK writes into the `results` folder the first file gave, creating it.
#[test]
fn ok_creates_the_results_folder() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("dest-run");
        let (mut h, _) = harness_answering(vec![FileDialogAnswer::Paths(vec![dir.join("a.png").to_string_lossy().into_owned()])]);
        click_answered(&mut h, "Browse…");
        // Checked before OK, so a wrong destination is never written to.
        assert_eq!(output(&h), results(&dir));
        assert!(!dir.join("results").exists());
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        let written = dir.join("results").join("a.psd").exists();
        std::fs::remove_dir_all(&dir).ok();
        assert!(h.state().ui.dialogs.is_empty(), "OK closes the dialog");
        assert!(written, "the results folder is created and written to");
    });
}
