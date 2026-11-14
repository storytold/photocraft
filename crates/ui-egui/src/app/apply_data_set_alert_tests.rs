//! #14499: Apply Data Set on a type layer under Lock All shows the reference application's alert;
//! its other refusals stay status-bar messages.

use super::*;
use serde_json::json;

const LOCKED: &str = "Could not apply data set because the layer is locked.";

/// An app with a type layer bound to a `headline` text variable and one data set `Zed` for it,
/// the layer under Lock All when `locked`.
fn app(locked: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = app.session.execute("type.create", json!({"x": 4, "y": 30, "text": "Old", "size": 12})).unwrap()["layer"].clone();
    app.session.execute("image.variables.define", json!({"defs": [{"name": "headline", "layer": id, "type": "textReplacement"}]})).unwrap();
    app.session
        .execute("image.variables.dataSets", json!({"dataSets": [{"name": "Zed", "values": [{"variable": "headline", "kind": "text", "value": "New"}]}]}))
        .unwrap();
    if locked {
        app.session.execute("layer.lockLayers", json!({"all": true})).unwrap();
    }
    app.sync_views();
    app
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == state::DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(String::from)).collect()
}

#[test]
fn a_locked_type_layer_refuses_with_an_alert() {
    let mut app = app(true);
    assert_eq!(app.run("image.applyDataSet", json!({"name": "Zed"})), Err(LOCKED.to_string()));
    assert_eq!(alerts(&app), [LOCKED]);
}

#[test]
fn other_refusals_and_an_unlocked_layer_show_no_alert() {
    let mut app = app(false);
    assert!(app.run("image.applyDataSet", json!({"name": "Missing"})).is_err());
    app.run("image.applyDataSet", json!({"name": "Zed"})).unwrap();
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
}
