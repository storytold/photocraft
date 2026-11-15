//! #14797: Image Size with Resample off. Turning it off switches the Width and Height fields from
//! Pixels or Percent to the physical unit they used last (Inches when none was used), so they stay
//! editable; Pixels is greyed in both unit menus while it is off, and turning Resample back on
//! keeps that unit. The method menu and the "Dimensions:" line are greyed, not hidden, and the
//! method menu still reads the method Resample goes back on with. Measured on a 400 x 300 px,
//! 72 ppi document: Width 28.22 cm keeps 400 x 300 px, Height 21.17 cm and 36.0028 px/in.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Map, Value, json};

use super::sizing;
use crate::PhotocraftApp;

/// The Dimensions readout while the measured 400 x 300 px document keeps its pixels.
const DIMENSIONS: &str = "400 px \u{d7} 300 px";

/// The units the Width and Height menus list, in order, Columns for Width only.
const UNIT_ROWS: [&str; 8] = ["Pixels", "Percent", "Inches", "Centimeters", "Millimeters", "Points", "Picas", "Columns"];

/// A live Image Size dialog for the measured document: 400 x 300 px at 72 ppi, the ruler unit
/// `rulers` and Resample on with method `interpolation`.
fn dialog(rulers: &str, interpolation: &str) -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72})).unwrap();
    app.session.execute("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": rulers})).unwrap();
    app.session.execute("prefs.set", json!({"path": "general.imageInterpolation", "value": interpolation})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = sizing::open(h.state_mut(), "image.imageSize").unwrap();
    h.run_steps(3);
    (h, id)
}

fn fields(h: &Harness<'static, PhotocraftApp>) -> Map<String, Value> {
    h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open")
}

fn num(f: &Map<String, Value>, k: &str) -> f64 {
    f.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// The shared Width/Height unit the dialog shows its fields in.
fn unit(h: &Harness<'static, PhotocraftApp>) -> String {
    fields(h).get("__unit").and_then(Value::as_str).unwrap_or("px").to_string()
}

/// Clicks the Resample checkbox.
fn click_resample(h: &mut Harness<'static, PhotocraftApp>) {
    h.get_by_role_and_label(Role::CheckBox, "Resample").click();
    h.run_steps(3);
}

/// The Width (`n` 0) or Height (`n` 1) unit menu.
fn unit_menu<'a>(h: &'a Harness<'static, PhotocraftApp>, n: usize) -> egui_kittest::Node<'a> {
    h.query_all_by_role_and_label(Role::ComboBox, "Units").nth(n).unwrap_or_else(|| panic!("the {n}. unit menu"))
}

/// The method menu, which shares its name with the Resample checkbox but not its role.
fn method_menu<'a>(h: &'a Harness<'static, PhotocraftApp>) -> egui_kittest::Node<'a> {
    h.query_all_by_role_and_label(Role::ComboBox, "Resample").next().expect("the method menu is drawn")
}

/// Opens the Width (`n` 0) or Height (`n` 1) unit menu.
fn open_unit_menu(h: &mut Harness<'static, PhotocraftApp>, n: usize) {
    unit_menu(h, n).click();
    h.run_steps(3);
    assert!(crate::widgets::dropdown_list_open(&h.ctx), "the unit list is open");
}

/// The open unit list's rows, top to bottom, each with whether it is greyed.
fn unit_rows(h: &Harness<'static, PhotocraftApp>) -> Vec<(String, bool)> {
    let mut rows: Vec<_> = h
        .query_all_by_role(Role::Button)
        .filter(|n| n.accesskit_node().label().is_some_and(|l| UNIT_ROWS.contains(&l.as_str())))
        .map(|n| (n.rect().top(), n.accesskit_node().label().unwrap_or_default(), n.accesskit_node().is_disabled()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, label, greyed)| (label, greyed)).collect()
}

/// Picks `label` from the open unit list.
fn pick_unit(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    h.query_all_by_label(label).find(|n| n.accesskit_node().role() == Role::Button).unwrap_or_else(|| panic!("the {label} row")).click();
    h.run_steps(3);
}

/// Types `text` into the `n`th number field (Width, Height, Resolution) and leaves it with Tab.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, n: usize, text: &str) {
    let field = h.query_all_by_role(Role::SpinButton).nth(n).map(|n| n.rect()).expect("the number field");
    h.hover_at(field.center());
    h.run_steps(1);
    h.drag_at(field.center());
    h.run_steps(1);
    h.drop_at(field.center());
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// The `n`th number field: (greyed, the value it holds).
fn field(h: &Harness<'static, PhotocraftApp>, n: usize) -> (bool, f64) {
    let node = h.query_all_by_role(Role::SpinButton).nth(n).unwrap_or_else(|| panic!("the {n}. number field"));
    (node.accesskit_node().is_disabled(), node.accesskit_node().numeric_value().unwrap_or(0.0))
}

/// The text a frame of the dialog paints, each with the colour it is painted in.
fn painted(f: &Map<String, Value>) -> Vec<(String, egui::Color32)> {
    let ctx = egui::Context::default();
    let mut fields = f.clone();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| sizing::body(ui, &mut fields));
    let texts: Vec<(String, egui::Color32)> = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => t.galley.job.sections.first().map(|sec| (t.galley.text().to_string(), sec.format.color)),
            _ => None,
        })
        .collect();
    // Nothing renders this frame, so its font texture deltas are never applied.
    out.textures_delta.clear();
    texts
}

/// The colour the Dimensions readout is painted in (None: the frame doesn't draw it). The bare
/// context takes the same fallback tokens the frame does.
fn dimensions_colour(f: &Map<String, Value>) -> Option<egui::Color32> {
    painted(f).into_iter().find(|(text, _)| text == DIMENSIONS).map(|(_, colour)| colour)
}

fn tokens() -> crate::theme::Tokens {
    crate::theme::Tokens::get(&egui::Context::default())
}

/// Done-when 1: with the fields in Pixels, Resample off switches them to the physical unit they
/// used last (here Centimeters) and they stay editable, with the pixels untouched.
#[test]
fn resample_off_switches_pixels_to_the_last_used_physical_unit() {
    let (mut h, _id) = dialog("cm", "bicubic");
    assert_eq!(unit_menu(&h, 0).value().as_deref(), Some("Centimeters"), "the dialog opens in the ruler unit");
    assert!(!field(&h, 0).0 && (field(&h, 0).1 - 14.111).abs() < 1e-3, "the Width field reads 14.111 cm: {}", field(&h, 0).1);
    open_unit_menu(&mut h, 0);
    pick_unit(&mut h, "Pixels");
    assert_eq!(unit(&h), "px", "the user put the fields back to Pixels");
    click_resample(&mut h);
    assert_eq!(unit(&h), "cm", "Resample off went back to Centimeters");
    assert_eq!(unit_menu(&h, 0).value().as_deref(), Some("Centimeters"), "and its menu reads so");
    assert_eq!((field(&h, 0).0, field(&h, 1).0), (false, false), "both fields are editable");
    let f = fields(&h);
    assert_eq!((num(&f, "width"), num(&f, "height"), num(&f, "resolution")), (400.0, 300.0, 72.0), "{f:?}");
}

/// Done-when 1's fallback: no physical unit was used in the dialog, so Pixels gives way to Inches
/// (the issue's fallback), still editable.
#[test]
fn resample_off_falls_back_to_inches_when_no_physical_unit_was_used() {
    let (mut h, _id) = dialog("pixels", "bicubic");
    assert_eq!(unit(&h), "px", "the ruler unit is Pixels");
    click_resample(&mut h);
    assert_eq!(unit(&h), "in", "Inches, the unit used when none was");
    assert_eq!((field(&h, 0).0, field(&h, 1).0), (false, false), "both fields are editable");
    assert!((field(&h, 0).1 - 5.556).abs() < 1e-3, "400 px at 72 ppi in inches: {}", field(&h, 0).1);
}

/// Done-when 2: Pixels is greyed in the Width and Height unit menus while Resample is off; every
/// other unit stays pickable, Percent included (its field keeps greying instead).
#[test]
fn pixels_is_greyed_in_both_unit_menus_while_resample_is_off() {
    let (mut h, _id) = dialog("pixels", "bicubic");
    open_unit_menu(&mut h, 0);
    let on = unit_rows(&h);
    assert!(on.iter().all(|(_, greyed)| !greyed), "every unit is pickable with Resample on: {on:?}");
    click_resample(&mut h);
    open_unit_menu(&mut h, 0);
    let want: Vec<(String, bool)> = UNIT_ROWS.iter().map(|l| (l.to_string(), *l == "Pixels")).collect();
    assert_eq!(unit_rows(&h), want, "the Width menu with Resample off");
    unit_menu(&h, 0).click();
    h.run_steps(3);
    assert!(!crate::widgets::dropdown_list_open(&h.ctx), "the Width list closed");
    open_unit_menu(&mut h, 1);
    let want_height: Vec<(String, bool)> = want.into_iter().filter(|(l, _)| l != "Columns").collect();
    assert_eq!(unit_rows(&h), want_height, "the Height menu with Resample off");
}

/// Not measured, so today's behaviour: Percent stays pickable while Resample is off, and its
/// field is the greyed one, as #2393 left it.
#[test]
fn percent_stays_pickable_and_greys_its_field_while_resample_is_off() {
    let (mut h, _id) = dialog("pixels", "bicubic");
    click_resample(&mut h);
    open_unit_menu(&mut h, 0);
    pick_unit(&mut h, "Percent");
    assert_eq!(unit(&h), "percent", "Percent is still pickable");
    assert!(field(&h, 0).0, "a Percent field is greyed while Resample is off");
    type_into(&mut h, 0, "50");
    let f = fields(&h);
    assert_eq!((num(&f, "width"), num(&f, "height"), num(&f, "resolution")), (400.0, 300.0, 72.0), "{f:?}");
}

/// Done-when 2's second half: Resample back on keeps the physical unit it switched to, and Pixels
/// is pickable again.
#[test]
fn resample_back_on_keeps_the_physical_unit() {
    let (mut h, _id) = dialog("cm", "bicubic");
    open_unit_menu(&mut h, 0);
    pick_unit(&mut h, "Pixels");
    click_resample(&mut h);
    assert_eq!(unit(&h), "cm", "Resample off switched to Centimeters");
    click_resample(&mut h);
    assert_eq!(unit(&h), "cm", "and back on keeps it");
    open_unit_menu(&mut h, 0);
    let rows = unit_rows(&h);
    assert!(rows.iter().all(|(_, greyed)| !greyed), "every unit is pickable again: {rows:?}");
}

/// Done-when 3: with Resample off the method menu is still drawn and greyed, and so is the
/// "Dimensions:" line; both are at full strength with Resample on.
#[test]
fn the_method_menu_and_the_dimensions_line_are_greyed_not_hidden() {
    let (mut h, _id) = dialog("pixels", "bilinear");
    let on = fields(&h);
    let menu = method_menu(&h);
    assert_eq!(menu.value().as_deref(), Some("Bilinear"), "the method menu reads Bilinear");
    assert!(!menu.accesskit_node().is_disabled(), "and is live with Resample on");
    assert_eq!(dimensions_colour(&on), Some(tokens().text), "the Dimensions line is at full strength");
    click_resample(&mut h);
    let menu = method_menu(&h);
    assert!(menu.accesskit_node().is_disabled(), "the method menu is greyed, not hidden");
    assert_eq!(menu.value().as_deref(), Some("Bilinear"), "and still reads its method");
    assert_eq!(dimensions_colour(&fields(&h)), Some(tokens().text_faint), "the Dimensions line is greyed");
    assert!(h.query_by_label(DIMENSIONS).is_some(), "its readout is still there");
}

/// Done-when 3's second half: the greyed menu keeps the method, and Resample back on picks it up.
#[test]
fn resample_back_on_shows_the_method_it_had() {
    let (mut h, _id) = dialog("pixels", "bilinear");
    assert_eq!(method_menu(&h).value().as_deref(), Some("Bilinear"));
    click_resample(&mut h);
    assert_eq!(method_menu(&h).value().as_deref(), Some("Bilinear"), "greyed, still Bilinear");
    click_resample(&mut h);
    let menu = method_menu(&h);
    assert!(!menu.accesskit_node().is_disabled(), "back on");
    assert_eq!(menu.value().as_deref(), Some("Bilinear"), "the same method");
    assert_eq!(fields(&h).get("resample"), Some(&json!("bilinear")));
}

/// Not measured: the method was changed and Resample then toggled. Today's rule stands - back on
/// takes the method the dialog opened with, the one Alt+Cancel restores too - and the greyed menu
/// reads exactly that method, so it never shows a method the checkbox will not put back.
#[test]
fn a_changed_method_comes_back_as_the_method_the_dialog_opened_with() {
    let (mut h, _id) = dialog("pixels", "bilinear");
    method_menu(&h).click();
    h.run_steps(3);
    h.query_all_by_label("Bicubic (smooth gradients)").find(|n| n.accesskit_node().role() == Role::Button).expect("the Bicubic row").click();
    h.run_steps(3);
    assert_eq!(fields(&h).get("resample"), Some(&json!("bicubic")), "the user picked Bicubic");
    click_resample(&mut h);
    assert_eq!(method_menu(&h).value().as_deref(), Some("Bilinear"), "greyed, it reads the method back-on will use");
    click_resample(&mut h);
    assert_eq!(fields(&h).get("resample"), Some(&json!("bilinear")), "back on: the method the dialog opened with");
}

/// Done-when 4, the measured case: on 400 x 300 px at 72 ppi, Resample off and Width 28.22 cm
/// keeps the pixels and only changes the resolution, to 36.0028 px/in; Height reads 21.17 cm
/// (its shown rounding is #14781).
#[test]
fn measured_resample_off_28_22_cm_keeps_the_pixels_at_36_0028_ppi() {
    let (mut h, id) = dialog("cm", "bicubic");
    open_unit_menu(&mut h, 0);
    pick_unit(&mut h, "Pixels");
    click_resample(&mut h);
    assert_eq!(unit(&h), "cm", "the fields went back to Centimeters");
    type_into(&mut h, 0, "28.22");
    let f = fields(&h);
    assert_eq!((num(&f, "width"), num(&f, "height")), (400.0, 300.0), "{f:?}");
    assert!((num(&f, "resolution") - 36.0028).abs() < 1e-3, "resolution {}", num(&f, "resolution"));
    assert!((field(&h, 1).1 - 21.165).abs() < 1e-3, "Height in cm: {}", field(&h, 1).1);
    assert!(!field(&h, 1).0, "Height stayed editable");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.size, photocraft_doc::Size::new(400, 300), "OK keeps the pixels");
    assert!((f64::from(doc.resolution_dpi) - 36.0028).abs() < 1e-3, "OK sets {}", doc.resolution_dpi);
}

/// Not measured, so today's behaviour stands: a dialog whose Resample was never toggled (set by
/// automation, as #2393 tests it) keeps its greyed Pixels fields and its Pixels unit.
#[test]
fn a_resample_off_state_reached_without_the_checkbox_keeps_greyed_pixels() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = sizing::open(h.state_mut(), "image.imageSize").unwrap();
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    d.fields.insert("resample".into(), json!("none"));
    d.fields.insert("__unit".into(), json!("px"));
    h.run_steps(3);
    assert_eq!(unit(&h), "px", "no checkbox click, no unit switch");
    assert!(field(&h, 0).0 && field(&h, 1).0, "both pixel fields stay greyed, as #2393 has them");
    type_into(&mut h, 0, "200");
    let f = fields(&h);
    assert_eq!((num(&f, "width"), num(&f, "height"), num(&f, "resolution")), (400.0, 300.0, 72.0), "{f:?}");
}

/// Not measured, so today's behaviour stands: a Width in Columns (a physical size Width-only)
/// keeps its Height in pixels greyed when Resample goes off, and the shared unit does not move.
#[test]
fn a_width_in_columns_keeps_its_pixels_and_its_greyed_height() {
    let (mut h, _id) = dialog("pixels", "bicubic");
    open_unit_menu(&mut h, 0);
    pick_unit(&mut h, "Columns");
    assert_eq!(unit_menu(&h, 0).value().as_deref(), Some("Columns"), "Width in Columns");
    click_resample(&mut h);
    assert_eq!(unit(&h), "px", "Height keeps Pixels, as the dialog's own comment has it");
    assert!(!field(&h, 0).0, "the Columns Width is editable");
    assert!(field(&h, 1).0, "the pixel Height is greyed");
}

/// Canvas Size has no Resample checkbox, so its unit menus keep every unit.
#[test]
fn canvas_size_unit_menus_keep_every_unit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    sizing::open(h.state_mut(), "image.canvasSize").unwrap();
    h.run_steps(3);
    open_unit_menu(&mut h, 0);
    let rows = unit_rows(&h);
    assert!(!rows.is_empty() && rows.iter().all(|(_, greyed)| !greyed), "no greyed unit: {rows:?}");
}
