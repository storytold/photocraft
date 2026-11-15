//! The develop view's mask overlay comes and goes by itself, as in Lightroom: it shows while a mask
//! is made or painted, hides as soon as one of the mask's effect sliders moves (so the effect shows
//! on the photo), and shows again when painting resumes, the mask's geometry is dragged, or a
//! mask is made or picked. Its eye button and O (while masking) still toggle it.
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key) {
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(2);
    // (the overlay shown is the one a render has landed for)
    settle(h);
}

fn button(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

/// A press at `a`, moved in steps to `b`, released there.
fn drag_screen(h: &mut Harness<'_, PhotocraftApp>, a: Pos2, b: Pos2) {
    h.event(Event::PointerMoved(a));
    h.run_steps(1);
    button(h, a, true);
    h.run_steps(1);
    for k in 1..=6 {
        h.event(Event::PointerMoved(a + (b - a) * (k as f32 / 6.0)));
        h.run_steps(1);
    }
    button(h, b, false);
    settle(h);
}

fn preview_rect(h: &Harness<'_, PhotocraftApp>) -> egui::Rect {
    let r = &inspect(h)["previewRect"];
    let f = |i: usize| r[i].as_f64().unwrap() as f32;
    egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3)))
}

/// A brush drag across the preview, from `from` to `to` (fractions of it).
fn paint(h: &mut Harness<'_, PhotocraftApp>, from: egui::Vec2, to: egui::Vec2) {
    let r = preview_rect(h);
    drag_screen(h, r.lerp_inside(from), r.lerp_inside(to));
}

/// Drags the Masking panel's slider labelled `label` a little to the right.
fn nudge_slider(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    let rect = h.get_by_role_and_label(egui::accesskit::Role::Slider, label).rect();
    drag_screen(h, rect.center(), rect.center() + vec2(30.0, 0.0));
}

fn shown(h: &Harness<'_, PhotocraftApp>) -> bool {
    let r = inspect(h);
    r["maskOverlay"] == true && !r["maskOverlayShown"].is_null()
}

fn mask(h: &Harness<'_, PhotocraftApp>) -> Value {
    inspect(h)["develop"]["masks"][0].clone()
}

fn dialog() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1600.0, 1000.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.open_bytes("IMG_0001.dng", &dng()).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    assert_eq!(inspect(&h)["panel"], "develop");
    h
}

/// Painting shows the overlay; an effect slider hides it (and changes nothing else about the
/// mask); painting again shows it; Refine Edges (the mask's shape) leaves it; O and the eye
/// toggle it; making, picking or adding to a mask shows it; the control channel's `adjust` hides
/// it as the sliders do.
#[test]
fn the_mask_overlay_hides_while_its_effects_are_adjusted() {
    let mut h = dialog();
    key(&mut h, Key::K);
    paint(&mut h, vec2(0.2, 0.3), vec2(0.5, 0.35));
    assert!(shown(&h), "painting shows the overlay: {}", inspect(&h));
    let before = mask(&h);
    // an effect slider: hidden after the drag, the mask itself unchanged but for its effect
    nudge_slider(&mut h, "Exposure");
    let after = mask(&h);
    assert!(after["adjust"]["exposure"].as_f64().unwrap() > 0.0, "the slider moved: {}", after["adjust"]);
    assert!(!shown(&h), "an effect slider hides the overlay");
    assert_eq!((after["components"].clone(), after["visible"].clone()), (before["components"].clone(), before["visible"].clone()));
    // painting again brings it back
    paint(&mut h, vec2(0.2, 0.6), vec2(0.5, 0.65));
    assert!(shown(&h), "resuming painting shows the overlay");
    // another effect (temperature) hides it too
    nudge_slider(&mut h, "Temp");
    assert!(!shown(&h));
    // O while masking shows it, and again hides it
    key(&mut h, Key::O);
    assert!(shown(&h), "O shows the overlay");
    // Refine Edges is the mask's shape: the overlay stays
    nudge_slider(&mut h, "Refine Edges");
    assert!(shown(&h), "Refine Edges keeps the overlay");
    key(&mut h, Key::O);
    assert!(!shown(&h), "O hides the overlay");
    // the eye button forces it on
    h.get_by_label("Show Overlay").click();
    h.run_steps(2);
    settle(&mut h);
    assert!(shown(&h), "the eye shows it");
    // a new mask shows its overlay; picking the first in the list shows it again
    nudge_slider(&mut h, "Exposure");
    assert!(!shown(&h));
    ui(&mut h, json!({"local": {"newMask": "radial"}}));
    assert!(shown(&h), "a new mask shows its overlay");
    nudge_slider(&mut h, "Exposure");
    assert!(!shown(&h));
    let first = inspect(&h)["develop"]["masks"][0]["name"].as_str().unwrap().to_string();
    h.get_by_label(&first).click();
    h.run_steps(2);
    settle(&mut h);
    assert!(shown(&h), "picking a mask shows its overlay");
    // dragging the radial's geometry brings it back (select it, hide with a slider, drag its pin)
    let radial = inspect(&h)["develop"]["masks"][1]["id"].clone();
    ui(&mut h, json!({"local": {"selectMask": radial}}));
    nudge_slider(&mut h, "Exposure");
    assert!(!shown(&h));
    let r = preview_rect(&h);
    let centre = r.lerp_inside(vec2(0.5, 0.5));
    drag_screen(&mut h, centre, centre + vec2(20.0, 10.0));
    assert!(shown(&h), "dragging the mask's geometry shows the overlay");
    // the control channel's adjust hides it as the sliders do, and O away from Masking is not the
    // overlay's
    let r = ui(&mut h, json!({"local": {"adjust": {"exposure": 0.7}}}));
    assert_eq!(r["maskOverlay"], false);
    assert_eq!(r["develop"]["masks"][1]["adjust"]["exposure"], 0.7);
    let ctx = h.ctx.clone();
    assert!(camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"local": {"adjust": {"nope": 1}}}})).unwrap().is_err());
    key(&mut h, Key::E);
    key(&mut h, Key::O);
    assert_eq!(inspect(&h)["maskOverlay"], false, "O on the Edit panel leaves the mask overlay alone");
}
