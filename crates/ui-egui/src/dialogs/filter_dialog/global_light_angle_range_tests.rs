//! #14980: Global Light's angle is stored within -180..=180: a recorded or scripted angle out of a turn wraps into
//! it, and 180 stays 180. The dialog's whole-degree -360..=360 field and its range alert are the framework's angle row.

use serde_json::json;

use crate::{PhotocraftApp, Services};

const CMD: &str = "layer.layerStyle.globalLight";

#[test]
fn a_scripted_global_light_angle_wraps_into_a_turn_and_keeps_180() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    for (input, stored) in [(200.0, -160.0), (-200.0, 160.0), (359.0, -1.0), (360.0, 0.0), (180.0, 180.0), (-180.0, -180.0)] {
        app.run(CMD, json!({"angle": input})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.global_light.angle, stored, "script {input}");
    }
}
