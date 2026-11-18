use serde_json::{Value, json};

use super::*;
use crate::smart_filter_dialog::tests::{confirm, newest, open_count, set};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    app
}

fn px(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.5 / 255.0)
}

/// Open the dialog, set `fields` (Contents first: the other rows show only for it), press OK.
fn fill(app: &mut PhotocraftApp, fields: Value) {
    let id = open(app);
    let map = fields.as_object().unwrap();
    for (k, v) in map.iter().filter(|(k, _)| *k == "contents").chain(map.iter().filter(|(k, _)| *k != "contents")) {
        set(app, id, k, v.clone());
    }
    confirm(app, id);
}

/// The dialog sends the chosen contents' option only, and each fill is one history step.
#[test]
fn every_contents_option_fills() {
    let mut app = app();
    fill(&mut app, json!({"contents": "color", "color": "#00ff00"}));
    assert!(close(px(&app, 3, 3), [0.0, 1.0, 0.0, 1.0]));
    // History: the default source is the opening state (the empty layer).
    fill(&mut app, json!({"contents": "history"}));
    assert_eq!(px(&app, 3, 3)[3], 0.0);
    let pattern = app.session.patterns.items.first().map(|p| p.id.clone()).unwrap();
    fill(&mut app, json!({"contents": "pattern", "pattern": pattern}));
    assert!(px(&app, 3, 3)[3] > 0.0);
    // Content-Aware fills a selected hole from around it.
    fill(&mut app, json!({"contents": "color", "color": "#336699"}));
    app.run("select.rect", json!({"x": 15, "y": 10, "width": 6, "height": 6})).unwrap();
    fill(&mut app, json!({"contents": "white"}));
    fill(&mut app, json!({"contents": "contentAware", "colorAdaptation": false}));
    assert!(close(px(&app, 17, 12), [0.2, 0.4, 0.6, 1.0]), "{:?}", px(&app, 17, 12));
    assert_eq!(app.session.active().unwrap().history.past_len(), 7);
}

/// The Blend row's ids reach the engine, and opacity and Preserve Transparency apply.
#[test]
fn mode_opacity_and_preserve_transparency() {
    let mut app = app();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    fill(&mut app, json!({"contents": "white", "opacity": 100, "mode": "normal", "preserveTransparency": false}));
    app.run("select.deselect", json!({})).unwrap();
    fill(&mut app, json!({"contents": "black", "opacity": 50, "preserveTransparency": true}));
    assert!(close(px(&app, 5, 5), [0.5, 0.5, 0.5, 1.0]), "{:?}", px(&app, 5, 5));
    assert_eq!(px(&app, 30, 5)[3], 0.0, "transparent pixels stay transparent");
    fill(&mut app, json!({"contents": "white", "opacity": 100, "mode": "multiply", "preserveTransparency": true}));
    assert!(close(px(&app, 5, 5), [0.5, 0.5, 0.5, 1.0]));
}

/// A targeted Smart Object mask is admitted and only the selected mask pixels fill.
#[test]
fn shift_f5_fills_the_selected_area_of_a_smart_object_mask() {
    let mut app = app();
    app.run("layer.new.layer", json!({"name": "Fill target"})).unwrap();
    app.run(COMMAND, json!({"contents": "white"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    app.run("select.rect", json!({"x": 2, "y": 3, "width": 8, "height": 6})).unwrap();

    let st = app.session.active().unwrap();
    let layer_id = st.active_layer.unwrap();
    let original = st.doc.layer(layer_id).unwrap().clone();
    let history = st.history.past_len();
    assert!(matches!(&original.content, photocraft_doc::LayerContent::Smart(_)));
    assert_eq!(original.mask.as_ref().unwrap().surface.sample_channel(4, 5, 0), 1.0);

    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).expect("Fill opens for a Smart Object mask");
    let id = newest(&app);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), history);
    assert_eq!(st.doc.layer(layer_id), Some(&original));
    set(&mut app, id, "contents", json!("black"));
    confirm(&mut app, id);

    let st = app.session.active().unwrap();
    let layer = st.doc.layer(layer_id).unwrap();
    let mask = &layer.mask.as_ref().unwrap().surface;
    assert_eq!(mask.sample_channel(4, 5, 0), 0.0);
    assert_eq!(mask.sample_channel(20, 20, 0), 1.0);
    assert_eq!(layer.content, original.content);
    assert_eq!(st.history.past_len(), history + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(layer_id), Some(&original));
}

/// Dialog admission must resolve the same mask target as menu enablement and execution.
#[test]
fn menu_and_context_fill_edit_non_raster_masks_and_undo() {
    for kind in ["adjustment icon", "adjustment mask", "shape", "filter"] {
        let mut app = app();
        match kind {
            "adjustment icon" | "adjustment mask" => {
                app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
                // Adjustment layers edit their mask even when its thumbnail isn't targeted.
                app.ui.mask_target = kind == "adjustment mask";
            }
            "shape" => {
                app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 40, 30], "fill": "#ff0000"})).unwrap();
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
                app.ui.mask_target = true;
            }
            _ => {
                app.run(COMMAND, json!({"contents": "white"})).unwrap();
                app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
                app.run("select.all", json!({})).unwrap();
                app.run("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
                app.ui.filter_mask_target = app.session.active().unwrap().active_layer.map(|id| id.0);
            }
        }
        app.run("select.rect", json!({"x": 2, "y": 3, "width": 8, "height": 6})).unwrap();
        let target = app.with_mask_target(COMMAND, json!({}));
        assert_eq!(target["target"], if kind == "filter" { "filterMask" } else { "mask" });
        assert!(crate::menus::is_enabled(&app, COMMAND), "{kind}");
        let sample = |app: &PhotocraftApp, x, y| {
            let st = app.session.active().unwrap();
            photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &target).unwrap().sample_channel(x, y, 0)
        };
        let history = app.session.active().unwrap().history.past_len();
        let ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).expect("enabled Fill opens its dialog");
        let dialog = newest(&app);
        assert_eq!(sample(&app, 4, 5), 1.0, "opening does not fill the {kind} mask");
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        set(&mut app, dialog, "contents", json!("black"));
        confirm(&mut app, dialog);
        assert_eq!(sample(&app, 4, 5), 0.0, "selected {kind} mask pixels are filled");
        assert_eq!(sample(&app, 20, 20), 1.0, "outside the selection is unchanged");
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(sample(&app, 4, 5), 1.0, "Undo restores the {kind} mask");
        let why = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"contents": "contentAware"})).unwrap_err();
        assert!(why.contains("layer pixels only"), "{why}");
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
}

/// A live control-channel target change must not redirect an admitted Fill.
#[test]
fn open_fill_retains_its_target_when_ui_set_changes_mask_target() {
    use crate::control::{ControlRequest, Reply, handle};
    for mask in [true, false] {
        let mut app = app();
        app.run(COMMAND, json!({"contents": "white"})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.ui.mask_target = mask;
        let ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
        let id = newest(&app);
        let (request, _) = ControlRequest::new("ui.set", json!({"maskTarget": !mask}));
        let Reply::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("ui.set should finish immediately") };
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(app.ui.mask_target, !mask);
        set(&mut app, id, "contents", json!("black"));
        let history = app.session.active().unwrap().history.past_len();
        confirm(&mut app, id);
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert_eq!(layer.mask.as_ref().unwrap().surface.sample_channel(4, 5, 0), if mask { 0.0 } else { 1.0 }, "the admitted mask target is retained");
        assert_eq!(layer.surface().unwrap().rgba(4, 5), if mask { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] }, "the admitted pixel target is retained");
        assert_eq!(st.history.past_len(), history + 1);
        app.run("edit.undo", json!({})).unwrap();
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert_eq!(layer.mask.as_ref().unwrap().surface.sample_channel(4, 5, 0), 1.0);
        assert_eq!(layer.surface().unwrap().rgba(4, 5), [1.0; 4]);
    }
}

/// The held OK refuses a different document through window.open, or a different layer in the same document.
#[test]
fn open_fill_and_stroke_refuse_a_different_document_or_layer() {
    use crate::control::{ControlRequest, Reply, handle};
    for command in [COMMAND, crate::stroke_ui::COMMAND] {
        for change in ["document", "layer"] {
            let mut app = app();
            app.run(COMMAND, json!({"contents": "white"})).unwrap();
            let original_layer = app.session.active().unwrap().active_layer.unwrap();
            app.run("layer.new.layer", json!({"name": "other layer"})).unwrap();
            let other_layer = app.session.active().unwrap().active_layer.unwrap();
            app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
            assert!(app.session.set_active(0));
            app.session.select_layer(original_layer).unwrap();
            let before: Vec<_> = app.session.documents().iter().map(|st| (st.doc.clone(), st.history.past_len())).collect();
            let ctx = app.ctx().clone();
            crate::menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
            let id = newest(&app);
            if command == COMMAND {
                set(&mut app, id, "contents", json!("black"));
            } else {
                set(&mut app, id, "location", json!("inside"));
            }
            if change == "document" {
                let (request, _) = ControlRequest::new("ui.window.open", json!({"document": 1}));
                let Reply::Done(_) = handle(&mut app, &ctx, &request) else { panic!("window.open should finish immediately") };
                assert_eq!(app.session.active_index(), Some(1), "the real control path changed the document while the dialog stayed open");
            } else {
                app.session.select_layer(other_layer).unwrap();
                assert_eq!(app.session.active_index(), Some(0));
                assert_eq!(app.session.active().unwrap().active_layer, Some(other_layer));
            }
            confirm(&mut app, id);
            assert_eq!(open_count(&app), 1, "{command}: the refused dialog stays open for a different {change}");
            for (st, (doc, history)) in app.session.documents().iter().zip(before) {
                assert_eq!(st.doc, doc, "neither document is edited");
                assert_eq!(st.history.past_len(), history);
            }
        }
    }
}

/// Capturing a mask target must retain the legitimate Alpha, Quick Mask and colour-row routes.
#[test]
fn open_fill_preserves_channel_targets() {
    for kind in ["alpha", "quick", "red", "red-blue"] {
        let mut app = app();
        app.run(COMMAND, json!({"contents": "white"})).unwrap();
        let target = match kind {
            "alpha" => {
                app.run("channel.new", json!({"name": "saved", "fill": "white"})).unwrap();
                app.run("channel.target", json!({"channel": "saved"})).unwrap();
                json!({"channel": 0})
            }
            "quick" => {
                app.run("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                json!("quickMask")
            }
            _ => {
                let colors = if kind == "red" { vec![0] } else { vec![0, 2] };
                app.run("channel.target", json!({"colors": colors})).unwrap();
                json!({"colors": colors})
            }
        };
        let target_before = {
            let st = app.session.active().unwrap();
            photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target": target})).unwrap().clone()
        };
        let id = open(&mut app);
        set(&mut app, id, "contents", json!("black"));
        let history = app.session.active().unwrap().history.past_len();
        confirm(&mut app, id);
        let st = app.session.active().unwrap();
        let color = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(4, 5);
        if matches!(kind, "alpha" | "quick") {
            let p = json!({"target": target});
            assert_eq!(photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &p).unwrap().sample_channel(4, 5, 0), 0.0);
            assert_eq!(color, [1.0; 4], "{kind} leaves layer pixels unchanged");
        } else {
            assert_eq!(color, if kind == "red" { [0.0, 1.0, 1.0, 1.0] } else { [0.0, 1.0, 0.0, 1.0] }, "{kind}");
        }
        assert_eq!(st.history.past_len(), history + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(px(&app, 4, 5), [1.0; 4]);
        let st = app.session.active().unwrap();
        assert_eq!(
            photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target": target})).unwrap(),
            &target_before,
            "Undo restores the {kind} samples too"
        );
    }
}
