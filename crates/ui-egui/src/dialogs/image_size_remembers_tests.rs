//! Image Size keeps its units, Resample and method from one OK to the next opening; Cancel keeps nothing (#14799).

use craft::ui::dialog::{DialogId, StackDialog as _};
use craft::ui::patterns::form_dialogs::{HostedImageSize, Resample};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};

use super::open;
use crate::PhotocraftApp;

const COMMAND: &str = "image.imageSize";

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 300, "height": 200, "resolution": 150})).unwrap();
    app
}

fn opened(app: &mut PhotocraftApp) -> DialogId {
    open(app, COMMAND).expect("a document is open")
}

fn set(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap_or_else(|e| panic!("{field}: {e}"));
}

fn fields(app: &PhotocraftApp, id: DialogId) -> Value {
    app.framework.dialogs.get_by_id::<HostedImageSize>(id).expect("Image Size is open").json_fields().expect("it has fields")
}

fn to(v: impl serde::Serialize) -> Value {
    serde_json::to_value(v).unwrap()
}

fn ok(app: &mut PhotocraftApp, id: DialogId) {
    crate::dialogs::testing::confirm(app, id).unwrap();
}

fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    crate::dialogs::testing::cancel(app, id).unwrap();
}

/// OK with Centimeters, Pixels/Centimeter and Nearest Neighbor: the next opening shows them, with Width and Height
/// from the document in that unit.
#[test]
fn ok_keeps_the_units_and_the_method() {
    let mut app = app();
    let id = opened(&mut app);
    set(&mut app, id, "widthUnit", to(UnitKind::Centimetres));
    set(&mut app, id, "resolutionUnit", to(UnitKind::PixelsPerCentimetre));
    set(&mut app, id, "resample", to(Resample::NearestNeighbor));
    ok(&mut app, id);

    let id = opened(&mut app);
    let f = fields(&app, id);
    assert_eq!(f["widthUnit"], to(UnitKind::Centimetres));
    assert_eq!(f["resolutionUnit"], to(UnitKind::PixelsPerCentimetre));
    assert_eq!(f["resample"], to(Resample::NearestNeighbor));
    // 300 px at 150 ppi are 2 in, about 5.08 cm: the width comes from the document, not from the last OK.
    let width = f["width"].as_f64().unwrap();
    assert!((width - 5.08).abs() < 0.01, "Width opens at the document's own size: {width}");
    cancel(&mut app, id);
}

/// OK with Resample off: the next opening has it off, and turning it on again gives the method it had.
#[test]
fn ok_keeps_resample_off_and_its_method() {
    let mut app = app();
    let id = opened(&mut app);
    set(&mut app, id, "resample", to(Resample::Bicubic));
    ok(&mut app, id);
    let id = opened(&mut app);
    set(&mut app, id, "resample", Value::Null);
    ok(&mut app, id);

    let id = opened(&mut app);
    assert_eq!(fields(&app, id)["resample"], Value::Null, "Resample opens off");
    cancel(&mut app, id);
    // The method kept with Resample off is the one the next OK with Resample on shows.
    let id = opened(&mut app);
    set(&mut app, id, "resample", to(Resample::Bicubic));
    ok(&mut app, id);
    let id = opened(&mut app);
    assert_eq!(fields(&app, id)["resample"], to(Resample::Bicubic));
    cancel(&mut app, id);
}

/// Cancel after changing the units, Resample and the method keeps nothing: the next opening shows the last OK's.
#[test]
fn cancel_keeps_nothing() {
    let mut app = app();
    let id = opened(&mut app);
    set(&mut app, id, "widthUnit", to(UnitKind::Centimetres));
    set(&mut app, id, "resample", to(Resample::BicubicSmoother));
    ok(&mut app, id);

    let id = opened(&mut app);
    set(&mut app, id, "widthUnit", to(UnitKind::Inches));
    set(&mut app, id, "resolutionUnit", to(UnitKind::PixelsPerCentimetre));
    set(&mut app, id, "resample", Value::Null);
    cancel(&mut app, id);

    let id = opened(&mut app);
    let f = fields(&app, id);
    assert_eq!(f["widthUnit"], to(UnitKind::Centimetres));
    assert_eq!(f["resolutionUnit"], to(UnitKind::PixelsPerInch));
    assert_eq!(f["resample"], to(Resample::BicubicSmoother));
    cancel(&mut app, id);
}

/// With nothing kept yet the dialog opens as before: the ruler unit, the Image Interpolation preference, Resample on.
#[test]
fn nothing_kept_opens_with_the_preferences() {
    let mut app = app();
    app.session.execute("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "inches"})).unwrap();
    app.session.execute("prefs.set", json!({"path": "general.imageInterpolation", "value": "bicubicSharper"})).unwrap();
    let id = opened(&mut app);
    let f = fields(&app, id);
    assert_eq!(f["widthUnit"], to(UnitKind::Inches));
    assert_eq!(f["resample"], to(Resample::BicubicSharper));
    cancel(&mut app, id);
}

/// A kept Bicubic opens as Nearest Neighbor on an Indexed Color document, which resamples with nothing else.
#[test]
fn a_kept_bicubic_opens_as_nearest_neighbor_on_an_indexed_document() {
    let mut app = app();
    let id = opened(&mut app);
    set(&mut app, id, "resample", to(Resample::Bicubic));
    ok(&mut app, id);
    app.session.execute("image.mode.indexedColor", json!({})).unwrap();
    let id = opened(&mut app);
    assert_eq!(fields(&app, id)["resample"], to(Resample::NearestNeighbor));
    cancel(&mut app, id);
}

/// On an Indexed Color document an OK with Resample off keeps the method the dialog showed (Nearest Neighbor),
/// not the Bicubic kept before it.
#[test]
fn resample_off_on_an_indexed_document_keeps_the_method_it_showed() {
    let mut app = app();
    let id = opened(&mut app);
    set(&mut app, id, "resample", to(Resample::Bicubic));
    ok(&mut app, id);
    app.session.execute("image.mode.indexedColor", json!({})).unwrap();
    let id = opened(&mut app);
    set(&mut app, id, "resample", Value::Null);
    ok(&mut app, id);

    let kept = super::image_memory().recall(app.framework.host()).expect("the OK kept its settings");
    assert!(!kept.resampling);
    assert_eq!(kept.method, Resample::NearestNeighbor);
}

/// A recorded step's method replaces the kept one: an OK with Resample off then keeps the method the dialog showed
/// (the recorded Nearest Neighbor), not the Bicubic kept before.
#[test]
fn resample_off_after_a_recorded_method_keeps_that_method() {
    let mut app = app();
    let id = opened(&mut app);
    set(&mut app, id, "resample", to(Resample::Bicubic));
    ok(&mut app, id);

    let recorded = json!({"width": 300, "height": 200, "resolution": 150, "resample": "nearest"});
    let id = super::open_with(&mut app, COMMAND, &recorded).expect("a document is open");
    assert_eq!(fields(&app, id)["resample"], to(Resample::NearestNeighbor), "the recorded method shows");
    set(&mut app, id, "resample", Value::Null);
    ok(&mut app, id);

    let kept = super::image_memory().recall(app.framework.host()).expect("the OK kept its settings");
    assert!(!kept.resampling);
    assert_eq!(kept.method, Resample::NearestNeighbor);
}

/// A recorded step's resolution is in pixels/inch: a kept Pixels/Centimeter unit doesn't reread it, so the
/// dialog shows the recorded resolution unchanged.
#[test]
fn a_recorded_resolution_opens_in_pixels_per_inch_with_a_kept_unit() {
    let mut app = app();
    let id = opened(&mut app);
    set(&mut app, id, "resolutionUnit", to(UnitKind::PixelsPerCentimetre));
    ok(&mut app, id);

    let recorded = json!({"width": 300, "height": 200, "resolution": 72, "resample": "none"});
    let id = super::open_with(&mut app, COMMAND, &recorded).expect("a document is open");
    let f = fields(&app, id);
    assert_eq!(f["resolutionUnit"], to(UnitKind::PixelsPerInch));
    let shown = f["resolution"].as_f64().unwrap();
    assert!((shown - 72.0).abs() < 0.01, "the recorded 72 ppi shows as 72 ppi: {shown}");
    cancel(&mut app, id);
}
