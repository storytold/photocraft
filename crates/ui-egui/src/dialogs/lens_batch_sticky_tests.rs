//! #13989: the batch Lens Correction dialog opens with the File Type, profile, corrections and Edge
//! of its last OK in this session, as in the reference application. Cancel and a refused OK keep
//! nothing; Use, the source list and the destination start over on every open.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::lens_batch_defaults_tests::{combo, ticked};
use super::lens_batch_layout_tests::{H, fields, fixture, pick, set, type_into};
use super::lens_batch_ok_state_tests::confirm;
use super::lens_batch_source_tests::{click_answered, harness_answering, input};
use crate::file_dialog::FileDialogAnswer;
use crate::state::DialogKind;

const CHECKS: [&str; 4] = ["Geometric Distortion", "Chromatic Aberration", "Vignette", "Auto Scale Image"];

/// File › Automate › Lens Correction… again, after the last one closed.
fn reopen(h: &mut H) {
    assert!(h.state().ui.dialogs.is_empty(), "the last dialog closed");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, super::COMMAND, json!({})).unwrap();
    h.run_steps(4);
}

/// File Type, Profile, the four checkboxes and Edge as shown.
fn shown(h: &H) -> (Option<String>, Option<String>, [bool; 4], Option<String>) {
    (combo(h, "File Type"), combo(h, "Profile"), CHECKS.map(|l| ticked(h, l)), combo(h, "Edge"))
}

fn changed() -> (Option<String>, Option<String>, [bool; 4], Option<String>) {
    (Some("TIFF".into()), Some("Generic".into()), [false, true, true, false], Some("White Color".into()))
}

/// Sets every remembered control away from its default by clicks: TIFF, Generic, each checkbox
/// flipped, White Color.
fn change_everything(h: &mut H) {
    pick(h, "File Type", "TIFF");
    pick(h, "Profile", "Generic");
    for label in CHECKS {
        h.get_by_role_and_label(Role::CheckBox, label).click();
        h.run_steps(2);
    }
    pick(h, "Edge", "White Color");
}

fn ok(h: &mut H) {
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run_steps(4);
}

/// The dialog open from the menu with `a.png` of a fresh `pc-lens-batch-<name>` folder listed
/// through Browse…, every remembered control changed and OK pressed.
fn after_ok(name: &str) -> (H, std::path::PathBuf) {
    let dir = fixture(name);
    let (mut h, _) = harness_answering(vec![FileDialogAnswer::Paths(vec![dir.join("a.png").to_string_lossy().into_owned()])]);
    click_answered(&mut h, "Browse…");
    change_everything(&mut h);
    ok(&mut h);
    (h, dir)
}

#[test]
fn ok_keeps_the_settings_for_the_next_open() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("sticky-ok");
        let (mut h, _) = harness_answering(vec![FileDialogAnswer::Paths(vec![dir.join("a.png").to_string_lossy().into_owned()])]);
        let first = shown(&h);
        click_answered(&mut h, "Browse…");
        change_everything(&mut h);
        ok(&mut h);
        let written: Vec<String> =
            std::fs::read_dir(dir.join("results")).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        reopen(&mut h);
        let again = shown(&h);
        let params = crate::filter_dialog::params_of(fields(&h));
        std::fs::remove_dir_all(&dir).ok();
        let defaults = (Some("PSD".into()), Some("Auto".into()), [true, false, false, true], Some("Transparency".into()));
        assert_eq!(first, defaults, "the first open keeps the #13980 defaults");
        assert!(written.iter().any(|n| n.starts_with("a.tif")), "OK ran the batch as TIFF: {written:?}");
        assert_eq!(again, changed(), "the next open shows the last OK's settings");
        assert_eq!(
            ["format", "profile", "correctDistortion", "correctCA", "correctVignette", "autoScale", "edge"].map(|k| params[k].clone()),
            [json!("tiff"), json!("generic"), json!(false), json!(true), json!(true), json!(false), json!("white")],
            "and OK would send them"
        );
    });
}

#[test]
fn cancel_changes_nothing() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut h, dir) = after_ok("sticky-cancel");
        reopen(&mut h);
        pick(&mut h, "File Type", "JPG");
        pick(&mut h, "Profile", "Auto");
        h.get_by_role_and_label(Role::CheckBox, "Vignette").click();
        h.run_steps(2);
        pick(&mut h, "Edge", "Black Color");
        h.get_by_role_and_label(Role::Button, "Cancel").click();
        h.run_steps(4);
        reopen(&mut h);
        let again = shown(&h);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(again, changed(), "the values of the last OK, not the cancelled ones");
    });
}

/// After a Folder run with its own destination, the next open is back on Files with an empty list
/// and an empty destination.
#[test]
fn use_the_list_and_the_destination_start_over() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("sticky-reset");
        let out = dir.join("out");
        let (mut h, _) = harness_answering(vec![FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned())]);
        pick(&mut h, "Use", "Folder");
        click_answered(&mut h, "Browse…");
        type_into(&mut h, "Output", &out.to_string_lossy());
        ok(&mut h);
        let written = out.join("a.psd").exists();
        reopen(&mut h);
        let use_shown = combo(&h, "Use");
        let (list, output) = (input(&h), fields(&h)["output"].clone());
        std::fs::remove_dir_all(&dir).ok();
        assert!(written, "the Folder run wrote its result");
        assert_eq!(use_shown.as_deref(), Some("Files"), "Use is back on Files");
        assert!(list.is_empty(), "the list starts empty: {list:?}");
        assert_eq!(output, json!(""), "the destination starts empty");
    });
}

/// `ui.dialog.confirm` keeps the settings as OK does; one refused for want of a source keeps
/// nothing.
#[test]
fn confirm_keeps_the_settings_and_a_refused_one_does_not() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("sticky-confirm");
        let (mut h, _) = harness_answering(Vec::new());
        set(&mut h, "input", json!([dir.join("a.png")]));
        set(&mut h, "output", json!(dir.join("out")));
        set(&mut h, "format", json!("png"));
        set(&mut h, "edge", json!("black"));
        let ran = confirm(&mut h);
        let written = dir.join("out").join("a.png").exists();
        reopen(&mut h);
        let kept = (combo(&h, "File Type"), combo(&h, "Edge"));
        set(&mut h, "format", json!("jpg"));
        set(&mut h, "edge", json!("edgeExtension"));
        let refused = confirm(&mut h);
        h.get_by_role_and_label(Role::Button, "Cancel").click();
        h.run_steps(4);
        reopen(&mut h);
        let after_refused = (combo(&h, "File Type"), combo(&h, "Edge"));
        std::fs::remove_dir_all(&dir).ok();
        assert!(written, "the confirm ran: {ran}");
        assert_eq!(kept, (Some("PNG".into()), Some("Black Color".into())), "confirm keeps what it ran with");
        assert_eq!(refused["ok"], json!(false), "no source: refused");
        assert_eq!(after_refused, kept, "the refused confirm kept nothing");
    });
}

/// A value its control cannot show (`same`, set by a script) is not kept: the next open shows the
/// default there, and keeps the rest.
#[test]
fn a_value_the_dialog_cannot_show_is_not_kept() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("sticky-unshown");
        let (mut h, _) = harness_answering(Vec::new());
        set(&mut h, "input", json!([dir.join("a.png")]));
        set(&mut h, "output", json!(dir.join("out")));
        set(&mut h, "format", json!("same"));
        set(&mut h, "correctCA", json!(true));
        let ran = confirm(&mut h);
        let written = dir.join("out").join("a.png").exists();
        reopen(&mut h);
        let (format, ca) = (combo(&h, "File Type"), ticked(&h, "Chromatic Aberration"));
        std::fs::remove_dir_all(&dir).ok();
        assert!(written, "the confirm ran in the source's own format: {ran}");
        assert_eq!(format.as_deref(), Some("PSD"), "File Type is back on its default");
        assert!(ca, "Chromatic Aberration, which it can show, is kept");
    });
}

/// OK whose run the command refuses (the destination cleared: `missing "output"`) keeps nothing.
#[test]
fn a_run_that_fails_keeps_nothing() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let dir = fixture("sticky-failed");
        let (mut h, _) = harness_answering(vec![FileDialogAnswer::Paths(vec![dir.join("a.png").to_string_lossy().into_owned()])]);
        click_answered(&mut h, "Browse…");
        pick(&mut h, "File Type", "TIFF");
        // Select the destination and delete it (typing "" over a selection inserts nothing).
        type_into(&mut h, "Output", "");
        h.key_press(egui::Key::Backspace);
        h.run_steps(2);
        let cleared = fields(&h)["output"].clone();
        ok(&mut h);
        let alerts: Vec<String> =
            h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect();
        let shown = format!("{alerts:?} {}", h.state().ui.status);
        h.state_mut().ui.dialogs.clear();
        reopen(&mut h);
        let format = combo(&h, "File Type");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(cleared, json!(""), "Output is cleared");
        assert!(shown.contains("missing \"output\""), "the run failed: {shown}");
        assert_eq!(format.as_deref(), Some("PSD"), "the failed run kept nothing");
    });
}

/// A recorded Actions step opens the command's generic form, which does not read the kept values.
#[test]
fn the_generic_form_of_the_command_ignores_them() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut h, dir) = after_ok("sticky-generic");
        std::fs::remove_dir_all(&dir).ok();
        let id = crate::filter_dialog::open(h.state_mut(), super::COMMAND).expect("the generic form opens");
        let form = &h.state().ui.dialogs.iter().find(|d| d.id == id).expect("it is open").fields;
        assert!(!super::owns(form), "not the batch dialog");
        assert_ne!(form.get("format"), Some(&json!("tiff")), "its File Type is not the kept one");
        assert_ne!(form.get("edge"), Some(&json!("white")), "nor its Edge");
    });
}
