//! #13339: Create Rectangle opens with Width 100 px, Height 100 px, the four radii linked at the
//! options bar's Radius (0) and From Center off every time, whatever the last OK or the gear's Fixed
//! Size was, and its OK leaves the tool options alone. Its Width also takes 0: OK then closes the
//! dialog and creates nothing, with no layer, no History state and no error. Create Ellipse,
//! Triangle and Polygon keep opening with their last OK.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerContent;
use photocraft_doc::vector::LiveShape;
use serde_json::{Map, Value, json};

use super::{geometry, owns};
use crate::PhotocraftApp;
use crate::canvas::{Drag, finish_gesture};
use crate::state::Tool;

/// A 400 × 300 white document.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app
}

/// A click (press and release at `at`) with `tool`, as on the canvas; returns the dialog it opened.
fn click(app: &mut PhotocraftApp, tool: Tool, at: [f64; 2]) -> u64 {
    app.ui.tool = tool;
    finish_gesture(app, Drag::new(tool, at, vec![[at[0], at[1], 1.0]; 2], egui::Modifiers::NONE, false));
    let [d] = app.ui.dialogs.as_slice() else { panic!("a click with {tool:?} opens one dialog") };
    assert!(owns(&d.fields), "{tool:?}: a Create dialog");
    d.id
}

fn fields(app: &PhotocraftApp, id: u64) -> &Map<String, Value> {
    &app.ui.dialogs.iter().find(|d| d.id == id).expect("the dialog").fields
}

/// What the dialog shows: Width, Height, the four radii, Link radii and From Center.
fn shown(f: &Map<String, Value>) -> (Value, Value, Value, Value, Value) {
    (f["width"].clone(), f["height"].clone(), f["radii"].clone(), f["linkRadii"].clone(), f["fromCenter"].clone())
}

/// Create Rectangle's defaults: 100 × 100 px, the four radii linked at 0 px, From Center off.
fn defaults() -> (Value, Value, Value, Value, Value) {
    (json!(100.0), json!(100.0), json!([0.0, 0.0, 0.0, 0.0]), json!(true), json!(false))
}

/// Sets `values` in the dialog `id` (as `ui.dialog.set` does) and presses OK.
fn ok_with(app: &mut PhotocraftApp, id: u64, values: Value) {
    let d = app.ui.dialog_mut(id).expect("the dialog");
    for (k, v) in values.as_object().expect("an object") {
        d.fields.insert(k.clone(), v.clone());
    }
    crate::dialogs::confirm(app, id).expect("OK");
}

fn active_live(app: &PhotocraftApp) -> LiveShape {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("not a shape layer") };
    sh.live.clone().unwrap()
}

/// Layer count and History length of the active document.
fn doc_state(app: &PhotocraftApp) -> (usize, usize) {
    let st = app.session.active().unwrap();
    (st.doc.layers.len(), st.history.past_len())
}

#[test]
fn the_next_click_opens_the_defaults_whatever_the_last_ok_was() {
    let mut app = app();
    // The gear's Fixed Size 120 × 60 does not size the dialog either.
    let o = &mut app.ui.tool_options;
    (o.rect_constraint, o.rect_fixed_width, o.rect_fixed_height) = ("fixedSize".into(), 120.0, 60.0);
    let options = serde_json::to_value(&app.ui.tool_options).unwrap();
    let id = click(&mut app, Tool::Rectangle, [400.0, 300.0]);
    assert_eq!(shown(fields(&app, id)), defaults(), "the first click");
    ok_with(&mut app, id, json!({"width": 150, "height": 80, "radii": [10, 10, 10, 10], "fromCenter": true}));
    // OK drew what it showed, centred on the click.
    assert_eq!(active_live(&app), LiveShape::Rect { rect: [325.0, 260.0, 150.0, 80.0], radii: [10.0; 4] });
    assert_eq!(serde_json::to_value(&app.ui.tool_options).unwrap(), options, "OK leaves the tool options (Radius 0, Fixed Size 120 × 60) alone");
    assert_eq!(app.session.prefs().dialogs.get("shape.create.rectangle"), None, "nothing is saved for the next Create Rectangle");
    // Clicks outside the new rectangle (a click inside the active one opens nothing, #13340).
    for at in [[20.0, 30.0], [380.0, 20.0]] {
        let id = click(&mut app, Tool::Rectangle, at);
        assert_eq!(shown(fields(&app, id)), defaults(), "a click at {at:?} after the OK");
        app.ui.close_dialog(id);
    }
}

#[test]
fn a_rectangle_remembered_by_an_older_version_is_ignored() {
    let mut app = app();
    app.session.prefs.edit(|p| p.dialogs.insert("shape.create.rectangle".into(), json!({"width": 150, "height": 80, "fromCenter": true})));
    let id = click(&mut app, Tool::Rectangle, [10.0, 10.0]);
    assert_eq!(shown(fields(&app, id)), defaults());
}

#[test]
fn ellipse_triangle_and_polygon_still_open_with_their_last_ok() {
    for tool in [Tool::EllipseShape, Tool::Triangle, Tool::Polygon] {
        let mut app = app();
        let id = click(&mut app, tool, [200.0, 150.0]);
        ok_with(&mut app, id, json!({"width": 150, "height": 80, "fromCenter": true}));
        let id = click(&mut app, tool, [20.0, 30.0]);
        let f = fields(&app, id);
        assert_eq!((&f["width"], &f["height"], &f["fromCenter"]), (&json!(150), &json!(80), &json!(true)), "{tool:?}");
        // Their Width keeps its 1 px minimum.
        let mut zero = f.clone();
        zero.insert("width".into(), json!(0));
        assert!(geometry(&zero).is_err(), "{tool:?}: Width 0 is still refused");
    }
}

/// The sides Create Rectangle's OK takes: Width 0 or 1 to 300000 px, Height 1 to 300000 px.
#[test]
fn width_zero_creates_nothing_and_the_other_sides_keep_their_minimum() {
    let mut app = app();
    let id = click(&mut app, Tool::Rectangle, [10.0, 10.0]);
    let with = |values: Value| {
        let mut f = fields(&app, id).clone();
        f.extend(values.as_object().unwrap().clone());
        geometry(&f)
    };
    assert_eq!(with(json!({"width": 0})), Ok(Value::Null), "nothing to create");
    assert_eq!(with(json!({"width": 0, "radii": [10, 10, 10, 10], "fromCenter": true})), Ok(Value::Null));
    assert_eq!(with(json!({"width": 1})), Ok(json!({"kind": "rect", "rect": [10.0, 10.0, 1.0, 100.0]})));
    for refused in [json!({"height": 0}), json!({"width": 0, "height": 0}), json!({"width": 0.5}), json!({"width": -1})] {
        assert!(with(refused.clone()).is_err(), "{refused}");
    }
}

/// A Width of 0 set over the control channel: OK answers, closes the dialog and creates nothing.
#[test]
fn ok_with_width_zero_over_the_control_channel_creates_nothing() {
    let mut app = app();
    let ctx = egui::Context::default();
    let call = |app: &mut PhotocraftApp, method: &str, params: Value| {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Outcome::Done(v) => v,
            _ => panic!("deferred"),
        }
    };
    let before = doc_state(&app);
    call(&mut app, "ui.pointer", json!({"tool": "rectangle", "events": [{"kind": "down", "x": 40, "y": 50}, {"kind": "up", "x": 40, "y": 50}]}));
    let id = app.ui.dialogs[0].id;
    call(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "width", "value": 0}));
    let r = call(&mut app, "ui.dialog.confirm", json!({"dialog": id}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert!(app.ui.dialogs.is_empty(), "OK closes the dialog");
    assert_eq!(doc_state(&app), before, "no layer and no History state");
    assert!(!app.ui.status_error, "no error: {}", app.ui.status);
}

/// The dialog shown by the app, opened by a click at (10, 20), fonts installed.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app());
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    super::open(h.state_mut(), Tool::Rectangle, [10.0, 20.0]).expect("Create Rectangle");
    h.run_steps(4);
    h
}

/// Types `text` over the field named `name`.
fn type_over(h: &mut Harness<'static, PhotocraftApp>, name: &str, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(name)).expect("the field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
}

fn side(h: &Harness<'static, PhotocraftApp>, key: &str) -> Option<f64> {
    h.state().ui.dialogs.iter().find(|d| owns(&d.fields))?.fields[key].as_f64()
}

#[test]
fn width_takes_a_typed_zero_and_height_keeps_its_1_px_minimum() {
    for (name, typed, key, want) in [("Width:", "0", "width", 0.0), ("Width:", "250", "width", 250.0), ("Height:", "0", "height", 1.0)] {
        let mut h = harness();
        type_over(&mut h, name, typed);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(side(&h, key), Some(want), "{name} {typed}");
        assert!(h.state().ui.dialogs.iter().all(|d| owns(&d.fields)), "{name} {typed}: no alert");
    }
}

#[test]
fn ok_or_enter_with_a_typed_width_of_zero_closes_and_creates_nothing() {
    for press_ok in [true, false] {
        let mut h = harness();
        let before = doc_state(h.state());
        let status = h.state().ui.status.clone();
        type_over(&mut h, "Width:", "0");
        if press_ok {
            h.key_press(egui::Key::Tab);
            h.run_steps(2);
            h.get_by_label("OK").click();
        } else {
            h.key_press(egui::Key::Enter);
        }
        h.run_steps(4);
        let app = h.state();
        assert!(app.ui.dialogs.is_empty(), "OK={press_ok}: the dialog closes, with no alert");
        assert_eq!(doc_state(app), before, "OK={press_ok}: no layer and no History state");
        assert!(!app.ui.status_error, "OK={press_ok}: no error");
        assert_eq!(app.ui.status, status, "OK={press_ok}: nothing in the status bar");
    }
}
