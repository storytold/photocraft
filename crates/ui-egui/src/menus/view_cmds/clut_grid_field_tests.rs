//! #13749: the Color Lookup Tables export form opens its grid row at 32 and takes 7 to 256. A typed
//! value outside them goes back to the closest end, once, with the range alert the hosted forms
//! share, and the form stays open; the export that follows writes that grid, points cubed rows.

use craft::ui::dialog::DialogId;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const COMMAND: &str = "file.export.colorLookupTables";
const RANGE_ALERT: &str = "between 7 and 256 is required. Closest value inserted.";

type H = Harness<'static, Box<PhotocraftApp>>;

/// A document with one Invert and its export form open, hosted with its alerts. `path` is the
/// file the form's File row writes to.
fn dialog(path: &str) -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).expect("the form is open");
    app.framework.dialogs.set_json_field(&ctx, id, "path", json!(path)).unwrap();
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(1200.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(4);
    (h, id)
}

/// The form's grid row.
fn grid_field<'a>(h: &'a H) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == "Grid Size"))
        .expect("the Grid Size field")
}

/// The number the grid row shows.
fn shown_points(h: &H) -> Option<f64> {
    grid_field(h).accesskit_node().numeric_value()
}

/// Types `text` over the grid row and leaves it with Tab, as the reference's check does.
fn type_and_tab(h: &mut H, text: &str) {
    let at = grid_field(h).rect().center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run_steps(1);
    }
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
}

/// The alerts open over the form.
fn alerts(h: &H) -> Vec<String> {
    h.state()
        .framework
        .dialogs
        .open_dialogs()
        .iter()
        .filter(|d| d.key.as_deref() == Some("alert"))
        .map(|d| d.fields.as_ref().and_then(|f| f.get("message")?.as_str().map(str::to_string)).unwrap_or_default())
        .collect()
}

/// Answers the open alerts, leaving the export form itself open.
fn dismiss_alerts(h: &mut H) {
    let ids: Vec<DialogId> = h.state().framework.dialogs.open_dialogs().iter().filter(|d| d.key.as_deref() == Some("alert")).map(|d| d.id).collect();
    for id in ids {
        h.state_mut().framework.dialogs.close_by_id(id);
    }
    h.run_steps(3);
}

/// The values the open form lists.
fn fields(app: &PhotocraftApp, id: DialogId) -> Value {
    app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields).expect("the form is open")
}

#[test]
fn the_grid_row_opens_at_32_and_outside_7_to_256_the_closest_value_is_inserted_once() {
    let dir = std::env::temp_dir().join(format!("photocraft-ui-clut-grid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("grid.cube").to_string_lossy().into_owned();
    let (mut h, id) = dialog(&path);
    assert_eq!(fields(h.state(), id)["size"], 32, "the row opens at 32");
    for (typed, closest) in [("3", 7.0), ("300", 256.0)] {
        type_and_tab(&mut h, typed);
        assert_eq!(shown_points(&h), Some(closest), "typed {typed}: the closest value goes back into the field");
        let open = alerts(&h);
        assert_eq!(open.len(), 1, "typed {typed}: exactly one alert");
        assert!(open[0].contains(RANGE_ALERT), "typed {typed}: {}", open[0]);
        assert!(h.state().framework.dialogs.open_dialogs().iter().any(|d| d.id == id), "typed {typed}: the export form stays open");
        dismiss_alerts(&mut h);
    }
    // A value inside the range is taken as typed and says nothing.
    type_and_tab(&mut h, "64");
    assert_eq!(shown_points(&h), Some(64.0), "typed 64");
    assert!(alerts(&h).is_empty(), "64 raises no alert");
    // Back to the low end for the export that follows: the clamped field is what OK sends, and the
    // file holds that many points cubed.
    type_and_tab(&mut h, "3");
    dismiss_alerts(&mut h);
    assert_eq!(fields(h.state(), id)["size"], 7, "OK sends the corrected value");
    let ctx = h.state().ctx().clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let rows = text.lines().filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit())).count();
    assert!(text.contains("LUT_3D_SIZE 7"), "{}", text.lines().next().unwrap_or_default());
    assert_eq!(rows, 7 * 7 * 7, "points cubed rows");
    let _ = std::fs::remove_dir_all(&dir);
}
