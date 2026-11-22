//! #15549: `ui.pointer` drives the Brush and Move tools through the whole app, with the window
//! focused or not and at 100 % and 125 % scale. The control case sends the same gestures through
//! `ui.click` / `ui.move` with a hold.

use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

fn app(ppp: f32, focused: bool) -> Harness {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).with_pixels_per_point(ppp).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"width": 600, "height": 400, "background": "#ffffff"})).expect("new document");
        app
    });
    h.run_steps(4);
    if !focused {
        h.event(egui::Event::WindowFocused(false));
        h.input_mut().focused = false;
        h.run_steps(1);
        h.input_mut().focused = false;
    }
    h
}

fn call(h: &mut Harness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    let v = match handle(h.state_mut(), &ctx, &req) {
        Reply::Done(v) => {
            assert_eq!(v["ok"], true, "{method} {params}: {v}");
            v
        }
        Reply::AfterInput => Value::Null,
        _ => panic!("{method} {params}: not done"),
    };
    h.run_steps(8);
    v
}

fn history(h: &Harness) -> Vec<String> {
    h.state().session.active().unwrap().history.entries().iter().map(|s| s.to_string()).collect()
}

fn pixels(h: &Harness) -> Vec<f32> {
    let d = &h.state().session.active().unwrap().doc;
    let id = d.layers.last().unwrap().id;
    d.layer(id).unwrap().surface().map(|s| s.read_region(d.bounds())).unwrap_or_default()
}

const SETUPS: [(f32, bool); 4] = [(1.0, true), (1.25, true), (1.0, false), (1.25, false)];

fn stroke_events() -> Value {
    json!([{"kind":"down","x":100,"y":100},{"kind":"move","x":200,"y":150},{"kind":"move","x":300,"y":200},{"kind":"up","x":300,"y":200}])
}

#[test]
fn control_pointer_brush_stroke_paints() {
    for (ppp, focused) in SETUPS {
        let mut h = app(ppp, focused);
        let before = pixels(&h);
        let n = history(&h).len();
        call(&mut h, "ui.pointer", json!({"tool": "Brush", "events": stroke_events()}));
        let ctx = format!("ppp {ppp} focused {focused}");
        assert_ne!(pixels(&h), before, "{ctx}: pixels changed; history {:?}", history(&h));
        assert!(history(&h).len() > n, "{ctx}: a Brush history state exists: {:?}", history(&h));
    }
}

#[test]
fn control_pointer_move_drag_moves_the_layer() {
    for (ppp, focused) in SETUPS {
        let mut h = app(ppp, focused);
        h.state_mut().run("layer.new.layer", json!({"name": "L"})).expect("new layer");
        h.run_steps(4);
        call(&mut h, "ui.pointer", json!({"tool": "Brush", "events": stroke_events()}));
        let before = pixels(&h);
        let n = history(&h).len();
        call(&mut h, "ui.pointer", json!({"tool": "Move", "events": stroke_events()}));
        let ctx = format!("ppp {ppp} focused {focused}");
        assert_ne!(pixels(&h), before, "{ctx}: the layer content moved; history {:?}", history(&h));
        assert!(history(&h).len() > n, "{ctx}: a Move history state exists: {:?}", history(&h));
    }
}

#[test]
fn control_click_and_move_with_hold_paint_a_stroke() {
    let mut h = app(1.0, true);
    let before = pixels(&h);
    call(&mut h, "ui.set", json!({"tool": "Brush"}));
    call(&mut h, "ui.click", json!({"x": 100, "y": 100, "hold": true}));
    call(&mut h, "ui.move", json!({"x": 200, "y": 150}));
    call(&mut h, "ui.move", json!({"x": 300, "y": 200}));
    call(&mut h, "ui.release", json!({}));
    assert_ne!(pixels(&h), before, "the control case paints; history {:?}", history(&h));
}
