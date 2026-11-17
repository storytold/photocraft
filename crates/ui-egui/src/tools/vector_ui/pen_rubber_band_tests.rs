//! #12900: the Rubber Band setting of the Pen's path options is a tool option: saved, reset by
//! Reset Tool, reported by `ui.inspect`.

use serde_json::json;

use crate::PhotocraftApp;
use crate::control::{ControlRequest, Reply, handle};
use crate::state::{Tool, ToolOptions};

#[test]
fn the_setting_is_saved_inspected_and_reset() {
    // Saved with the tool options.
    let mut options = ToolOptions::default();
    assert!(!options.path_options.rubber_band, "off by default, as in Photoshop");
    options.path_options.rubber_band = true;
    let back: ToolOptions = serde_json::from_value(serde_json::to_value(&options).unwrap()).unwrap();
    assert!(back.path_options.rubber_band);

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    let ctx = app.ctx().clone();
    let inspect = |app: &mut PhotocraftApp| {
        let (req, _rx) = ControlRequest::new("ui.inspect", json!({}));
        match handle(app, &ctx, &req) {
            Reply::Done(v) => v["result"]["toolOptions"]["path_options"]["rubberBand"].clone(),
            _ => panic!("ui.inspect: expected an immediate reply"),
        }
    };
    assert_eq!(inspect(&mut app), json!(false));
    app.ui.tool_options.path_options.rubber_band = true;
    app.ui.tool_options.feather = 12.0;
    assert_eq!(inspect(&mut app), json!(true));

    // Reset Tool turns it off and leaves another tool's option alone.
    crate::menus::invoke(&mut app, &ctx, "tool.reset", json!({})).unwrap();
    assert!(!app.ui.tool_options.path_options.rubber_band);
    assert_eq!(app.ui.tool_options.feather, 12.0);
    assert_eq!(inspect(&mut app), json!(false));
}
