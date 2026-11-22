//! #14791: Image Size's Resample list by colour mode. Bitmap offers only Automatic and Nearest
//! Neighbor, as Indexed Color does, and the command resamples a Bitmap document with Nearest
//! Neighbor; an 8-bit RGB document keeps the full list.

use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::HostedImageSize;
use serde_json::{Value, json};

use super::*;

fn method(m: Resample) -> Value {
    serde_json::to_value(m).unwrap()
}

/// A 2 x 1 document, left pixel black and right pixel white, in `mode` (`"rgb"` or `"bitmap"`).
fn app_in(mode: &str) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 2, "height": 1, "mode": "grayscale", "background": "white"})).unwrap();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    match mode {
        "bitmap" => app.session.execute("image.mode.bitmap", json!({"method": "threshold"})).unwrap(),
        _ => app.session.execute("image.mode.rgb", json!({})).unwrap(),
    };
    app
}

/// Whether the open Image Size offers `m`: the framework refuses a method its list leaves out.
fn offers(app: &mut PhotocraftApp, m: Resample) -> bool {
    let id = open(app, IMAGE_COMMAND).expect("a document is open");
    let ctx = app.ctx().clone();
    let offered = app.framework.dialogs.set_json_field(&ctx, id, "resample", method(m)).is_ok();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    offered
}

#[test]
fn bitmap_offers_automatic_and_nearest_neighbor_only() {
    let mut app = app_in("bitmap");
    app.session.execute("prefs.set", json!({"path": "general.imageInterpolation", "value": "bicubic"})).unwrap();
    let id = open(&mut app, IMAGE_COMMAND).unwrap();
    let fields = app.framework.dialogs.get_by_id::<HostedImageSize>(id).unwrap().json_fields().unwrap();
    assert_eq!(fields["resample"], method(Resample::NearestNeighbor), "a remembered Bicubic opens on Nearest Neighbor");
    let ctx = app.ctx().clone();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    for m in [Resample::Bicubic, Resample::BicubicSmoother, Resample::BicubicSharper, Resample::Bilinear, Resample::PreserveDetails] {
        assert!(!offers(&mut app, m), "{m:?} is not offered for Bitmap");
    }
    assert!(offers(&mut app, Resample::Automatic) && offers(&mut app, Resample::NearestNeighbor));
}

#[test]
fn rgb_keeps_the_full_list() {
    let mut app = app_in("rgb");
    for m in [Resample::Automatic, Resample::Bicubic, Resample::BicubicSmoother, Resample::Bilinear, Resample::NearestNeighbor] {
        assert!(offers(&mut app, m), "{m:?} is offered for RGB");
    }
}

#[test]
fn image_size_resamples_bitmap_with_nearest_neighbor() {
    let mut app = app_in("bitmap");
    for resample in ["bicubic", "automatic", "bilinear", "lanczos"] {
        app.session.execute("image.imageSize", json!({"width": 8, "resample": resample})).unwrap();
        let surface = app.session.active().unwrap().doc.layers[0].surface().unwrap().clone();
        let row: Vec<f32> = (0..8).map(|x| surface.pixel(x, 0)[0]).collect();
        assert_eq!(row, [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], "{resample}: only black and white, split in the middle");
        app.session.execute("history.undo", json!({})).unwrap();
    }
}
