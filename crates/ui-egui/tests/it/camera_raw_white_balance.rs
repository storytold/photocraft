//! Camera Raw's White Balance: the menu's Auto, the White Balance tool (I) on the preview, and
//! the control channel's `whiteBalance` / `whiteBalanceTool`.
use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::ThemeKind};
use serde_json::{Value, json};

fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, p: Value) -> Result<Value, String> {
    camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &p).ok_or("missing handler")?
}

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

fn wb(h: &Harness<'_, PhotocraftApp>) -> (f64, f64) {
    let p = &inspect(h)["params"];
    (p["temperature"].as_f64().unwrap_or(f64::NAN), p["tint"].as_f64().unwrap_or(f64::NAN))
}

/// A warm grey image (red over blue), the dialog open on it.
fn warm() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#b49a84"})).unwrap();
        camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        app
    });
    settle(&mut h);
    h
}

fn settle(h: &mut Harness<'_, PhotocraftApp>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    h.run_steps(3);
    while inspect(h)["previewPending"] == true {
        assert!(std::time::Instant::now() < deadline, "the Camera Raw preview worker never finished");
        std::thread::sleep(std::time::Duration::from_millis(2));
        h.run_steps(1);
    }
    h.run_steps(2);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, p: Pos2) {
    h.event(Event::PointerMoved(p));
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

#[test]
fn auto_and_sample_cool_a_warm_image_and_bad_requests_change_nothing() {
    let mut h = warm();
    let ctx = h.ctx.clone();
    menu(h.state_mut(), &ctx, json!({"ui":{"whiteBalance":"auto"}})).unwrap();
    let (t, n) = wb(&h);
    assert!(t < 0.0, "a warm image needs a cooler balance: {t}/{n}");
    let auto = (t, n);
    // `set` in the same request builds on Auto's result, not on the settings before it.
    menu(h.state_mut(), &ctx, json!({"ui":{"whiteBalance":"asShot"}})).unwrap();
    menu(h.state_mut(), &ctx, json!({"ui":{"whiteBalance":"auto","set":{"contrast":10}}})).unwrap();
    assert_eq!(wb(&h), auto, "set kept Auto's balance");
    assert_eq!(inspect(&h)["params"]["contrast"], 10.0);
    menu(h.state_mut(), &ctx, json!({"ui":{"whiteBalance":"asShot"}})).unwrap();
    assert_eq!(wb(&h), (0.0, 0.0));
    menu(h.state_mut(), &ctx, json!({"ui":{"whiteBalance":{"sample":[0.5,0.5]}}})).unwrap();
    let sampled = wb(&h);
    assert!(sampled.0 < 0.0, "the eyedropper agrees with Auto on a flat image: {sampled:?} vs {auto:?}");
    for bad in [
        json!({"ui":{"whiteBalance":"daylight"}}),
        json!({"ui":{"whiteBalance":{"sample":[2.0,0.5]}}}),
        json!({"ui":{"whiteBalance":{"sample":[0.5]}}}),
        json!({"ui":{"whiteBalance":3}}),
        json!({"ui":{"whiteBalanceTool":"yes"}}),
        json!({"ui":{"whiteBalance":"auto","panels":{"open":["nope"]}}}),
    ] {
        assert!(menu(h.state_mut(), &ctx, bad.clone()).is_err(), "{bad}");
        assert_eq!(wb(&h), sampled, "{bad} must change nothing");
    }
}

#[test]
fn the_white_balance_tool_neutralises_the_clicked_spot() {
    let mut h = warm();
    let ctx = h.ctx.clone();
    menu(h.state_mut(), &ctx, json!({"ui":{"whiteBalanceTool":true}})).unwrap();
    h.run_steps(3);
    assert_eq!(inspect(&h)["panels"]["whiteBalanceTool"], true);
    let r = inspect(&h)["previewRect"].clone();
    let at = pos2(((r[0].as_f64().unwrap() + r[2].as_f64().unwrap()) / 2.0) as f32, ((r[1].as_f64().unwrap() + r[3].as_f64().unwrap()) / 2.0) as f32);
    click(&mut h, at);
    let (t, n) = wb(&h);
    assert!(t < 0.0, "the click cooled the warm image: {t}/{n}");
    assert!(inspect(&h)["samplerReadouts"].as_array().is_none_or(Vec::is_empty), "the click is not a colour sampler");
    // I turns the tool off again.
    h.event(Event::Key { key: egui::Key::I, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(inspect(&h)["panels"]["whiteBalanceTool"], false);
}
