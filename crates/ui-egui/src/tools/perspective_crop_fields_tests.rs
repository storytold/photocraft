//! The engine's resolution limit for Perspective Crop: up to 29999.999 ppi commits, more is refused.

use serde_json::{Value, json};

use super::*;

/// A 200 × 150 document with the Perspective Crop tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

fn crop(app: &mut PhotocraftApp, resolution: f64) -> Result<Value, String> {
    let corners = [[10.0, 10.0], [190.0, 10.0], [190.0, 140.0], [10.0, 140.0]];
    app.run("image.perspectiveCrop", json!({"corners": corners, "resolution": resolution})).map_err(|e| e.to_string())
}

#[test]
fn the_crop_accepts_up_to_29999_999_ppi() {
    let mut app = app();
    assert!(crop(&mut app, 20000.0).is_ok(), "20000 ppi typed into the field commits");
    let mut app2 = self::app();
    assert!(crop(&mut app2, 29_999.999).is_ok());
    let mut app3 = self::app();
    let err = crop(&mut app3, 30_000.0).unwrap_err();
    assert!(err.contains("29999.999"), "{err}");
}
