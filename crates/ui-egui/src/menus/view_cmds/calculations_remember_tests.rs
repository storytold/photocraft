//! #14531: after OK, Calculations opened again on the same document shows the values that were used, Result
//! included; Cancel keeps nothing. On another document only Blending is kept, and a remembered channel the document
//! no longer has falls back to the default.

use craft::ui::dialog::DialogId;
use serde_json::{Value, json};

use crate::PhotocraftApp;

const CALC: &str = "image.calculations";

/// A grey document with the alpha channel "Alpha 1".
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    new_doc(&mut app);
    app
}

fn new_doc(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 40, "height": 30, "background": "#808080"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 15})).unwrap();
    app.run("select.saveSelection", json!({"channel": "new"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
}

fn open(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(app, &ctx, CALC, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).unwrap();
    ctx.run_ui(Default::default(), |ui| crate::dialogs::run(app, ui)).textures_delta.clear();
    id
}

fn shown(app: &PhotocraftApp, id: DialogId) -> Value {
    app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields).unwrap()
}

fn set(app: &mut PhotocraftApp, id: DialogId, values: Value) {
    let ctx = app.ctx().clone();
    for (k, v) in values.as_object().unwrap() {
        app.framework.dialogs.set_json_field(&ctx, id, k, v.clone()).unwrap();
    }
}

fn ok(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::cancel(app, &ctx, id).unwrap();
}

fn pick(f: &Value) -> Value {
    json!([f["blending"], f["opacity"], f["source1Invert"], f["source1Channel"], f["source2Invert"], f["source2Channel"], f["result"]])
}

/// The values OK confirms in the tests.
fn used() -> Value {
    json!({"blending": "screen", "opacity": 70.0, "source1Invert": true, "source1Channel": "Alpha 1", "source2Invert": true, "source2Channel": "Red", "result": "selection"})
}

#[test]
fn ok_values_come_back_on_the_same_document() {
    let mut app = app();
    let id = open(&mut app);
    let defaults = pick(&shown(&app, id));
    set(&mut app, id, used());
    ok(&mut app, id);
    let id = open(&mut app);
    assert_eq!(pick(&shown(&app, id)), json!(["screen", 70.0, true, "Alpha 1", true, "Red", "selection"]), "defaults were {defaults}");
}

#[test]
fn cancel_keeps_nothing() {
    let mut app = app();
    let id = open(&mut app);
    set(&mut app, id, used());
    ok(&mut app, id);
    let id = open(&mut app);
    set(&mut app, id, json!({"opacity": 30.0, "blending": "add"}));
    cancel(&mut app, id);
    let id = open(&mut app);
    let f = shown(&app, id);
    assert_eq!((f["opacity"].clone(), f["blending"].clone()), (json!(70.0), json!("screen")), "the last OK's values");
}

#[test]
fn another_document_keeps_only_blending() {
    let mut app = app();
    let id = open(&mut app);
    let defaults = shown(&app, id);
    set(&mut app, id, used());
    ok(&mut app, id);
    new_doc(&mut app);
    let id = open(&mut app);
    let f = shown(&app, id);
    assert_eq!(f["blending"], json!("screen"), "Blending is kept");
    for key in ["opacity", "source1Invert", "source2Invert", "result", "source1Channel"] {
        assert_eq!(f[key], defaults[key], "{key} is the default on another document");
    }
}

#[test]
fn a_channel_that_is_gone_falls_back_to_the_default() {
    let mut app = app();
    let id = open(&mut app);
    let default = shown(&app, id)["source1Channel"].clone();
    set(&mut app, id, used());
    ok(&mut app, id);
    app.run("channel.delete", json!({"channel": "Alpha 1"})).unwrap();
    let id = open(&mut app);
    let f = shown(&app, id);
    assert_eq!(f["source1Channel"], default, "Alpha 1 is gone: the default");
    assert_eq!(f["opacity"], json!(70.0), "the rest comes back");
}
