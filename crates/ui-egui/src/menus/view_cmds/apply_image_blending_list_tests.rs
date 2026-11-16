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
    open_on(command, &json!({"width": 8, "height": 8}), &[])
}

/// The dialog `command` opens on a fresh document made by `new`, with `steps` run on it first.
fn open_on(command: &str, new: &Value, steps: &[(&str, Value)]) -> Map<String, Value> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", new.clone()).unwrap();
    for (id, params) in steps {
        app.run(id, params.clone()).unwrap();
    }
    app.sync_views();
    let ctx = egui::Context::default();
    let dialog = front(&mut app, &ctx, command, &json!({})).unwrap().unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.clone()
}

/// The fields of `command` on a document whose top pixel layer is named `layer_name`.
fn open_with_layer(command: &str, layer_name: &str) -> Map<String, Value> {
    open_on(
        command,
        &json!({"width": 8, "height": 8}),
        &[("edit.fill", json!({"color": "#c08040"})), ("layer.new.layer", json!({"name": layer_name})), ("edit.fill", json!({"color": "#406080"}))],
    )
}

/// The number of nodes labelled `text` that `form_body` draws for these fields, with the dropdown
/// of `combo` open (`open_list`) or shut. A fresh harness per call, so only one list is ever open.
fn label_nodes(fields: &Map<String, Value>, combo: &str, text: &str, open_list: bool) -> usize {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};

    let f = fields.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 1600.0)).build_ui_state(|ui, f: &mut Map<String, Value>| super::form_body(ui, f), f);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let mut found = 0;
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        h.run_steps(3);
        if open_list {
            h.get_by_role_and_label(Role::ComboBox, combo).click();
            h.run_steps(2);
        }
        found = h.query_all_by_label(text).count();
    });
    found
}

/// The text the button of the dropdown `combo` shows while its list is shut.
fn combo_value(fields: &Map<String, Value>, combo: &str) -> Option<String> {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};

    let f = fields.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 1600.0)).build_ui_state(|ui, f: &mut Map<String, Value>| super::form_body(ui, f), f);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let mut shown = None;
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        h.run_steps(3);
        shown = h.get_by_role_and_label(Role::ComboBox, combo).value().map(|v| v.to_string());
    });
    shown
}

/// The value the field `field` holds after the row `row` of the dropdown `combo` is clicked.
fn pick_row(fields: &Map<String, Value>, field: &str, combo: &str, row: &str) -> Value {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};

    let f = fields.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 1600.0)).build_ui_state(|ui, f: &mut Map<String, Value>| super::form_body(ui, f), f);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let mut picked = Value::Null;
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        h.run_steps(3);
        h.get_by_role_and_label(Role::ComboBox, combo).click();
        h.run_steps(2);
        h.get_all_by_label(row).last().unwrap_or_else(|| panic!("{row} is a row of the {combo} list")).click();
        h.run_steps(2);
        picked = h.state()[field].clone();
    });
    picked
}

fn choices(f: &Map<String, Value>) -> Vec<String> {
    f["__choices"]["blending"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

/// The label the dropdown of `field` draws for an option: the name the dialog scoped to that one
/// field, else the dialog's label for it, else the form's sentence-case label.
fn shown(f: &Map<String, Value>, field: &str, key: &str) -> String {
    f.get("__choiceLabels")
        .and_then(|m| m.get(field))
        .and_then(|m| m.get(key))
        .and_then(Value::as_str)
        .or_else(|| f.get("__labels").and_then(|l| l.get(key)).and_then(Value::as_str))
        .map_or_else(|| label_of(key), str::to_string)
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
        let labels: Vec<String> = keys.iter().map(|k| shown(&f, "blending", k)).collect();
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

/// A literal "---" in a list the dialog does not mark as grouped is a value, not a line: an action
/// or set named "---" in Batch, and a layer or channel named "---" in Apply Image, stay selectable.
#[test]
fn a_literal_triple_dash_is_kept_in_every_ungrouped_list() {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};

    // The Apply Image dialog marks only its Blending list as grouped.
    assert_eq!(open("image.applyImage")["__groupedChoices"], json!(["blending"]));

    let cases = [
        ("file.automate.batch", "action", "Action", json!(["Retouch", "---", "Invert"])),
        ("file.automate.batch", "set", "Set", json!(["Default Actions", "---", "Mine"])),
        ("image.applyImage", "sourceLayer", "Source layer", json!(["merged", "---", "Skin"])),
        ("image.applyImage", "sourceChannel", "Source channel", json!(["red", "---", "Alpha 1"])),
    ];
    for (command, key, combo, options) in cases {
        let first = options[0].clone();
        let mut f = Map::new();
        f.insert("__command".into(), json!(command));
        f.insert("__form".into(), json!(true));
        f.insert("__choices".into(), json!({ key: options }));
        f.insert(key.into(), first);
        let mut h = Harness::builder().with_size(egui::vec2(600.0, 600.0)).build_ui_state(|ui, f: &mut Map<String, Value>| super::form_body(ui, f), f);
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        crate::i18n::with_language(crate::i18n::Lang::EN, || {
            h.run_steps(3);
            h.get_by_role_and_label(Role::ComboBox, combo).click();
            h.run_steps(2);
            assert_eq!(h.query_all_by_label("---").count(), 1, "{command} {key}: the literal \"---\" is a row of the list");
            h.get_by_label("---").click();
            h.run_steps(2);
            assert_eq!(h.state()[key], json!("---"), "{command} {key}: it can be selected");
        });
    }
}

/// A document layer named after one of the blending keys must not steal that mode's label: the
/// Blending list shows "Color Burn" and the Source layer list shows the layer's own name. The two
/// dropdowns shared one label map, so the layer's name won in both (#14479 review).
#[test]
fn a_layer_named_after_a_blend_key_leaves_both_lists_right() {
    let f = open_with_layer("image.applyImage", "colorBurn");
    let layers: Vec<&str> = f["__choices"]["sourceLayer"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(layers.contains(&"colorBurn"), "the layer is offered by its own name: {layers:?}");
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        assert_eq!(label_nodes(&f, "Blending", "Color Burn", true), 1, "the Blending row shows the mode's reference name");
        assert_eq!(label_nodes(&f, "Blending", "colorBurn", true), 0, "and not the key the layer happens to be named");
        assert_eq!(label_nodes(&f, "Source layer", "colorBurn", true), 1, "the Source layer row shows the layer's own name");
        assert_eq!(label_nodes(&f, "Source layer", "Color Burn", true), 0, "and no blend-mode label leaks into it");
    });
}

/// A mode stored under the key an older dialog or a recorded action wrote ("colorBurn") still
/// selects its mode, and the dropdown shows the reference name for it (#14479 review).
#[test]
fn an_older_mode_key_still_selects_and_shows_its_name() {
    let mut f = open_with_layer("image.applyImage", "colorBurn");
    f.insert("blending".into(), json!("colorBurn"));
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        assert_eq!(pick_row(&f, "blending", "Blending", "Color Burn"), json!("colorBurn"), "the row is a label, the value stays the command key");
        assert_eq!(combo_value(&f, "Blending").as_deref(), Some("Color Burn"), "the shut box names the key it holds as the reference application does");
    });
}

/// The per-field option labels (`__choiceLabels`) change only the dropdown they name: another
/// field with the same option key keeps the shared `__labels` entry (#14479 review).
#[test]
fn a_scoped_option_label_reaches_only_the_field_it_names() {
    let mut f = Map::new();
    f.insert("__command".into(), json!("image.applyImage"));
    f.insert("__form".into(), json!(true));
    f.insert("__choices".into(), json!({"blending": ["multiply", "colorBurn"], "sourceLayer": ["merged", "colorBurn"]}));
    f.insert("blending".into(), json!("multiply"));
    f.insert("sourceLayer".into(), json!("merged"));
    f.insert("__labels".into(), json!({"colorBurn": "colorBurn"}));
    f.insert("__choiceLabels".into(), json!({"blending": {"colorBurn": "Color Burn"}}));
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        assert_eq!(label_nodes(&f, "Blending", "Color Burn", true), 1, "the field the dialog names uses its own label");
        assert_eq!(label_nodes(&f, "Source layer", "colorBurn", true), 1, "the other field keeps the shared label");
    });
}

/// The issue marks "entries greyed for other colour modes or bit depths" *not measured*: the list,
/// its lines and its names are the same on a Grayscale 16-bit document as on the RGB one.
#[test]
fn the_list_and_its_names_are_the_same_in_grayscale_16_bit() {
    let rgb = open("image.applyImage");
    let gray = open_on("image.applyImage", &json!({"width": 8, "height": 8, "mode": "grayscale", "depth": 16}), &[("edit.fill", json!({"color": "#808080"}))]);
    assert_eq!(choices(&gray), choices(&rgb), "no entry appears or disappears with the mode or the depth");
    assert_eq!(gray["__groupedChoices"], rgb["__groupedChoices"], "and the lines stay in the same list");
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let keys: Vec<String> = choices(&gray).into_iter().filter(|k| k != SEPARATOR).collect();
        assert_eq!(
            keys.iter().map(|k| shown(&gray, "blending", k)).collect::<Vec<String>>(),
            keys.iter().map(|k| shown(&rgb, "blending", k)).collect::<Vec<String>>(),
            "and every mode is named the same way"
        );
    });
}
