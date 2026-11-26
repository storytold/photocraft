//! The Curves dialog's gear menu (#14379): Save Preset…, Load Preset… and Delete Current Preset,
//! the separator between them, the greyed Delete row, and the engine commands behind the three
//! rows. The app is driven through its own dialog host with real clicks; its file dialogs are
//! scripted (`craft::host::ScriptedFiles`), as the file-dialog tests do.

use craft::host::{FileFilter, PickKind, PickRequest, Picked, ScriptedFiles};
use craft::ui::dialog::DialogId;
use egui::Vec2;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::presets::PresetsExt;
use photocraft_engine::presets::curves::{self, CurvePreset};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::adjust_editors::{preset_menu_entries, preset_menu_rows};
use crate::dialogs::adjust_dialog::{self, open_params};
use crate::{PhotocraftApp, Services};

/// The gear button's accessible name.
const GEAR: &str = "Preset options";
/// The menu's three rows, in the reference application's order.
const SAVE: &str = "Save Preset…";
const LOAD: &str = "Load Preset…";
const DELETE: &str = "Delete Current Preset";

type H = Harness<'static, Box<PhotocraftApp>>;

/// A temp folder one test owns, named after the test and this process.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("photocraft-curves-gear-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp folder");
    d
}

/// The curves a test puts in the dialog: a lifted midtone plus a red curve, so a channel lost
/// between the dialog and the file shows up.
fn lifted() -> Value {
    json!({"points": [[0.0, 0.0], [100.0, 150.0], [255.0, 255.0]], "red": [[0.0, 20.0], [255.0, 235.0]]})
}

/// `lifted()` as the engine writes the same curves back, which is what a preset file holds.
fn stored_curves() -> Value {
    photocraft_engine::adjust_params::to_params(
        &photocraft_engine::adjust_params::from_params("curves", &lifted(), None, photocraft_doc::ColorMode::Rgb).expect("the parameters parse"),
    )
}

/// An app with a document in `mode`, a scripted file service, and a writer that really writes.
fn app_in(files: Arc<ScriptedFiles>, mode: &str) -> Box<PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "mode": mode})).expect("a document");
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).expect("a selection");
    s.execute("edit.fill", json!({"color": "#b04020"})).expect("a fill");
    s.execute("select.deselect", json!({})).expect("deselect");
    let services = Services {
        files: Some(files),
        write: Some(Box::new(|path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    PhotocraftApp::new(s, services)
}

/// An app with an RGB document.
fn app(files: Arc<ScriptedFiles>) -> Box<PhotocraftApp> {
    app_in(files, "rgb")
}

/// A harness over `app(files)`, running the app's dialogs.
fn harness(app: Box<PhotocraftApp>) -> H {
    let h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(Vec2::new(1200.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h
}

/// The Curves dialog of `image.adjustments.curves`, open in a harness.
fn curves_dialog(files: Arc<ScriptedFiles>) -> (H, DialogId) {
    let mut h = harness(app(files));
    let id = adjust_dialog::open(h.state_mut(), "image.adjustments.curves").expect("the Curves dialog opens");
    h.run_steps(4);
    (h, id)
}

/// The newest request the file service was given.
fn last_request(files: &ScriptedFiles) -> PickRequest {
    files.requests().pop().expect("a file dialog was asked")
}

/// The user answers the newest open dialog with `picked`; the app polls it on the next pass.
fn answer(h: &mut H, files: &ScriptedFiles, picked: Picked) {
    files.answer(picked).expect("a dialog is open");
    h.state_mut().poll_file_dialog();
    h.run_steps(3);
}

/// Opens the gear's menu, leaving it alone when it is already open (the open menu answers to the
/// same name, so the button and the menu are then two nodes).
fn open_gear(h: &mut H) {
    if h.query_all_by_label(GEAR).count() > 1 {
        h.run_steps(1);
        return;
    }
    h.get_by_label(GEAR).click();
    h.run_steps(3);
}

/// The open menu's rows, top to bottom: (label, enabled).
fn menu_rows(h: &H) -> Vec<(String, bool)> {
    h.query_all_by_role(Role::MenuItem).map(|n| (n.accesskit_node().label().unwrap_or_default().to_string(), !n.accesskit_node().is_disabled())).collect()
}

/// The enabled flag of each open menu row, top to bottom.
fn rows_enabled(h: &H) -> Vec<bool> {
    menu_rows(h).iter().map(|(.., on)| *on).collect()
}

/// Picks one row of the open menu.
fn pick(h: &mut H, label: &str) {
    h.get_by_role_and_label(Role::MenuItem, label).click();
    h.run_steps(2);
}

/// The dialog's engine parameters now (`open_params` reads the one adjustment dialog that is open).
fn params(h: &H, _id: DialogId) -> Value {
    open_params(h.state()).expect("the Curves dialog is open")
}

/// A preset file for `name` written into `dir(test)`, with its bytes.
fn preset_file(test: &str, name: &str) -> (std::path::PathBuf, Vec<u8>) {
    let path = dir(test).join(format!("{name}.pcrv"));
    let bytes = curves::to_bytes(&CurvePreset { name: name.into(), curves: stored_curves() }).expect("a preset writes");
    std::fs::write(&path, &bytes).expect("the test can write");
    (path, bytes)
}

/// Answers the open Load dialog with the file `path` holding `bytes`.
fn choose_file(h: &mut H, files: &ScriptedFiles, path: &std::path::Path, bytes: &[u8]) {
    answer(h, files, Picked::Contents(vec![(path.to_string_lossy().to_string(), bytes.to_vec())]));
}

// -------------------------------------------------------------------------------- the menu itself

#[test]
fn the_gear_lists_save_load_and_delete_in_that_order() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, _id) = curves_dialog(files);
    assert_eq!(h.get_by_label(GEAR).accesskit_node().role(), Role::Button, "the gear is a button");
    let rows: Vec<(&str, bool)> = preset_menu_rows(None).iter().map(|(.., label, on)| (*label, *on)).collect();
    assert_eq!(rows, [(SAVE, true), (LOAD, true), (DELETE, false)], "the three rows in order, Delete greyed with Default current");
    open_gear(&mut h);
    let shown = menu_rows(&h);
    assert_eq!(shown.iter().map(|(label, ..)| label.as_str()).collect::<Vec<_>>(), [SAVE, LOAD, DELETE], "the menu lists exactly the three rows, in order");
    assert_eq!(rows_enabled(&h), vec![true, true, false], "Delete is greyed while Default is current");
}

#[test]
fn a_separator_divides_the_two_file_rows_from_the_delete_row() {
    let entries = preset_menu_entries(None);
    let separators: Vec<usize> = entries.iter().enumerate().filter(|(.., entry)| entry.is_separator()).map(|(at, ..)| at).collect();
    assert_eq!(separators, vec![2], "one separator, between the second and the third row");
    let labels: Vec<&str> = entries.iter().filter(|entry| entry.is_item()).map(|entry| entry.label().as_str()).collect();
    assert_eq!(labels, [SAVE, LOAD, DELETE], "the three rows keep their order around the separator");
    assert_eq!(entries.iter().filter(|entry| entry.is_item()).count(), 3, "the menu has exactly three rows to pick");
    assert!(
        entries.iter().filter(|entry| entry.is_item()).all(|entry| entry.is_enabled()) || entries.iter().any(|entry| !entry.is_enabled()),
        "the rows carry their own enabled flag"
    );
    let live = preset_menu_entries(Some("Mine"));
    assert_eq!(live.iter().filter(|entry| entry.is_item() && entry.is_enabled()).count(), 3, "with a user preset current, all three rows run");
    assert_eq!(
        preset_menu_entries(None).iter().filter(|entry| entry.is_item() && entry.is_enabled()).count(),
        2,
        "with Default current, the third row is greyed"
    );
}

#[test]
fn a_current_user_preset_makes_the_delete_row_live() {
    let rows: Vec<(&str, bool)> = preset_menu_rows(Some("My Curve")).iter().map(|(.., label, on)| (*label, *on)).collect();
    assert_eq!(rows, [(SAVE, true), (LOAD, true), (DELETE, true)], "with a user preset current, all three rows run");
}

#[test]
fn the_other_adjustment_dialogs_have_no_gear() {
    for command in ["image.adjustments.levels", "image.adjustments.exposure", "image.adjustments.hueSaturation"] {
        let files = Arc::new(ScriptedFiles::held());
        let mut h = harness(app(files));
        adjust_dialog::open(h.state_mut(), command).expect("the dialog opens");
        h.run_steps(4);
        assert!(h.query_all_by_label(GEAR).next().is_none(), "{command} shows no preset gear");
    }
}

// -------------------------------------------------------------------------------- colour modes

/// The ink curves of a CMYK document, as the engine writes them back.
fn ink_curves() -> Value {
    let cmyk = json!({"points": [[0.0, 10.0], [255.0, 245.0]], "cyan": [[0.0, 0.0], [255.0, 200.0]], "magenta": [[0.0, 30.0], [255.0, 255.0]]});
    photocraft_engine::adjust_params::to_params(
        &photocraft_engine::adjust_params::from_params("curves", &cmyk, None, photocraft_doc::ColorMode::Cmyk).expect("the ink parameters parse"),
    )
}

#[test]
fn a_preset_of_another_colour_mode_is_refused_naming_its_channels() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, id) = curves_dialog(files.clone());
    let before = params(&h, id);
    let bytes = curves::to_bytes(&CurvePreset { name: "Ink".into(), curves: ink_curves() }).expect("an ink preset writes");

    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("Ink.pcrv"), &bytes);

    let state = h.state();
    assert!(state.ui.status_error, "the load is reported as an error");
    for key in ["cyan", "magenta"] {
        assert!(state.ui.status.contains(key), "the message names the channel that has no place in this dialog: {}", state.ui.status);
    }
    assert_eq!(params(&h, id), before, "the dialog keeps the curves it had");
    assert!(state.session.presets().curves_presets.is_empty(), "and no preset was added to the user list");
    assert!(state.session.presets().curves_preset.is_none(), "and none was selected");
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, false], "so Delete is still greyed");
}

#[test]
fn an_rgb_preset_is_refused_by_a_grayscale_dialog_that_edits_only_the_composite() {
    let files = Arc::new(ScriptedFiles::held());
    let mut h = harness(app_in(files.clone(), "grayscale"));
    let id = adjust_dialog::open(h.state_mut(), "image.adjustments.curves").expect("the Curves dialog opens");
    h.run_steps(4);
    let before = params(&h, id);
    let bytes = curves::to_bytes(&CurvePreset { name: "Red lift".into(), curves: stored_curves() }).expect("a preset writes");

    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("Red lift.pcrv"), &bytes);

    let state = h.state();
    assert!(state.ui.status_error, "the gray dialog refuses the preset instead of applying half of it");
    assert!(state.ui.status.contains("red"), "the message names the channel this dialog has no place for: {}", state.ui.status);
    assert_eq!(params(&h, id), before, "and the dialog keeps the curve it had");
    assert!(state.session.presets().curves_presets.is_empty(), "no preset was added to the user list");
}

#[test]
fn a_gear_load_into_a_green_channel_dialog_refuses_a_preset_that_moves_red() {
    let files = Arc::new(ScriptedFiles::held());
    let mut application = app(files.clone());
    application.run("channel.target", json!({"channel": {"color": 1}})).expect("the Green channel is the edit target");
    let mut h = harness(application);
    let id = adjust_dialog::open(h.state_mut(), "image.adjustments.curves").expect("the Curves dialog opens");
    h.run_steps(4);
    let before = params(&h, id);
    let red_lift = curves::to_bytes(&CurvePreset { name: "Red lift".into(), curves: stored_curves() }).expect("a preset writes");

    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("Red lift.pcrv"), &red_lift);

    let state = h.state();
    assert!(state.ui.status_error, "the aimed dialog refuses a preset whose red curve it cannot show, instead of dropping it on the way to OK");
    assert!(state.ui.status.contains("red"), "the message names the channel the dialog does not edit: {}", state.ui.status);
    assert_eq!(params(&h, id), before, "and the dialog keeps the curve it had");
    assert!(state.session.presets().curves_presets.is_empty(), "nothing was added to the user's preset list");
    assert!(state.session.presets().curves_preset.is_none(), "and nothing became current: the refusal runs before the selection changes");
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, false], "and the gear is as it was before the file was chosen");

    // The channel the dialog is aimed at is all it edits: that curve, with the composite left
    // straight, loads, and the dialog keeps its own straight composite as it does today.
    let green = json!({"points": [[0.0, 0.0], [255.0, 255.0]], "green": [[0.0, 0.0], [128.0, 100.0], [255.0, 255.0]]});
    let stored_green = photocraft_engine::adjust_params::to_params(
        &photocraft_engine::adjust_params::from_params("curves", &green, None, photocraft_doc::ColorMode::Rgb).expect("the parameters parse"),
    );
    let bytes = curves::to_bytes(&CurvePreset { name: "Green lift".into(), curves: stored_green.clone() }).expect("a preset writes");
    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("Green lift.pcrv"), &bytes);
    assert!(!h.state().ui.status_error, "the aimed channel loads: {}", h.state().ui.status);
    let after = params(&h, id);
    assert_eq!(after["green"], stored_green["green"], "its green curve reaches the dialog");
    assert_eq!(after["points"], stored_green["points"], "and the composite stays the straight line this dialog holds");
    assert_eq!(h.state().session.presets().curves_presets.len(), 1, "and that preset is in the user's list");

    // A preset that bends the composite has no place here either: this dialog lists Green alone.
    let composite = json!({"points": [[0.0, 0.0], [100.0, 150.0], [255.0, 255.0]]});
    let lifted = curves::to_bytes(&CurvePreset { name: "Lift".into(), curves: composite }).expect("a preset writes");
    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("Lift.pcrv"), &lifted);
    let state = h.state();
    assert!(state.ui.status_error, "a composite curve is refused rather than dropped: {}", state.ui.status);
    assert!(state.ui.status.contains("composite"), "the message says which curve: {}", state.ui.status);
    assert_eq!(params(&h, id), after, "and the dialog keeps the curve it had");
    assert_eq!(state.session.presets().curves_presets.len(), 1, "and no second preset was recorded");
}

#[test]
fn a_gear_load_into_a_green_and_blue_target_refuses_a_preset_that_moves_red() {
    let files = Arc::new(ScriptedFiles::held());
    let mut application = app(files.clone());
    application.run("channel.target", json!({"colors": [1, 2]})).expect("Green and Blue are the edit target");
    let mut h = harness(application);
    let id = adjust_dialog::open(h.state_mut(), "image.adjustments.curves").expect("the Curves dialog opens");
    h.run_steps(4);
    let before = params(&h, id);
    // A preset of all three channels: the set lists Green and Blue and their combined entry, so the
    // Red curve it also carries has no line to show it on.
    let three = json!({
        "points": [[0.0, 0.0], [100.0, 150.0], [255.0, 255.0]],
        "red": [[0.0, 20.0], [255.0, 235.0]],
        "green": [[0.0, 0.0], [128.0, 100.0], [255.0, 255.0]],
        "blue": [[0.0, 0.0], [255.0, 200.0]],
    });
    let stored_three = photocraft_engine::adjust_params::to_params(
        &photocraft_engine::adjust_params::from_params("curves", &three, None, photocraft_doc::ColorMode::Rgb).expect("the parameters parse"),
    );
    let bytes = curves::to_bytes(&CurvePreset { name: "All three".into(), curves: stored_three.clone() }).expect("a preset writes");

    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("All three.pcrv"), &bytes);

    let state = h.state();
    assert!(state.ui.status_error, "the set refuses a preset whose red curve it has no line for, instead of dropping it on the way to OK");
    assert!(state.ui.status.contains("red"), "the message names the component this target left out: {}", state.ui.status);
    assert_eq!(params(&h, id), before, "and the dialog keeps the curve it had");
    assert!(state.session.presets().curves_presets.is_empty(), "nothing was added to the user's preset list");
    assert!(state.session.presets().curves_preset.is_none(), "and nothing became current: the refusal runs before the selection changes");

    // What the set does list - its two components and their combined entry - loads.
    let inside = json!({
        "points": [[0.0, 0.0], [100.0, 150.0], [255.0, 255.0]],
        "green": [[0.0, 0.0], [128.0, 100.0], [255.0, 255.0]],
        "blue": [[0.0, 0.0], [255.0, 200.0]],
    });
    let stored = photocraft_engine::adjust_params::to_params(
        &photocraft_engine::adjust_params::from_params("curves", &inside, None, photocraft_doc::ColorMode::Rgb).expect("the parameters parse"),
    );
    let bytes = curves::to_bytes(&CurvePreset { name: "Green and blue".into(), curves: stored.clone() }).expect("a preset writes");
    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("Green and blue.pcrv"), &bytes);
    assert!(!h.state().ui.status_error, "the set's own curves load: {}", h.state().ui.status);
    let after = params(&h, id);
    assert_eq!(after["green"], stored["green"], "its green curve reaches the dialog");
    assert_eq!(after["blue"], stored["blue"], "and so does its blue one");
    assert_eq!(after["points"], stored["points"], "and so does the combined entry the panel lists");
    assert_eq!(h.state().session.presets().curves_presets.len(), 1, "and that preset is in the user's list");
}

#[test]
fn a_preset_of_the_same_colour_mode_loads_into_the_dialog() {
    // The dialog's own mode: the same curves that `stored_curves()` holds, named rgb.
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, id) = curves_dialog(files.clone());
    let bytes = curves::to_bytes(&CurvePreset { name: "Mine".into(), curves: stored_curves() }).expect("a preset writes");
    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("Mine.pcrv"), &bytes);
    assert!(!h.state().ui.status_error, "a matching preset loads: {}", h.state().ui.status);
    assert_eq!(params(&h, id)["red"], stored_curves()["red"], "its red curve reaches the dialog");
}

// --------------------------------------------------------------------------- the row follows the list

#[test]
fn a_preset_changed_while_the_dialog_is_open_updates_the_delete_row() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, _id) = curves_dialog(files);
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, false], "Delete starts greyed");
    // A preset arrives through the control channel while the dialog stays open.
    h.state_mut().run("curves.presets.save", json!({"curves": stored_curves(), "name": "From the channel"})).expect("the command runs");
    h.run_steps(3);
    assert_eq!(h.state().session.presets().curves_presets.len(), 1, "the session holds the preset");
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, true], "the open dialog's Delete row lights up");
    // And back off when that preset is deleted the same way.
    h.state_mut().run("curves.presets.delete", json!({})).expect("the command runs");
    h.run_steps(3);
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, false], "and greys again");
}

#[test]
fn a_file_that_cannot_be_written_leaves_no_preset_behind() {
    let files = Arc::new(ScriptedFiles::held());
    let mut h = harness({
        let mut a = app(files.clone());
        a.services.write = Some(Box::new(|_path: &str, _bytes: &[u8]| Err("the disk is full".into())));
        a
    });
    let id = adjust_dialog::open(h.state_mut(), "image.adjustments.curves").expect("the dialog opens");
    h.run_steps(4);
    adjust_dialog::apply_params(h.state_mut(), id, &lifted());
    h.run_steps(2);

    open_gear(&mut h);
    pick(&mut h, SAVE);
    let path = dir("write-fails").join("Curves.pcrv");
    answer(&mut h, &files, Picked::Paths(vec![path.clone()]));

    let state = h.state();
    assert!(state.ui.status_error, "the failed write is reported");
    assert!(state.ui.status.contains("disk is full"), "with the writer's own words: {}", state.ui.status);
    assert!(state.session.presets().curves_presets.is_empty(), "no preset was added for a file that does not exist");
    assert!(state.session.presets().curves_preset.is_none(), "and nothing is selected");
    assert!(std::fs::metadata(&path).is_err(), "no file was written");
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, false], "so the Delete row stays greyed");
}

// --------------------------------------------------------------------------- points and pencil

/// A drawn curve at the same lifted midtone as `lifted()`, in the form the Pencil mode holds it:
/// 256 output levels, not control points.
fn pencil_params() -> Value {
    json!({"points": (0..=255).map(|k| (k as f64 * 1.5).min(255.0)).collect::<Vec<f64>>(), "pencil": true})
}

#[test]
fn a_point_preset_loaded_into_a_pencil_dialog_keeps_its_points() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, id) = curves_dialog(files.clone());
    adjust_dialog::apply_params(h.state_mut(), id, &pencil_params());
    h.run_steps(2);
    let drawn = params(&h, id);
    assert_eq!(drawn["pencil"], json!(true), "the dialog is in Pencil mode");
    assert_eq!(drawn["points"].as_array().map(Vec::len), Some(256), "a drawn curve is a map of 256 levels");

    let bytes = curves::to_bytes(&CurvePreset { name: "Points".into(), curves: stored_curves() }).expect("a point preset writes");
    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("Points.pcrv"), &bytes);

    let after = params(&h, id);
    assert_ne!(after["pencil"], json!(true), "the point preset leaves Pencil mode: {after}");
    assert_eq!(after["points"], stored_curves()["points"], "the preset's control points are what the dialog now edits");
    assert_eq!(after["points"].as_array().map(Vec::len), Some(3), "they stayed points, not a sampled map");
    assert_eq!(after["red"], stored_curves()["red"], "and the preset's red curve is there");
    assert!(!h.state().ui.status_error, "the load reports no error: {}", h.state().ui.status);
}

#[test]
fn a_pencil_preset_loaded_into_a_point_dialog_draws_instead() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, id) = curves_dialog(files.clone());
    adjust_dialog::apply_params(h.state_mut(), id, &lifted());
    h.run_steps(2);
    assert!(params(&h, id).get("pencil").is_none(), "the dialog starts in point mode");

    let bytes = curves::to_bytes(&CurvePreset { name: "Drawn".into(), curves: pencil_params() }).expect("a pencil preset writes");
    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, std::path::Path::new("Drawn.pcrv"), &bytes);

    let after = params(&h, id);
    assert_eq!(after["pencil"], json!(true), "the preset's Pencil mode wins over the dialog's");
    assert_eq!(after["points"].as_array().map(Vec::len), Some(256), "with the 256-level map it carries");
}

#[test]
fn a_load_answered_after_the_dialog_closed_runs_no_command() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, _id) = curves_dialog(files.clone());
    open_gear(&mut h);
    pick(&mut h, LOAD);
    let bytes = curves::to_bytes(&CurvePreset { name: "Late".into(), curves: stored_curves() }).expect("a preset writes");
    // Another adjustment takes the front while the file picker is still up.
    adjust_dialog::open(h.state_mut(), "image.adjustments.levels").expect("the Levels dialog opens");
    h.run_steps(3);
    choose_file(&mut h, &files, std::path::Path::new("Late.pcrv"), &bytes);

    let state = h.state();
    assert!(state.session.presets().curves_presets.is_empty(), "the late answer recorded no preset");
    assert!(state.session.presets().curves_preset.is_none(), "and selected none");
}

// ---------------------------------------------------------------------------- save, load and delete

#[test]
fn save_preset_asks_for_a_preset_file_and_writes_every_channel() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, id) = curves_dialog(files.clone());
    let path = dir("save").join("My Curve.pcrv");
    adjust_dialog::apply_params(h.state_mut(), id, &lifted());
    h.run_steps(2);
    assert_eq!(params(&h, id)["points"], stored_curves()["points"], "the dialog holds the lifted curve");

    open_gear(&mut h);
    pick(&mut h, SAVE);
    let asked = last_request(&files);
    assert_eq!(asked.kind, PickKind::Save, "Save Preset… asks where to save");
    let suggested = asked.file_name.clone().unwrap_or_default();
    assert!(suggested.ends_with("Curves.pcrv"), "Save Preset… suggests a curve preset file: {suggested}");
    answer(&mut h, &files, Picked::Paths(vec![path.clone()]));

    let bytes = std::fs::read(&path).expect("the preset file was written");
    let preset = curves::from_bytes(&bytes).expect("the file is a curve preset");
    assert_eq!(preset.name, "My Curve", "the preset is named after the file");
    assert_eq!(preset.curves, stored_curves(), "every channel of the dialog's curve is in the file");
    let state = h.state();
    assert_eq!(state.session.presets().curves_preset.as_deref(), Some("My Curve"), "the saved preset is selected");
    assert_eq!(state.session.presets().curves_presets.len(), 1, "and it is in the user list");
}

#[test]
fn load_preset_applies_the_file_and_makes_it_the_current_one() {
    let (path, bytes) = preset_file("load", "Bright");
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, id) = curves_dialog(files.clone());
    let before = params(&h, id);
    assert_ne!(before["points"], stored_curves()["points"], "the dialog starts on the default curve");

    open_gear(&mut h);
    pick(&mut h, LOAD);
    let asked = last_request(&files);
    assert_eq!(asked.kind, PickKind::Open, "Load Preset… asks for a file");
    assert_eq!(asked.filters, [FileFilter::new("Supported Files", &[curves::EXT])], "the picker offers curve preset files");
    choose_file(&mut h, &files, &path, &bytes);

    let now = params(&h, id);
    for key in ["points", "red", "blue"] {
        assert_eq!(now[key], stored_curves()[key], "{key} of the preset is in the dialog");
    }
    assert_eq!(h.state().session.presets().curves_preset.as_deref(), Some("Bright"), "the loaded preset is selected");
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, true], "with a user preset current, all three rows run");
}

#[test]
fn a_bad_preset_file_shows_an_error_and_changes_nothing() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, id) = curves_dialog(files.clone());
    let before = params(&h, id);
    for bytes in [b"{ not json".to_vec(), Vec::new(), vec![b'{'; 4096], br#"{"photoCraftCurvePreset":1,"curves":{"points":[[0,0]]}}"#.to_vec()] {
        open_gear(&mut h);
        pick(&mut h, LOAD);
        last_request(&files);
        answer(&mut h, &files, Picked::Contents(vec![("Broken.pcrv".into(), bytes)]));
        assert!(h.state().ui.status_error, "the bad file is reported");
        assert_eq!(params(&h, id), before, "the dialog keeps its curves");
        assert!(h.state().session.presets().curves_presets.is_empty(), "and the user list stays empty");
        open_gear(&mut h);
        assert_eq!(rows_enabled(&h), vec![true, true, false], "Delete stays greyed after a refused load");
    }
}

#[test]
fn delete_current_preset_drops_it_and_greys_the_row_again() {
    let (path, bytes) = preset_file("delete", "Bright");
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, _id) = curves_dialog(files.clone());
    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, &path, &bytes);
    assert_eq!(h.state().session.presets().curves_presets.len(), 1, "the loaded preset is in the list");

    open_gear(&mut h);
    pick(&mut h, DELETE);
    let state = h.state();
    assert!(state.session.presets().curves_presets.is_empty(), "the preset left the user list");
    assert!(state.session.presets().curves_preset.is_none(), "and nothing is current again");
    assert!(std::fs::metadata(&path).is_ok(), "the user's file stays on disk");
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, false], "Delete is greyed again");
}

#[test]
fn a_greyed_delete_row_runs_nothing() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, _id) = curves_dialog(files.clone());
    open_gear(&mut h);
    pick(&mut h, DELETE);
    let state = h.state();
    assert!(state.session.presets().curves_presets.is_empty(), "the greyed row deletes nothing");
    assert_eq!(files.open(), 0, "and it opens no file dialog");
}

#[test]
fn a_preset_loaded_once_is_still_current_when_the_dialog_opens_again() {
    let (path, bytes) = preset_file("reopen", "Kept");
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, id) = curves_dialog(files.clone());
    open_gear(&mut h);
    pick(&mut h, LOAD);
    choose_file(&mut h, &files, &path, &bytes);
    let ctx = h.ctx.clone();
    crate::dialogs::cancel(h.state_mut(), &ctx, id).expect("the dialog cancels");
    h.run_steps(3);

    adjust_dialog::open(h.state_mut(), "image.adjustments.curves").expect("the dialog opens again");
    h.run_steps(4);
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, true], "the preset loaded before is still current");
    assert_eq!(h.state().session.presets().curves_preset.as_deref(), Some("Kept"));
}

#[test]
fn a_cancelled_save_writes_nothing() {
    let files = Arc::new(ScriptedFiles::held());
    let (mut h, id) = curves_dialog(files.clone());
    adjust_dialog::apply_params(h.state_mut(), id, &lifted());
    h.run_steps(2);
    open_gear(&mut h);
    pick(&mut h, SAVE);
    let asked = last_request(&files);
    assert_eq!(asked.kind, PickKind::Save);
    answer(&mut h, &files, Picked::Cancelled);
    let state = h.state();
    assert!(state.session.presets().curves_presets.is_empty(), "a cancelled Save adds no preset");
    assert!(!state.ui.status_error, "cancelling the file dialog is not reported as an error");
    open_gear(&mut h);
    assert_eq!(rows_enabled(&h), vec![true, true, false], "and Delete stays greyed");
}
