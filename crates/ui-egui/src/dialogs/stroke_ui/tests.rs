use serde_json::json;

use super::*;
use crate::smart_filter_dialog::tests::{cancel, confirm, newest, open_count, set};

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    app
}

fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

/// Stroke on an ordinary adjustment mask is disabled and opens no dialog.
#[test]
fn menu_and_canvas_invocations_open_options_but_explicit_params_run_directly() {
    let mut app = app();
    let ctx = app.ctx().clone();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    assert!(!crate::menus::is_enabled(&app, COMMAND), "ordinary adjustment masks retain their existing refusal");
    assert!(crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).is_err());
    assert_eq!(open_count(&app), 0);
}

/// A real filter mask supports direct Stroke and must also admit its dialog, selection and Undo.
#[test]
fn menu_stroke_edits_the_targeted_filter_mask_and_undoes() {
    let mut app = app();
    app.run("edit.fill", json!({"contents": "white"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
    app.run("layer.setProps", json!({"locks": {"pixels": true, "transparency": true}})).unwrap();
    app.ui.filter_mask_target = app.session.active().unwrap().active_layer.map(|id| id.0);
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    assert!(app.filter_mask_targeted());
    assert!(crate::menus::is_enabled(&app, COMMAND));
    let sample = |app: &PhotocraftApp, x, y| {
        let st = app.session.active().unwrap();
        photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target": "filterMask"})).unwrap().sample_channel(x, y, 0)
    };
    let history = app.session.active().unwrap().history.past_len();
    assert_eq!(sample(&app, 10, 15), 1.0, "the real filter mask starts white");

    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    let dialog = newest(&app);
    set(&mut app, dialog, "width", json!(2));
    set(&mut app, dialog, "location", json!("inside"));
    set(&mut app, dialog, "color", json!("#000000"));
    confirm(&mut app, dialog);
    assert_eq!(sample(&app, 10, 15), 0.0, "the mask's selected edge is stroked");
    assert_eq!(sample(&app, 15, 15), 1.0, "the selection's interior is unchanged");
    assert_eq!(sample(&app, 9, 15), 1.0, "outside is unchanged");
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(sample(&app, 10, 15), 1.0);
}

/// An implicit white smart-filter mask is a dialog target, and ui.set cannot redirect its Stroke.
#[test]
fn implicit_filter_mask_dialog_retains_its_target_when_ui_set_changes_it() {
    use crate::control::{ControlRequest, Reply, handle};
    use photocraft_doc::LayerContent;
    let mut app = app();
    app.run("edit.fill", json!({"contents": "white"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    assert!(matches!(&app.session.active().unwrap().doc.layer(layer).unwrap().content, LayerContent::Smart(sm) if sm.filter_mask.is_none()));
    app.ui.filter_mask_target = Some(layer.0);
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    let id = newest(&app);
    let (request, _) = ControlRequest::new("ui.set", json!({"filterMaskTarget": false}));
    let Reply::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("ui.set should finish immediately") };
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!app.filter_mask_targeted());
    set(&mut app, id, "color", json!("#000000"));
    set(&mut app, id, "location", json!("inside"));
    let history = app.session.active().unwrap().history.past_len();
    confirm(&mut app, id);
    let st = app.session.active().unwrap();
    let surface = photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target": "filterMask"})).unwrap();
    assert_eq!(surface.sample_channel(0, 15, 0), 0.0);
    assert_eq!(surface.sample_channel(15, 15, 0), 1.0);
    assert_eq!(st.history.past_len(), history + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert!(
        matches!(&app.session.active().unwrap().doc.layer(layer).unwrap().content, LayerContent::Smart(sm) if sm.filter_mask.is_none()),
        "Undo restores the implicit representation"
    );
}

/// Stroke retains its existing layer edit in Alpha and Quick Mask views, leaving their samples alone.
#[test]
fn stroke_dialog_keeps_layer_editing_in_channel_views() {
    for quick in [false, true] {
        let mut app = app();
        app.run("edit.fill", json!({"contents": "white"})).unwrap();
        let target = if quick {
            app.run("select.editInQuickMaskMode", json!({"on": true})).unwrap();
            json!("quickMask")
        } else {
            app.run("channel.new", json!({"fill": "white"})).unwrap();
            app.run("channel.target", json!({"channel": 0})).unwrap();
            json!({"channel": 0})
        };
        let samples = |app: &PhotocraftApp| {
            let st = app.session.active().unwrap();
            photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target": target})).unwrap().clone()
        };
        let before = samples(&app);
        let history = app.session.active().unwrap().history.past_len();
        let id = open(&mut app);
        set(&mut app, id, "color", json!("#00ff00"));
        set(&mut app, id, "location", json!("inside"));
        confirm(&mut app, id);
        assert_eq!(pixel(&app, 0, 15), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(pixel(&app, 15, 15), [1.0; 4]);
        assert_eq!(samples(&app), before, "the viewed channel is not stroked");
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&app, 0, 15), [1.0; 4]);
    }
}

/// Width, colour, location and mode reach `edit.stroke`; Cancel changes nothing.
#[test]
fn stroke_choices_apply_on_confirm_and_cancel_does_not_modify_document() {
    let mut app = app();
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    let initial = app.session.active().unwrap().history.past_len();
    let cancelled = open(&mut app);
    set(&mut app, cancelled, "color", json!("#00ff00"));
    set(&mut app, cancelled, "width", json!(4.0));
    cancel(&mut app, cancelled);
    assert_eq!(open_count(&app), 0);
    assert_eq!(app.session.active().unwrap().history.past_len(), initial);
    assert_eq!(pixel(&app, 10, 15)[3], 0.0);

    let applied = open(&mut app);
    set(&mut app, applied, "color", json!("#00ff00"));
    set(&mut app, applied, "width", json!(4.0));
    set(&mut app, applied, "location", json!("inside"));
    set(&mut app, applied, "mode", json!("normal"));
    confirm(&mut app, applied);
    assert!(pixel(&app, 10, 15)[1] > 0.98, "inside edge receives selected green stroke");
    assert_eq!(pixel(&app, 15, 15)[3], 0.0, "unselected interior unchanged");
    assert_eq!(pixel(&app, 9, 15)[3], 0.0, "outside unchanged for Inside location");
    assert_eq!(app.session.active().unwrap().history.past_len(), initial + 1);
}

/// #15539: Width shows px and Opacity % after their numbers.
#[test]
fn width_and_opacity_declare_their_units() {
    let rows = rows([0.0, 0.0, 0.0]);
    let unit = |key: &str| rows.iter().find(|p| p.key == key).and_then(|p| p.unit);
    assert_eq!(unit("width"), Some(UnitKind::Pixels));
    assert_eq!(unit("opacity"), Some(UnitKind::Percent));
}
