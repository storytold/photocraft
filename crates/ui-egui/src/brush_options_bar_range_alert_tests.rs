//! The Brush's Opacity, Flow and Smoothing typed out of range (#16809): the closest value is kept and,
//! as on the Eraser, one "Closest value inserted" alert with OK says so. Text that is no number keeps
//! the old value without an alert.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The options bar of the Brush at 50 % opacity and flow, with the dialog host that draws its alerts.
fn bar() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"opacity": 0.5, "flow": 0.5}})).unwrap();
    app.ui.tool = Tool::Brush;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(2400.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The messages of the open framework alerts.
fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// Types `text` into the number field named `name` and commits it with Enter.
fn type_into(h: &mut Harness<'static, Box<PhotocraftApp>>, name: &str, text: &str) {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == name))
        .unwrap_or_else(|| panic!("the {name} field"))
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    if text.is_empty() {
        h.key_press(egui::Key::Backspace);
    } else {
        h.event(egui::Event::Text(text.into()));
    }
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

type Read = fn(&PhotocraftApp) -> f32;

fn opacity(app: &PhotocraftApp) -> f32 {
    app.session.tools.brush.opacity
}

fn flow(app: &PhotocraftApp) -> f32 {
    app.session.tools.brush.flow
}

fn smoothing(app: &PhotocraftApp) -> f32 {
    app.session.tools.brush.smoothing.amount
}

#[test]
fn a_value_typed_past_the_range_keeps_the_closest_and_alerts_once() {
    // PLAN: Opacity and Flow 0 and -5 (kept 1 %, the alert naming 1 %) come with the craft-libs pin that gives
    // `PaintBar`'s Opacity and Flow a 1 to 100 % range; until then the field holds them to 0 while the brush keeps
    // 1 %, and the field drops the draft without reporting it.
    let rows: [(&str, &str, f32, Read); 4] =
        [("Opacity", "150", 1.0, opacity), ("Flow", "150", 1.0, flow), ("Smoothing", "150", 1.0, smoothing), ("Smoothing", "-5", 0.0, smoothing)];
    for (field, typed, kept, read) in rows {
        let mut h = bar();
        type_into(&mut h, field, typed);
        let what = format!("Brush {field} {typed}");
        assert!((read(h.state()) - kept).abs() < 1e-6, "{what}: keeps {kept}, got {}", read(h.state()));
        let shown = alerts(&h);
        assert_eq!(shown.len(), 1, "{what}: one alert, got {shown:?}");
        assert!(shown[0].contains("Closest value inserted"), "{what}: {shown:?}");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert!(alerts(&h).is_empty(), "{what}: OK closes the alert");
        assert!((read(h.state()) - kept).abs() < 1e-6, "{what}: the value stays after OK");
    }
}

#[test]
fn text_that_is_no_number_keeps_the_old_value_without_an_alert() {
    for (field, read) in [("Opacity", opacity as Read), ("Flow", flow), ("Smoothing", smoothing)] {
        for typed in ["abc", ""] {
            let mut h = bar();
            let old = read(h.state());
            type_into(&mut h, field, typed);
            let what = format!("Brush {field} {typed:?}");
            assert_eq!(read(h.state()), old, "{what}: the old value stays");
            assert!(alerts(&h).is_empty(), "{what}: no alert, got {:?}", alerts(&h));
        }
    }
}
