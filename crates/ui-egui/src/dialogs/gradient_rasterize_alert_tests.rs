//! #12016: a Classic gradient drag on a type layer or a Smart Object shows the reference
//! application's alert and changes nothing, instead of offering to rasterize.

use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::tool_event;

fn app_with(kind: Kind) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    match kind {
        Kind::Type => app.run("type.create", json!({"x": 10, "y": 40, "text": "Hello"})).map(|_| ()),
        Kind::Shape => app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 60, 40], "fill": "#ff0000"})).map(|_| ()),
        Kind::SmartObject => {
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
            app.run("layer.smartObjects.convertToSmartObject", json!({})).map(|_| ())
        }
        Kind::Fill => app.run("layer.newFillLayer.solidColor", json!({"color": "#00ff00"})).map(|_| ()),
    }
    .unwrap();
    app.ui.mask_target = false;
    app.sync_mask_targets();
    app.ui.tool = Tool::Gradient;
    app.ui.tool_options.gradient_classic = true;
    app
}

fn drag(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 10.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 80.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 80.0, y: 20.0 }, Modifiers::NONE);
}

fn kind_name(app: &PhotocraftApp) -> &'static str {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name()
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_string)).collect()
}

#[test]
fn a_classic_gradient_on_type_or_a_smart_object_alerts_and_changes_nothing() {
    for (kind, name, message) in [
        (Kind::Type, "Type", "Could not use the gradient tool because the pixels in a type layer cannot be edited without first rasterizing the layer."),
        (Kind::SmartObject, "Smart Object", "Could not use the gradient tool because the smart object is not directly editable."),
    ] {
        let mut app = app_with(kind);
        let past = app.session.active().unwrap().history.past_len();
        drag(&mut app);
        assert_eq!(alerts(&app), [message], "{kind:?}");
        assert!(!app.ui.dialogs.iter().any(|d| owns(&d.fields)), "{kind:?}: no rasterize prompt");
        assert_eq!(kind_name(&app), name);
        assert_eq!(app.session.active().unwrap().history.past_len(), past, "{kind:?}: no history state");
        // A second drag while the alert is up doesn't stack another.
        drag(&mut app);
        assert_eq!(app.ui.dialogs.len(), 1, "{kind:?}");
    }
}

#[test]
fn shape_and_fill_layers_and_the_brush_still_ask_to_rasterize() {
    for kind in [Kind::Shape, Kind::Fill] {
        let mut app = app_with(kind);
        drag(&mut app);
        assert!(app.ui.dialogs.iter().any(|d| owns(&d.fields)), "{kind:?}: the Classic gradient still asks");
        assert!(alerts(&app).is_empty());
    }
    for kind in [Kind::Type, Kind::SmartObject] {
        let mut app = app_with(kind);
        app.ui.tool = Tool::Brush;
        drag(&mut app);
        assert!(app.ui.dialogs.iter().any(|d| owns(&d.fields)), "{kind:?}: the Brush still asks");
    }
}
