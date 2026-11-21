//! #14581: Canvas Size opens with the Relative, unit and extension colour of the last OK (the anchor at the centre);
//! Cancel stores nothing; a change of the ruler units since makes it open in the new ruler unit; a run without the
//! dialog sets the extension colour it opens on; fresh settings keep today's defaults.

use craft::behavior::Anchor9;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::CanvasExtension;
use craft::ui::runtime::env::UnitKind;
use photocraft_engine::prefs::Unit;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// A 400 × 300 px document at 72 ppi with a Background.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "resolution": 72})).unwrap();
    app
}

fn open(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(app, &ctx, "image.canvasSize", json!({})).unwrap();
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
        app.framework.dialogs.set_json_field(&ctx, id, k, v.clone()).unwrap_or_else(|e| panic!("{k}: {e}"));
    }
}

fn ok(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
    crate::dialogs::testing::show(app);
}

fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::cancel(app, &ctx, id).unwrap();
}

fn unit(u: UnitKind) -> Value {
    json!(u)
}

#[test]
fn relative_unit_and_colour_come_back_and_the_anchor_is_centred() {
    let mut app = app();
    let id = open(&mut app);
    set(
        &mut app,
        id,
        json!({"relative": true, "unit": unit(UnitKind::Centimetres), "extension": json!(CanvasExtension::White), "anchor": json!(Anchor9::TopLeft)}),
    );
    set(&mut app, id, json!({"width": 1.0, "height": 1.0}));
    ok(&mut app, id);
    let id = open(&mut app);
    let f = shown(&app, id);
    assert_eq!(f["relative"], json!(true));
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(0.0), Some(0.0)), "relative fields open at 0: {f}");
    assert_eq!(f["unit"], unit(UnitKind::Centimetres));
    assert_eq!(f["extension"], json!(CanvasExtension::White));
    assert_eq!(f["anchor"], json!(Anchor9::Centre), "the anchor opens at the centre");
}

#[test]
fn absolute_inches_come_back_with_the_current_size() {
    let mut app = app();
    let id = open(&mut app);
    set(&mut app, id, json!({"unit": unit(UnitKind::Inches), "anchor": json!(Anchor9::TopLeft)}));
    set(&mut app, id, json!({"width": 6.0}));
    ok(&mut app, id);
    let width = app.session.active().unwrap().doc.size.width;
    assert!(width > 400, "it ran: {width}");
    let id = open(&mut app);
    let f = shown(&app, id);
    assert_eq!((f["relative"].clone(), f["unit"].clone(), f["anchor"].clone()), (json!(false), unit(UnitKind::Inches), json!(Anchor9::Centre)));
    assert!((f["width"].as_f64().unwrap() - f64::from(width) / 72.0).abs() < 0.01, "the current width in inches: {f}");
}

#[test]
fn cancel_stores_nothing() {
    let mut app = app();
    let id = open(&mut app);
    set(&mut app, id, json!({"unit": unit(UnitKind::Inches), "extension": json!(CanvasExtension::Black)}));
    set(&mut app, id, json!({"width": 6.0}));
    ok(&mut app, id);
    let id = open(&mut app);
    set(&mut app, id, json!({"relative": true, "unit": unit(UnitKind::Millimetres), "extension": json!(CanvasExtension::White)}));
    cancel(&mut app, id);
    let id = open(&mut app);
    let f = shown(&app, id);
    assert_eq!((f["relative"].clone(), f["unit"].clone(), f["extension"].clone()), (json!(false), unit(UnitKind::Inches), json!(CanvasExtension::Black)));
}

#[test]
fn a_ruler_unit_change_since_the_ok_opens_in_the_ruler_unit() {
    let mut app = app();
    let id = open(&mut app);
    set(&mut app, id, json!({"unit": unit(UnitKind::Inches)}));
    set(&mut app, id, json!({"width": 6.0}));
    ok(&mut app, id);
    app.session.prefs.edit(|p| p.units_and_rulers.rulers = Unit::Millimeters);
    let id = open(&mut app);
    assert_eq!(shown(&app, id)["unit"], unit(UnitKind::Millimetres));
}

#[test]
fn a_run_without_the_dialog_sets_the_colour_it_opens_on() {
    let mut app = app();
    app.run("image.canvasSize", json!({"width": 420, "height": 320, "anchor": "center", "extensionColor": "black"})).unwrap();
    let id = open(&mut app);
    assert_eq!(shown(&app, id)["extension"], json!(CanvasExtension::Black));
}

/// A run that leaves `extensionColor` out extends with the Background, so that is what the dialog opens on
/// (CodeRabbit on #16146).
#[test]
fn a_run_without_a_colour_opens_the_dialog_on_the_background() {
    let mut app = app();
    app.run("image.canvasSize", json!({"width": 420, "height": 320, "anchor": "center", "extensionColor": "black"})).unwrap();
    app.run("image.canvasSize", json!({"width": 440, "height": 340, "anchor": "center"})).unwrap();
    let id = open(&mut app);
    assert_eq!(shown(&app, id)["extension"], json!(CanvasExtension::Background));
}

/// A Canvas Size action step that runs without a dialog records its colour too.
#[test]
fn a_canvas_size_action_step_sets_the_colour_the_dialog_opens_on() {
    use craft::automation::actions::{Action, ActionStep};
    let mut app = app();
    let step = ActionStep::from(("image.canvasSize".to_string(), json!({"width": 420, "height": 320, "anchor": "center", "extensionColor": "black"})));
    app.session.actions.list.push(Action::new("Size", vec![step])).expect("a new name");
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let id = open(&mut app);
    assert_eq!(shown(&app, id)["extension"], json!(CanvasExtension::Black));
}

#[test]
fn fresh_settings_keep_todays_defaults() {
    let mut app = app();
    let id = open(&mut app);
    let f = shown(&app, id);
    assert_eq!((f["relative"].clone(), f["unit"].clone(), f["extension"].clone()), (json!(false), unit(UnitKind::Pixels), json!(CanvasExtension::Background)));
}
