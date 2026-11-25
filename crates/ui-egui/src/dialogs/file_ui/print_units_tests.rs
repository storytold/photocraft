//! #14136: File > Print's Units menu lists Inches, Centimeters, Millimeters, Points and Picas, in
//! the reference application's order, and Points (1/72 in) and Picas (12 pt) show and take Top,
//! Left and the custom paper's Width and Height. The measured document is 600 x 400 px at 300 ppi
//! (2 x 1.333 in, 144 x 96 pt); centred on A4 as the reference's PDF printer reports it (595 x
//! 841 pt), its Top and Left are 372.5 and 225.5 pt.
use craft::ui::dialog::DialogId;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::custom_paper_tests::{app_with_document, click, enter_length, fields, harness, set_fields};
use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The Print dialog over the measured document, with `settings` set as its fields.
fn measured(settings: Value) -> (H, DialogId) {
    let (mut app, id) = app_with_document(json!({"width": 600, "height": 400, "resolution": 300, "name": "Units"}), None);
    set_fields(&mut app, id, settings);
    (harness(app), id)
}

/// The number the length field `label` shows, in the units it shows.
fn shown(h: &H, label: &str) -> f64 {
    let node = h.get_by_role_and_label(Role::SpinButton, label);
    let node = node.accesskit_node();
    node.numeric_value()
        .or_else(|| node.value().and_then(|text| text.split_whitespace().next().and_then(|n| n.replace(',', ".").parse().ok())))
        .unwrap_or_else(|| panic!("{label} shows no number"))
}

/// Asserts that the fields `labels` show `expected`, to the two decimals they keep.
fn assert_shown(h: &H, labels: &[&str], expected: &[f64], what: &str) {
    let actual: Vec<f64> = labels.iter().map(|label| shown(h, label)).collect();
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 0.006, "{what}: {labels:?} show {actual:?}, expected {expected:?}");
    }
}

#[test]
fn units_menu_lists_inches_centimeters_millimeters_points_picas_in_that_order() {
    let (mut h, _) = measured(json!({"units": "cm"}));
    click(&mut h, "Centimeters");
    let rows = ["Inches", "Centimeters", "Millimeters", "Points", "Picas"].map(|label| h.query_all_by_label(label).last().unwrap().rect().center().y);
    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "the open Units menu must list the units top to bottom in this order: {rows:?}");
}

#[test]
fn points_and_picas_show_the_measured_values() {
    // The document placed where centring puts it on the reference's A4: 372.5 pt down, 225.5 pt in.
    let (mut h, id) = measured(json!({
        "paper": "custom", "paperWidth": 595.0, "paperHeight": 841.0,
        "center": false, "top": 372.5 / 72.0, "left": 225.5 / 72.0, "units": "cm"
    }));
    let labels = ["Top", "Left", "Paper Width", "Paper Height"];
    click(&mut h, "Centimeters");
    click(&mut h, "Points");
    assert_eq!(fields(&h, id)["units"], "pt");
    assert_shown(&h, &labels, &[372.5, 225.5, 595.0, 841.0], "Points");
    click(&mut h, "Points");
    click(&mut h, "Picas");
    assert_eq!(fields(&h, id)["units"], "pica");
    assert_shown(&h, &labels, &[372.5 / 12.0, 225.5 / 12.0, 595.0 / 12.0, 841.0 / 12.0], "Picas");
}

#[test]
fn lengths_typed_in_points_and_picas_reach_the_print_settings() {
    let (mut h, id) = measured(json!({"center": false, "top": 0.0, "left": 0.0, "units": "pt"}));
    // Top and Left are kept in inches.
    enter_length(&mut h, "Top", "36");
    assert_eq!(fields(&h, id)["top"], 0.5, "36 pt is half an inch");
    click(&mut h, "Points");
    click(&mut h, "Picas");
    enter_length(&mut h, "Left", "9");
    assert_eq!(fields(&h, id)["left"], 1.5, "9 picas are 108 pt, an inch and a half");
    // The custom paper is kept in points.
    set_fields(h.state_mut(), id, json!({"paper": "custom", "paperWidth": 612.0, "paperHeight": 792.0}));
    h.run_steps(3);
    enter_length(&mut h, "Paper Width", "50");
    assert_eq!(fields(&h, id)["paperWidth"], 600.0, "50 picas are 600 pt");
}

#[test]
fn inches_typed_in_points_or_picas_stay_inches() {
    let (mut h, id) = measured(json!({"center": false, "top": 0.0, "left": 0.0, "units": "pt"}));
    enter_length(&mut h, "Top", "1in");
    assert_eq!(fields(&h, id)["top"], 1.0, "1 in typed in Points is 72 pt, one inch");
    click(&mut h, "Points");
    click(&mut h, "Picas");
    enter_length(&mut h, "Left", "2in");
    assert_eq!(fields(&h, id)["left"], 2.0, "2 in typed in Picas is 12 picas, two inches");
}
