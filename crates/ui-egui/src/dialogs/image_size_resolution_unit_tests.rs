//! #14782: the Image Size dialog's Resolution unit list (Pixels/Inch, Pixels/Centimeter). The
//! stored `resolution` stays in pixels per inch; only the shown number converts.

use super::*;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

/// An open Image Size dialog for a 200 x 100 px document at `ppi`, and the dialog's id.
fn dialog(ppi: f64) -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100, "resolution": ppi})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), "image.imageSize").unwrap();
    h.run_steps(3);
    (h, id)
}

fn fields(h: &Harness<'static, PhotocraftApp>) -> Map<String, Value> {
    h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open")
}

fn set_field(h: &mut Harness<'static, PhotocraftApp>, key: &str, value: Value) {
    if let Some(d) = h.state_mut().ui.dialogs.first_mut() {
        d.fields.insert(key.into(), value);
    }
    h.run_steps(3);
}

/// The number the Resolution field shows.
fn shown(h: &Harness<'static, PhotocraftApp>) -> f64 {
    h.get_by_role_and_label(Role::SpinButton, "Resolution").accesskit_node().numeric_value().expect("a number")
}

/// Types `text` into the `n`th number field (Width, Height, Resolution) and leaves it with Tab.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, n: usize, text: &str) {
    let width = h.query_all_by_role(Role::SpinButton).nth(n).map(|n| n.rect()).expect("the number field");
    h.hover_at(width.center());
    h.run_steps(1);
    h.drag_at(width.center());
    h.run_steps(1);
    h.drop_at(width.center());
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn res(h: &Harness<'static, PhotocraftApp>) -> f64 {
    num(&fields(h), "resolution")
}

fn doc_res(h: &Harness<'static, PhotocraftApp>) -> f64 {
    f64::from(h.state().session.active().unwrap().doc.resolution_dpi)
}

/// The list sits right of Resolution, offers Pixels/Inch then Pixels/Centimeter, and the dialog
/// opens in Pixels/Inch.
#[test]
fn the_dialog_opens_in_pixels_per_inch_with_a_unit_list() {
    let (mut h, _) = dialog(72.0);
    assert_eq!(fields(&h).get("__resUnit"), None, "no unit stored: Pixels/Inch");
    assert_eq!(shown(&h), 72.0);
    let field = h.get_by_role_and_label(Role::SpinButton, "Resolution").rect();
    let list = h.query_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some("Pixels/Inch")).expect("the unit list shows Pixels/Inch");
    assert!(list.rect().left() >= field.right() - 1.0 && (list.rect().center().y - field.center().y).abs() < 10.0, "right of the field");
    list.click();
    h.run_steps(3);
    let first = h.query_by_label("Pixels/Inch").map(|n| n.rect().top());
    let second = h.query_by_label("Pixels/Centimeter").map(|n| n.rect().top());
    let (first, second) = (first.expect("Pixels/Inch is listed"), second.expect("Pixels/Centimeter is listed"));
    assert!(first < second, "Pixels/Inch comes first");
}

/// Opens the Resolution unit list (the one showing `from`) and picks `to` from it.
fn pick_unit(h: &mut Harness<'static, PhotocraftApp>, from: &str, to: &str) {
    h.query_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some(from)).expect("the Resolution unit list").click();
    h.run_steps(3);
    h.query_by_label(to).expect("the unit is listed").click();
    h.run_steps(3);
}

/// Picking a unit from the list changes what is shown, never what is stored.
#[test]
fn switching_the_unit_converts_the_display_only() {
    let (mut h, _) = dialog(72.0);
    pick_unit(&mut h, "Pixels/Inch", "Pixels/Centimeter");
    assert_eq!(fields(&h).get("__resUnit"), Some(&json!("ppcm")));
    assert!((shown(&h) - 28.346).abs() < 1e-3, "{}", shown(&h));
    assert_eq!(res(&h), 72.0);
    pick_unit(&mut h, "Pixels/Centimeter", "Pixels/Inch");
    assert_eq!(fields(&h).get("__resUnit"), Some(&json!("ppi")));
    assert_eq!(shown(&h), 72.0);
    assert_eq!(res(&h), 72.0);
}

/// 10 px/cm is 25.4 ppi, stored and applied to the document.
#[test]
fn a_value_typed_in_pixels_per_centimeter_is_stored_in_pixels_per_inch() {
    let (mut h, id) = dialog(72.0);
    set_field(&mut h, "__resUnit", json!("ppcm"));
    type_into(&mut h, 2, "10");
    assert!((res(&h) - 25.4).abs() < 1e-9, "{}", res(&h));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert!((doc_res(&h) - 25.4).abs() < 1e-4, "{}", doc_res(&h));
}

/// An untouched Resolution stays exactly the document's, in either unit and beside a Width edit.
#[test]
fn an_untouched_resolution_stays_exact() {
    let (mut h, id) = dialog(72.0);
    set_field(&mut h, "__resUnit", json!("ppcm"));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(doc_res(&h), 72.0);

    let (mut h, id) = dialog(72.0);
    set_field(&mut h, "__resUnit", json!("ppcm"));
    type_into(&mut h, 0, "400");
    assert_eq!(res(&h), 72.0);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(doc_res(&h), 72.0);
}

/// 1 ppi is the smallest value the field takes; shown in px/cm it is not clamped into a change.
#[test]
fn one_pixel_per_inch_is_not_clamped_when_shown_per_centimeter() {
    let (mut h, id) = dialog(1.0);
    set_field(&mut h, "__resUnit", json!("ppcm"));
    assert_eq!(res(&h), 1.0);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(doc_res(&h), 1.0);
}

/// Resampling in inches keeps the print size: 56.693 px/cm is about 144 ppi, twice 72, so the
/// pixels double.
#[test]
fn a_resolution_typed_per_centimeter_keeps_the_print_size() {
    let (mut h, _) = dialog(72.0);
    set_field(&mut h, "__unit", json!("in"));
    set_field(&mut h, "__resUnit", json!("ppcm"));
    type_into(&mut h, 2, "56.693");
    let f = fields(&h);
    let ratio = num(&f, "resolution") / 72.0;
    assert!((ratio - 2.0).abs() < 1e-3, "{f:?}");
    assert!((num(&f, "width") - 400.0).abs() < 0.5 && (num(&f, "height") - 200.0).abs() < 0.5, "{f:?}");
}
