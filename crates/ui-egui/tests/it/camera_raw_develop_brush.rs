//! The develop view's mask brush in the real shell: Masking (K, M or the rail) arms the Brush so a
//! drag paints at once, its first stroke makes the mask (none is left empty when nothing is
//! painted), a selected mask stays selected, and Auto Mask (on for a first-time user, A toggles
//! it) goes onto the strokes and is remembered as a preference.
//!
//! `PHOTOCRAFT_LC_SNAPSHOTS=<dir>` saves `automask-live.png` there (on the GPU): an Auto Mask stroke
//! along an edge while the button is still held, its overlay as the preview renders it during the
//! drag; `PHOTOCRAFT_LC_RAW=<file>` uses that raw.
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
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

fn raw() -> (String, Vec<u8>) {
    match std::env::var("PHOTOCRAFT_LC_RAW") {
        Ok(path) => ("test.CR2".to_string(), std::fs::read(path).unwrap()),
        Err(_) => ("IMG_0001.dng".to_string(), dng()),
    }
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
}

fn button(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

/// The preview's image rect on screen.
fn preview_rect(h: &Harness<'_, PhotocraftApp>) -> egui::Rect {
    let r = &inspect(h)["previewRect"];
    let f = |i: usize| r[i].as_f64().unwrap() as f32;
    egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3)))
}

/// A press at `from` (as fractions of the preview), moved in `steps` to `to`, released there
/// unless `hold`.
fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Vec2, to: egui::Vec2, steps: usize, hold: bool) {
    let r = preview_rect(h);
    let (a, b) = (r.lerp_inside(from), r.lerp_inside(to));
    h.event(Event::PointerMoved(a));
    h.run_steps(1);
    button(h, a, true);
    h.run_steps(1);
    for k in 1..=steps {
        h.event(Event::PointerMoved(a + (b - a) * (k as f32 / steps as f32)));
        h.run_steps(1);
    }
    if !hold {
        button(h, b, false);
    }
    settle(h);
}

fn masks(h: &Harness<'_, PhotocraftApp>) -> Vec<Value> {
    inspect(h)["develop"]["masks"].as_array().cloned().unwrap_or_default()
}

fn strokes(m: &Value) -> Vec<Value> {
    m["components"][0]["shape"]["strokes"].as_array().cloned().unwrap_or_default()
}

fn dialog_on(gpu: bool, session: photocraft_engine::Session) -> Harness<'static, PhotocraftApp> {
    let (name, bytes) = raw();
    let builder = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1600.0, 1000.0));
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(session, services);
        app.open_bytes(&name, &bytes).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    assert_eq!(inspect(&h)["panel"], "develop");
    h
}

/// K arms the Brush with no mask made; leaving Masking unpainted leaves nothing behind; the first
/// stroke makes the brush mask; back in Masking the mask is still selected and a drag paints on it;
/// a selected radial stays selected (its drag draws the radial); the rail's button arms the Brush
/// as K does.
#[test]
fn masking_arms_the_brush_and_the_first_stroke_makes_the_mask() {
    let mut h = dialog_on(false, photocraft_engine::Session::new());
    key(&mut h, Key::K);
    let r = inspect(&h);
    assert_eq!(r["tool"], "masking");
    assert_eq!((r["maskDraw"].as_str(), r["brush"]["armed"].as_bool(), r["selectedMask"].is_null()), (Some("brush"), Some(true), true), "{}", r["brush"]);
    assert!(masks(&h).is_empty(), "no mask is made until something is painted");
    // leaving without painting leaves no empty mask
    key(&mut h, Key::E);
    key(&mut h, Key::M);
    assert!(masks(&h).is_empty() && inspect(&h)["brush"]["armed"] == true);
    // the first stroke makes the mask, selected, with the stroke in it
    drag(&mut h, vec2(0.2, 0.3), vec2(0.4, 0.35), 6, false);
    let m = masks(&h);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0]["components"][0]["shape"]["kind"], "brush");
    assert_eq!(strokes(&m[0]).len(), 1);
    let id = m[0]["id"].clone();
    assert_eq!(inspect(&h)["selectedMask"], id);
    // one undo step takes the stroke and the mask it made back
    ui(&mut h, json!({"undo": true}));
    assert!(masks(&h).is_empty(), "{:?}", masks(&h));
    ui(&mut h, json!({"redo": true}));
    assert_eq!(masks(&h).len(), 1);
    ui(&mut h, json!({"local": {"selectMask": id.clone()}}));
    // Edit and back: the mask stays selected, and the next drag paints on it
    key(&mut h, Key::E);
    key(&mut h, Key::K);
    let r = inspect(&h);
    assert_eq!((r["selectedMask"].clone(), r["maskDraw"].as_str(), r["brush"]["armed"].as_bool()), (id.clone(), Some("brush"), Some(false)));
    drag(&mut h, vec2(0.2, 0.6), vec2(0.4, 0.65), 6, false);
    let m = masks(&h);
    assert_eq!((m.len(), strokes(&m[0]).len()), (1, 2), "{m:?}");
    // a radial selected stays selected, and a drag draws it as before
    ui(&mut h, json!({"local": {"newMask": "radial"}}));
    let radial = inspect(&h)["selectedMask"].clone();
    key(&mut h, Key::E);
    h.get_by_label("Masking (K)").click();
    h.run_steps(2);
    let r = inspect(&h);
    assert_eq!((r["selectedMask"].clone(), r["maskDraw"].as_str()), (radial, Some("radial")));
    // nothing selected: the rail's button arms the Brush as K does
    ui(&mut h, json!({"local": {"selectMask": null}}));
    key(&mut h, Key::E);
    h.get_by_label("Masking (K)").click();
    h.run_steps(2);
    assert_eq!(inspect(&h)["brush"]["armed"], true);
    assert_eq!(masks(&h).len(), 2, "arming made no mask");
    // the control channel's stroke makes a mask as a drag does
    let r = ui(&mut h, json!({"local": {"brush": {"points": [[0.6, 0.2], [0.7, 0.25]]}}}));
    let m = r["develop"]["masks"].as_array().unwrap();
    assert_eq!(m.len(), 3);
    assert_eq!(strokes(&m[2]).len(), 1);
}

use egui_kittest::kittest::Queryable;

/// Auto Mask is on for a first-time user, goes onto every stroke, toggles with A and from the
/// control channel, and is a preference: saved in the preferences file, read by the next dialog
/// and by the next session that loads that file.
#[test]
fn auto_mask_is_on_by_default_goes_onto_strokes_and_is_remembered() {
    let mut h = dialog_on(false, photocraft_engine::Session::new());
    assert!(h.state().session.prefs().raw_defaults.brush_auto_mask, "on by default");
    key(&mut h, Key::K);
    assert_eq!(inspect(&h)["brush"]["autoMask"], true);
    assert!(h.query_by_label("Auto Mask").is_some(), "the brush's checkbox");
    drag(&mut h, vec2(0.2, 0.3), vec2(0.4, 0.35), 6, false);
    assert_eq!(strokes(&masks(&h)[0])[0]["auto_mask"], true, "{:?}", masks(&h));
    // A turns it off: the next stroke paints without it, and the preference follows
    key(&mut h, Key::A);
    assert_eq!(inspect(&h)["brush"]["autoMask"], false);
    h.run_steps(2);
    assert!(!h.state().session.prefs().raw_defaults.brush_auto_mask, "saved as the preference");
    drag(&mut h, vec2(0.2, 0.6), vec2(0.4, 0.65), 6, false);
    assert_eq!(strokes(&masks(&h)[0])[1]["auto_mask"], false);
    // the control channel sets it (and the checkbox shows it)
    let r = ui(&mut h, json!({"local": {"brush": {"autoMask": true}}}));
    assert_eq!(r["brush"]["autoMask"], true);
    assert!(h.state().session.prefs().raw_defaults.brush_auto_mask);
    let ctx = h.ctx.clone();
    assert!(camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"local": {"brush": {"autoMask": 1}}}})).unwrap().is_err());
    ui(&mut h, json!({"local": {"brush": {"autoMask": false}}}));
    // the next dialog starts from the preference
    ui(&mut h, json!({"cancel": true}));
    let (name, bytes) = raw();
    h.state_mut().open_bytes(&name, &bytes).unwrap();
    settle(&mut h);
    assert_eq!(inspect(&h)["brush"]["autoMask"], false, "the next dialog remembers it");
    // and the next session, from the saved preferences
    let saved = h.state().session.prefs_to_json();
    assert!(saved.contains("\"brushAutoMask\": false") || saved.contains("\"brushAutoMask\":false"), "in the preferences file");
    let mut next = photocraft_engine::Session::new();
    next.load_prefs_json(&saved).unwrap();
    assert!(!next.prefs().raw_defaults.brush_auto_mask);
    let mut h = dialog_on(false, next);
    assert_eq!(inspect(&h)["brush"]["autoMask"], false, "after a restart");
    // a preference changed elsewhere reaches the open dialog
    h.state_mut().run("prefs.set", json!({"path": "rawDefaults.brushAutoMask", "value": true})).unwrap();
    h.run_steps(2);
    assert_eq!(inspect(&h)["brush"]["autoMask"], true);
}

/// An Auto Mask stroke along an edge, its overlay as the preview shows it while the button is
/// still held: the edge-aware result during the drag, the same as once the stroke is let go.
#[test]
fn an_auto_mask_strokes_overlay_shows_while_it_is_painted() {
    let snapshots = std::env::var("PHOTOCRAFT_LC_SNAPSHOTS").ok();
    let _lock = snapshots.is_some().then(photocraft_testkit::gpu_lock);
    let mut h = dialog_on(snapshots.is_some(), photocraft_engine::Session::new());
    key(&mut h, Key::K);
    // down the left side of the post in the middle of the frame (the synthetic raw has an edge
    // there too), the brush's rim reaching over it
    drag(&mut h, vec2(0.512, 0.36), vec2(0.512, 0.8), 24, true);
    let r = inspect(&h);
    let id = r["selectedMask"].clone();
    assert!(!id.is_null(), "the stroke made its mask while the button is held");
    assert_eq!(r["maskOverlayShown"], id, "the overlay shows the stroke during the drag");
    let during = strokes(&masks(&h)[0]);
    assert!(during[0]["points"].as_array().is_some_and(|p| p.len() > 10), "{during:?}");
    if let Some(dir) = &snapshots {
        std::fs::create_dir_all(dir).unwrap();
        h.render().unwrap().save(std::path::Path::new(dir).join("automask-live.png")).unwrap();
    }
    let r = preview_rect(&h);
    button(&mut h, r.lerp_inside(vec2(0.512, 0.8)), false);
    settle(&mut h);
    assert_eq!(strokes(&masks(&h)[0]).len(), 1);
}

/// The same Auto Mask stroke over a dark, textured surface on process 8 (each pixel compared with the
/// one under the dab) and process 9 (compared on a smoothing of the photo): process 9's fill is
/// solid where 8's follows the texture. `PHOTOCRAFT_LC_SNAPSHOTS` saves both overlays
/// (`automask-quality-before.png`, `automask-quality-after.png`).
#[test]
fn auto_mask_quality_by_process() {
    let snapshots = std::env::var("PHOTOCRAFT_LC_SNAPSHOTS").ok();
    let _lock = snapshots.is_some().then(photocraft_testkit::gpu_lock);
    let mut h = dialog_on(snapshots.is_some(), photocraft_engine::Session::new());
    assert_eq!(inspect(&h)["develop"]["process"], 9, "new edits take process 9");
    ui(&mut h, json!({"develop": {"process": 8}}));
    key(&mut h, Key::K);
    // a zigzag over the shadow on the perforated panel, lower left (dark, so noisy, and pitted)
    let r = preview_rect(&h);
    let pts: Vec<egui::Vec2> = (0..4).flat_map(|row| [vec2(0.12, 0.72 + 0.04 * row as f32), vec2(0.42, 0.72 + 0.04 * row as f32)]).collect();
    h.event(Event::PointerMoved(r.lerp_inside(pts[0])));
    h.run_steps(1);
    button(&mut h, r.lerp_inside(pts[0]), true);
    h.run_steps(1);
    for w in pts.windows(2) {
        for k in 1..=6 {
            h.event(Event::PointerMoved(r.lerp_inside(w[0] + (w[1] - w[0]) * (k as f32 / 6.0))));
            h.run_steps(1);
        }
    }
    button(&mut h, r.lerp_inside(pts[pts.len() - 1]), false);
    settle(&mut h);
    let painted = strokes(&masks(&h)[0]);
    assert_eq!(painted.len(), 1);
    assert_eq!(painted[0]["auto_mask"], true);
    let save = |h: &mut Harness<'_, PhotocraftApp>, name: &str| {
        if let Some(dir) = &snapshots {
            std::fs::create_dir_all(dir).unwrap();
            h.render().unwrap().save(std::path::Path::new(dir).join(name)).unwrap();
        }
    };
    save(&mut h, "automask-quality-before.png");
    ui(&mut h, json!({"develop": {"process": 9}}));
    assert_eq!(strokes(&masks(&h)[0]), painted, "the same stroke");
    save(&mut h, "automask-quality-after.png");
}
