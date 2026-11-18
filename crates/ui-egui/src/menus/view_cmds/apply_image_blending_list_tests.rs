//! #14479: Image ▸ Apply Image's Blending dropdown lists the reference application's modes in its
//! order and with its names, a line above each group, and still starts at Multiply.
//! Image ▸ Calculations keeps the list it has today: the issue marks it *not measured*.
//!
//! The reference list has 23 rows; Darker Color and Lighter Color are not in here because the
//! engine still refuses them for Apply Image (#14478), so this dialog offers 21: the same order,
//! without those two rows.
//!
//! Since #15449 the dialog is a framework parameter form and the row is a blend select
//! (`ParamKind::Blend`): the framework draws the group lines from the modes' Shift+Alt
//! positions, the same rule as every other blend list. Add and Subtract have no position, so the
//! line the reference draws between Hard Mix and Add needs a craft-libs feature; the test pins the
//! four lines the framework draws today.

use craft::ui::contract::Entry;
use craft::ui::patterns::blend_keys::{BlendKeyEntry, blend_entries};
use craft::ui::patterns::form_dialogs::{Param, ParamKind};
use craft::ui::runtime::env::Label;
use serde_json::json;

use super::channel_form;
use crate::PhotocraftApp;

/// The reference application's Apply Image Blending list, a group per row, as the command's keys.
const GROUPS: [&[&str]; 6] = [
    &["normal"],
    &["darken", "multiply", "colorBurn", "linearBurn"],
    &["lighten", "screen", "colorDodge", "linearDodge"],
    &["overlay", "softLight", "hardLight", "vividLight", "linearLight", "pinLight", "hardMix"],
    &["add", "subtract"],
    &["difference", "exclusion", "divide"],
];

/// The same groups as the reference application names them.
const GROUP_LABELS: [&[&str]; 6] = [
    &["Normal"],
    &["Darken", "Multiply", "Color Burn", "Linear Burn"],
    &["Lighten", "Screen", "Color Dodge", "Linear Dodge (Add)"],
    &["Overlay", "Soft Light", "Hard Light", "Vivid Light", "Linear Light", "Pin Light", "Hard Mix"],
    &["Add", "Subtract"],
    &["Difference", "Exclusion", "Divide"],
];

/// The list Image ▸ Calculations keeps (#14479 marks it not measured), untouched.
const CALCULATIONS: [&str; 16] = [
    "multiply",
    "screen",
    "normal",
    "overlay",
    "softLight",
    "hardLight",
    "darken",
    "lighten",
    "colorDodge",
    "colorBurn",
    "linearBurn",
    "linearDodge",
    "difference",
    "exclusion",
    "add",
    "subtract",
];

fn flat(groups: &[&[&str]]) -> Vec<String> {
    groups.iter().flat_map(|g| g.iter().map(|s| (*s).to_string())).collect()
}

/// The rows `command` opens with on a fresh document made by `new`, with `steps` run on it first.
fn rows_on(command: &str, new: serde_json::Value, steps: &[(&str, serde_json::Value)]) -> Vec<Param> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", new).unwrap();
    for (id, params) in steps {
        app.run(id, params.clone()).unwrap();
    }
    app.sync_views();
    crate::i18n::with_language(crate::i18n::Lang::EN, || channel_form(&app, command).expect("the dialog opens on a document").0)
}

fn rows(command: &str) -> Vec<Param> {
    rows_on(command, json!({"width": 8, "height": 8}), &[])
}

fn row<'a>(rows: &'a [Param], key: &str) -> &'a Param {
    rows.iter().find(|p| p.key == key).unwrap_or_else(|| panic!("the dialog has a {key} row"))
}

/// The Blending row's modes and the mode it opens on.
fn blend(rows: &[Param]) -> (Vec<(BlendKeyEntry<String>, Label)>, Option<String>) {
    match &row(rows, "blending").kind {
        ParamKind::Blend { modes, default, .. } => (modes.clone(), default.clone()),
        other => panic!("Blending is a blend select, not {other:?}"),
    }
}

fn keys(modes: &[(BlendKeyEntry<String>, Label)]) -> Vec<String> {
    modes.iter().map(|(e, _)| e.key.clone()).collect()
}

fn names(modes: &[(BlendKeyEntry<String>, Label)]) -> Vec<String> {
    modes.iter().map(|(_, l)| l.as_str().to_string()).collect()
}

#[test]
fn the_apply_image_list_is_the_reference_list_in_its_order() {
    let (modes, _) = blend(&rows("image.applyImage"));
    assert_eq!(keys(&modes), flat(&GROUPS), "the modes, in the reference order");
    // Nothing the engine refuses for this command is listed: Darker Color and Lighter Color wait
    // on #14478, and Dissolve plus the non-separable modes are not in the reference list at all.
    for pending in ["darkerColor", "lighterColor", "dissolve", "hue", "saturation", "color", "luminosity"] {
        assert!(!keys(&modes).contains(&pending.to_string()), "{pending} is not listed");
    }
}

#[test]
fn the_apply_image_list_names_the_modes_as_the_reference_application_does() {
    let (modes, _) = blend(&rows("image.applyImage"));
    assert_eq!(names(&modes), flat(&GROUP_LABELS));
}

#[test]
fn apply_image_still_starts_at_multiply() {
    assert_eq!(blend(&rows("image.applyImage")).1.as_deref(), Some("multiply"));
}

#[test]
fn every_listed_mode_blends_in_the_engine() {
    for key in keys(&blend(&rows("image.applyImage")).0) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        assert!(app.run("image.applyImage", json!({"blending": key})).is_ok(), "{key} is listed but the engine refuses it");
    }
}

#[test]
fn calculations_keeps_the_list_it_had() {
    let rows = rows("image.calculations");
    match &row(&rows, "blending").kind {
        ParamKind::Choice { options, default, .. } => {
            let ids: Vec<&str> = options.iter().map(|(id, _)| id.as_str()).collect();
            assert_eq!(ids, CALCULATIONS.to_vec(), "not measured: the shared list stays as it was");
            assert_eq!(default.as_deref(), Some("multiply"));
        }
        other => panic!("Calculations keeps a plain select, not {other:?}"),
    }
}

/// The lines of the open list, as the framework builds its entries for this row (the rule every
/// blend list uses): a separator above Darken, Lighten, Overlay and Difference.
#[test]
fn the_apply_image_list_draws_a_line_above_each_group_head() {
    let (modes, _) = blend(&rows("image.applyImage"));
    let entries_in: Vec<BlendKeyEntry<String>> = modes.iter().map(|(e, _)| e.clone()).collect();
    let entries: Vec<Entry<String>> = blend_entries(&entries_in, |e| Label::from(e.key.clone()));
    let mut lines = Vec::new();
    for (at, e) in entries.iter().enumerate() {
        if e.is_separator() {
            let before = entries[at - 1].value().cloned().unwrap_or_default();
            let after = entries[at + 1].value().cloned().unwrap_or_default();
            lines.push((before, after));
        }
    }
    let want: Vec<(String, String)> = [("normal", "darken"), ("linearBurn", "lighten"), ("linearDodge", "overlay"), ("subtract", "difference")]
        .iter()
        .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
        .collect();
    assert_eq!(lines, want, "a line above each group head; Hard Mix | Add waits for a craft-libs group rule");
}

/// A document layer named after one of the blending keys keeps its own name in Source layer, and
/// the Blending row keeps the mode's reference name: each row carries its own labels (#14479 review).
#[test]
fn a_layer_named_after_a_blend_key_leaves_both_lists_right() {
    let rows = rows_on(
        "image.applyImage",
        json!({"width": 8, "height": 8}),
        &[("edit.fill", json!({"color": "#c08040"})), ("layer.new.layer", json!({"name": "colorBurn"})), ("edit.fill", json!({"color": "#406080"}))],
    );
    match &row(&rows, "sourceLayer").kind {
        ParamKind::Choice { options, .. } => {
            let layer = options.iter().find(|(id, _)| id == "colorBurn").expect("the layer is offered");
            assert_eq!(layer.1.as_str(), "colorBurn", "by its own name");
            assert!(!options.iter().any(|(_, l)| l.as_str() == "Color Burn"), "and no blend-mode name leaks into Source layer");
        }
        other => panic!("Source layer is a plain select, not {other:?}"),
    }
    let (modes, _) = blend(&rows);
    let burn = modes.iter().find(|(e, _)| e.key == "colorBurn").expect("Color Burn is listed under its command key");
    assert_eq!(burn.1.as_str(), "Color Burn", "the Blending row shows the mode's reference name");
}

/// The issue marks "entries greyed for other colour modes or bit depths" *not measured*: the list
/// and its names are the same on a Grayscale 16-bit document as on the RGB one.
#[test]
fn the_list_and_its_names_are_the_same_in_grayscale_16_bit() {
    let rgb = blend(&rows("image.applyImage")).0;
    let gray =
        blend(&rows_on("image.applyImage", json!({"width": 8, "height": 8, "mode": "grayscale", "depth": 16}), &[("edit.fill", json!({"color": "#808080"}))]))
            .0;
    assert_eq!(keys(&gray), keys(&rgb), "no entry appears or disappears with the mode or the depth");
    assert_eq!(names(&gray), names(&rgb), "and every mode is named the same way");
}
