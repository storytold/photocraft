//! #12538: a Paint Bucket click on a fill layer asks the reference application's question and OK
//! only rasterizes the fill.

use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::testing::{ToolEvent, tool_event};

/// A Solid Color fill layer over a white Background, its content (not its mask) targeted, with
/// the Paint Bucket.
fn fill_layer_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("layer.newFillLayer.solidColor", json!({"color": "#00ff00"})).unwrap();
    app.ui.mask_target = false;
    app.sync_mask_targets();
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    app.ui.tool = Tool::PaintBucket;
    app
}

fn bucket_click(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 30.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 30.0, y: 20.0 }, Modifiers::NONE);
}

fn active_content(app: &PhotocraftApp) -> &'static str {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name()
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn the_paint_bucket_on_a_fill_layer_shows_the_reference_question() {
    let mut app = fill_layer_app();
    let before = past(&app);
    bucket_click(&mut app);
    let id = alerts::prompt_tagged::<Rasterize>(&app).expect("the prompt opens");
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(active_content(&app), "Pixel");
    assert_eq!(past(&app), before + 1, "one state: the rasterize, no fill");
}
