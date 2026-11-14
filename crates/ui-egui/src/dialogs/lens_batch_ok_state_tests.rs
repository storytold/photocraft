//! #13986: OK in the batch Lens Correction dialog is greyed while the source list is empty, or
//! with no profile (`profile` `none`), as in the reference application; the corrections do not
//! count. `ui.dialog.confirm` and Enter refuse what the greyed OK does.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::lens_batch_layout_tests::{H, fields, fixture, harness, harness_with, pick, set};
use super::lens_batch_source_tests::{click, click_answered, disabled, harness_answering};
use crate::PhotocraftApp;
use crate::file_dialog::FileDialogAnswer;

fn ok_greyed(h: &H) -> bool {
    disabled(h, "OK")
}

/// `ui.dialog.confirm` on the open dialog, as scripts do: its reply.
pub(super) fn confirm(h: &mut H) -> Value {
    let dialog = h.state().ui.dialogs.last().expect("the dialog is open").id;
    let (req, _rx) = crate::ControlRequest::new("ui.dialog.confirm", json!({"dialog": dialog}));
    let ctx = h.ctx.clone();
    let crate::control::Outcome::Done(reply) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!("ui.dialog.confirm replies at once") };
    h.run_steps(2);
    reply
}

fn open(h: &H) -> bool {
    !h.state().ui.dialogs.is_empty()
}

#[test]
fn ok_is_greyed_until_a_file_is_listed_and_again_after_the_last_is_removed() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut h, _) = harness_answering(vec![FileDialogAnswer::Paths(vec!["/tmp/pc-lens-ok/a.png".into()])]);
        assert!(ok_greyed(&h), "greyed with the empty list it opens with");
        assert!(!disabled(&h, "Cancel"), "Cancel stays enabled");
        click_answered(&mut h, "Browse…");
        assert!(!ok_greyed(&h), "one listed file enables OK");
        click(&mut h, "a.png");
        click(&mut h, "Remove");
        assert!(ok_greyed(&h), "greyed again once the last file is removed");
    });
}

/// Folder + Browse… on a folder without images lists nothing, so OK stays greyed; a folder with
/// an image enables it.
#[test]
fn a_folder_without_images_keeps_ok_greyed() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let empty = std::env::temp_dir().join(format!("pc-lens-batch-empty-{}", std::process::id()));
        std::fs::remove_dir_all(&empty).ok();
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join("notes.txt"), b"").unwrap();
        let images = fixture("ok-folder");
        let answers = [&empty, &images].map(|d| FileDialogAnswer::FolderTo(d.to_string_lossy().into_owned()));
        let (mut h, _) = harness_answering(answers.into());
        pick(&mut h, "Use", "Folder");
        click_answered(&mut h, "Browse…");
        let after_empty = ok_greyed(&h);
        click_answered(&mut h, "Browse…");
        let after_images = ok_greyed(&h);
        std::fs::remove_dir_all(&empty).ok();
        std::fs::remove_dir_all(&images).ok();
        assert!(after_empty, "a folder with no images leaves OK greyed");
        assert!(!after_images, "a folder with an image enables OK");
    });
}

#[test]
fn add_open_files_enables_ok() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        session.active_mut().unwrap().path = Some("/tmp/grid.jpg".into());
        let mut h = harness_with(PhotocraftApp::new(session, crate::Services::default()));
        assert!(ok_greyed(&h), "an open document is not a source until it is added");
        click(&mut h, "Add Open Files");
        assert!(!ok_greyed(&h));
    });
}

/// Profile None stands for Match Best Profile off with no profile file; Auto (Match Best
/// Profile) or Generic enables OK again.
#[test]
fn profile_none_greys_ok_and_auto_or_generic_enable_it() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        set(&mut h, "input", json!(["/tmp/a.png"]));
        assert!(!ok_greyed(&h), "Auto, the default, is a profile choice");
        pick(&mut h, "Profile", "None");
        assert!(ok_greyed(&h), "greyed with no profile");
        pick(&mut h, "Profile", "Generic");
        assert!(!ok_greyed(&h), "Generic enables OK");
        pick(&mut h, "Profile", "None");
        pick(&mut h, "Profile", "Auto");
        assert!(!ok_greyed(&h), "Auto enables OK");
    });
}

/// Turning off Geometric Distortion, Chromatic Aberration and Vignette does not grey OK; nor, not
/// measured, does an empty destination (it stays as today).
#[test]
fn ok_stays_enabled_with_every_correction_off_and_no_destination() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        set(&mut h, "input", json!(["/tmp/a.png"]));
        set(&mut h, "output", json!(""));
        h.get_by_role_and_label(Role::CheckBox, "Geometric Distortion").click();
        h.run_steps(2);
        let corrections = ["correctDistortion", "correctCA", "correctVignette"].map(|k| fields(&h)[k].clone());
        assert_eq!(corrections, [json!(false), json!(false), json!(false)], "every correction is off");
        assert!(!ok_greyed(&h));
    });
}

/// `ui.dialog.confirm` refuses with no source or no profile, the dialog stays open and nothing is
/// written; with both it runs.
#[test]
fn confirm_refuses_without_sources_or_profile() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("ok-confirm");
        let out = dir.join("out");
        let mut h = harness();
        set(&mut h, "output", json!(out.to_string_lossy()));
        let no_sources = confirm(&mut h);
        let open_after_no_sources = open(&h);
        set(&mut h, "input", json!([dir.join("a.png")]));
        set(&mut h, "profile", json!("none"));
        let no_profile = confirm(&mut h);
        let open_after_no_profile = open(&h);
        let written_before = out.exists();
        set(&mut h, "profile", json!("generic"));
        let ran = confirm(&mut h);
        let written = out.join("a.psd").exists();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(no_sources, json!({"ok": false, "error": "Lens Correction needs at least one source file"}));
        assert_eq!(no_profile, json!({"ok": false, "error": "Lens Correction needs a lens profile: auto or generic"}));
        assert!(open_after_no_sources && open_after_no_profile, "a refused confirm keeps the dialog");
        assert!(!written_before, "the command did not run");
        assert_ne!(ran["ok"], json!(false), "with a source and a profile it runs: {ran}");
        assert!(written && !open(&h), "it corrected the file and closed the dialog");
    });
}

/// Enter does what OK does: nothing while OK is greyed, the run once a file is listed.
#[test]
fn enter_with_ok_greyed_runs_nothing() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("ok-enter");
        let out = dir.join("out");
        let mut h = harness();
        set(&mut h, "output", json!(out.to_string_lossy()));
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        let open_after_greyed = open(&h);
        let written_before = out.exists();
        set(&mut h, "input", json!([dir.join("a.png")]));
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        let written = out.join("a.psd").exists();
        std::fs::remove_dir_all(&dir).ok();
        assert!(open_after_greyed && !written_before, "Enter with OK greyed keeps the dialog and runs nothing");
        assert!(written && !open(&h), "Enter with a listed file runs the batch");
    });
}

/// The generic params form of the same command (a recorded step opened at a pause) is not this
/// dialog, so its OK is not greyed.
#[test]
fn the_generic_form_of_the_command_keeps_ok() {
    let generic = json!({"__command": super::COMMAND, "__filter": true, "input": [], "profile": "none"});
    assert!(super::can_confirm(generic.as_object().expect("an object")));
}
