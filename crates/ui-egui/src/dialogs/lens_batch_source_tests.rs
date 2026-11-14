//! #13979: the batch Lens Correction dialog's Source Files group lists the images to correct by
//! name (Use: Files or Folder, Browse…, Remove, Add Open Files) and OK sends their full paths.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::lens_batch_layout_tests::{H, entry, fields, fixture, harness, harness_with, pick, set, type_into};
use crate::PhotocraftApp;
use crate::dialogs::lens_batch_sources::UNSAVED;
use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
use crate::state::DialogKind;

pub(super) type Asked = std::rc::Rc<std::cell::RefCell<Vec<FileDialogRequest>>>;

/// [`harness`] whose file dialogs give `answers` in turn.
pub(super) fn harness_answering(answers: Vec<FileDialogAnswer>) -> (H, Asked) {
    let (show, asked) = crate::file_dialog::fake(answers.into_iter().map(Some).collect());
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() });
    (harness_with(app), asked)
}

/// Clicks `button` and lets the file dialog it opens answer.
pub(super) fn click_answered(h: &mut H, button: &str) {
    h.get_by_role_and_label(Role::Button, button).click();
    h.run_steps(2);
    let ctx = h.ctx.clone();
    h.state_mut().poll_file_dialog(&ctx, None);
    h.run_steps(2);
}

pub(super) fn click(h: &mut H, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

/// The paths OK sends.
pub(super) fn input(h: &H) -> Vec<String> {
    crate::filter_dialog::params_of(fields(h))["input"]
        .as_array()
        .expect("input is a list")
        .iter()
        .map(|p| p.as_str().unwrap_or_default().to_string())
        .collect()
}

/// The list's rows top to bottom: the file names it shows.
fn rows(h: &H, names: &[&str]) -> Vec<f32> {
    names.iter().map(|n| h.get_by_role_and_label(Role::Button, n).rect().top()).collect()
}

pub(super) fn disabled(h: &H, button: &str) -> bool {
    h.get_by_role_and_label(Role::Button, button).accesskit_node().is_disabled()
}

fn alerts(h: &H) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

#[test]
fn use_offers_files_then_folder_and_opens_on_files() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        assert!(h.query_by_label("Choose files for batch lens correction.").is_some());
        assert_eq!(h.get_by_role_and_label(Role::ComboBox, "Use").accesskit_node().value().as_deref(), Some("Files"));
        h.get_by_role_and_label(Role::ComboBox, "Use").click();
        h.run_steps(3);
        assert!(entry(&h, "Files").rect().top() < entry(&h, "Folder").rect().top(), "Files, then Folder");
        h.get_by_role_and_label(Role::ComboBox, "Use").click();
        h.run_steps(3);
        assert!(input(&h).is_empty(), "the list starts empty");
        for button in ["Browse…", "Remove", "Add Open Files"] {
            assert!(h.query_by_role_and_label(Role::Button, button).is_some(), "{button}");
        }
        assert!(h.query_by_label("Input").is_none(), "the folder field is gone");
    });
}

/// The reference case: `grid.jpg` listed, then Browse… picking `grid16.tif, gray.psd, grid.jpg`.
#[test]
fn files_browse_appends_the_picked_files_once_each() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let paths = |names: &[&str]| names.iter().map(|n| format!("/tmp/pc-lens-src/{n}")).collect::<Vec<_>>();
        let answers = vec![FileDialogAnswer::Paths(paths(&["grid.jpg"])), FileDialogAnswer::Paths(paths(&["grid16.tif", "gray.psd", "grid.jpg"]))];
        let (mut h, asked) = harness_answering(answers);
        click_answered(&mut h, "Browse…");
        click_answered(&mut h, "Browse…");
        assert_eq!(input(&h), paths(&["grid.jpg", "grid16.tif", "gray.psd"]));
        assert!(matches!(asked.borrow().as_slice(), [FileDialogRequest::Open { multiple: true, .. }, FileDialogRequest::Open { multiple: true, .. }]));
        let tops = rows(&h, &["grid.jpg", "grid16.tif", "gray.psd"]);
        assert!(tops.windows(2).all(|p| p[0] < p[1]), "the rows follow the list: {tops:?}");
        assert!(h.query_by_label("/tmp/pc-lens-src/grid.jpg").is_none(), "the rows show names, not paths");
    });
}

/// The reference case: a folder with `gray.psd, grid.jpg, grid16.tif, notes.txt` (and here a
/// sub-folder) lists `gray.psd, grid.jpg, grid16.tif`.
#[test]
fn folder_browse_lists_the_folders_images_by_name() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = std::env::temp_dir().join(format!("pc-lens-batch-folder-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for name in ["grid16.tif", "notes.txt", "gray.psd", "grid.jpg", "sub/inner.png"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let (mut h, asked) = harness_answering(vec![FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned())]);
        pick(&mut h, "Use", "Folder");
        click_answered(&mut h, "Browse…");
        let listed = input(&h);
        std::fs::remove_dir_all(&dir).ok();
        let want: Vec<String> = ["gray.psd", "grid.jpg", "grid16.tif"].iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect();
        assert_eq!(listed, want);
        assert!(matches!(asked.borrow().as_slice(), [FileDialogRequest::Folder { .. }]));
        let tops = rows(&h, &["gray.psd", "grid.jpg", "grid16.tif"]);
        assert!(tops.windows(2).all(|p| p[0] < p[1]), "alphabetical: {tops:?}");
        for left_out in ["notes.txt", "sub", "inner.png"] {
            assert!(h.query_by_label(left_out).is_none(), "{left_out} is not listed");
        }
    });
}

#[test]
fn remove_needs_a_selected_row_and_removes_it_in_both_modes() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = std::env::temp_dir().join(format!("pc-lens-batch-remove-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["d.png", "e.png"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let (mut h, _) = harness_answering(vec![FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned())]);
        set(&mut h, "input", json!(["/tmp/a.png", "/tmp/b.png", "/tmp/c.png"]));
        assert!(disabled(&h, "Remove"), "no row selected");
        click(&mut h, "b.png");
        assert!(!disabled(&h, "Remove"));
        // The selection is the row's file, wherever the list moves it.
        set(&mut h, "input", json!(["/tmp/c.png", "/tmp/b.png", "/tmp/a.png"]));
        click(&mut h, "Remove");
        assert_eq!(input(&h), ["/tmp/c.png", "/tmp/a.png"]);
        assert!(disabled(&h, "Remove"), "the selection went with its row");

        pick(&mut h, "Use", "Folder");
        click_answered(&mut h, "Browse…");
        click(&mut h, "d.png");
        click(&mut h, "Remove");
        let listed = input(&h);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(listed, [dir.join("e.png").to_string_lossy()]);
    });
}

/// A folder `ui.dialog.set` puts in `input` that cannot be read stays as it is, with the error in
/// the status bar, and OK reports the command's own error instead of correcting nothing.
#[test]
fn an_unreadable_folder_is_kept_and_ok_reports_the_error() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let missing = std::env::temp_dir().join(format!("pc-lens-batch-missing-{}", std::process::id())).to_string_lossy().into_owned();
        set(&mut h, "input", json!(missing));
        assert_eq!(crate::filter_dialog::params_of(fields(&h))["input"], json!(missing), "the folder is kept");
        assert!(h.state().ui.status.contains(&missing), "the status names the folder: {}", h.state().ui.status);
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        let shown = format!("{:?} {}", alerts(&h), h.state().ui.status);
        assert!(shown.contains(&missing), "OK reports the command's error: {shown}");
    });
}

/// Add Open Files with a saved `grid.jpg`, an untitled document and a modified saved one: only
/// `grid.jpg` is added, with the alert, and a second click adds nothing more.
#[test]
fn add_open_files_adds_saved_documents_once_and_alerts_for_the_others() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        assert!(disabled(&harness(), "Add Open Files"), "greyed with no document open");
        let mut session = photocraft_engine::Session::new();
        for (path, edit) in [(Some("/tmp/grid.jpg"), false), (None, false), (Some("/tmp/edited.png"), true)] {
            session.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
            session.active_mut().unwrap().path = path.map(str::to_string);
            if edit {
                session.execute("image.adjustments.invert", json!({})).unwrap();
            }
        }
        let mut h = harness_with(PhotocraftApp::new(session, crate::Services::default()));
        assert!(!disabled(&h, "Add Open Files"));
        let dialog_ok = h.get_by_role_and_label(Role::Button, "OK").rect();
        for _ in 0..2 {
            click(&mut h, "Add Open Files");
            assert_eq!(alerts(&h), [UNSAVED]);
            // The alert's own OK, not the dialog's below it.
            h.get_all_by_role_and_label(Role::Button, "OK").find(|n| n.rect() != dialog_ok).expect("the alert's OK").click();
            h.run_steps(3);
            assert!(alerts(&h).is_empty(), "OK closes the alert");
            assert_eq!(input(&h), ["/tmp/grid.jpg"]);
        }
    });
}

/// OK runs the command over the listed files: the full paths, in list order.
#[test]
fn ok_sends_the_listed_paths_in_list_order() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("sources");
        std::fs::copy(dir.join("a.png"), dir.join("b.png")).unwrap();
        let [a, b] = [dir.join("a.png"), dir.join("b.png")].map(|p| p.to_string_lossy().into_owned());
        let (mut h, _) = harness_answering(vec![FileDialogAnswer::Paths(vec![b.clone(), a.clone()])]);
        click_answered(&mut h, "Browse…");
        type_into(&mut h, "Output", &dir.join("out").to_string_lossy());
        assert_eq!(input(&h), [b, a], "OK sends the list as it is");
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        let written = ["a.psd", "b.psd"].map(|n| dir.join("out").join(n).exists());
        std::fs::remove_dir_all(&dir).ok();
        assert!(h.state().ui.dialogs.is_empty(), "OK closes the dialog");
        assert_eq!(written, [true, true], "both listed files are corrected");
    });
}
