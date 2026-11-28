use super::*;
use crate::PhotocraftApp;
use crate::control::{ControlRequest, Reply};
use crate::state::Tool;
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use serde_json::json;

fn move_controls_fixture() -> (Harness<'static, Box<PhotocraftApp>>, [LayerId; 2]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
    let layers = [[60, 60, 80, 60], [230, 150, 40, 40]].map(|rect| {
        app.run("shape.create", json!({"kind": "rect", "rect": rect, "fill": "#ff0000"})).unwrap();
        app.session.active().unwrap().active_layer.unwrap()
    });
    app.run("layer.select", json!({"layer": layers[0].0, "mode": "replace"})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    app.ui.tool_options.move_auto_select = true;
    app.ui.tool_options.move_target = "layer".into();
    app.ui.extras.snap = false;
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(ctx).with_size(vec2(1000.0, 800.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::testing::document_area(app, ui));
        },
        app,
    );
    h.run_steps(4);
    (h, layers)
}

/// Delivers one pointer event at the document point `at`: through `ui.pointer` (`control`), which the frames after
/// the request carry out, or as window input.
fn move_pointer(h: &mut Harness<'static, Box<PhotocraftApp>>, control: bool, kind: &str, [x, y]: [f64; 2]) {
    if control {
        let (request, _receiver) = ControlRequest::new("ui.pointer", json!({"events": [{"kind": kind, "x": x, "y": y}]}));
        let ctx = h.ctx.clone();
        let Reply::Done(reply) = crate::control::handle(h.state_mut(), &ctx, &request) else { panic!("ui.pointer: expected an immediate reply") };
        assert_eq!(reply["ok"], true, "{reply}");
        for _ in 0..8 {
            if craft::ui::runtime::windows::WindowHost::of(&ctx).injected_idle() {
                break;
            }
            h.run_steps(1);
        }
    } else {
        let pos = crate::canvas::testing::to_screen(h.state_mut(), [x, y]);
        h.hover_at(pos);
        if kind != "move" {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: kind == "down", modifiers: Modifiers::NONE });
        }
        h.run_steps(2);
    }
}

/// Whether the screen point of the document point `at` lies in the Move controls' press band: the frame grown by
/// 18 points.
fn in_press_band(h: &mut Harness<'static, Box<PhotocraftApp>>, at: [f64; 2]) -> bool {
    let frame = frame(h.state()).expect("the first layer has visible transform controls");
    let a = crate::canvas::testing::to_screen(h.state_mut(), [frame.x0, frame.y0]);
    let b = crate::canvas::testing::to_screen(h.state_mut(), [frame.x1, frame.y1]);
    let p = crate::canvas::testing::to_screen(h.state_mut(), at);
    egui::Rect::from_two_pos(a, b).expand(18.0).contains(p)
}

#[test]
fn move_controls_allow_auto_select_on_another_layer_outside_the_press_band() {
    for control in [false, true] {
        let (mut h, layers) = move_controls_fixture();
        let at = [250.0, 170.0];
        assert!(!in_press_band(&mut h, at));
        assert_eq!(h.state().session.active().unwrap().selected_layers(), vec![layers[0]]);
        let before = h.state().session.active().unwrap().doc.clone();
        move_pointer(&mut h, control, "down", at);
        move_pointer(&mut h, control, "up", at);
        let state = h.state().session.active().unwrap();
        assert!(h.state().ui.transform.is_none(), "control={control}: Auto-Select does not start Free Transform");
        assert_eq!(state.selected_layers(), vec![layers[1]], "control={control}");
        assert_eq!(state.doc, before);
    }
}

#[test]
fn move_controls_allow_a_layer_marquee_from_empty_canvas_outside_the_press_band() {
    for control in [false, true] {
        let (mut h, layers) = move_controls_fixture();
        let start = [190.0, 125.0];
        let end = [285.0, 210.0];
        assert!(!in_press_band(&mut h, start));
        assert_eq!(h.state().session.active().unwrap().selected_layers(), vec![layers[0]]);
        let before = h.state().session.active().unwrap().doc.clone();
        move_pointer(&mut h, control, "down", start);
        move_pointer(&mut h, control, "move", end);
        assert!(h.state().layer_box.is_some(), "control={control}: the empty-canvas drag starts a layer marquee");
        move_pointer(&mut h, control, "up", end);
        let state = h.state().session.active().unwrap();
        assert!(h.state().ui.transform.is_none(), "control={control}: the marquee does not start Free Transform");
        assert!(h.state().layer_box.is_none(), "the marquee ends on release");
        assert_eq!(state.selected_layers(), vec![layers[1]], "control={control}");
        assert_eq!(state.doc, before);
    }
}
