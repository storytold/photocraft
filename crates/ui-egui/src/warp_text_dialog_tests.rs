//! #218: Type › Warp Text… opens with the type layer's current warp (it always started at Arc)
//! and previews the warp live on the canvas.

use photocraft_doc::LayerContent;
use photocraft_doc::text::TextWarp;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

/// A document with one type layer, active.
fn app_with_type() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("type.create", json!({"x": 20, "y": 120, "text": "Warp", "size": 60})).unwrap();
    app
}

fn flag() -> Value {
    json!({"style": "flag", "bend": -30.0, "horizontalDistortion": 20.0, "verticalDistortion": -10.0, "orientation": "vertical"})
}

fn warp(app: &PhotocraftApp) -> Option<TextWarp> {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Text(t) => t.warp.clone(),
        _ => panic!("the active layer is a type layer"),
    }
}

/// Open Warp Text from the Type menu and return the dialog with its fields.
fn open(app: &mut PhotocraftApp) -> (u64, Map<String, Value>) {
    let r = crate::menus::invoke(app, &egui::Context::default(), "type.warpText", json!({})).unwrap();
    let id = r["dialog"].as_u64().unwrap();
    (id, app.ui.dialog_mut(id).unwrap().fields.clone())
}

fn shown(fields: &Map<String, Value>) -> (&str, f64, f64, f64, &str) {
    let n = |k: &str| fields[k].as_f64().unwrap();
    (fields["style"].as_str().unwrap(), n("bend"), n("horizontalDistortion"), n("verticalDistortion"), fields["orientation"].as_str().unwrap())
}

#[test]
fn warp_text_opens_with_the_type_layers_current_warp() {
    let mut app = app_with_type();
    app.run("type.warpText", flag()).unwrap();
    let (_, fields) = open(&mut app);
    assert_eq!(shown(&fields), ("flag", -30.0, 20.0, -10.0, "vertical"));
}

#[test]
fn confirming_the_warp_text_dialog_unchanged_keeps_the_layers_warp() {
    let mut app = app_with_type();
    app.run("type.warpText", flag()).unwrap();
    let before = warp(&app);
    assert_eq!(before.as_ref().map(|w| w.style.as_str()), Some("warpFlag"));
    let (id, _) = open(&mut app);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(warp(&app), before);
}

#[test]
fn warp_text_opens_at_none_for_a_layer_without_warp() {
    let mut app = app_with_type();
    let (id, fields) = open(&mut app);
    assert_eq!(shown(&fields), ("none", 50.0, 0.0, 0.0, "horizontal"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(warp(&app), None);
}

#[test]
fn remembered_warp_values_never_override_the_layers_warp() {
    let mut app = app_with_type();
    let (id, _) = open(&mut app);
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("style".into(), json!("twist"));
    f.insert("bend".into(), json!(80.0));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(warp(&app).map(|w| (w.style, w.value)), Some(("warpTwist".to_string(), 80.0)));
    app.run("type.create", json!({"x": 20, "y": 60, "text": "Other", "size": 30})).unwrap();
    app.run("type.warpText", flag()).unwrap();
    let (_, fields) = open(&mut app);
    assert_eq!(shown(&fields), ("flag", -30.0, 20.0, -10.0, "vertical"));
}

/// A PSD can carry a style without a short id: the dialog shows None with the layer's values.
#[test]
fn an_unknown_warp_style_opens_at_none_with_the_layers_values() {
    let mut app = app_with_type();
    let st = app.session.active_mut().unwrap();
    let id = st.active_layer.unwrap();
    let mut doc = (*st.doc).clone();
    if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
        t.warp = Some(TextWarp { style: "warpCustom".into(), value: 12.0, horizontal_distortion: -5.0, vertical_distortion: 7.0, horizontal: false });
    }
    st.doc = std::sync::Arc::new(doc);
    let (_, fields) = open(&mut app);
    assert_eq!(shown(&fields), ("none", 12.0, -5.0, 7.0, "vertical"));
}
