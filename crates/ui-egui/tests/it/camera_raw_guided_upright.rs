//! Camera Raw › Geometry › Upright › Guided, driven with real keys and pointer drags: Shift+T
//! takes the tool up, guides drawn on the preview correct the picture, and each guide is one
//! undo step.
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::ThemeKind};
use serde_json::{Value, json};

fn fixture() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 800.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":600,"height":400})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#808080"})).unwrap();
        camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        app
    });
    h.run_steps(4);
    h
}
fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}
fn preview_rect(h: &Harness<'_, PhotocraftApp>) -> Rect {
    let v = inspect(h);
    let a = v["previewRect"].as_array().unwrap();
    Rect::from_min_max(pos2(a[0].as_f64().unwrap() as f32, a[1].as_f64().unwrap() as f32), pos2(a[2].as_f64().unwrap() as f32, a[3].as_f64().unwrap() as f32))
}
fn press(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, down: bool) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
    h.run_steps(2);
}
fn drag(h: &mut Harness<'_, PhotocraftApp>, start: Pos2, end: Pos2) {
    press(h, start, true);
    for n in 1..=6 {
        h.event(Event::PointerMoved(start + (end - start) * n as f32 / 6.0));
        h.run_steps(2);
    }
    press(h, end, false);
    h.run_steps(4);
}
fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}
fn params(h: &Harness<'_, PhotocraftApp>) -> Value {
    inspect(h)["params"].clone()
}
/// Runs frames until the preview worker has nothing in flight and `done` holds, for up to 10 s
/// (CI runners are busy).
fn settle(h: &mut Harness<'_, PhotocraftApp>, done: impl Fn(&Value) -> bool) {
    for _ in 0..2000 {
        let d = inspect(h);
        if d["previewPending"] == false && done(&d) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.run_steps(1);
    }
}
/// A point at fractions of the preview image.
fn at(r: Rect, x: f32, y: f32) -> Pos2 {
    pos2(r.left() + x * r.width(), r.top() + y * r.height())
}

#[test]
fn shift_t_then_two_guides_straighten_and_undo_one_at_a_time() {
    let mut h = fixture();
    assert_eq!(params(&h)["upright"], "off");
    key(&mut h, Key::T, Modifiers::SHIFT);
    assert_eq!(params(&h)["upright"], "guided");
    assert_eq!(inspect(&h)["guidedUprightTool"]["active"], true);
    let r = preview_rect(&h);
    let zoom_before = inspect(&h)["view"].clone();
    // Two posts leaning in toward the top: a camera tilted up.
    drag(&mut h, at(r, 0.22, 0.1), at(r, 0.18, 0.9));
    assert_eq!(params(&h)["uprightGuides"].as_array().unwrap().len(), 1);
    assert_eq!(inspect(&h)["view"], zoom_before, "drawing a guide doesn't zoom");
    // Let the first guide's render land, so the revision below can only move for the second.
    settle(&mut h, |_| true);
    let revision = inspect(&h)["previewRevision"].clone();
    drag(&mut h, at(r, 0.78, 0.1), at(r, 0.82, 0.9));
    let guides = params(&h)["uprightGuides"].clone();
    assert_eq!(guides.as_array().unwrap().len(), 2);
    let g0 = &guides[0];
    assert!((g0[0][0].as_f64().unwrap() - 0.22).abs() < 0.01 && (g0[1][1].as_f64().unwrap() - 0.9).abs() < 0.01, "{g0}");
    // The preview develops on a worker.
    settle(&mut h, |d| d["previewRevision"] != revision);
    assert_ne!(inspect(&h)["previewRevision"], revision, "the second guide re-renders the preview");
    // Delete removes the selected (last drawn) guide; ⌘Z brings it back.
    key(&mut h, Key::Delete, Modifiers::NONE);
    assert_eq!(params(&h)["uprightGuides"].as_array().unwrap().len(), 1);
    key(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(params(&h)["uprightGuides"], guides);
    // Each guide was its own undo step, and Shift+T's switch to Guided another.
    key(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(params(&h)["uprightGuides"].as_array().unwrap().len(), 1);
    key(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(params(&h)["uprightGuides"].as_array().unwrap().len(), 0);
    key(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(params(&h)["upright"], "off");
}

#[test]
fn shift_t_again_puts_the_tool_down_and_clicks_zoom_again() {
    let mut h = fixture();
    key(&mut h, Key::T, Modifiers::SHIFT);
    key(&mut h, Key::T, Modifiers::SHIFT);
    assert_eq!(inspect(&h)["guidedUprightTool"]["active"], false);
    assert_eq!(params(&h)["upright"], "guided", "the mode stays");
    let r = preview_rect(&h);
    drag(&mut h, at(r, 0.3, 0.2), at(r, 0.3, 0.8));
    assert_eq!(params(&h)["uprightGuides"].as_array().unwrap().len(), 0, "no tool, no guide");
    // Escape puts the tool down before it cancels the dialog.
    key(&mut h, Key::T, Modifiers::SHIFT);
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(inspect(&h)["guidedUprightTool"]["active"], false);
    assert!(inspect(&h).is_object(), "the dialog is still open");
}

/// With the Color Sampler chosen, Shift+T or the Guided button takes the guide tool up and puts
/// the sampler down (the newest tool wins), so the next drag draws a guide.
#[test]
fn shift_t_or_the_guided_button_puts_the_color_sampler_down() {
    let mut h = fixture();
    key(&mut h, Key::S, Modifiers::NONE);
    assert_eq!(inspect(&h)["scope"]["samplerTool"], true);
    key(&mut h, Key::T, Modifiers::SHIFT);
    assert_eq!(params(&h)["upright"], "guided");
    assert_eq!(inspect(&h)["guidedUprightTool"]["active"], true, "Shift+T takes the guide tool up");
    assert_eq!(inspect(&h)["scope"]["samplerTool"], false, "and puts the sampler down");
    assert!(h.query_by_label("Guided").is_some(), "Shift+T opens Geometry");
    let r = preview_rect(&h);
    drag(&mut h, at(r, 0.3, 0.2), at(r, 0.3, 0.8));
    assert_eq!(params(&h)["uprightGuides"].as_array().unwrap().len(), 1, "the drag draws a guide");
    // S takes the sampler back up and puts the guide tool down.
    key(&mut h, Key::S, Modifiers::NONE);
    assert_eq!((inspect(&h)["scope"]["samplerTool"].clone(), inspect(&h)["guidedUprightTool"]["active"].clone()), (json!(true), json!(false)));
    // The Guided button does what Shift+T does.
    h.get_by_label("Guided").click();
    h.run_steps(2);
    assert_eq!(inspect(&h)["guidedUprightTool"]["active"], true, "the Guided button takes the guide tool up");
    assert_eq!(inspect(&h)["scope"]["samplerTool"], false, "and puts the sampler down");
}
