use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::json;

use super::*;
use crate::canvas::tool_event;

/// A document whose active layer is a `kind` layer over a white Background.
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
    // A new fill layer targets its mask (Photoshop); painting its content means targeting the
    // layer itself, as a click on its thumbnail does.
    app.ui.mask_target = false;
    app.sync_mask_targets();
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    app
}

fn active_content(app: &PhotocraftApp) -> &'static str {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name()
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
}

fn prompt(app: &PhotocraftApp) -> Option<&crate::state::Dialog> {
    app.ui.dialogs.iter().find(|d| owns(&d.fields))
}

#[test]
fn pdf_smart_object_requires_manual_rasterization_before_painting() {
    let mut app = app_with(Kind::SmartObject);
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "pdf", &Default::default()).unwrap().bytes;
    app.open_file("placed.pdf", &bytes).unwrap();
    app.confirm_pdf_pages(&[0]).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = past(&app);
    assert!(app.run("paint.stroke", json!({"points": [[30, 20]], "color": "#0000ff"})).is_err());
    assert_eq!(app.session.active().unwrap().doc, before);
    for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser, Tool::CloneStamp, Tool::PaintBucket, Tool::Smudge, Tool::Gradient] {
        app.ui.tool = tool;
        app.ui.tool_options.gradient_classic = true;
        click(&mut app, 30.0, 20.0);
        assert!(prompt(&app).is_none(), "{tool:?}: painting must not offer to rasterize a smart object");
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_eq!(past(&app), history);
        assert!(app.drag.is_none() && app.live_stroke.is_none());
        assert!(app.ui.status_error);
    }
    // A retained/automation-created prompt must not bypass the explicit rasterize command.
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let id = open(&mut app, Kind::SmartObject, layer, Tool::Brush, [30.0, 20.0, 1.0]);
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(past(&app), history);
    app.ui.close_dialog(id);
    app.run("layer.rasterize.smartObject", json!({})).unwrap();
    assert_eq!(active_content(&app), "Pixel");
    assert_eq!(past(&app), history + 1);
    app.ui.tool = Tool::Brush;
    click(&mut app, 30.0, 20.0);
    assert_eq!(past(&app), history + 2);
    assert!(app.session.undo() && app.session.undo());
    assert_eq!(app.session.active().unwrap().doc, before);
}

#[test]
fn ok_rasterizes_then_paints_as_two_history_states() {
    for kind in Kind::ALL.into_iter().filter(|kind| *kind != Kind::SmartObject) {
        for tool in [Tool::Brush, Tool::Pencil] {
            let mut app = app_with(kind);
            app.ui.tool = tool;
            let before = past(&app);
            click(&mut app, 30.0, 20.0);
            let d = prompt(&app).unwrap_or_else(|| panic!("{kind:?}/{tool:?}: no prompt"));
            assert_eq!(d.fields["message"], json!(kind.message()));
            assert_eq!(past(&app), before, "nothing happens until the answer");
            let id = d.id;
            let r = crate::dialogs::confirm(&mut app, id).unwrap();
            assert_eq!(r["painted"], json!(true), "{kind:?}/{tool:?}");
            assert_eq!(active_content(&app), "Pixel");
            assert_eq!(past(&app), before + 2, "{kind:?}/{tool:?}: rasterize, then paint");
            let st = app.session.active().unwrap();
            let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(30, 20);
            assert!(c[2] > 0.99 && c[0] < 0.01, "{kind:?}/{tool:?}: painted blue at the click: {c:?}");
            // One undo takes the paint back; the layer stays rasterized.
            app.session.undo();
            assert_eq!(active_content(&app), "Pixel");
            app.session.undo();
            assert_eq!(active_content(&app), kind_name(kind));
        }
    }
}

#[test]
fn smart_object_masks_remain_paintable_without_rasterizing_contents() {
    let mut app = app_with(Kind::SmartObject);
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    let state = app.session.active().unwrap();
    let layer = state.active_layer.unwrap();
    let before = state.doc.layer(layer).unwrap().content.clone();
    let history = past(&app);
    app.ui.tool = Tool::Brush;
    app.ui.mask_target = true;
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    click(&mut app, 30.0, 20.0);
    assert!(prompt(&app).is_none());
    assert_eq!(past(&app), history + 1);
    let state = app.session.active().unwrap();
    assert_eq!(state.doc.layer(layer).unwrap().content, before);
    assert!(state.doc.layer(layer).unwrap().mask.as_ref().unwrap().surface.rgba(30, 20)[0] < 0.5);
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Type => "Type",
        Kind::Shape => "Shape",
        Kind::SmartObject => "Smart Object",
        Kind::Fill => "Fill",
    }
}

#[test]
fn cancel_does_nothing() {
    let mut app = app_with(Kind::Type);
    app.ui.tool = Tool::Brush;
    let before = (past(&app), app.session.active().unwrap().doc.clone());
    click(&mut app, 30.0, 20.0);
    let id = prompt(&app).unwrap().id;
    // A second press while it is up doesn't stack another prompt.
    click(&mut app, 40.0, 20.0);
    assert_eq!(app.ui.dialogs.len(), 1);
    app.ui.close_dialog(id).unwrap();
    assert_eq!(past(&app), before.0);
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before.1) || *app.session.active().unwrap().doc == *before.1);
    assert_eq!(active_content(&app), "Type");
}

#[test]
fn only_pixel_painting_asks() {
    let mut app = app_with(Kind::Type);
    for tool in [Tool::Move, Tool::RectMarquee, Tool::Eyedropper, Tool::Hand, Tool::Type] {
        app.ui.tool = tool;
        click(&mut app, 30.0, 20.0);
        assert!(prompt(&app).is_none(), "{tool:?} must not ask");
        app.ui.text_edit = None;
    }
    for tool in [Tool::Eraser, Tool::CloneStamp, Tool::PaintBucket, Tool::Smudge] {
        app.ui.tool = tool;
        click(&mut app, 30.0, 20.0);
        let id = prompt(&app).unwrap_or_else(|| panic!("{tool:?} asks")).id;
        app.ui.close_dialog(id);
    }
    // ⌥-click with the Clone Stamp sets the source.
    app.ui.tool = Tool::CloneStamp;
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 20.0, pressure: 1.0 }, Modifiers::ALT);
    assert!(prompt(&app).is_none());
    // Painting the layer's mask needs no rasterizing.
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    app.ui.tool = Tool::Brush;
    click(&mut app, 30.0, 20.0);
    assert!(prompt(&app).is_none());
    assert!(app.session.active().unwrap().doc.layers.iter().any(|l| matches!(l.content, LayerContent::Text(_))));
}

#[test]
fn a_stale_or_bad_prompt_fails_gracefully() {
    let mut app = app_with(Kind::Type);
    app.ui.tool = Tool::Brush;
    click(&mut app, 30.0, 20.0);
    let id = prompt(&app).unwrap().id;
    // The layer was rasterized elsewhere meanwhile: OK reports the error, nothing panics.
    app.run("layer.rasterize.type", json!({})).unwrap();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    // Garbage fields.
    for f in [json!({"__rasterize": "plaid", "layer": 1}), json!({"__rasterize": "type"}), json!({"__rasterize": "type", "layer": 999_999})] {
        let id = app.ui.open_dialog(crate::state::DialogKind::Command, f.as_object().unwrap().clone());
        assert!(crate::dialogs::confirm(&mut app, id).is_err(), "{f}");
    }
    // A bad tool or point rasterizes without painting.
    let mut app = app_with(Kind::Fill);
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let id = open(&mut app, Kind::Fill, layer, Tool::Brush, [f64::NAN, 1.0, 1.0]);
    app.ui.dialog_mut(id).unwrap().fields.insert("tool".into(), json!("no such tool"));
    let r = crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(r["painted"], json!(false));
    assert_eq!(active_content(&app), "Pixel");
}

#[test]
fn the_control_channel_sees_and_confirms_the_prompt() {
    let mut app = app_with(Kind::Shape);
    let ctx = egui::Context::default();
    app.ui.tool = Tool::Pencil;
    let call = |app: &mut PhotocraftApp, method: &str, params: serde_json::Value| {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Outcome::Done(v) => v,
            _ => panic!("deferred"),
        }
    };
    call(&mut app, "ui.pointer", json!({"events": [{"kind": "down", "x": 30, "y": 20}, {"kind": "up", "x": 30, "y": 20}]}));
    let id = prompt(&app).map(|d| d.id).expect("prompt");
    let r = call(&mut app, "ui.dialog.confirm", json!({"dialog": id}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(active_content(&app), "Pixel");
}
