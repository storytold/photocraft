//! #12035: Image Rotation › Arbitrary pre-fills the ruler's straightening angle at the
//! precision the reference application shows. A ruler from 100,100 to 400,300 opens the Rotate
//! Canvas dialog at 33.69°, Counter clockwise; the angle keeps hundredths.

use craft::ui::dialog::{DialogId, HeldOk};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::filter_dialog::open;
use crate::PhotocraftApp;

const COMMAND: &str = "image.rotation.arbitrary";

/// A 318 px square document, a ruler from 100,100 to `end`, and the opened Arbitrary dialog.
fn open_ruler(end: [i32; 2]) -> (Box<PhotocraftApp>, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 318, "height": 318})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": end})).unwrap();
    let id = open(&mut app, COMMAND).expect("Arbitrary opens");
    (app, id)
}

/// The values the host holds for the dialog: angle and direction.
fn shown(app: &PhotocraftApp, id: DialogId) -> (f64, String) {
    let dialog = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("open dialog");
    let f = dialog.fields();
    (f.number("angle").expect("angle"), f.text("direction").expect("direction").to_string())
}

/// OK answered with no pass in between: the angle the dialog files for the command. The dialog holds its OK
/// for the app to answer, so the filed answer is read from its values.
fn confirmed_angle(app: &mut PhotocraftApp, id: DialogId) -> Option<f64> {
    let ctx = app.ctx().clone();
    app.framework.dialogs.confirm_by_id(&ctx, id);
    assert_eq!(app.framework.dialogs.held_state(id), Some(HeldOk::Waiting), "OK files its answer");
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the dialog stays open until the app answers").fields().number("angle")
}

/// A ruler falling to the right straightens counter-clockwise at the measured two decimals; a
/// rising one straightens clockwise at the same precision.
#[test]
fn ruler_prefill_keeps_the_measured_two_decimal_angle() {
    let (app, id) = open_ruler([400, 300]);
    let (angle, direction) = shown(&app, id);
    assert!((angle - 33.69).abs() < 1e-3, "the dialog opens at {angle}, want the measured 33.69");
    assert_eq!(direction, "ccw", "a ruler falling to the right straightens counter-clockwise");
    // (100,100) → (400,50) rises to the right: a 9.46° clockwise turn.
    let (app, id) = open_ruler([400, 50]);
    let (angle, direction) = shown(&app, id);
    assert_eq!(direction, "cw", "a rising ruler pre-fills Clockwise");
    assert!((angle - 9.46).abs() < 1e-3, "the clockwise pre-fill is {angle}, want 9.46");
}

/// #13067 (f32 double rounding): the 43x27 ruler is 32.1249984 degrees. OK with no shown pass
/// between open and OK, and again after the dialog was drawn, sends 32.12 exactly. (Rotate Canvas
/// has no Alt+Reset, #14462.)
#[test]
fn the_ruler_angle_is_rounded_before_the_first_frame() {
    let (mut app, id) = open_ruler([143, 127]);
    assert_eq!(confirmed_angle(&mut app, id), Some(32.12), "undrawn, OK sends the rounded angle");

    let (app, id) = open_ruler([143, 127]);
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    assert!(h.query_by_label("Cancel").is_some(), "the dialog is drawn");
    assert_eq!(confirmed_angle(h.state_mut(), id), Some(32.12), "drawn, OK sends the rounded angle");
}
