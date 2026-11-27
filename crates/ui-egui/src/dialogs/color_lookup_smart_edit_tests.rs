//! #14419: a Color Lookup Smart Filter recorded without interpolation or dither keeps its look through its editor.
//!
//! Image › Adjustments › Color Lookup's dialog defaults to tetrahedral interpolation with Dither on, but a filter whose
//! params leave those keys out (recorded before #14419, or set by a script) renders trilinear without dither. Its
//! editor opens on what the filter renders with, so OK without an edit, which replaces the params, changes nothing.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use photocraft_doc::{Adjustment, LayerContent, LayerId, SmartObject};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// A 2x2x2 table that is the identity except white, pulled to mid gray: trilinear and tetrahedral interpolation weigh
/// that corner differently for any colour whose channels differ.
const BENT_LUT: &str = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n0.5 0.5 0.5\n";

/// A Smart Object filled with a colour whose channels all differ, carrying one Color Lookup Smart Filter whose params
/// are replaced by `params` (with the bent table): the shape of a filter recorded without some keys.
fn app_with_filter(params: Value) -> (Box<PhotocraftApp>, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8, "background": "#000000"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#bf8040"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("image.adjustments.colorLookup", json!({"data": BENT_LUT, "fileName": "bent.cube"})).unwrap();
    let mut params = params;
    params["data"] = json!(BENT_LUT);
    params["fileName"] = json!("bent.cube");
    app.run("layer.smartFilter.setParams", json!({"index": 0, "params": params, "replace": true})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    (app, layer)
}

fn smart(app: &PhotocraftApp, layer: LayerId) -> SmartObject {
    match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Smart(sm) => sm.clone(),
        _ => panic!("not a smart object"),
    }
}

/// The interpolation (tetrahedral or not) and dither the filter renders with, as the engine reads its params.
fn rendered_with(app: &PhotocraftApp, layer: LayerId) -> (bool, bool) {
    match photocraft_engine::adjust_cmds::lookup_from_params(&smart(app, layer).smart_filters[0].params, None).unwrap() {
        Adjustment::ColorLookup { tetrahedral, dither, .. } => (tetrahedral, dither),
        other => panic!("not a colour lookup: {other:?}"),
    }
}

fn field(app: &PhotocraftApp, id: DialogId, key: &str) -> Value {
    let fields = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("open dialog").fields().clone();
    serde_json::to_value(fields).unwrap().get(key).cloned().unwrap_or(Value::Null)
}

/// Opens the filter's editor as the Layers panel does, checks it shows what the filter renders with, and confirms it
/// without an edit; then the filter still renders the same way and the pixels are unchanged.
fn assert_ok_keeps_the_look(params: Value) {
    let (mut app, layer) = app_with_filter(params.clone());
    let ctx = app.ctx().clone();
    assert_eq!(rendered_with(&app, layer), (false, false), "{params}: the filter renders trilinear without dither");
    let before = photocraft_compose::flatten(&app.session.active().unwrap().doc);

    let sm = smart(&app, layer);
    crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
    let id = app.framework.dialogs.stack().ids().next().expect("the Color Lookup editor opens");
    assert_eq!(field(&app, id, "interpolation"), json!("trilinear"), "{params}: the editor shows the interpolation it renders with");
    assert_eq!(field(&app, id, "dither"), json!(false), "{params}: the editor shows the dither it renders with");
    let _ = crate::dialogs::confirm(&mut app, &ctx, id);
    for _ in 0..3 {
        crate::testing::run_ui(&ctx, Default::default(), |ui| crate::dialogs::run(&mut app, ui));
    }
    assert!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_none(), "{params}: OK closes the editor");

    assert_eq!(rendered_with(&app, layer), (false, false), "{params}: OK without an edit keeps trilinear without dither");
    assert!(before == photocraft_compose::flatten(&app.session.active().unwrap().doc), "{params}: OK without an edit keeps the pixels");
}

#[test]
fn ok_without_edits_keeps_a_filter_recorded_without_interpolation_or_dither() {
    assert_ok_keeps_the_look(json!({}));
}

/// The legacy `tetrahedral` flag stands in for `interpolation`; the editor shows it as Trilinear, not the default.
#[test]
fn ok_without_edits_keeps_a_filter_recorded_with_the_legacy_flag() {
    assert_ok_keeps_the_look(json!({"tetrahedral": false}));
}
