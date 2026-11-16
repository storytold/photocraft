//! #14136: File > Print's Units menu lists Inches, Centimeters, Millimeters, Points and Picas, in
//! Photoshop's order, and Points (1/72 in) and Picas (12 pt) show and take Top, Left, Height and
//! Width. The measured document is 600 x 400 px at 300 ppi (2 x 1.333 in), centred on A4.
use super::custom_print_paper::{app_with_document, assert_no_system_listing, click, enter_length, fields, harness};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_ui_egui::{PhotocraftApp, file_ui::open_print};
use serde_json::{Value, json};

/// A4 as the reference's PDF printer reports it, in whole points (595 x 841 pt).
const REFERENCE_A4: [f64; 2] = [595.0, 841.0];

/// The Print dialog over the measured 600 x 400 px, 300 ppi document, with `settings` merged into
/// its fields.
fn measured(settings: Value) -> (Harness<'static, PhotocraftApp>, u64) {
    let (mut app, id) = app_with_document(json!({"width": 600, "height": 400, "resolution": 300, "name": "Units"}));
    app.ui.dialog_mut(id).unwrap().fields.extend(settings.as_object().unwrap().clone());
    (harness(app), id)
}

/// The number the length field `label` holds, in the units it shows.
fn shown_length(h: &Harness<'_, PhotocraftApp>, label: &str) -> f64 {
    h.get_by_role_and_label(egui::accesskit::Role::SpinButton, label).accesskit_node().numeric_value().unwrap()
}

/// Top, Left, Height and Width as the fields show them.
fn position_and_size(h: &Harness<'_, PhotocraftApp>) -> [f64; 4] {
    ["Top", "Left", "Height", "Width"].map(|label| shown_length(h, label))
}

/// Asserts each shown value is `expected` to within the f32 the fields keep.
fn assert_lengths(actual: [f64; 4], expected: [f64; 4], what: &str) {
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-4, "{what}: Top, Left, Height, Width {actual:?}, expected {expected:?}");
    }
}

#[test]
fn units_menu_lists_inches_centimeters_millimeters_points_picas_in_that_order() {
    let (mut h, _) = measured(json!({"units": "cm"}));
    click(&mut h, "Centimeters");
    let rows = ["Inches", "Centimeters", "Millimeters", "Points", "Picas"].map(|label| h.get_by_label(label).rect().center().y);
    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "the open Units menu must list the units top to bottom in this order: {rows:?}");
}

#[test]
fn points_and_picas_show_the_measured_values_for_the_document_centred_on_a4() {
    let (mut h, id) = measured(json!({"paper": REFERENCE_A4, "center": true, "scale": 100, "scaleToFit": false, "units": "cm"}));
    click(&mut h, "Centimeters");
    click(&mut h, "Points");
    assert_eq!(fields(&h, id)["units"], "pt");
    assert_lengths(position_and_size(&h), [372.5, 225.5, 96.0, 144.0], "Points");
    click(&mut h, "Points");
    click(&mut h, "Picas");
    assert_eq!(fields(&h, id)["units"], "pica");
    assert_lengths(position_and_size(&h), [372.5 / 12.0, 225.5 / 12.0, 8.0, 12.0], "Picas");
    // PhotoCraft's own A4 preset is the exact 595.28 x 841.89 pt sheet.
    let mut f = fields(&h, id);
    f.insert("paper".into(), json!("a4"));
    h.state_mut().ui.dialog_mut(id).unwrap().fields = f;
    h.run_steps(3);
    assert_lengths(position_and_size(&h), [(841.89 - 96.0) / 2.0 / 12.0, (595.28 - 144.0) / 2.0 / 12.0, 8.0, 12.0], "Picas on the A4 preset");
}

#[test]
fn lengths_typed_in_points_and_picas_reach_the_print_settings() {
    let (mut h, id) = measured(json!({"paper": REFERENCE_A4, "center": false, "top": 0.0, "left": 0.0, "scale": 100, "scaleToFit": false, "units": "pt"}));
    // Top and Left are kept in inches.
    enter_length(&mut h, "Top", "36");
    assert_eq!(fields(&h, id)["top"], 0.5, "36 pt is half an inch");
    click(&mut h, "Points");
    click(&mut h, "Picas");
    enter_length(&mut h, "Left", "9");
    assert_eq!(fields(&h, id)["left"], 1.5, "9 picas are 108 pt, an inch and a half");
    // The 8 pica (1.333 in) high print, made 16 picas high, is printed at twice the size.
    enter_length(&mut h, "Height", "16");
    assert_eq!(fields(&h, id)["scale"], 200.0);
    assert_lengths(position_and_size(&h), [3.0, 9.0, 16.0, 24.0], "Picas after typing");
}

#[test]
fn points_and_picas_are_remembered_for_the_next_print() {
    for (unit, label) in [("pt", "Points"), ("pica", "Picas")] {
        let output = std::env::temp_dir().join(format!("pc-print-units-{unit}-{}.pdf", std::process::id()));
        let (mut h, id) = measured(json!({"paper": "a4", "units": "cm", "output": output.to_string_lossy()}));
        click(&mut h, "Centimeters");
        click(&mut h, label);
        click(&mut h, "Print");
        assert!(output.exists(), "the print to PDF must have run");
        std::fs::remove_file(&output).unwrap();
        assert_eq!(h.state().session.file_menu.last_print.as_ref().unwrap()["units"], unit);
        let id2 = open_print(h.state_mut());
        assert_no_system_listing();
        assert_ne!(id2, id);
        h.state_mut().ui.dialog_mut(id2).unwrap().fields.insert("dryRun".into(), json!(true));
        h.run_steps(3);
        assert_eq!(fields(&h, id2)["units"], unit);
        assert!(h.query_all_by_value(label).next().is_some(), "the reopened Units menu must show {label}");
    }
}
