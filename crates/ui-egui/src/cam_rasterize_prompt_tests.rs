use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

fn type_app_with_selection() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 40, "text": "Hello"})).unwrap();
    app.run("select.rect", json!({"x": 8, "y": 16, "width": 40, "height": 30})).unwrap();
    app.ui.tool = Tool::ContentAwareMove;
    app.ui.tool_options.sample_all.content_aware_move = false;
    app.ui.tool_options.cam_transform_on_drop = false;
    app
}

fn active_content(app: &PhotocraftApp) -> &'static str {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name()
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn prompt(app: &PhotocraftApp) -> Option<&crate::state::Dialog> {
    app.ui.dialogs.iter().find(|d| owns(&d.fields))
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
}

#[test]
fn content_aware_move_on_type_layer_prompts_to_rasterize_and_ok_runs_move() {
    let mut app = type_app_with_selection();
    let before = past(&app);

    // Pressing inside the selection does not prompt until the non-zero drag is released.
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(prompt(&app).is_none(), "mouse-down alone must not ask to rasterize");
    tool_event(&mut app, ToolEvent::Move { x: 45.0, y: 35.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 45.0, y: 35.0 }, Modifiers::NONE);

    let d = prompt(&app).expect("releasing a move drag on a type layer opens the rasterize prompt");
    assert_eq!(d.fields["message"], json!(Kind::Type.message()));
    assert_eq!(d.fields["at"], json!([25.0, 5.0, 1.0]));
    assert_eq!(past(&app), before, "nothing happens before answering the prompt");
    assert_eq!(active_content(&app), "Type");

    let id = d.id;
    let r = crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(r["painted"], json!(true));
    assert_eq!(active_content(&app), "Pixel");
    assert_eq!(past(&app), before + 2, "rasterize type and content-aware move are two history states");
    let (cmd, params) = app.session.journal.last().expect("journal recorded move");
    assert_eq!(cmd, "paint.contentAwareMove");
    assert_eq!(params["offset"], json!([25, 5]));
}

#[test]
fn content_aware_move_on_type_layer_with_transform_on_drop_opens_box_after_rasterize() {
    let mut app = type_app_with_selection();
    app.ui.tool_options.cam_transform_on_drop = true;
    let before = past(&app);

    drag(&mut app, [20.0, 30.0], [44.0, 36.0]);
    assert!(app.ui.cam_transform.is_none(), "Transform On Drop waits for the rasterize prompt");
    let id = prompt(&app).expect("rasterize prompt opened").id;

    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(active_content(&app), "Pixel");
    assert_eq!(past(&app), before + 1, "rasterized first");
    assert!(app.ui.cam_transform.is_some(), "Transform On Drop box opened on the rasterized layer");

    crate::cam_transform::commit(&mut app);
    assert!(app.ui.cam_transform.is_none());
    assert_eq!(past(&app), before + 2, "committing Transform On Drop records the second history state");
}

#[test]
fn content_aware_move_cancel_leaves_type_layer_and_selection_untouched() {
    let mut app = type_app_with_selection();
    let before = (past(&app), app.session.active().unwrap().doc.clone());

    drag(&mut app, [20.0, 30.0], [50.0, 40.0]);
    let id = prompt(&app).expect("rasterize prompt opened").id;
    crate::dialogs::cancel(&mut app, id).unwrap();

    assert!(app.ui.dialogs.is_empty());
    assert_eq!(past(&app), before.0);
    assert_eq!(active_content(&app), "Type");
    assert_eq!(*app.session.active().unwrap().doc, *before.1, "document and selection are untouched after Cancel");
    assert!(app.session.active().unwrap().doc.selection.as_ref().is_some_and(|s| s.sample_channel(20, 30, 0) > 0.0));
}

#[test]
fn drawing_new_selection_or_sample_all_layers_on_type_layer_does_not_prompt() {
    // Drawing a new selection with Content-Aware Move on a type layer does not ask to rasterize.
    let mut app = type_app_with_selection();
    app.run("select.deselect", json!({})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 10.0, y: 30.0 }, Modifiers::NONE);
    assert!(prompt(&app).is_none(), "drawing a lasso selection on a type layer must not ask to rasterize");
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert_eq!(active_content(&app), "Type");

    // With Sample All Layers on, moving refuses a type layer without asking to rasterize.
    let mut app = type_app_with_selection();
    app.ui.tool_options.sample_all.content_aware_move = true;
    drag(&mut app, [20.0, 30.0], [45.0, 35.0]);
    assert!(prompt(&app).is_none(), "Sample All Layers on must not ask to rasterize");
    assert!(app.ui.status_error);
    assert_eq!(active_content(&app), "Type");
}

#[test]
fn content_aware_move_second_drop_and_selection_change_preserve_dialog_offset() {
    let mut app = type_app_with_selection();
    drag(&mut app, [20.0, 30.0], [45.0, 35.0]);
    assert_eq!(prompt(&app).unwrap().fields["at"], json!([25.0, 5.0, 1.0]));

    // A second drop while the prompt is open updates the pending dialog's offset and selection.
    drag(&mut app, [20.0, 30.0], [32.0, 26.0]);
    assert_eq!(app.ui.dialogs.len(), 1);
    let d = prompt(&app).unwrap();
    assert_eq!(d.fields["at"], json!([12.0, -4.0, 1.0]));
    let id = d.id;

    // Changing the selection before confirming fails without rasterizing, while keeping the dialog and offset open.
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20})).unwrap();
    let before_confirm = past(&app);
    let err = crate::dialogs::confirm(&mut app, id).unwrap_err();
    assert_eq!(err, "the selection changed");
    assert_eq!(active_content(&app), "Type");
    assert_eq!(past(&app), before_confirm);
    let reopened = prompt(&app).expect("dialog remains open with error and preserved offset");
    assert_eq!(reopened.fields["at"], json!([12.0, -4.0, 1.0]));
    assert_eq!(reopened.fields["__error"], json!("the selection changed"));
}

#[test]
fn content_aware_move_on_shape_smart_object_and_fill_layers_keeps_refusal_path() {
    for kind in [Kind::Shape, Kind::SmartObject, Kind::Fill] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
        match kind {
            Kind::Shape => {
                app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 60, 40], "fill": "#ff0000"})).unwrap();
            }
            Kind::SmartObject => {
                app.run("layer.new.layer", json!({})).unwrap();
                app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
                app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            }
            Kind::Fill => {
                app.run("layer.newFillLayer.solidColor", json!({"color": "#00ff00"})).unwrap();
            }
            Kind::Type => continue,
        }
        app.ui.mask_target = false;
        app.sync_mask_targets();
        app.run("select.rect", json!({"x": 8, "y": 16, "width": 40, "height": 30})).unwrap();
        app.ui.tool = Tool::ContentAwareMove;
        app.ui.tool_options.sample_all.content_aware_move = false;

        drag(&mut app, [20.0, 30.0], [45.0, 35.0]);
        assert!(prompt(&app).is_none(), "{kind:?} must not open the rasterize prompt for Content-Aware Move");
        assert!(app.ui.status_error, "{kind:?} must keep the refusal status error");
    }
}
