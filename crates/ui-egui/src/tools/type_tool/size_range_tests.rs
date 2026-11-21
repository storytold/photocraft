//! #12053: the Type options bar's size field takes 0.01 to 1296 pt. A size typed outside the range is kept as the
//! nearest limit and, once the field commits, one modal alert with OK names the range and says "Closest value
//! inserted."; the change is one history state. The same field with its Type unit in pixels or millimetres keeps
//! today's silent clamp (*not measured*), and the size menu's rows are unchanged.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// A 100 x 100 document with one 20 pt text layer (its id). The harness draws the Type tool's options bar and,
/// as its dialog host, the frame that hosts the alerts.
fn bar() -> (Harness<'static, Box<PhotocraftApp>>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("type.create", json!({"text": "Hello", "size": 20, "x": 30, "y": 60})).unwrap()["layer"].as_u64().unwrap();
    app.ui.tool = Tool::Type;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).with_max_steps(16).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            // The alert the bar raises is a hosted dialog, drawn by the dialog host.
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    (h, id)
}

/// The size the text layer's first run carries, points.
fn layer_size(h: &Harness<'static, Box<PhotocraftApp>>, id: u64) -> f32 {
    let st = h.state().session.active().unwrap();
    super::text_layer(&st.doc, photocraft_doc::LayerId(id)).unwrap().char_runs()[0].style.size_pt
}

/// The messages of the open framework alerts (the list `ui.inspect` reports as its `dialogs`).
fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// Types `text` into the options bar's size field and commits it with Enter.
fn type_size(h: &mut Harness<'static, Box<PhotocraftApp>>, text: &str) {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == "Set font size"))
        .expect("the size field")
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

#[test]
fn the_size_field_takes_001_to_1296_and_the_nearest_limit_outside_it() {
    // (typed, kept, alert once) — typed values are integers only: the field's reader follows the env's regional
    // decimal separator, and this test does not pin that behaviour.
    let rows: [(&str, f32, bool); 6] =
        [("2000", 1296.0, true), ("1297", 1296.0, true), ("0", 0.01, true), ("-5", 0.01, true), ("1", 1.0, false), ("1296", 1296.0, false)];
    for (typed, kept, alert) in rows {
        let (mut h, id) = bar();
        type_size(&mut h, typed);
        assert_eq!(layer_size(&h, id), kept, "size {typed}");
        let shown = alerts(&h);
        if alert {
            assert_eq!(shown.len(), 1, "size {typed}: one alert, got {shown:?}");
            // The sentence and the measured note are the reference's words; the two ends follow the env's
            // number format (its decimal separator is not pinned here).
            let message = &shown[0];
            assert!(
                message.starts_with("A value between ") && message.ends_with(" is required. Closest value inserted."),
                "size {typed}: the alert names the range and the note: {message:?}"
            );
            assert!(message.contains(" pt and "), "size {typed}: both ends carry the unit: {message:?}");
        } else {
            assert!(shown.is_empty(), "size {typed}: no alert, got {shown:?}");
        }
    }
}

#[test]
fn the_alert_shows_once_and_ok_closes_it_with_the_value_kept() {
    let (mut h, id) = bar();
    type_size(&mut h, "2000");
    h.run_steps(10);
    assert_eq!(alerts(&h).len(), 1, "one alert, not one per frame");
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(alerts(&h).is_empty(), "OK closes the alert");
    assert_eq!(layer_size(&h, id), 1296.0, "the closest value stays after OK");
}

#[test]
fn a_typed_out_of_range_size_is_one_history_state() {
    let (mut h, id) = bar();
    let before = h.state().session.active().unwrap().history.entries().len();
    type_size(&mut h, "2000");
    let after = h.state().session.active().unwrap().history.entries().len();
    assert_eq!(after, before + 1, "the size change and its alert together are one step");
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), before + 1, "closing the alert records nothing extra");
    assert_eq!(layer_size(&h, id), 1296.0);
}

#[test]
fn a_type_unit_other_than_points_clamps_without_an_alert() {
    // *Not measured*: the same field with Units & Rulers > Type in pixels or millimetres keeps today's behaviour.
    for unit in ["pixels", "mm"] {
        let (mut h, id) = bar();
        h.state_mut().run("prefs.set", json!({"path": "unitsAndRulers.typeUnits", "value": unit})).unwrap();
        h.run_steps(3);
        type_size(&mut h, "2000");
        assert_eq!(layer_size(&h, id), 1296.0, "{unit}: the typed size still goes to the maximum, in points");
        assert!(alerts(&h).is_empty(), "{unit}: today's silent clamp, no alert");
    }
}

#[test]
fn the_size_menu_rows_still_set_the_size_without_an_alert() {
    let (mut h, id) = bar();
    h.get_by_role_and_label(Role::SpinButton, "Set font size").get_by_role_and_label(Role::Button, "Presets").click();
    h.run_steps(2);
    crate::tools::type_tool::canvas_tests::reveal(&mut h, "72 pt");
    h.get_by_label("72 pt").click();
    h.run_steps(3);
    assert_eq!(layer_size(&h, id), 72.0, "a preset row sets the size as it does today");
    assert!(alerts(&h).is_empty(), "a preset is not an out-of-range value");
}
