//! The develop view's pointer over the preview, and switching between the mask brush and the
//! Zoom and Hand with real key and pointer events: the brush ring's + or − follows ⌥ and the Erase
//! brush the moment the key goes down, Z and H (and Space) show the magnifier or the hand at once
//! with no ring, and Z and H are spring-loaded (held: back to the brush on release) as well as
//! sticky (tapped: until a second tap or a tool's key), never painting or ending a stroke.
use egui::{CursorIcon, Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, Services, camera_raw_ui, theme::ThemeKind};
use serde_json::{Value, json};

fn dng() -> Vec<u8> {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (w, h) = (96, 64);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    spec.build()
}

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

fn settle(h: &mut Harness<'_, PhotocraftApp>) {
    for _ in 0..600 {
        h.run_steps(1);
        let r = inspect(h);
        if !r.is_null() && r["previewPending"] == false && r["detailPending"] == false {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn ui(h: &mut Harness<'_, PhotocraftApp>, v: Value) -> Value {
    let ctx = h.ctx.clone();
    let r = camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({ "ui": v })).unwrap().unwrap();
    settle(h);
    r
}

fn dialog() -> Harness<'static, PhotocraftApp> {
    let bytes = dng();
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1600.0, 1000.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.open_bytes("IMG_0001.dng", &bytes).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    h
}

fn cursor(h: &Harness<'_, PhotocraftApp>) -> CursorIcon {
    h.output().platform_output.cursor_icon
}

/// The key going down or up alone (no pointer move), and one frame.
fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key, pressed: bool, repeat: bool) {
    h.event(Event::Key { key, physical_key: Some(key), pressed, repeat, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn modifiers(h: &mut Harness<'_, PhotocraftApp>, m: Modifiers) {
    h.event(Event::ModifiersChanged(m));
    h.run_steps(1);
}

fn button(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, pressed: bool, m: Modifiers) {
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: m });
    h.run_steps(1);
}

/// The brush's strokes so far (the selected brush mask's).
fn strokes(h: &Harness<'_, PhotocraftApp>) -> Vec<Value> {
    let r = inspect(h);
    r["develop"]["masks"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["components"].as_array().cloned().unwrap_or_default())
        .flat_map(|c| c["shape"]["strokes"].as_array().cloned().unwrap_or_default())
        .collect()
}

/// The Masking brush over the middle of the preview, the pointer resting there.
fn brush_over_preview(h: &mut Harness<'_, PhotocraftApp>) -> Pos2 {
    key(h, Key::K, true, false);
    key(h, Key::K, false, false);
    settle(h);
    assert_eq!(inspect(h)["tool"], "masking");
    let r = &inspect(h)["previewRect"];
    let f = |i: usize| r[i].as_f64().unwrap() as f32;
    let at = egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3))).center();
    h.event(Event::PointerMoved(at));
    h.run_steps(2);
    at
}

#[test]
fn option_and_the_erase_brush_show_on_the_brush_ring_at_once() {
    let mut h = dialog();
    let at = brush_over_preview(&mut h);
    assert_eq!(inspect(&h)["brushCursor"], json!({"erase": false}), "a + : it adds");
    assert_eq!(cursor(&h), CursorIcon::Crosshair);
    // ⌥ down, no pointer move: a − straight away, and back on release
    modifiers(&mut h, Modifiers::ALT);
    assert_eq!(inspect(&h)["brushCursor"], json!({"erase": true}));
    modifiers(&mut h, Modifiers::NONE);
    assert_eq!(inspect(&h)["brushCursor"], json!({"erase": false}));
    // the Erase brush shows a −; ⌥ with it adds
    ui(&mut h, json!({"local": {"brush": {"erase": true}}}));
    h.run_steps(1);
    assert_eq!(inspect(&h)["brushCursor"], json!({"erase": true}));
    modifiers(&mut h, Modifiers::ALT);
    assert_eq!(inspect(&h)["brushCursor"], json!({"erase": false}));
    // and a stroke paints as the cursor said
    button(&mut h, at, true, Modifiers::ALT);
    for k in 1..6 {
        h.event(Event::PointerMoved(at + vec2(k as f32 * 6.0, 0.0)));
        h.run_steps(1);
    }
    button(&mut h, at + vec2(30.0, 0.0), false, Modifiers::ALT);
    modifiers(&mut h, Modifiers::NONE);
    settle(&mut h);
    let s = strokes(&h);
    assert_eq!(s.len(), 1, "{s:?}");
    assert_eq!(s[0]["erase"], false, "⌥ with the Erase brush added: {}", s[0]);
}

#[test]
fn z_and_h_show_their_pointer_at_once_and_spring_back_to_the_brush() {
    let mut h = dialog();
    let at = brush_over_preview(&mut h);
    let mask = inspect(&h)["selectedMask"].clone();
    let before = inspect(&h)["develop"]["masks"].clone();

    // Z held: the magnifier with no ring, on the key alone; ⌥ turns it to zoom out
    key(&mut h, Key::Z, true, false);
    assert_eq!(cursor(&h), CursorIcon::ZoomIn);
    assert_eq!(inspect(&h)["brushCursor"], Value::Null, "no brush ring under the Zoom");
    assert_eq!(inspect(&h)["toolPaused"], true);
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.event(Event::Key { key: Key::Z, physical_key: Some(Key::Z), pressed: true, repeat: true, modifiers: Modifiers::ALT });
    h.run_steps(1);
    assert_eq!(cursor(&h), CursorIcon::ZoomOut);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    // the key's auto-repeat for half a second toggles nothing
    for _ in 0..30 {
        key(&mut h, Key::Z, true, true);
    }
    assert_eq!(cursor(&h), CursorIcon::ZoomIn);
    assert_eq!(inspect(&h)["toolPaused"], true);
    // let go: the brush, exactly as it was
    key(&mut h, Key::Z, false, false);
    h.run_steps(1);
    let r = inspect(&h);
    assert_eq!(r["toolPaused"], false);
    assert_eq!(r["brushCursor"], json!({"erase": false}));
    assert_eq!(cursor(&h), CursorIcon::Crosshair);
    assert_eq!((r["selectedMask"].clone(), r["develop"]["masks"].clone()), (mask.clone(), before.clone()), "no stroke, the same mask");
    assert_eq!(r["view"]["zoomTool"], false, "the navigation as it was");

    taps(&mut h);
    hand_and_space(&mut h, at);
    zoom_click_then_paint(&mut h, at, &before);
}

/// A tap of Z: the Zoom stays until Z is tapped again, or the tool's own key.
fn taps(h: &mut Harness<'_, PhotocraftApp>) {
    let h = &mut *h;
    // a tap: the Zoom stays (sticky) until Z is tapped again
    key(h, Key::Z, true, false);
    key(h, Key::Z, false, false);
    h.run_steps(30);
    assert_eq!(inspect(h)["toolPaused"], true);
    assert_eq!(cursor(h), CursorIcon::ZoomIn);
    key(h, Key::Z, true, false);
    key(h, Key::Z, false, false);
    assert_eq!(inspect(h)["toolPaused"], false, "the second tap goes back");
    assert_eq!(cursor(h), CursorIcon::Crosshair);
    // a tap, then the tool's own key (M): back too
    key(h, Key::Z, true, false);
    key(h, Key::Z, false, false);
    key(h, Key::M, true, false);
    key(h, Key::M, false, false);
    let r = inspect(h);
    assert_eq!((r["toolPaused"].clone(), r["tool"].clone(), r["view"]["zoomTool"].clone()), (json!(false), json!("masking"), json!(false)));
}

/// H held: the open hand, a fist while it drags, back to the brush after; Space as before.
fn hand_and_space(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    // H held: the open hand, a fist while it drags; back to the brush after, nothing painted
    key(h, Key::H, true, false);
    assert_eq!(cursor(h), CursorIcon::Grab);
    assert_eq!(inspect(h)["brushCursor"], Value::Null);
    button(h, at, true, Modifiers::NONE);
    for k in 1..6 {
        h.event(Event::PointerMoved(at + vec2(k as f32 * 8.0, 0.0)));
        h.run_steps(1);
    }
    assert_eq!(cursor(h), CursorIcon::Grabbing);
    button(h, at + vec2(40.0, 0.0), false, Modifiers::NONE);
    key(h, Key::H, false, false);
    h.run_steps(1);
    assert_eq!(inspect(h)["toolPaused"], false);
    assert_eq!(cursor(h), CursorIcon::Crosshair);
    assert!(strokes(h).is_empty(), "the Hand's drag painted nothing");

    // Space held: the hand and no ring, as before
    key(h, Key::Space, true, false);
    h.run_steps(1);
    assert_eq!(cursor(h), CursorIcon::Grab);
    assert_eq!(inspect(h)["brushCursor"], Value::Null);
    key(h, Key::Space, false, false);
    h.run_steps(1);
    assert_eq!(cursor(h), CursorIcon::Crosshair);
}

/// Z held for a zoom click let go of before the button: the button's release paints nothing, and
/// the brush paints again after.
fn zoom_click_then_paint(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, before: &Value) {
    // Z held for a zoom click, let go of before the button: the button's release paints nothing
    let zoom = inspect(h)["zoom"].as_f64().unwrap();
    let p = h.ctx.pointer_hover_pos().unwrap_or(at);
    key(h, Key::Z, true, false);
    button(h, p, true, Modifiers::NONE);
    h.run_steps(20);
    key(h, Key::Z, false, false);
    button(h, p, false, Modifiers::NONE);
    settle(h);
    let r = inspect(h);
    assert_eq!(r["toolPaused"], false);
    assert!(strokes(h).is_empty(), "no dab from the zoom click: {:?}", strokes(h));
    assert_eq!(&r["develop"]["masks"], before);
    // (the click zoomed only if it was the Zoom's; with the button let go of after Z, it isn't a
    // dab either way)
    let _ = zoom;
    // the brush paints again afterwards
    button(h, at, true, Modifiers::NONE);
    for k in 1..6 {
        h.event(Event::PointerMoved(at + vec2(0.0, k as f32 * 6.0)));
        h.run_steps(1);
    }
    button(h, at + vec2(0.0, 30.0), false, Modifiers::NONE);
    settle(h);
    assert_eq!(strokes(h).len(), 1);
}
