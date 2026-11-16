//! The develop view's mask overlay comes and goes by itself, as in LightCraft: it shows while a new
//! mask is painted, hides as soon as one of the mask's effect sliders moves (so the effect shows on
//! the photo) and stays hidden while the mask is painted on; a new mask, or one picked, shows it
//! again. Its eye button, Show Overlay and O (while masking) set it; the panel chooses how it
//! shows (mode, colour, opacity), ⇧O cycles the colour.
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

/// The overlay's transitions, one switch for the dialog (LightCraft's): a new mask shows it while
/// it is painted; an effect slider hides it, and painting on, drawing or dragging the mask's
/// geometry leave it hidden (the regression: adjust → paint again → still hidden, during the
/// stroke too); Refine Edges (the mask's shape) leaves it; O, the eye and Show Overlay set it; a
/// new mask, or picking one, shows it; a component added to a mask with effects leaves it hidden,
/// to one without shows it; the control channel's `adjust` hides it as the sliders do.
#[test]
fn the_mask_overlay_hides_on_an_effect_and_stays_hidden_while_painting() {
    let mut h = dialog();
    key(&mut h, Key::K);
    paint(&mut h, vec2(0.2, 0.3), vec2(0.5, 0.35));
    assert!(shown(&h), "a new mask shows its overlay while it is painted: {}", inspect(&h));
    let before = mask(&h);
    // an effect slider: hidden after the drag, the mask itself unchanged but for its effect
    nudge_slider(&mut h, "Exposure");
    let after = mask(&h);
    assert!(after["adjust"]["exposure"].as_f64().unwrap() > 0.0, "the slider moved: {}", after["adjust"]);
    assert!(!shown(&h), "an effect slider hides the overlay");
    assert_eq!((after["components"].clone(), after["visible"].clone()), (before["components"].clone(), before["visible"].clone()));
    // painting on: hidden during the stroke and after it
    let r = preview_rect(&h);
    let (a, b) = (r.lerp_inside(vec2(0.2, 0.6)), r.lerp_inside(vec2(0.5, 0.65)));
    h.event(Event::PointerMoved(a));
    h.run_steps(1);
    button(&mut h, a, true);
    h.run_steps(1);
    for k in 1..=6 {
        h.event(Event::PointerMoved(a + (b - a) * (k as f32 / 6.0)));
        h.run_steps(1);
        let r = inspect(&h);
        assert_eq!((r["maskOverlay"].clone(), r["maskOverlayShown"].clone()), (json!(false), Value::Null), "move {k} of the stroke");
    }
    button(&mut h, b, false);
    settle(&mut h);
    assert_eq!(mask(&h)["components"][0]["shape"]["strokes"].as_array().map(Vec::len), Some(2), "the stroke went on the mask");
    assert!(!shown(&h), "painting on a mask whose overlay hid leaves it hidden");
    // another effect (temperature) keeps it hidden
    nudge_slider(&mut h, "Temp");
    assert!(!shown(&h));
    // O while masking shows it, and again hides it
    key(&mut h, Key::O);
    assert!(shown(&h), "O shows the overlay");
    // painting with it shown keeps it shown
    paint(&mut h, vec2(0.6, 0.3), vec2(0.7, 0.35));
    assert!(shown(&h), "painting leaves a shown overlay shown");
    // Refine Edges is the mask's shape: the overlay stays
    nudge_slider(&mut h, "Refine Edges");
    assert!(shown(&h), "Refine Edges keeps the overlay");
    key(&mut h, Key::O);
    assert!(!shown(&h), "O hides the overlay");
    // Show Overlay turns it on
    h.get_by_label("Show Overlay").click();
    h.run_steps(2);
    settle(&mut h);
    assert!(shown(&h), "Show Overlay shows it");
    // a component added to a mask with effects: the photo shows it, the overlay stays hidden
    nudge_slider(&mut h, "Exposure");
    assert!(!shown(&h));
    ui(&mut h, json!({"local": {"addComponent": {"kind": "brush"}}}));
    assert!(!shown(&h), "adding to a mask with effects leaves the overlay hidden");
    // a new mask (no effects yet) shows its overlay
    ui(&mut h, json!({"local": {"newMask": "radial"}}));
    assert!(shown(&h), "a new mask shows its overlay");
    // dragging its geometry with it shown keeps it shown; hidden by an effect, the drag keeps it hidden
    let r = preview_rect(&h);
    let centre = r.lerp_inside(vec2(0.5, 0.5));
    drag_screen(&mut h, centre, centre + vec2(20.0, 10.0));
    assert!(shown(&h), "dragging the geometry leaves a shown overlay shown");
    nudge_slider(&mut h, "Exposure");
    assert!(!shown(&h));
    let centre = centre + vec2(20.0, 10.0);
    drag_screen(&mut h, centre, centre + vec2(20.0, 10.0));
    assert!(!shown(&h), "dragging the mask's geometry leaves a hidden overlay hidden");
    // a component added to a mask without effects shows it (only the overlay shows the new part)
    let ctx = h.ctx.clone();
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"local": {"newMask": "radial", "overlay": false}}})).unwrap().unwrap();
    settle(&mut h);
    assert!(!shown(&h));
    ui(&mut h, json!({"local": {"addComponent": {"kind": "linear"}}}));
    assert!(shown(&h), "adding to a mask without effects shows the overlay");
    // picking a mask in the list shows it
    nudge_slider(&mut h, "Exposure");
    assert!(!shown(&h));
    let first = inspect(&h)["develop"]["masks"][0]["name"].as_str().unwrap().to_string();
    h.get_by_label(&first).click();
    h.run_steps(2);
    settle(&mut h);
    assert!(shown(&h), "picking a mask shows its overlay");
    // the control channel's adjust hides it as the sliders do, and O away from Masking is not the
    // overlay's
    let r = ui(&mut h, json!({"local": {"adjust": {"exposure": 0.7}}}));
    assert_eq!(r["maskOverlay"], false);
    assert_eq!(r["develop"]["masks"][0]["adjust"]["exposure"], 0.7);
    let ctx = h.ctx.clone();
    assert!(camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"local": {"adjust": {"nope": 1}}}})).unwrap().is_err());
    key(&mut h, Key::E);
    key(&mut h, Key::O);
    assert_eq!(inspect(&h)["maskOverlay"], false, "O on the Edit panel leaves the mask overlay alone");
}

/// The overlay's view: Show Overlay with its key in the tooltip, the five modes, the colour
/// swatches (⇧O cycles them and shows the overlay), the opacity; each remembered in the
/// preferences, and none of them developing the photo again.
#[test]
fn the_overlay_modes_colours_and_opacity_are_chosen_and_remembered() {
    let mut h = dialog();
    key(&mut h, Key::K);
    paint(&mut h, vec2(0.2, 0.3), vec2(0.5, 0.35));
    let view = |h: &Harness<'_, PhotocraftApp>| inspect(h)["maskOverlayView"].clone();
    assert_eq!(view(&h), json!({"mode": "color", "color": "#e61e28", "opacity": 50.0}), "red at half strength by default");
    assert_eq!(inspect(&h)["shortcuts"]["mask.overlayColor"], "Shift+O");
    let rev = inspect(&h)["previewRevision"].clone();
    // ⇧O: the next colour, the overlay shown
    key(&mut h, Key::O);
    assert!(!shown(&h));
    h.event(Event::Key { key: Key::O, physical_key: Some(Key::O), pressed: true, repeat: false, modifiers: Modifiers::SHIFT });
    h.event(Event::Key { key: Key::O, physical_key: Some(Key::O), pressed: false, repeat: false, modifiers: Modifiers::SHIFT });
    h.run_steps(2);
    settle(&mut h);
    assert!(shown(&h), "⇧O shows the overlay");
    assert_eq!(view(&h)["color"], "#28c846", "⇧O: green after red");
    // the modes from the panel's menu
    let mut current = "Color Overlay";
    for (label, id) in [
        ("Color Overlay on B&W", "colorOnBw"),
        ("Image on Black", "imageOnBlack"),
        ("Image on White", "imageOnWhite"),
        ("White on Black", "whiteOnBlack"),
        ("Color Overlay", "color"),
    ] {
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, current).click();
        h.run_steps(2);
        h.get_by_label(label).click();
        h.run_steps(2);
        settle(&mut h);
        assert_eq!(view(&h)["mode"], id);
        assert!(shown(&h), "{id}");
        current = label;
    }
    // a swatch, the opacity and a mode from the control channel
    h.get_by_label("Blue").click();
    h.run_steps(2);
    let r = ui(&mut h, json!({"local": {"overlayOpacity": 80, "overlayMode": "colorOnBw"}}));
    assert_eq!(r["maskOverlayView"], json!({"mode": "colorOnBw", "color": "#286ef0", "opacity": 80.0}));
    let ctx = h.ctx.clone();
    for bad in [json!({"overlayMode": "nope"}), json!({"overlayColor": "#12"}), json!({"overlayColor": [1, 2]}), json!({"overlayOpacity": "x"})] {
        assert!(camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"local": bad}})).unwrap().is_err(), "{bad}");
    }
    ui(&mut h, json!({"local": {"overlayColor": [255, 255, 255]}}));
    h.run_steps(2);
    // choosing how the overlay shows develops nothing again
    assert_eq!(inspect(&h)["previewRevision"], rev, "no develop for the overlay's view");
    // the preferences keep it
    let p = &h.state().session.prefs().raw_defaults;
    assert_eq!((p.mask_overlay_mode.as_str(), p.mask_overlay_color, p.mask_overlay_opacity), ("colorOnBw", [255, 255, 255], 80.0));
}
