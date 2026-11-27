//! Preserve Transparency in Fill and Stroke follows the target layer (#14651): greyed off on the Background, greyed on
//! with Lock Transparent Pixels, and neither forced state replaces the remembered choice.

use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::json;

use super::*;
use crate::smart_filter_dialog::tests::{cancel, confirm, set};

/// A document with a white Background and a pixel layer "Paint" above it, active, with a selection to fill or stroke.
fn app() -> (Box<PhotocraftApp>, photocraft_doc::LayerId, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    let background = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({"name": "Paint"})).unwrap();
    let paint = app.session.active().unwrap().active_layer.unwrap();
    app.run("select.rect", json!({"x": 5, "y": 5, "width": 20, "height": 15})).unwrap();
    (app, background, paint)
}

/// The box's value in the open dialog `id`, and whether it takes a change.
fn preserve(app: &mut PhotocraftApp, id: DialogId) -> (bool, bool) {
    let dialog = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("a hosted dialog");
    let on = dialog.fields().bool("preserveTransparency").expect("the box has a value");
    let ctx = app.ctx().clone();
    let editable = app.framework.dialogs.set_json_field(&ctx, id, "preserveTransparency", json!(on)).is_ok();
    (on, editable)
}

#[test]
fn the_box_is_greyed_at_the_targets_state_and_the_remembered_choice_survives() {
    for (name, open) in [("Fill", open as fn(&mut PhotocraftApp) -> DialogId), ("Stroke", crate::stroke_ui::open)] {
        let (mut app, background, paint) = app();
        let id = open(&mut app);
        assert_eq!(preserve(&mut app, id), (false, true), "{name}: a pixel layer offers the default");
        set(&mut app, id, "preserveTransparency", json!(true));
        confirm(&mut app, id);

        app.session.select_layer(background).unwrap();
        let id = open(&mut app);
        assert_eq!(preserve(&mut app, id), (false, false), "{name}: greyed off on the Background");
        confirm(&mut app, id);

        app.session.select_layer(paint).unwrap();
        let id = open(&mut app);
        assert_eq!(preserve(&mut app, id), (true, true), "{name}: the Background's forced state is not remembered");
        set(&mut app, id, "preserveTransparency", json!(false));
        confirm(&mut app, id);

        app.run("layer.toggleLockTransparency", json!({})).unwrap();
        let id = open(&mut app);
        assert_eq!(preserve(&mut app, id), (true, false), "{name}: greyed on with Lock Transparent Pixels");
        confirm(&mut app, id);

        app.run("layer.toggleLockTransparency", json!({})).unwrap();
        let id = open(&mut app);
        assert_eq!(preserve(&mut app, id), (false, true), "{name}: the lock's forced state is not remembered");
        cancel(&mut app, id);
    }
}
