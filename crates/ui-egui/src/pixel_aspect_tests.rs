//! View › Pixel Aspect Ratio (#2594): with correction on, the canvas shows each document pixel
//! that many times wider, and the pointer still lands on the pixel drawn under it.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::active_view;
use crate::state::Tool;

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "transparent"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 1, "smoothing": {"amount": 0.0}}})).unwrap();
    app.sync_views();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::testing::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h
}

fn menu(h: &mut Harness<'static, Box<PhotocraftApp>>, id: &str, params: serde_json::Value) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, id, params).unwrap();
    h.run_steps(3);
}

fn click(h: &mut Harness<'static, Box<PhotocraftApp>>, pos: Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

fn alpha(h: &Harness<'static, Box<PhotocraftApp>>, x: i32, y: i32) -> f32 {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

#[test]
fn a_click_paints_the_pixel_drawn_under_the_pointer() {
    let mut h = harness();
    menu(&mut h, "view.pixelAspectRatio.anamorphic2To1", json!({}));
    let view = active_view(h.state()).expect("the canvas showed a view");
    let at = view.to_screen(craft::behavior::kurbo::Point::new(120.5, 30.5));
    click(&mut h, at);
    assert_eq!(alpha(&h, 120, 30), 1.0, "painted under the pointer");
    // A square mapping of the same screen point is a different pixel, left blank.
    let square = view.aspect(1.0).to_doc(at);
    let (sx, sy) = (square.x.floor() as i32, square.y.floor() as i32);
    assert_ne!((sx, sy), (120, 30));
    assert_eq!(alpha(&h, sx, sy), 0.0);
}
