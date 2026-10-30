use serde_json::json;

use super::*;
use crate::menus::invoke as menu;

/// One 4 × 2 inch document at 100 ppi, with the rulers in `unit`.
fn app(unit: &str) -> (PhotocraftApp, egui::Context) {
    app_at_resolution(unit, 100)
}

fn app_at_resolution(unit: &str, dpi: u32) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200, "resolution": dpi})).unwrap();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit})).unwrap();
    app.sync_views();
    (app, egui::Context::default())
}

fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> u64 {
    menu(app, ctx, "view.newGuide", json!({})).unwrap()["dialog"].as_u64().unwrap()
}

fn set(app: &mut PhotocraftApp, d: u64, orientation: &str, position: &str) {
    let f = &mut app.ui.dialog_mut(d).unwrap().fields;
    f.insert("orientation".into(), json!(orientation));
    f.insert("position".into(), json!(position));
}

fn guides(app: &PhotocraftApp) -> (Vec<f64>, Vec<f64>) {
    let g = &app.session.active().unwrap().doc.guides;
    (g.horizontal.clone(), g.vertical.clone())
}

#[test]
fn opens_in_the_ruler_unit_and_takes_photoshop_units() {
    let (mut app, ctx) = app("cm");
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert!(owns(f), "New Guide has its own dialog, not the generic one");
    assert_eq!((f["orientation"].as_str(), f["position"].as_str()), (Some("horizontal"), Some("0 cm")));
    // Percent is of the width for a vertical guide.
    set(&mut app, d, "vertical", "50%");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app), (vec![], vec![200.0]));
    // The last guide opens next time, in the ruler unit (200 px at 100 ppi is 5.08 cm); a bare
    // number is in the ruler unit, as in Photoshop.
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["orientation"].as_str(), f["position"].as_str()), (Some("vertical"), Some("5.08 cm")));
    set(&mut app, d, "horizontal", "2.54");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app), (vec![100.0], vec![200.0]));
    // Any unit overrides the ruler unit.
    let d = open(&mut app, &ctx);
    set(&mut app, d, "horizontal", "36 pt");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app).0, vec![100.0, 50.0]);
}

#[test]
fn percent_rulers_open_at_zero_percent() {
    let (mut app, ctx) = app("percent");
    let d = open(&mut app, &ctx);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["position"], "0 %");
    set(&mut app, d, "horizontal", "25");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app).0, vec![50.0], "25% of the 200 px height");
}

#[test]
fn a_refused_position_reopens_the_dialog_without_a_guide() {
    let (mut app, ctx) = app("pixels");
    let d = open(&mut app, &ctx);
    set(&mut app, d, "vertical", "12 furlongs");
    let err = crate::dialogs::confirm(&mut app, d).unwrap_err();
    assert!(err.contains("not a length"), "{err}");
    assert_eq!(guides(&app), (vec![], vec![]));
    let again = app.ui.dialogs.iter().find(|d| owns(&d.fields)).expect("the dialog is open again");
    assert_eq!(again.fields["position"], "12 furlongs", "what was typed is kept");
    assert_eq!(again.fields["orientation"], "vertical");
    assert_eq!(again.fields["__error"].as_str(), Some(err.as_str()));
    // A refused position is not remembered.
    assert!(!app.session.prefs().dialogs.contains_key("view.newGuide"));
}

#[test]
fn remembered_values_that_no_longer_parse_are_ignored() {
    let (mut app, ctx) = app("mm");
    app.session.prefs.edit(|p| p.dialogs.insert("view.newGuide".into(), json!({"orientation": "diagonal", "positionPx": "far away"})));
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["orientation"].as_str(), f["position"].as_str()), (Some("horizontal"), Some("0 mm")));
}

fn color_of(app: &PhotocraftApp, vertical: bool, position: f64) -> Option<[u8; 3]> {
    app.session.active().unwrap().doc.guides.color(vertical, position)
}

#[test]
fn guides_store_the_chosen_colour_cyan_by_default() {
    let (mut app, ctx) = app("pixels");
    let d = open(&mut app, &ctx);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["color"], "cyan");
    set(&mut app, d, "vertical", "10");
    crate::dialogs::confirm(&mut app, d).unwrap();
    // Stored on the guide itself, so a later change of the preference colour leaves it alone.
    assert_eq!(color_of(&app, true, 10.0), Some([0x4a, 0xff, 0xff]));
    let d = open(&mut app, &ctx);
    set(&mut app, d, "vertical", "20");
    app.ui.dialog_mut(d).unwrap().fields.insert("color".into(), json!("magenta"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(color_of(&app, true, 20.0), Some([0xff, 0x00, 0xff]));
    // Custom… sends the picked colour; the list's choice and the custom colour open next time.
    let d = open(&mut app, &ctx);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["color"], "magenta");
    set(&mut app, d, "horizontal", "30");
    let f = &mut app.ui.dialog_mut(d).unwrap().fields;
    f.insert("color".into(), json!("custom"));
    f.insert("customColor".into(), json!("#abcdef"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(color_of(&app, false, 30.0), Some([0xab, 0xcd, 0xef]));
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["color"].as_str(), f["customColor"].as_str()), (Some("custom"), Some("#abcdef")));
}

#[test]
fn remembered_colours_that_are_not_on_the_list_are_ignored() {
    let (mut app, ctx) = app("pixels");
    app.session.prefs.edit(|p| p.dialogs.insert("view.newGuide".into(), json!({"color": "puce", "customColor": "#zzzzzz"})));
    let d = open(&mut app, &ctx);
    let f = &app.ui.dialog_mut(d).unwrap().fields;
    assert_eq!((f["color"].as_str(), f["customColor"].as_str()), (Some("cyan"), Some("#4affff")));
}

#[test]
fn the_last_position_reopens_in_whatever_the_ruler_unit_is_now() {
    let (mut app, ctx) = app("pixels");
    let d = open(&mut app, &ctx);
    set(&mut app, d, "horizontal", "1in");
    crate::dialogs::confirm(&mut app, d).unwrap();
    for (unit, shown) in [("pixels", "100 px"), ("inches", "1 in"), ("mm", "25.4 mm"), ("points", "72 pt"), ("picas", "6 pica"), ("percent", "50 %")] {
        app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit})).unwrap();
        let d = open(&mut app, &ctx);
        assert_eq!(app.ui.dialog_mut(d).unwrap().fields["position"], shown, "{unit}");
        crate::dialogs::cancel(&mut app, d).unwrap();
    }
}

#[test]
fn bare_positions_use_the_selected_unit_at_the_document_resolution() {
    for (unit, position, expected) in [("px", "12,5", 12.5), ("mm", "25,4", 300.0), ("in", "1,5", 450.0)] {
        let (mut app, ctx) = app_at_resolution("pixels", 300);
        let d = open(&mut app, &ctx);
        set(&mut app, d, "vertical", position);
        app.ui.dialog_mut(d).unwrap().fields.insert(UNIT.into(), json!(unit));
        crate::dialogs::confirm(&mut app, d).unwrap();
        assert_eq!(guides(&app), (vec![], vec![expected]), "{position} {unit}");
    }
}

#[test]
fn unit_conversion_preserves_fractional_negative_positions_and_percent_extents() {
    let (app, _) = app_at_resolution("pixels", 300);
    let mut position = "-123.456789 px".to_string();
    let mut from = Unit::Pixels;
    for to in [Unit::Millimeters, Unit::Inches, Unit::Pixels] {
        position = convert_position(&app, &position, from, to, true).unwrap();
        from = to;
    }
    assert!((Unit::parse_length(&position).unwrap().0 + 123.456789).abs() < 1e-8, "{position}");
    assert_eq!(convert_position(&app, "50%", Unit::Pixels, Unit::Pixels, true).unwrap(), "200 px");
    assert_eq!(convert_position(&app, "50%", Unit::Pixels, Unit::Pixels, false).unwrap(), "100 px");
    for invalid in ["", "-", "12 furlongs", "NaN", "inf"] {
        assert!(convert_position(&app, invalid, Unit::Pixels, Unit::Inches, true).is_none());
    }
}

#[test]
fn editing_an_existing_guide_accepts_the_selected_unit_and_undo_restores_it() {
    let (mut app, _) = app("pixels");
    app.run(COMMAND, json!({"orientation": "horizontal", "position": 10})).unwrap();
    crate::guide_select::select(&mut app, false, 0);
    let d = open_edit(&mut app).unwrap();
    set(&mut app, d, "vertical", "25,4");
    app.ui.dialog_mut(d).unwrap().fields.insert(UNIT.into(), json!("mm"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(guides(&app), (vec![], vec![100.0]));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(guides(&app), (vec![10.0], vec![]));
}

#[test]
fn unit_dropdown_converts_the_draft_and_bare_typing_uses_its_selection() {
    use egui_kittest::{Harness, kittest::Queryable};

    let (app, ctx) = app_at_resolution("pixels", 300);
    let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    let d = open(h.state_mut(), &ctx);
    set(h.state_mut(), d, "vertical", "300 px");
    h.run_steps(3);
    for (label, position, suffix) in [("Millimeters", "25.4 mm", "mm"), ("Inches", "1 in", "in"), ("Pixels", "300 px", "px")] {
        h.get_by_label("Units").click();
        h.run_steps(3);
        h.get_by_label(label).click();
        h.run_steps(3);
        let fields = &h.state().ui.dialogs.iter().find(|dialog| dialog.id == d).unwrap().fields;
        assert_eq!(fields["position"], position);
        assert_eq!(fields[UNIT], suffix);
        assert_eq!(guides(h.state()), (vec![], vec![]), "changing units only edits the draft");
    }
    // An explicit typed suffix also updates the dropdown; Tab returns to the position field.
    set(h.state_mut(), d, "vertical", "25.4 mm");
    h.run_steps(3);
    assert_eq!(h.get_by_label("Units").value().as_deref(), Some("Millimeters"));
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert!(h.get_by_label("Position").is_focused());
    h.event(egui::Event::Text("12,7".into()));
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(guides(h.state()), (vec![], vec![150.0]));
    assert!(h.state().ui.dialogs.is_empty());
}
