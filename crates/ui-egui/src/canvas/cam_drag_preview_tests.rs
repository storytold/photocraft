//! Content-Aware Move drag (#12195): the source stays, a copy of the selected pixels follows the
//! outline, and a readout beside the pointer shows the offset. The filled result stays on the
//! Transform On Drop box. Release still runs `paint.contentAwareMove`.

use egui::{Pos2, Rect, vec2};
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::super::paint::{cam_drag_copy_image, cam_drag_readout, draw_drag_preview};
use super::super::view_xform::ViewXform;
use super::{DEBOUNCE_MS, step};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("paint.pencil", json!({"points": [[55, 55]], "size": 6, "hardness": 100, "color": "#0000ff"})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 40, "height": 40})).unwrap();
    app.ui.tool = Tool::ContentAwareMove;
    app
}

fn blue(app: &PhotocraftApp, x: i32, y: i32) -> bool {
    let st = app.session.active().unwrap();
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    layer.surface().unwrap().rgba(x, y)[0] < 0.5
}

fn drag_to(app: &mut PhotocraftApp, x: f64, y: f64) {
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
}

fn view() -> ViewXform {
    ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0)), zoom: 1.0, center: [200.0, 150.0], flip: false, rotation: 0.0, aspect: 1.0 }
}

#[test]
fn the_readout_is_the_horizontal_and_vertical_offset() {
    assert_eq!(cam_drag_readout([100, 50]), ["100 px".to_string(), "50 px".to_string()]);
    assert_eq!(cam_drag_readout([-12, 0]), ["-12 px".to_string(), "0 px".to_string()]);
}

#[test]
fn a_drag_keeps_the_source_moves_a_copy_and_shows_the_offset_until_release() {
    let mut app = app();
    drag_to(&mut app, 150.0, 100.0);
    assert!(step(&mut app, 0.0).is_none() && app.cam_preview.is_none());
    assert!(step(&mut app, DEBOUNCE_MS + 50.0).is_none() && app.cam_preview.is_none(), "resting does not fill the source");
    assert!(blue(&app, 55, 55) && !blue(&app, 155, 105), "the document stays at the source");

    let (area, image) = cam_drag_copy_image(&app).expect("a copy of the selected pixels");
    assert_eq!((area.x0, area.y0, area.width(), area.height()), (40, 40, 40, 40));
    let px = image.pixels[(15 * 40 + 15) as usize];
    assert!(px.r() < 40 && px.b() > 200, "the copy carries the blue pixel: {px:?}");
    let outside = image.pixels[0];
    assert!(outside.r() > 200 && outside.a() > 200, "the rest of the selection is copied too");

    let mut h = egui_kittest::Harness::builder().with_size(vec2(640.0, 480.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            draw_drag_preview(app, ui.painter(), &view());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.event(egui::Event::PointerMoved(egui::pos2(200.0, 160.0)));
    h.run_steps(4);
    assert!(h.query_by_label("→|").is_some() && h.query_by_label("100 px").is_some(), "horizontal offset");
    assert!(h.query_by_label("↓").is_some() && h.query_by_label("50 px").is_some(), "vertical offset");

    // A further move updates the readout; release removes it and runs the same command.
    let m = egui::Modifiers::NONE;
    tool_event(h.state_mut(), ToolEvent::Move { x: 190.0, y: 80.0, pressure: 1.0 }, m);
    h.run_steps(2);
    assert!(h.query_by_label("140 px").is_some() && h.query_by_label("30 px").is_some(), "the readout follows the pointer");
    tool_event(h.state_mut(), ToolEvent::Up { x: 190.0, y: 80.0 }, m);
    h.run_steps(2);
    assert!(h.query_by_label("→|").is_none() && h.query_by_label("↓").is_none(), "the readout is gone on release");
    let app = h.state();
    assert!(blue(app, 195, 85) && !blue(app, 55, 55), "release still moves the pixels");
    let (id, p) = app.session.journal.last().unwrap();
    assert_eq!(id, "paint.contentAwareMove");
    assert_eq!(p["offset"], json!([140, 30]));
    assert!(p.get("scaleX").is_none() && p.get("scaleY").is_none() && p.get("rotate").is_none());
}

#[test]
fn the_transform_box_still_previews_the_filled_result() {
    let mut app = app();
    app.ui.tool_options.cam_transform_on_drop = true;
    drag_to(&mut app, 150.0, 100.0);
    assert!(step(&mut app, DEBOUNCE_MS).is_none() && app.cam_preview.is_none());
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 100.0 }, egui::Modifiers::NONE);
    assert!(crate::cam_transform::active(&app));
    assert!(blue(&app, 55, 55) && !blue(&app, 155, 105), "the document waits for Enter");
    let mut now = 0.0;
    let mut shown = None;
    for _ in 0..40 {
        now += DEBOUNCE_MS;
        if let Some((doc, _)) = step(&mut app, now) {
            shown = Some(doc);
            break;
        }
        if let Some(running) = app.cam_preview.as_ref().and_then(|p| p.running.as_ref()) {
            let _ = running.finished.recv_timeout(std::time::Duration::from_secs(30));
        }
    }
    let preview = shown.expect("the box shows the filled result");
    let layer = preview.layers[0].surface().unwrap();
    assert!(layer.rgba(155, 105)[0] < 0.5 && layer.rgba(55, 55)[0] > 0.5, "the preview shows the move");
    assert!(blue(&app, 55, 55), "the document is still unchanged");
}
