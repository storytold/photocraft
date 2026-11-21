//! The options bar's range-alert table: a value typed past a ranged field's range is kept as the
//! closest one and, once the field commits, one alert with OK says so. One table for the Eraser
//! group's fields and the retouch bars' (#11909, #11930, #12650, #12949, #12962, #12976). Also the
//! Background Eraser's own angle and pressure.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

fn bar(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    hosted_bar(app, tool)
}

fn hosted_bar(mut app: Box<PhotocraftApp>, tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    app.ui.tool = tool;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            // The alert a field raises is a hosted dialog, drawn by the dialog host.
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
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

#[test]
fn options_bar_fields_that_alert_past_their_range() {
    type Read = fn(&PhotocraftApp) -> f32;
    let o = |a: &PhotocraftApp| -> ToolOptions { a.ui.tool_options.clone() };
    let _ = o;
    // (tool, field, typed, kept, alerts)
    let rows: [(Tool, &str, &str, f32, bool, Read); 11] = [
        (Tool::BackgroundEraser, "Tolerance", "0", 1.0, true, |a| a.ui.tool_options.bg_tolerance),
        (Tool::BackgroundEraser, "Tolerance", "150", 100.0, true, |a| a.ui.tool_options.bg_tolerance),
        (Tool::BackgroundEraser, "Angle", "400", 180.0, true, |a| a.session.tools.brush.angle),
        (Tool::BackgroundEraser, "Angle", "-200", -180.0, true, |a| a.session.tools.brush.angle),
        (Tool::MagicEraser, "Tolerance", "300", 255.0, false, |a| a.ui.tool_options.tolerance),
        (Tool::Sponge, "Flow", "150", 100.0, true, |a| a.ui.tool_options.sponge_flow),
        (Tool::Blur, "Strength", "0", 1.0, true, |a| a.ui.tool_options.strength),
        (Tool::Sharpen, "Strength", "0", 1.0, true, |a| a.ui.tool_options.strength),
        (Tool::Smudge, "Strength", "0", 1.0, true, |a| a.ui.tool_options.strength),
        (Tool::ColorReplacement, "Tolerance", "150", 100.0, true, |a| a.ui.tool_options.cr_tolerance),
        (Tool::BackgroundEraser, "Tolerance", "33.6", 34.0, false, |a| a.ui.tool_options.bg_tolerance),
    ];
    for (tool, field, typed, kept, alerts_once, read) in rows {
        let mut h = bar(tool);
        type_into(&mut h, field, typed);
        let what = format!("{tool:?} {field} {typed}");
        assert_eq!(read(h.state()), kept, "{what}: the closest value is kept");
        let shown = alerts(&h);
        if alerts_once {
            assert_eq!(shown.len(), 1, "{what}: one alert, got {shown:?}");
            assert!(shown[0].contains("Closest value inserted"), "{what}: {shown:?}");
            h.get_by_label("OK").click();
            h.run_steps(4);
            assert!(alerts(&h).is_empty(), "{what}: OK closes the alert");
            assert_eq!(read(h.state()), kept, "{what}: the value stays after OK");
        } else {
            assert!(shown.is_empty(), "{what}: no alert, got {shown:?}");
        }
    }
}

#[test]
fn sponge_flow_clamped_in_the_bar_reaches_the_painted_pixel() {
    let seeded = || {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 80, "height": 80, "depth": 32, "background": "#c82828"})).unwrap();
        app.ui.tool = Tool::Sponge;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0, "spacing": 0.1}})).unwrap();
        app.ui.tool_options.sponge_mode = "desaturate".into();
        app.ui.tool_options.vibrance = false;
        app
    };
    let pixel = |app: &PhotocraftApp| {
        let state = app.session.active().unwrap();
        let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
        let surface = layer.surface().unwrap();
        assert_eq!(surface.format().sample, photocraft_doc::SampleType::F32);
        surface.rgba(40, 40)
    };
    let mut h = hosted_bar(seeded(), Tool::Sponge);
    let untouched = pixel(h.state());
    let history = h.state().session.active().unwrap().history.past_len();
    type_into(&mut h, "Flow", "0");
    let messages = alerts(&h);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("Closest value inserted"));
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(alerts(&h).is_empty());
    assert_eq!(h.state().ui.tool_options.sponge_flow, 1.0);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);

    crate::canvas::testing::click(h.state_mut(), [40.0, 40.0], egui::Modifiers::NONE);
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    let (command, params) = h.state().session.journal.last().unwrap();
    assert_eq!(command, "paint.sponge");
    assert_eq!(params["flow"].as_f64(), Some(1.0));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);

    let mut reference = seeded();
    assert_eq!(pixel(&reference), untouched);
    reference
        .run(
            "paint.sponge",
            json!({"points": [[40.0, 40.0, 1.0]], "size": 40, "hardness": 100, "spacing": 10, "mode": "desaturate", "vibrance": false, "flow": 1}),
        )
        .unwrap();
    let expected = pixel(&reference);
    assert!(expected[0] < untouched[0] - 1e-6, "the 1% reference dab lowers the red channel: {expected:?} from {untouched:?}");
    let painted = pixel(h.state());
    assert!(painted.iter().zip(expected).all(|(actual, expected)| (actual - expected).abs() < 1e-6), "{painted:?} vs {expected:?}");
}

#[test]
fn angle_and_pressure_are_the_background_erasers_own() {
    const PRESSURE: &str = "Always use pressure for size";
    let mut h = bar(Tool::Brush);
    h.state_mut().run("tools.setBrush", json!({"brush": {"angle": -20.0}})).unwrap();
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::BackgroundEraser;
    h.run_steps(3);
    type_into(&mut h, "Angle", "30");
    h.get_by_label(PRESSURE).click();
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(3);
    let brush = &h.state().session.tools.brush;
    assert_eq!((brush.angle, brush.pressure_size), (-20.0, false), "the Brush keeps its own angle and pressure");
    h.state_mut().ui.tool = Tool::BackgroundEraser;
    h.run_steps(3);
    let brush = &h.state().session.tools.brush;
    assert_eq!((brush.angle, brush.pressure_size), (30.0, true), "the Background Eraser keeps its own");
}
