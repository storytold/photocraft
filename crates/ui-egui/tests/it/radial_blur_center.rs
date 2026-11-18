//! Radial Blur's centre is one point row of the hosted dialog. Automation sets `centerX` and
//! `centerY`, the preview leaves the document alone until OK, and OK sends both to the command.
//! Public control API only; no GPU adapter is needed.

use egui::vec2;
use egui_kittest::Harness;
use photocraft_ui_egui::{
    PhotocraftApp,
    control::{ControlRequest, Reply, handle},
    dialogs, i18n,
    theme::AppTheme,
};
use serde_json::{Value, json};

const COMMAND: &str = "filter.blur.radialBlur";

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("paint.stroke", json!({"points": [[8, 8, 1]], "size": 4, "color": "#ff8000"})).unwrap();
    let h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1000.0, 800.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let _language = i18n::language_scope(i18n::Lang::EN);
            dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, AppTheme::pro());
    h
}

/// Sends one control request, requires success and returns its `result`.
fn control(h: &mut Harness<'_, PhotocraftApp>, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (request, _rx) = ControlRequest::new(method, params);
    let Reply::Done(reply) = handle(h.state_mut(), &ctx, &request) else { panic!("{method} did not finish") };
    assert_eq!(reply["ok"], true, "{method}: {reply}");
    h.run_steps(3);
    reply["result"].clone()
}

/// The value under `key` anywhere in an inspect reply.
fn find<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|v| find(v, key))),
        Value::Array(items) => items.iter().find_map(|v| find(v, key)),
        _ => None,
    }
}

fn revision(h: &Harness<'_, PhotocraftApp>) -> u64 {
    h.state().session.active().unwrap().revision
}

#[test]
fn automation_moves_the_centre_and_ok_sends_both_coordinates() {
    let mut h = harness();
    let opened = control(&mut h, "ui.menu.invoke", json!({"id": COMMAND}));
    // The reply's own id form goes back into the next request unchanged.
    let id = opened["dialog"].clone();
    control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "method", "value": "zoom"}));
    let before = revision(&h);
    for (field, value) in [("centerX", 0.12), ("centerY", 0.87)] {
        control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}));
    }
    // PLAN: assumed `ui.inspect` lists the open dialog's values, including the `center` row.
    let inspected = control(&mut h, "ui.inspect", json!({}));
    assert_eq!(find(&inspected, "center"), Some(&json!([0.12, 0.87])), "both fields moved the one centre row: {inspected}");
    assert_eq!(revision(&h), before, "editing the centre is only a preview until confirmed");

    control(&mut h, "ui.dialog.confirm", json!({"dialog": id}));
    assert!(revision(&h) > before, "OK applied the filter");
    let applied = h.state().session.active().unwrap().doc.clone();
    // The command run directly with the same centre gives the same picture: OK sent centerX and centerY.
    let mut direct = photocraft_engine::Session::new();
    direct.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    direct.execute("paint.stroke", json!({"points": [[8, 8, 1]], "size": 4, "color": "#ff8000"})).unwrap();
    direct.execute(COMMAND, json!({"method": "zoom", "centerX": 0.12, "centerY": 0.87})).unwrap();
    let expected = direct.active().unwrap().doc.clone();
    assert_eq!(photocraft_compose::render(&applied, applied.bounds()), photocraft_compose::render(&expected, expected.bounds()));

    // Cancel after an edit leaves the document as it was.
    let opened = control(&mut h, "ui.menu.invoke", json!({"id": COMMAND}));
    let id = opened["dialog"].clone();
    let before = revision(&h);
    control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "centerX", "value": 0.8}));
    control(&mut h, "ui.dialog.cancel", json!({"dialog": id}));
    assert_eq!(revision(&h), before);
}
