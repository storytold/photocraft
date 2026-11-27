//! #13749: the Color Lookup Tables export form opens its grid row at 32 and takes 7 to 256. A typed
//! value outside them goes back to the closest end, once, with the range alert the hosted forms
//! share, and the form stays open; the export that follows writes that grid, points cubed rows. A
//! grid remembered under the row's old minimum lifts to 7 rather than silently to the 32 default.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::DialogFields;
use craft::ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const COMMAND: &str = "file.export.colorLookupTables";
/// The whole sentence the row raises for a value outside 7..256. The issue measured "A number between
/// 7 and 256 is required. Closest value inserted."; the framework's whole-number row words it as an
/// integer, the same sentence #14649 shipped for the Opacity row, and that wording is the form's.
const RANGE_ALERT: &str = "An integer between 7 and 256 is required. Closest value inserted.";

type H = Harness<'static, Box<PhotocraftApp>>;

/// `number` written with the decimal mark the fields read on this system, so a typed fraction does
/// not depend on the machine (#14649's way).
fn local(number: &str) -> String {
    let mark = craft::i18n::Region::system().decimal.filter(|c| *c == ',').unwrap_or('.');
    number.replace('.', &mark.to_string())
}

/// A fresh app with a 40 x 30 RGB document and one Invert adjustment.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    app
}

/// Opens the export form and runs the frame that shows it: a hosted form reads what it remembered there.
fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> DialogId {
    let reply = crate::menus::invoke(app, ctx, COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).expect("the form is open");
    ctx.run_ui(Default::default(), |ui| crate::dialogs::run(app, ui)).textures_delta.clear();
    id
}

/// One export run through the form, so it remembers its rows, and then that remembered grid rewritten
/// to `points`: the state of a form last used while the row still allowed 2.
fn remember_grid_then(app: &mut PhotocraftApp, path: &str, points: f64) {
    let ctx = app.ctx().clone();
    let id = open(app, &ctx);
    app.framework.dialogs.set_json_field(&ctx, id, "path", json!(path)).unwrap();
    crate::dialogs::confirm(app, &ctx, id).expect("the export ran");
    let key = SaveKey::named(format!("photocraft.{COMMAND}"));
    let mut saved = saved_value::<Option<DialogFields>>(&ctx, &key).expect("a completed export remembers its rows");
    saved.set("size", points);
    set_saved_value(&ctx, &key, Some(saved));
}

/// The export form open over a harness, hosted with its alerts, writing to `path`.
fn dialog(path: &str) -> (H, DialogId) {
    let mut app = app();
    let ctx = app.ctx().clone();
    let id = open(&mut app, &ctx);
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
    h.event(egui::Event::Text(local(text)));
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

/// The CUBE file's data rows: the lines that begin with a number.
fn rows(text: &str) -> usize {
    text.lines().filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit())).count()
}

/// A temp folder for one test's exports.
fn out_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-ui-clut-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_grid_row_opens_at_32_and_outside_7_to_256_the_closest_value_is_inserted_once() {
    let dir = out_dir("grid");
    let path = dir.join("grid.cube").to_string_lossy().into_owned();
    let (mut h, id) = dialog(&path);
    assert_eq!(fields(h.state(), id)["size"], 32, "the row opens at 32");
    for (typed, closest) in [("3", 7.0), ("300", 256.0)] {
        type_and_tab(&mut h, typed);
        assert_eq!(shown_points(&h), Some(closest), "typed {typed}: the closest value goes back into the field");
        let open = alerts(&h);
        assert_eq!(open.len(), 1, "typed {typed}: exactly one alert");
        assert_eq!(open[0], RANGE_ALERT, "typed {typed}: the whole sentence");
        assert!(h.state().framework.dialogs.open_dialogs().iter().any(|d| d.id == id), "typed {typed}: the export form stays open");
        dismiss_alerts(&mut h);
    }
    // A value inside the range is taken as typed and says nothing.
    type_and_tab(&mut h, "64");
    assert_eq!(shown_points(&h), Some(64.0), "typed 64");
    assert!(alerts(&h).is_empty(), "64 raises no alert");
    // Letters: today's number row restores what was there and raises nothing, the way #14649 pinned
    // it for Opacity. The reference's "Invalid numeric value. Default value inserted." is that row's
    // own behaviour, which photocraft cannot reach from this arm.
    type_and_tab(&mut h, "abc");
    assert_eq!(shown_points(&h), Some(64.0), "letters leave the value as it was");
    assert!(alerts(&h).is_empty(), "letters raise no alert");
    // Back to the low end for the export that follows: the corrected field is what OK sends, and the
    // file holds that many points cubed.
    type_and_tab(&mut h, "3");
    dismiss_alerts(&mut h);
    assert_eq!(fields(h.state(), id)["size"], 7, "OK sends the corrected value");
    let ctx = h.state().ctx().clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("LUT_3D_SIZE 7"), "the header names the grid");
    assert_eq!(rows(&text), 7 * 7 * 7, "points cubed rows");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The two rules are different on purpose, and both are pinned: the form's whole-number row ROUNDS
/// what is typed, while the command truncates it (9.6 alone exports 9). 6.9 is outside the range
/// either way, so the row lifts it to 7 with the alert.
#[test]
fn the_row_rounds_a_typed_fraction_that_the_command_would_truncate() {
    let dir = out_dir("round");
    let path = dir.join("round.cube").to_string_lossy().into_owned();
    let (mut h, id) = dialog(&path);
    type_and_tab(&mut h, "9.6");
    assert_eq!(shown_points(&h), Some(10.0), "typed 9.6: the row rounds to 10");
    assert!(alerts(&h).is_empty(), "10 is inside the range: no alert");
    assert_eq!(fields(h.state(), id)["size"], 10, "OK would send the rounded 10");
    type_and_tab(&mut h, "6.9");
    assert_eq!(shown_points(&h), Some(7.0), "typed 6.9: below the range, lifted to 7");
    assert_eq!(alerts(&h), [RANGE_ALERT.to_owned()], "typed 6.9 raises the range alert");
    dismiss_alerts(&mut h);
    let ctx = h.state().ctx().clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("LUT_3D_SIZE 7"), "the file holds the lifted grid");
    assert_eq!(rows(&text), 7 * 7 * 7, "points cubed rows");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_remembered_grid_below_7_lifts_to_7_rather_than_to_the_32_default() {
    let dir = out_dir("lift");
    let path = dir.join("lift.cube").to_string_lossy().into_owned();
    let mut app = app();
    let ctx = app.ctx().clone();
    remember_grid_then(&mut app, &path, 6.0);
    let id = open(&mut app, &ctx);
    assert_eq!(fields(&app, id)["size"].as_f64(), Some(7.0), "the remembered 6 comes back as the row's minimum");
    app.framework.dialogs.set_json_field(&ctx, id, "path", json!(path)).unwrap();
    // The file from the first run is gone: what comes back can only be this reopened form's export.
    std::fs::remove_file(&path).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("LUT_3D_SIZE 7"), "and OK writes that grid");
    assert_eq!(rows(&text), 7 * 7 * 7, "points cubed rows");
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the form remembered from an earlier export wins over the row's new default: the export the
/// user last made comes back, not 32. Seeded through the form itself (a size typed into the open
/// form, then OK), so it follows the same remember path a real session writes.
#[test]
fn a_remembered_grid_inside_the_range_survives_the_new_default() {
    let dir = out_dir("kept");
    let path = dir.join("kept.cube").to_string_lossy().into_owned();
    let mut app = app();
    let ctx = app.ctx().clone();
    let id = open(&mut app, &ctx);
    app.framework.dialogs.set_json_field(&ctx, id, "path", json!(path)).unwrap();
    app.framework.dialogs.set_json_field(&ctx, id, "size", json!(17)).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).expect("the export ran at 17");
    let id = open(&mut app, &ctx);
    assert_eq!(fields(&app, id)["size"].as_f64(), Some(17.0), "the remembered 17 is kept, not forced to 32");
    app.framework.dialogs.set_json_field(&ctx, id, "path", json!(path)).unwrap();
    // The file from the first run is gone: what comes back can only be this reopened form's export.
    std::fs::remove_file(&path).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("LUT_3D_SIZE 17"), "and OK writes the remembered grid again");
    assert_eq!(rows(&text), 17 * 17 * 17, "points cubed rows");
    let _ = std::fs::remove_dir_all(&dir);
}
