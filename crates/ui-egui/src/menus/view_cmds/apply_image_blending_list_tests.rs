//! #14479: Image ▸ Apply Image's Blending dropdown lists the reference application's modes in its
//! order and in its six groups, a line between the groups, and still starts at Multiply.
//! Image ▸ Calculations keeps the list it has today: the issue marks it *not measured*.
//!
//! The reference list has 23 rows; Darker Color and Lighter Color are not in here because the
//! engine still refuses them for Apply Image (#14478), so this dialog offers 21 — the same order,
//! without those two rows.

use serde_json::{Map, Value, json};

use super::{front, label_of};
use crate::PhotocraftApp;

/// A `__choices` entry that is a line between the groups, never a value (#14479, as in the menu
/// catalog).
const SEPARATOR: &str = "---";

/// The reference application's Apply Image Blending list, a group per row, as the command's keys
/// (`Blending::parse` takes every one of them; `mode_key` gives the camelCase spelling).
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

/// The list Image ▸ Calculations keeps (#14479 marks it not measured): the one this file's
/// dialogs shared before the split, untouched.
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

fn flat<T: Clone>(groups: &[&[T]]) -> Vec<T> {
    groups.iter().flat_map(|g| g.iter().cloned()).collect()
}

/// The measured list with a line between each pair of groups.
fn reference_choices() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in GROUPS {
        if !out.is_empty() {
            out.push(SEPARATOR.to_string());
        }
        out.extend(g.iter().map(|k| (*k).to_string()));
    }
    out
}

/// The dialog `command` opens on a fresh RGB document, with its fields.
fn open(command: &str) -> Map<String, Value> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    let dialog = front(&mut app, &ctx, command, &json!({})).unwrap().unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.clone()
}

fn choices(f: &Map<String, Value>) -> Vec<String> {
    f["__choices"]["blending"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

/// The label the dropdown draws for an option: the dialog's own name for it, else the form's
/// sentence-case label.
fn shown(f: &Map<String, Value>, key: &str) -> String {
    f.get("__labels").and_then(|l| l.get(key)).and_then(Value::as_str).map_or_else(|| label_of(key), str::to_string)
}

#[test]
fn the_apply_image_list_is_the_reference_list_in_its_groups() {
    let f = open("image.applyImage");
    assert_eq!(choices(&f), reference_choices(), "the modes, in order, with a line between the six groups");
    // Nothing the engine refuses for this command is listed: Darker Color and Lighter Color wait
    // on #14478, and Dissolve plus the non-separable modes are not in the reference list at all.
    for pending in ["darkerColor", "lighterColor", "dissolve", "hue", "saturation", "color", "luminosity"] {
        assert!(!choices(&f).contains(&pending.to_string()), "{pending} is not listed");
    }
}

#[test]
fn the_apply_image_list_names_the_modes_as_the_reference_application_does() {
    let f = open("image.applyImage");
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let keys: Vec<String> = choices(&f).into_iter().filter(|k| k != SEPARATOR).collect();
        assert_eq!(keys.len(), flat(&GROUP_LABELS).len(), "one label per entry");
        let labels: Vec<String> = keys.iter().map(|k| shown(&f, k)).collect();
        assert_eq!(labels, flat(&GROUP_LABELS));
    });
}

#[test]
fn apply_image_still_starts_at_multiply() {
    assert_eq!(open("image.applyImage")["blending"], json!("multiply"));
}

#[test]
fn every_listed_mode_blends_in_the_engine() {
    for key in choices(&open("image.applyImage")).iter().filter(|k| *k != SEPARATOR) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        assert!(app.run("image.applyImage", json!({"blending": key})).is_ok(), "{key} is listed but the engine refuses it");
    }
}

#[test]
fn calculations_keeps_the_list_it_had() {
    let f = open("image.calculations");
    assert_eq!(choices(&f), CALCULATIONS.to_vec(), "not measured: the shared list stays as it was");
    assert!(!choices(&f).iter().any(|k| k == SEPARATOR), "and it has no group lines");
    assert_eq!(f["blending"], json!("multiply"));
}

/// The line between the groups, as drawn: the gaps between the open list's rows.
#[test]
fn the_apply_image_dropdown_draws_a_line_between_each_group() {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.sync_views();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 1600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                crate::dialogs::show(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let ctx = h.ctx.clone();
        let dialog = crate::menus::invoke(h.state_mut(), &ctx, "image.applyImage", json!({})).unwrap()["dialog"].as_u64().unwrap();
        h.run_steps(4);
        assert_eq!(h.state_mut().ui.dialog_mut(dialog).unwrap().fields["blending"], json!("multiply"), "the dialog is open at Multiply");
        h.get_by_role_and_label(Role::ComboBox, "Blending").click();
        h.run_steps(2);
        assert!(h.ctx.memory(|m| m.layer_ids().any(|l| l.order == egui::Order::Foreground)), "the list is open");
        // Each mode's row in the open list: its label's last node (the box itself shows the
        // chosen mode's first). Rows scrolled out of the list keep their place in it.
        let rows: Vec<egui::Rect> =
            flat(&GROUP_LABELS).iter().map(|l| h.get_all_by_label(l).map(|n| n.rect()).next_back().unwrap_or_else(|| panic!("{l} is in the list"))).collect();
        let gaps: Vec<f32> = rows.windows(2).map(|w| w[1].top() - w[0].bottom()).collect();
        let row_gap = gaps.iter().copied().fold(f32::INFINITY, f32::min);
        let lines: Vec<(&str, &str)> =
            gaps.iter().enumerate().filter(|(_, g)| **g > row_gap + 2.0).map(|(i, _)| (flat(&GROUP_LABELS)[i], flat(&GROUP_LABELS)[i + 1])).collect();
        assert_eq!(
            lines,
            [("Normal", "Darken"), ("Linear Burn", "Lighten"), ("Linear Dodge (Add)", "Overlay"), ("Hard Mix", "Add"), ("Subtract", "Difference"),],
            "row gaps {gaps:?}"
        );
    });
}
